# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Helm-template pins for the channel wait, durable-consumer prefetch, and the
five durable-pool knobs (SDD SR-7.1).

SR-7.1 enumerates a group of settings that had no chart path: the non-durable
`channel_wait_ms`, the `durable_consumer_prefetch_count`, and the five
`rabbitmq_durable_channel_*` settings. This Task's engineer subtask templates
all seven into settings-configmap.yaml with `## @param` rows; this module pins
that surface.

These are all plain non-null-defaulted ints, so unlike the cap override they
render unconditionally on every branch and none is autoscaling-branch dependent
-- which keeps this file structurally simpler than the cap module.

Same two silent hazards apply: an un-rendered key leaves the setting stuck at
its code default with no knob and no error, and a mis-spelled key is silently
ignored by pydantic-settings. Note `rabbitmq_durable_channel_prefetch_count` is
effectively INERT (every durable consumer overrides prefetch per checkout via
set_qos); it is still templated and pinned here -- the point is that the key
reaches the pod under its correct name, not that it changes behaviour.
"""

import re
import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

from app.config.settings import Settings
from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"
VALUES_YAML = CHART_PATH / "values.yaml"

SETTINGS_CONFIGMAP_TEMPLATE = "templates/settings-configmap.yaml"

# (env var, values key, expected chart default, override sentinel). Values keys
# and defaults read from the landed values.yaml, not assumed. Every override
# sentinel is distinct from every default in play (1, 3, 5, 8, 10, 15, 20, 75,
# 80, 100, 500, 600, 1024, 1800, 8000, 50051) and from every other sentinel, so
# neither a hardcoded literal nor a cross-wired values key can pass by accident.
KNOBS = [
    ("OMNI_EVENTS_RABBITMQ_CHANNEL_WAIT_MS", "rabbitmq.channel_wait_ms", "500", "271"),
    (
        "OMNI_EVENTS_DURABLE_CONSUMER_PREFETCH_COUNT",
        "rabbitmq.durable_consumer_prefetch_count",
        "20",
        "13",
    ),
    (
        "OMNI_EVENTS_RABBITMQ_DURABLE_CHANNEL_POOL_SIZE",
        "rabbitmq.durable_channel_pool_size",
        "100",
        "41",
    ),
    (
        "OMNI_EVENTS_RABBITMQ_DURABLE_CHANNEL_POOL_MIN_SIZE",
        "rabbitmq.durable_channel_pool_min_size",
        "0",
        "29",
    ),
    (
        "OMNI_EVENTS_RABBITMQ_DURABLE_CHANNEL_PREFETCH_COUNT",
        "rabbitmq.durable_channel_prefetch_count",
        "500",
        "23",
    ),
    (
        "OMNI_EVENTS_RABBITMQ_DURABLE_CHANNEL_WAIT_MS",
        "rabbitmq.durable_channel_wait_ms",
        "500",
        "17",
    ),
    (
        "OMNI_EVENTS_RABBITMQ_DURABLE_CHANNEL_CLIENT_LIMIT",
        "rabbitmq.durable_channel_client_limit",
        "1",
        "7",
    ),
]

# (env, values_key, default, sentinel) unpacked for readability in ids.
KNOB_IDS = [env for env, _, _, _ in KNOBS]

pytestmark = pytest.mark.skipif(
    shutil.which("helm") is None,
    reason="helm CLI not available",
)


def _render(*extra_set: str, show_only: str | None = None) -> str:
    args = ["helm", "template", "ec", str(CHART_PATH), "--namespace", "storage-api"]
    if show_only is not None:
        args.extend(["--show-only", show_only])
    # Before the caller's own overrides: helm takes the last --set, so an
    # explicit policyHook setting in extra_set still wins.
    disable_policy_hook(args)
    select_tokenless_auth(args)
    for kv in extra_set:
        args.extend(["--set", kv])
    result = subprocess.run(args, capture_output=True, text=True)
    assert result.returncode == 0, (
        f"helm template failed:\nstdout:\n{result.stdout}\n" f"stderr:\n{result.stderr}"
    )
    return result.stdout


def _settings_configmap_data(*extra_set: str) -> dict:
    """Return the settings ConfigMap's `data` mapping from a scoped render."""
    out = _render(*extra_set, show_only=SETTINGS_CONFIGMAP_TEMPLATE)
    docs = [d for d in yaml.safe_load_all(out) if d]
    assert len(docs) == 1, f"expected exactly one document, got {len(docs)}:\n{out}"
    assert docs[0]["kind"] == "ConfigMap", f"unexpected kind:\n{out}"
    return docs[0]["data"]


@pytest.fixture(scope="module")
def default_data() -> dict:
    """The settings ConfigMap data of a stock render. Read-only; shared.

    A per-test `helm template` subprocess is the dominant cost of this file, so
    the single default render is shared across every value/presence test.
    """
    return _settings_configmap_data()


class TestKnobIsTemplated:
    """Each knob renders a key on the default render.

    Absence is asserted apart from value because absence IS the primary bug and
    several chart defaults equal the code default -- a value-only assertion
    would hide the regression forever.
    """

    @pytest.mark.parametrize("env, values_key, default, sentinel", KNOBS, ids=KNOB_IDS)
    def test_key_is_present_in_the_default_render(
        self, default_data, env, values_key, default, sentinel
    ):
        assert env in default_data


