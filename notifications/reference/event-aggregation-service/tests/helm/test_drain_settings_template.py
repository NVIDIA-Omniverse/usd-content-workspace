# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Helm-template pins for the notification-common shutdown settings surface.

The 6.3.2 -> 6.10.1 pin bump brought two library behaviors that applied to
deployed pods with no chart knob at all -- the non-durable channel pool's
overflow band and the graceful-drain SIGTERM sequence -- plus a
`rabbitmq_channel_wait_ms` setting the chart had never exposed. The 6.10.1 ->
6.12.0 bump added a fifth, `shutdown_settle_delay_seconds`, which the library
ships at 0.0 so that the settle phase is inert until a chart templates it. The
chart now templates all five into settings-configmap.yaml, and this module pins
that ConfigMap surface per row.

The two silent hazards this guards are the usual pair: an un-rendered key leaves
the setting stuck at its library default with no knob and no error
(TestDrainKnobIsTemplated), and a mis-spelled key is silently ignored by
pydantic-settings (TestDrainKnobNamesMatchDeclaredSettings, which matches each
templated name against Settings.model_fields rather than against the same
literal the rest of the module compares the chart to).

The chart defaults render with helm's numeric normalization: the floats `5.0`
and `15.0` render as "5" and "15" (NOT "5.0" / "15.0"), and `0.5` as "0.5".

SCOPE: this covers ONLY the settings-ConfigMap rows. The deployment-level render
-- `terminationGracePeriodSeconds` and the render-time invariant guard -- is
owned by test_drain_lifecycle_template.py and is deliberately NOT asserted here
to avoid double-coverage.
"""

import pytest

from app.config.settings import Settings
from tests.helm.conftest import render, requires_helm, single_document

SETTINGS_CONFIGMAP_TEMPLATE = "templates/settings-configmap.yaml"

# The drain-window and settle sentinels below exceed the default grace period,
# and deployment.yaml's render-time invariant guard (grace >= settleDelaySeconds
# + drainWindowSeconds + a 10s teardown tail) trips on them even under a
# settings-ConfigMap-scoped --show-only render, because helm evaluates every
# template before filtering the output. Override renders therefore pin a grace
# period with headroom over the largest sentinel PLUS the other two terms, so a
# value-follows-key assertion is not masked by that guard; the guard itself is
# pinned in test_drain_lifecycle_template.py. 200 clears both of the sentinels
# that can drive the sum: 15 + 137 + 10 = 162 for the drain window, and
# 83 + 5 + 10 = 98 for the settle.
GRACE_HEADROOM_OVERRIDE = "shutdown.terminationGracePeriodSeconds=200"

# (env var, values key, expected chart default, override sentinel). Values keys
# and defaults read from the landed values.yaml, not assumed. Each numeric
# sentinel is distinct from every number the settings ConfigMap renders (1, 5,
# 25, 500, 600, 1800, 4317, 8000, 50051, 0.5, 3000, ...) and from the other
# sentinels, so neither a hardcoded literal nor a cross-wired values key can
# pass by coincidence -- note in particular that 5 is already the
# channel_pool_min_size default AND the drain-window default here, so no
# sentinel may be 5, and 15 is the settle default here so no sentinel may be 15
# either. The overflow knob is a boolean, whose only other value is
# `false`; there the default assertion carries the discrimination, since a row
# cross-wired to any of the ConfigMap's false-defaulting booleans would render
# "false" on the stock render and fail TestDrainKnobRendersItsChartDefault.
KNOBS = [
    (
        "OMNI_EVENTS_RABBITMQ_CHANNEL_OVERFLOW_ENABLED",
        "rabbitmq.channel_overflow_enabled",
        "true",
        "false",
    ),
    (
        "OMNI_EVENTS_RABBITMQ_CHANNEL_WAIT_MS",
        "rabbitmq.channel_wait_ms",
        "500",
        "3719",
    ),
    (
        "OMNI_EVENTS_SHUTDOWN_DRAIN_WINDOW_SECONDS",
        "shutdown.drainWindowSeconds",
        "5",
        "137",
    ),
    (
        "OMNI_EVENTS_SHUTDOWN_DRAIN_POLL_INTERVAL_SECONDS",
        "shutdown.drainPollIntervalSeconds",
        "0.5",
        "0.017",
    ),
    (
        "OMNI_EVENTS_SHUTDOWN_SETTLE_DELAY_SECONDS",
        "shutdown.settleDelaySeconds",
        "15",
        "83",
    ),
]

KNOB_ENV_NAMES = [env for env, _, _, _ in KNOBS]

# The env-var name identifies each parametrized row unambiguously.
KNOB_IDS = KNOB_ENV_NAMES

pytestmark = requires_helm


def _settings_configmap_data(*extra_set: str) -> dict:
    """Return the settings ConfigMap's `data` mapping from a scoped render."""
    out = render(*extra_set, show_only=SETTINGS_CONFIGMAP_TEMPLATE)
    return single_document(out, "ConfigMap")["data"]


