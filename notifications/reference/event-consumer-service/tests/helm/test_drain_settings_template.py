# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Helm-template pins for the graceful-drain shutdown settings (SR-6.1 / SR-7.1).

`CommonSettings` carries three shutdown knobs -- the endpoint-settle delay, the
drain window and the drain poll interval -- and settings-configmap.yaml
templates all three, with `## @param` rows under the `shutdown:` block. This
module pins that ConfigMap surface, following the per-row table style of
`test_channel_pool_settings_template.py`.

The settle delay is the one the application spends BEFORE the drain: on SIGTERM
it keeps serving normally for that long so an endpoint change can propagate,
then drains. It is the app-side replacement for the Deployment's former
`lifecycle.preStop.sleep` hook (Issue b.wj6), so the ConfigMap row IS the
mechanism now -- an un-rendered row does not fall back to the old behavior, it
drops the settle to the library default of 0.

All three settings are plain non-null-defaulted floats with safe code defaults,
so -- unlike `max_re_routable_streams_per_pod` -- they render UNCONDITIONALLY as
quoted strings on every branch (no nil test, no null-suppression or
ordering-preservation branch to pin). The chart defaults render with helm's
numeric normalization: the floats `15.0` and `30.0` render as "15" and "30", and
`0.5` as "0.5".

The two silent hazards this guards are the usual pair: an un-rendered key leaves
the setting stuck at its code default with no knob and no error, and a
mis-spelled key is silently ignored by pydantic-settings.

