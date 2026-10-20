# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Helm-template pins for the SR-3.5 memory-safe stream cap surface.

Epic 7 promoted the memory-safe stream cap to THE hard refusal point on the
non-durable path, but its three inputs are env-var-only in the code:
`OMNI_EVENTS_BROKER_STREAM_MEMORY_BUDGET_MIB`,
`OMNI_EVENTS_MAX_NON_DURABLE_STREAMS_PER_POD`, and the non-durable prefetch
`OMNI_EVENTS_RABBITMQ_CHANNEL_PREFETCH_COUNT`. This module pins the chart
surface that templates them.

Three silent hazards this file exists for:
1. Absence. pydantic-settings raises nothing for an unset OMNI_EVENTS_ var, so a
   key that never renders leaves the setting stuck at its code default with no
   operator knob and no error.
2. Wrong name. pydantic-settings also silently ignores an UNKNOWN OMNI_EVENTS_
   var, so a misspelled key renders happily and does nothing. The rename guards
   below (and the whole-ConfigMap invariant) turn that into a CI failure.
3. `max_non_durable_streams_per_pod` is `int | None` with unset meaning "compute
   from the budget". An unconditionally rendered empty-string value would fail
   the pydantic int parse and crash-loop the pod. The chart must OMIT the key
   when null and SURFACE 0 -- exactly the contract already pinned for the shed
   threshold at test_settings_configmap_template.py.