@pytest.fixture(scope="module")
def default_render() -> str:
    """The raw stdout of a stock settings-ConfigMap-scoped render. Shared.

    A per-test `helm template` subprocess is the dominant cost of this file, so
    the single default render is shared by every test that needs it -- both the
    parsed-mapping tests, via `default_data` below, and the raw-text duplicate
    count, which needs the pre-parse text.
    """
    return render(show_only=SETTINGS_CONFIGMAP_TEMPLATE)


@pytest.fixture(scope="module")
def default_data(default_render) -> dict:
    """The settings ConfigMap data of a stock render. Read-only; shared."""
    return single_document(default_render, "ConfigMap")["data"]


class TestDrainKnobIsTemplated:
    """Each new knob renders a key on the default render.

    Absence is asserted apart from value because absence IS the primary bug --
    no knob, no error, the setting silently sticks at its library default.
    """

    @pytest.mark.parametrize("env", KNOB_ENV_NAMES)
    def test_key_is_present_in_the_default_render(self, default_data, env):
        assert env in default_data


class TestDrainKnobRendersItsChartDefault:
    """Each new knob renders its chart default on a stock render.

    The float defaults render with helm's numeric normalization: `5.0` -> "5"
    and `0.5` -> "0.5".
    """

    @pytest.mark.parametrize("env, values_key, default, sentinel", KNOBS, ids=KNOB_IDS)
    def test_default_matches(self, default_data, env, values_key, default, sentinel):
        assert default_data[env] == default


class TestDrainKnobFollowsItsValuesKey:
    """Rendering with `--set <values key>=<sentinel>` yields the sentinel.

    Proves the row reads its values key rather than a literal that merely
    matches the default.
    """

    @pytest.mark.parametrize("env, values_key, default, sentinel", KNOBS, ids=KNOB_IDS)
    def test_follows_values_key(self, env, values_key, default, sentinel):
        data = _settings_configmap_data(
            f"{values_key}={sentinel}", GRACE_HEADROOM_OVERRIDE
        )

        assert data[env] == sentinel


class TestDrainKnobNamesMatchDeclaredSettings:
    """The rename guard: each templated key names a real Settings field.

    Every other class here compares the chart against the same hardcoded env
    name from KNOBS, so a name misspelled identically in both the template and
    this module passes them all -- and pydantic-settings silently ignores an
    unknown OMNI_EVENTS_ var, so the knob would do nothing in production with no
    error. Resolving the name against Settings.model_fields instead of a literal
    is the only assertion here that a typo cannot travel through.
    """

    @pytest.mark.parametrize("env", KNOB_ENV_NAMES)
    def test_name_resolves_to_a_declared_setting(self, env):
        field = env.removeprefix("OMNI_EVENTS_").lower()

        assert field in Settings.model_fields


class TestEachDrainKnobIsEmittedExactlyOnce:
    """Each env var appears on exactly one line of the scoped render.

    yaml.safe_load_all silently collapses duplicate mapping keys to the last
    occurrence, so every structural test above would keep passing if a
    copy-paste left two rows for the same key; only a raw-text count sees it.
    The template's explanatory comments are worded without the env-var names, so
    a bare-name match counts data rows only.
    """

    @pytest.mark.parametrize("env", KNOB_ENV_NAMES)
    def test_emitted_once(self, default_render, env):
        occurrences = sum(1 for line in default_render.splitlines() if env in line)

        assert occurrences == 1