SCOPE: this covers ONLY the three settings-ConfigMap rows. The deployment-level
render -- `terminationGracePeriodSeconds`, the absence of any lifecycle hook,
and the render-time value guards -- is owned by
`test_drain_lifecycle_template.py` and is deliberately NOT asserted here to
avoid double-coverage.
"""

import re
import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"
VALUES_YAML = CHART_PATH / "values.yaml"

SETTINGS_CONFIGMAP_TEMPLATE = "templates/settings-configmap.yaml"

# (env var, values key, expected chart default, override sentinel). Values keys
# and defaults read from the landed values.yaml, not assumed. Each override
# sentinel is distinct from every number the settings ConfigMap renders (1, 3,
# 5, 8, 10, 15, 20, 30, 0.5, 60, 75, 80, 100, 500, 600, 1024, 1800, 8000,
# 50051, ...) and from the other sentinels, so neither a hardcoded literal nor a
# cross-wired values key can pass by coincidence.
KNOBS = [
    (
        "OMNI_EVENTS_SHUTDOWN_SETTLE_DELAY_SECONDS",
        "shutdown.settleDelaySeconds",
        "15",
        "83",
    ),
    (
        "OMNI_EVENTS_SHUTDOWN_DRAIN_WINDOW_SECONDS",
        "shutdown.drainWindowSeconds",
        "30",
        "137",
    ),
    (
        "OMNI_EVENTS_SHUTDOWN_DRAIN_POLL_INTERVAL_SECONDS",
        "shutdown.drainPollIntervalSeconds",
        "0.5",
        "0.017",
    ),
]

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


class TestShutdownKnobIsTemplated:
    """Each shutdown knob renders a key on the default render.

    Absence is asserted apart from value because absence IS the primary bug --
    no knob, no error, the setting silently sticks at its code default.
    """

    @pytest.mark.parametrize("env, values_key, default, sentinel", KNOBS, ids=KNOB_IDS)
    def test_key_is_present_in_the_default_render(
        self, default_data, env, values_key, default, sentinel
    ):
        assert env in default_data


class TestShutdownKnobRendersItsChartDefault:
    """Each shutdown knob renders its chart default on a stock render.

    The float defaults render with helm's numeric normalization: `15.0` -> "15",
    `30.0` -> "30" and `0.5` -> "0.5".
    """

    @pytest.mark.parametrize("env, values_key, default, sentinel", KNOBS, ids=KNOB_IDS)
    def test_default_matches(self, default_data, env, values_key, default, sentinel):
        assert default_data[env] == default


class TestShutdownKnobFollowsItsValuesKey:
    """Rendering with `--set <values key>=<sentinel>` yields the sentinel.

    Proves the row reads its values key rather than a literal that merely
    matches the default.
    """

    @pytest.mark.parametrize("env, values_key, default, sentinel", KNOBS, ids=KNOB_IDS)
    def test_follows_values_key(self, env, values_key, default, sentinel):
        # The drain-window sentinel (137) exceeds the default grace period, and
        # the render-time invariant guard in deployment.yaml (grace >=
        # settleDelaySeconds + drainWindowSeconds + the post-drain teardown
        # tail) trips on it even under this settings-ConfigMap-scoped
        # --show-only render, because helm evaluates every template before
        # filtering the output. Pin a grace period with headroom over the
        # largest sentinel so the value-follows-key assertion is not masked by
        # that guard; the guard itself is pinned in
        # test_drain_lifecycle_template.py.
        data = _settings_configmap_data(
            f"{values_key}={sentinel}",
            "shutdown.terminationGracePeriodSeconds=200",
        )

        assert data[env] == sentinel


class TestShutdownKnobRendersAsAString:
    """Each ConfigMap data value must be a string, else the API server rejects
    the manifest at apply time."""

    @pytest.mark.parametrize("env, values_key, default, sentinel", KNOBS, ids=KNOB_IDS)
    def test_value_is_a_string(self, default_data, env, values_key, default, sentinel):
        assert isinstance(default_data[env], str)


class TestEachShutdownKnobIsEmittedExactlyOnce:
    """Each env var appears on exactly one line of the scoped render.

    yaml.safe_load_all silently collapses duplicate mapping keys to the last
    occurrence, so every structural test above would keep passing if a
    copy-paste left two rows for the same key; only a raw-text count sees it.
    The template's explanatory comments are worded without the env-var names, so
    a bare-name match counts data rows only.
    """

    @pytest.mark.parametrize("env, values_key, default, sentinel", KNOBS, ids=KNOB_IDS)
    def test_emitted_once(self, env, values_key, default, sentinel):
        out = _render(show_only=SETTINGS_CONFIGMAP_TEMPLATE)
        occurrences = sum(1 for line in out.splitlines() if env in line)

        assert occurrences == 1


# ---------------------------------------------------------------------------
# SR-7.1 `## @param` documentation pin, matching the `rabbitmq:` block's pin in
# test_channel_pool_settings_template.py. No helm CLI needed: parse values.yaml
# as text and assert every key under the `shutdown:` block is documented by a
# `## @param shutdown.<key>` row. The claim is "a documented row exists", not a
# format lint, so it is forgiving of blank and comment lines between the row and
# the key.
# ---------------------------------------------------------------------------


def _shutdown_block_lines() -> list[str]:
    lines = VALUES_YAML.read_text().splitlines()
    start = next(i for i, ln in enumerate(lines) if ln.rstrip() == "shutdown:")
    block: list[str] = []
    first = start + 1
    for ln in lines[first:]:
        # The block ends at the next TOP-LEVEL mapping key (column 0, not a
        # comment). Column-0 comment lines that trail the block are harmless --
        # they carry no shutdown keys.
        if ln and not ln[0].isspace() and not ln.lstrip().startswith("#"):
            break
        block.append(ln)
    return block


_SHUTDOWN_BLOCK = _shutdown_block_lines()
# Keys are 2-space-indented camelCase `name:` rows; @param rows carry the
# documented name.
_SHUTDOWN_KEYS = [
    m.group(1)
    for m in (re.match(r"^  ([a-zA-Z][a-zA-Z0-9_]*):", ln) for ln in _SHUTDOWN_BLOCK)
    if m
]
_DOCUMENTED_PARAMS = {
    m.group(1)
    for m in (
        re.search(r"##\s*@param\s+shutdown\.([a-zA-Z][a-zA-Z0-9_]*)", ln)
        for ln in _SHUTDOWN_BLOCK
    )
    if m
}

# Removed by Issue b.wj6 in favour of `settleDelaySeconds`. deployment.yaml
# `fail`s the render outright if it is set, so a leftover KEY here would make
# the chart un-renderable and every other test in tests/helm/ would fail. A
# leftover `## @param` ROW would not -- it would just document a value that
# poisons any chart that adopts it.
REMOVED_PARAM = "preStopDelaySeconds"


class TestEveryShutdownValuesKeyIsDocumented:
    """Every shutdown.* values key carries a `## @param shutdown.<key>` row.

    The `## @param` convention is the input to the chart's generated README, so
    an undocumented knob is an invisible knob.
    """

    def test_the_block_was_actually_parsed(self):
        # Guard against a silent empty parse making the parametrization below
        # vacuous. Every KNOB key lives here, plus terminationGracePeriodSeconds.
        assert len(_SHUTDOWN_KEYS) > len(KNOBS)

    @pytest.mark.parametrize("key", _SHUTDOWN_KEYS)
    def test_key_has_a_param_row(self, key):
        assert key in _DOCUMENTED_PARAMS


class TestTheRemovedPreStopKeyIsGoneFromValues:
    """values.yaml neither defines nor documents `shutdown.preStopDelaySeconds`."""

    def test_no_key_remains(self):
        assert REMOVED_PARAM not in _SHUTDOWN_KEYS

    def test_no_param_row_remains(self):
        assert REMOVED_PARAM not in _DOCUMENTED_PARAMS