"""

import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

from app.config.settings import Settings
from notification_common.config.auth_settings import AuthSettings
from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"

SETTINGS_CONFIGMAP_TEMPLATE = "templates/settings-configmap.yaml"

ENV_BUDGET = "OMNI_EVENTS_BROKER_STREAM_MEMORY_BUDGET_MIB"
ENV_CAP_OVERRIDE = "OMNI_EVENTS_MAX_NON_DURABLE_STREAMS_PER_POD"
ENV_PREFETCH = "OMNI_EVENTS_RABBITMQ_CHANNEL_PREFETCH_COUNT"
ENV_TTL = "OMNI_EVENTS_NON_DURABLE_QUEUE_TTL_SECONDS"
ENV_HEALTH_CHECKING = "OMNI_EVENTS_GRPC_HEALTH_CHECKING_ENABLED"

# Values keys as landed in helm/event-consumer-service/values.yaml.
VALUES_BUDGET = "rabbitmq.broker_stream_memory_budget_mib"
VALUES_CAP_OVERRIDE = "rabbitmq.max_non_durable_streams_per_pod"
VALUES_PREFETCH = "rabbitmq.channel_prefetch_count"
VALUES_TTL = "rabbitmq.non_durable_queue_ttl_seconds"

# Chart defaults as landed (values.yaml:529/543 and the inherited prefetch 15).
DEFAULT_BUDGET = "1024"
DEFAULT_PREFETCH = "15"
DEFAULT_TTL = "20"

# Override sentinels distinct from every default in play (1, 3, 5, 8, 10, 15,
# 20, 75, 80, 100, 500, 600, 1024, 1800, 8000, 50051) so a key that picked up an
# unrelated value cannot pass by coincidence.
OVERRIDE_BUDGET = "4096"
OVERRIDE_CAP = "333"
OVERRIDE_PREFETCH = "37"
OVERRIDE_TTL = "45"

# Keys read directly via os.environ rather than declared as a Settings field:
# permissions_yaml_file_path is consulted at app/config/settings.py:205 to
# locate the YAML settings source, so it never appears in Settings.model_fields
# yet is legitimately templated. This is a named, comment-justified allowlist,
# not a catch-all.
DIRECT_OS_ENVIRON_KEYS = frozenset({"permissions_yaml_file_path"})

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
    """Return the settings ConfigMap's `data` mapping from a scoped render.

    Parsed structurally rather than scanned as a substring: the template
    carries explanatory comments that mention key names, and a scan cannot
    distinguish an absent key from an empty one nor see a value's YAML type.
    """
    out = _render(*extra_set, show_only=SETTINGS_CONFIGMAP_TEMPLATE)
    docs = [d for d in yaml.safe_load_all(out) if d]
    assert len(docs) == 1, f"expected exactly one document, got {len(docs)}:\n{out}"
    assert docs[0]["kind"] == "ConfigMap", f"unexpected kind:\n{out}"
    return docs[0]["data"]


@pytest.fixture(scope="module")
def default_data() -> dict:
    """The settings ConfigMap data of a stock render. Read-only; shared."""
    return _settings_configmap_data()


class TestBrokerStreamMemoryBudget:
    """The fleet-wide broker memory budget templates unconditionally."""

    def test_key_is_present(self, default_data):
        # Absence IS the bug: no knob, no error, budget stuck at code default.
        assert ENV_BUDGET in default_data

    def test_renders_the_chart_default(self, default_data):
        assert default_data[ENV_BUDGET] == DEFAULT_BUDGET

    def test_follows_its_values_key(self):
        data = _settings_configmap_data(f"{VALUES_BUDGET}={OVERRIDE_BUDGET}")

        assert data[ENV_BUDGET] == OVERRIDE_BUDGET

    def test_renders_as_a_string(self, default_data):
        # A ConfigMap data value must be a string, else the API server rejects
        # the manifest at apply time.
        assert isinstance(default_data[ENV_BUDGET], str)


class TestNonDurableChannelPrefetch:
    """The dominant broker-memory lever templates unconditionally.

    The chart default is the post-Task-2 value (500 -> 15). Verified against
    the landed values.yaml and notification_common CommonSettings, not assumed.
    """

    def test_key_is_present(self, default_data):
        assert ENV_PREFETCH in default_data

    def test_renders_the_chart_default(self, default_data):
        assert default_data[ENV_PREFETCH] == DEFAULT_PREFETCH

    def test_follows_its_values_key(self):
        data = _settings_configmap_data(f"{VALUES_PREFETCH}={OVERRIDE_PREFETCH}")

        assert data[ENV_PREFETCH] == OVERRIDE_PREFETCH

    def test_renders_as_a_string(self, default_data):
        assert isinstance(default_data[ENV_PREFETCH], str)


class TestNonDurableQueueTtlSeconds:
    """The broker TTL for non-durable queues templates unconditionally.

    A plain int with a non-null default (20): the row is unconditional and
    renders on every branch. b.tz2 added it as a values-driven knob; without
    it the setting would be stuck at its code default with no operator knob
    and no error.
    """

    def test_key_is_present(self, default_data):
        # Absence IS the bug: no knob, no error, TTL stuck at code default.
        assert ENV_TTL in default_data

    def test_renders_the_chart_default(self, default_data):
        assert default_data[ENV_TTL] == DEFAULT_TTL

    def test_follows_its_values_key(self):
        data = _settings_configmap_data(f"{VALUES_TTL}={OVERRIDE_TTL}")

        assert data[ENV_TTL] == OVERRIDE_TTL

    def test_renders_as_a_string(self, default_data):
        # A ConfigMap data value must be a string, else the API server rejects
        # the manifest at apply time.
        assert isinstance(default_data[ENV_TTL], str)


class TestMaxNonDurableStreamsPerPodOmission:
    """The cap override is suppressed by null alone, never by a value.

    `max_non_durable_streams_per_pod` is `int | None`: unset means "compute the
    cap from the budget and prefetch". Because pydantic parses an empty-string
    env var as an invalid int rather than as None, an unconditionally rendered
    empty value would crash the pod at startup. The chart guards the key with an
    explicit nil test, so null (or omitted) is the ONLY input that omits it and
    every non-null value -- including the falsy 0 -- renders as supplied.
    """

    def test_default_render_omits_the_key_entirely(self, default_data):
        # Not "renders empty" -- absent. Asserted with `not in`, not `== ""`.
        assert ENV_CAP_OVERRIDE not in default_data

    def test_set_value_renders_the_quoted_value(self):
        data = _settings_configmap_data(f"{VALUES_CAP_OVERRIDE}={OVERRIDE_CAP}")

        assert data[ENV_CAP_OVERRIDE] == OVERRIDE_CAP

    def test_set_value_renders_as_a_yaml_string(self):
        data = _settings_configmap_data(f"{VALUES_CAP_OVERRIDE}={OVERRIDE_CAP}")

        assert isinstance(data[ENV_CAP_OVERRIDE], str)

    def test_zero_renders_rather_than_being_swallowed(self):
        # An operator-supplied 0 must SURFACE so pydantic's ge=1 rejects it and
        # the pod crash-loops loudly, never silently falling back to the
        # computed cap. A `{{- with }}` guard treats 0 as absent and would
        # swallow it -- this is the test that catches a revert to `with`, which
        # every other test in this class passes under.
        data = _settings_configmap_data(f"{VALUES_CAP_OVERRIDE}=0")

        assert data[ENV_CAP_OVERRIDE] == "0"


class TestCapKeysDoNotDisturbTheRestOfTheConfigMap:
    """The optional cap key's chomp markers must not eat a neighbour's line.

    `{{- if }}` chomp markers are the classic way to eat an adjacent line's
    newline. Render default vs render-with-cap-override, strip the newly present
    key, and assert the remaining data items are identical in value AND order.
    """

    def test_adding_the_cap_override_leaves_every_other_key_untouched(
        self, default_data
    ):
        set_data = _settings_configmap_data(f"{VALUES_CAP_OVERRIDE}={OVERRIDE_CAP}")
        set_data_without_new_key = {
            key: value for key, value in set_data.items() if key != ENV_CAP_OVERRIDE
        }

        assert list(set_data_without_new_key.items()) == list(default_data.items())


class TestCapKeyNamesMatchDeclaredSettings:
    """The rename guard for the three cap-surface keys.

    pydantic-settings silently ignores an unknown OMNI_EVENTS_ var, so a typo'd
    templated key does nothing in production. Stripping the prefix and matching
    the result against Settings.model_fields makes a typo fail in CI instead.
    `rabbitmq_channel_prefetch_count` arrives in Settings.model_fields by
    inheritance from CommonSettings.
    """

    @pytest.mark.parametrize("env", [ENV_BUDGET, ENV_CAP_OVERRIDE, ENV_PREFETCH])
    def test_name_resolves_to_a_declared_setting(self, env):
        field = env.removeprefix("OMNI_EVENTS_").lower()

        assert field in Settings.model_fields


class TestBtz2KeyNamesMatchDeclaredSettings:
    """Rename guard for the two b.tz2 keys.

    pydantic-settings silently ignores an unknown OMNI_EVENTS_ var, so a typo'd
    templated key does nothing in production. Stripping the prefix and matching
    the result against Settings.model_fields makes a typo fail in CI instead.
    `non_durable_queue_ttl_seconds` is declared on this service's Settings
    directly; `grpc_health_checking_enabled` arrives in Settings.model_fields
    by inheritance from CommonSettings (model_fields includes inherited fields).
    """

    @pytest.mark.parametrize("env", [ENV_TTL, ENV_HEALTH_CHECKING])
    def test_name_resolves_to_a_declared_setting(self, env):
        field = env.removeprefix("OMNI_EVENTS_").lower()

        assert field in Settings.model_fields


class TestEveryConfigMapKeyResolvesToADeclaredSetting:
    """Whole-ConfigMap rename invariant (SR-7.2, PM addition).

    The per-key rename guards cover only the 10 new keys; 19 pre-existing keys
    stayed unguarded, and the shipped GRPC_HEALTH_CHECKING_ENABLE misspelling
    (repaired by this Task's sweep) proves the bug class is real. This class
    enforces the invariant chart-wide: EVERY rendered key must strip to a
    declared setting field, an AuthSettings field, or the named
    os.environ-read allowlist. It is the only artifact in the set that would
    have caught the bug this Task repairs.
    """

    @staticmethod
    def _declared_names() -> frozenset:
        return (
            frozenset(Settings.model_fields)
            | frozenset(AuthSettings.model_fields)
            | DIRECT_OS_ENVIRON_KEYS
        )

    def test_every_default_render_key_resolves(self, default_data):
        declared = self._declared_names()
        unresolved = {
            key
            for key in default_data
            if key.removeprefix("OMNI_EVENTS_").lower() not in declared
        }

        assert not unresolved, f"undeclared rendered keys: {sorted(unresolved)}"

    def test_the_cap_override_key_resolves_when_present(self):
        # The one key omitted from the default render still must resolve when
        # it is emitted, so the whole surface is covered.
        declared = self._declared_names()
        data = _settings_configmap_data(f"{VALUES_CAP_OVERRIDE}={OVERRIDE_CAP}")

        assert ENV_CAP_OVERRIDE in data
        assert ENV_CAP_OVERRIDE.removeprefix("OMNI_EVENTS_").lower() in declared