class TestKnobRendersItsChartDefault:
    """Each knob renders its chart default on a stock render.

    `durable_channel_pool_min_size` expects "0"; that is the one that catches a
    `{{- with }}` or `| default` guard swallowing a falsy value.
    """

    @pytest.mark.parametrize("env, values_key, default, sentinel", KNOBS, ids=KNOB_IDS)
    def test_default_matches(self, default_data, env, values_key, default, sentinel):
        assert default_data[env] == default


class TestKnobFollowsItsValuesKey:
    """Rendering with `--set <values key>=<sentinel>` yields the sentinel.

    Proves the row reads its values key rather than a literal that merely
    matches the default.
    """

    @pytest.mark.parametrize("env, values_key, default, sentinel", KNOBS, ids=KNOB_IDS)
    def test_follows_values_key(self, env, values_key, default, sentinel):
        data = _settings_configmap_data(f"{values_key}={sentinel}")

        assert data[env] == sentinel


class TestKnobRendersAsAString:
    """Each ConfigMap data value must be a string, else the API server rejects
    the manifest at apply time."""

    @pytest.mark.parametrize("env, values_key, default, sentinel", KNOBS, ids=KNOB_IDS)
    def test_value_is_a_string(self, default_data, env, values_key, default, sentinel):
        assert isinstance(default_data[env], str)


class TestKnobNamesMatchDeclaredSettings:
    """Rename guard: each key strips to a declared Settings field.

    pydantic-settings silently ignores an unknown OMNI_EVENTS_ var, so a typo'd
    key does nothing in production. The five durable names and
    `rabbitmq_channel_wait_ms` arrive in Settings.model_fields by inheritance
    from CommonSettings; `durable_consumer_prefetch_count` is declared on
    Settings directly.
    """

    @pytest.mark.parametrize("env, values_key, default, sentinel", KNOBS, ids=KNOB_IDS)
    def test_name_resolves(self, env, values_key, default, sentinel):
        field = env.removeprefix("OMNI_EVENTS_").lower()

        assert field in Settings.model_fields


class TestEachKnobIsEmittedExactlyOnce:
    """Each env var appears on exactly one line of the scoped render.

    yaml.safe_load_all silently collapses duplicate mapping keys to the last
    occurrence, so every structural test above would keep passing if a
    copy-paste left two rows for the same key; only a raw-text count sees it.
    The template's explanatory comments are deliberately worded without the
    env-var names, so a bare-name match counts data rows only.
    """

    @pytest.mark.parametrize("env, values_key, default, sentinel", KNOBS, ids=KNOB_IDS)
    def test_emitted_once(self, env, values_key, default, sentinel):
        out = _render(show_only=SETTINGS_CONFIGMAP_TEMPLATE)
        occurrences = sum(1 for line in out.splitlines() if env in line)

        assert occurrences == 1


# ---------------------------------------------------------------------------
# SR-7.1 `## @param` documentation pin (PM addition). No helm CLI needed: parse
# values.yaml as text and assert every rabbitmq.* values key under the
# `rabbitmq:` block is documented by a `## @param rabbitmq.<key>` row. The claim
# is "a documented row exists", not a format lint, so it is forgiving of blank
# and comment lines between the row and the key.
# ---------------------------------------------------------------------------


def _rabbitmq_block_lines() -> list[str]:
    lines = VALUES_YAML.read_text().splitlines()
    start = next(i for i, ln in enumerate(lines) if ln.rstrip() == "rabbitmq:")
    block: list[str] = []
    first = start + 1
    for ln in lines[first:]:
        # The block ends at the next TOP-LEVEL mapping key (column 0, not a
        # comment). Column-0 comment lines that trail the block are harmless --
        # they carry no rabbitmq keys.
        if ln and not ln[0].isspace() and not ln.lstrip().startswith("#"):
            break
        block.append(ln)
    return block


_RABBITMQ_BLOCK = _rabbitmq_block_lines()
# Keys are 2-space-indented `name:` rows; @param rows carry the documented name.
_RABBITMQ_KEYS = [
    m.group(1)
    for m in (re.match(r"^  ([a-z][a-z0-9_]*):", ln) for ln in _RABBITMQ_BLOCK)
    if m
]
_DOCUMENTED_PARAMS = {
    m.group(1)
    for m in (
        re.search(r"##\s*@param\s+rabbitmq\.([a-z][a-z0-9_]*)", ln)
        for ln in _RABBITMQ_BLOCK
    )
    if m
}


class TestEveryRabbitmqValuesKeyIsDocumented:
    """Every rabbitmq.* values key carries a `## @param rabbitmq.<key>` row.

    SR-7.1's "documents each with the `## @param` convention" is the input to
    the AC-9 doc contract Task 5 consumes; nothing else pins it.
    """

    def test_the_block_was_actually_parsed(self):
        # Guard against a silent empty parse making the loop below vacuous.
        assert len(_RABBITMQ_KEYS) >= len(KNOBS)

    @pytest.mark.parametrize("key", _RABBITMQ_KEYS)
    def test_key_has_a_param_row(self, key):
        assert key in _DOCUMENTED_PARAMS
