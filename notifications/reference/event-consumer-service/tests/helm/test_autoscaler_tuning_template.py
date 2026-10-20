# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

# Regression pin for the autoscaler pod's configuration surface at
# templates/autoscaler-deployment.yaml:36-70. That pod is configured
# ENTIRELY by the inline env list on this Deployment -- it has no `envFrom`
# and never reads the service settings ConfigMap -- so this template is the
# ONLY way an operator can reach a scaler tunable in Kubernetes. That gives
# every tunable two independent failure modes, both of them silent:
#
# 1. Chart-value drift. values.yaml renders its own number into the env
#    unconditionally, so a deployed cluster keeps the chart's value no
#    matter what the code default becomes. Retuning the settings default
#    alone is inert in Kubernetes until the chart value follows.
# 2. A never-templated setting. If no channelPoolAutoscaling.* key is wired
#    into the env, the setting is unreachable from helm -- and an env var
#    that is simply never set produces no error from pydantic-settings, just
#    the code default. Nothing fails; the knob just does not exist.
#
# Both retuned defaults and both new smoothing tunables come from the
# 2026-08-11 flapping incident (see SDD SR-3.x / the anti-flapping section);
# these tests pin the wiring so neither failure mode can land unnoticed.
#
# Renders are scoped with --show-only templates/autoscaler-deployment.yaml:
# the whole-chart render also contains the SERVICE Deployment, whose env
# list is a different surface owned by different tests.

import shutil
import subprocess
from pathlib import Path

import pytest
import yaml
from notification_common.scaler.scaler_settings import ScalerSettings
from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"

AUTOSCALER_TEMPLATE = "templates/autoscaler-deployment.yaml"

ENV_MIN_REPLICAS = "OMNI_EVENTS_SCALER_MIN_REPLICAS"
ENV_SCALE_DOWN_COOLDOWN = "OMNI_EVENTS_SCALER_SCALE_DOWN_COOLDOWN_SECONDS"
ENV_SCALE_DOWN_WINDOW = "OMNI_EVENTS_SCALER_SCALE_DOWN_WINDOW_SECONDS"
ENV_POST_SCALE_UP_STABILIZATION = (
    "OMNI_EVENTS_SCALER_POST_SCALE_UP_STABILIZATION_SECONDS"
)
ENV_MIN_AVAILABLE_CAPACITY = "OMNI_EVENTS_SCALER_MIN_AVAILABLE_CAPACITY"

VALUES_SCALE_DOWN_WINDOW = "channelPoolAutoscaling.scaleDownWindowSeconds"
VALUES_POST_SCALE_UP_STABILIZATION = (
    "channelPoolAutoscaling.postScaleUpStabilizationSeconds"
)
VALUES_MIN_AVAILABLE_CAPACITY = "channelPoolAutoscaling.minAvailableCapacity"

# Expected values are read from the code defaults in
# notification_common/notification_common/scaler/scaler_settings.py, the
# source of truth for all four fields; the cross-check became valid once the
# pin bump to notification-common 6.5.0 made the installed library report
# these defaults regardless of how the suite is invoked.
EXPECTED_MIN_REPLICAS = str(ScalerSettings.model_fields["scaler_min_replicas"].default)
EXPECTED_SCALE_DOWN_COOLDOWN = str(
    ScalerSettings.model_fields["scaler_scale_down_cooldown_seconds"].default
)
EXPECTED_SCALE_DOWN_WINDOW = str(
    ScalerSettings.model_fields["scaler_scale_down_window_seconds"].default
)
EXPECTED_POST_SCALE_UP_STABILIZATION = str(
    ScalerSettings.model_fields["scaler_post_scale_up_stabilization_seconds"].default
)

# minAvailableCapacity is pinned as a LITERAL, not a
# ScalerSettings.model_fields[...].default cross-check: Task 1 retuned this via
# the helm value ONLY, so the library default (scaler_min_available_capacity=30)
# is deliberately left divergent and is never the shipped number -- a
# model_fields cross-check here would assert 30 and be wrong.
EXPECTED_MIN_AVAILABLE_CAPACITY = "800"

# Differs from every default in play (3, 1800, the two smoothing defaults, 800,
# and the other scaler numbers), so a template that ignored its values key could
# not render it; in particular it does not collide with 800.
OVERRIDE_SECONDS = "42"

# The five keys this file owns. maxReplicas stays deliberately absent: its
# number belongs to a different Task.
OWNED_ENV_KEYS = [
    ENV_MIN_REPLICAS,
    ENV_SCALE_DOWN_COOLDOWN,
    ENV_SCALE_DOWN_WINDOW,
    ENV_POST_SCALE_UP_STABILIZATION,
    ENV_MIN_AVAILABLE_CAPACITY,
]

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


def _autoscaler_env(*extra_set: str) -> dict[str, str]:
    """Flatten the autoscaler container's env list into {name: value}.

    Parsed structurally rather than scanned as a substring: a scan cannot
    tell OMNI_EVENTS_SCALER_SCALE_DOWN_COOLDOWN_SECONDS apart from a future
    OMNI_EVENTS_SCALER_SCALE_DOWN_COOLDOWN_SECONDS_MAX, and it cannot see
    the YAML type of the rendered value. Entries sourced with `valueFrom`
    (TARGET_NAMESPACE at :43-46) carry no `value` key at all, so they are
    skipped rather than raising KeyError.
    """
    out = _render(*extra_set, show_only=AUTOSCALER_TEMPLATE)
    docs = [d for d in yaml.safe_load_all(out) if d]
    assert len(docs) == 1, f"expected exactly one document, got {len(docs)}:\n{out}"
    assert docs[0]["kind"] == "Deployment", f"unexpected kind:\n{out}"

    container = docs[0]["spec"]["template"]["spec"]["containers"][0]
    return {e["name"]: e["value"] for e in container["env"] if "value" in e}


@pytest.fixture(scope="module")
def default_env() -> dict[str, str]:
    """The env of a stock render. Read-only; shared to avoid re-rendering."""
    return _autoscaler_env()


class TestRetunedAntiFlappingDefaults:
    """The chart must ship the retuned anti-flapping floor and cooldown.

    During the 2026-08-11 incident the fleet oscillated in the 2-4 replica
    band for roughly an hour against a real need of about 9 pods. The floor
    moved from 1 to 3 to keep the fleet out of that band, and the
    scale-down cooldown from 300s to 1800s so a single quiet trough cannot
    unwind a burst. Because the autoscaler pod takes no ConfigMap, the code
    default alone changes nothing in a cluster -- the chart value is what
    ships. Any further retune of these numbers belongs to Epic t1.tj2.wi,
    which owns re-deriving them; the expected side follows the code default
    automatically, so such a retune must move the chart value to match.
    """

    def test_min_replicas_renders_the_retuned_floor(self, default_env):
        assert default_env[ENV_MIN_REPLICAS] == EXPECTED_MIN_REPLICAS

    def test_scale_down_cooldown_renders_the_retuned_cooldown(self, default_env):
        assert default_env[ENV_SCALE_DOWN_COOLDOWN] == EXPECTED_SCALE_DOWN_COOLDOWN


class TestRetunedMinAvailableCapacity:
    """The chart must ship the retuned minAvailableCapacity floor (800).

    Epic 6 Task 1 retuned channelPoolAutoscaling.minAvailableCapacity from 30
    to 800 stream units via the helm value ONLY -- the notification_common
    library default (scaler_min_available_capacity=30) is deliberately left in
    place as a never-hit fallback. Because the autoscaler pod takes no
    ConfigMap and never reads the settings default, the chart value is the only
    number that ships; this pins that it renders the retuned 800 rather than
    silently regressing to the divergent library default.
    """

    def test_min_available_capacity_renders_the_retuned_floor(self, default_env):
        assert (
            default_env[ENV_MIN_AVAILABLE_CAPACITY] == EXPECTED_MIN_AVAILABLE_CAPACITY
        )


class TestSmoothingTunablesAreTemplated:
    """The two new smoothing tunables must be reachable from helm.

    scale_down_window_seconds and post_scale_up_stabilization_seconds are
    new settings; a setting that exists in code but is never templated is
    silently stuck at its code default, with no way for an operator to
    change it and no error anywhere to say so. Presence is asserted apart
    from value because absence IS the bug, and an override render proves
    the template reads the values key rather than a hardcoded number.
    """

    @pytest.mark.parametrize(
        "env_key",
        [ENV_SCALE_DOWN_WINDOW, ENV_POST_SCALE_UP_STABILIZATION],
    )
    def test_tunable_is_present_in_the_env(self, default_env, env_key):
        assert env_key in default_env

    @pytest.mark.parametrize(
        "env_key, expected",
        [
            (ENV_SCALE_DOWN_WINDOW, EXPECTED_SCALE_DOWN_WINDOW),
            (
                ENV_POST_SCALE_UP_STABILIZATION,
                EXPECTED_POST_SCALE_UP_STABILIZATION,
            ),
        ],
    )
    def test_tunable_renders_its_default(self, default_env, env_key, expected):
        assert default_env[env_key] == expected

    @pytest.mark.parametrize(
        "env_key, values_path",
        [
            (ENV_SCALE_DOWN_WINDOW, VALUES_SCALE_DOWN_WINDOW),
            (
                ENV_POST_SCALE_UP_STABILIZATION,
                VALUES_POST_SCALE_UP_STABILIZATION,
            ),
            (ENV_MIN_AVAILABLE_CAPACITY, VALUES_MIN_AVAILABLE_CAPACITY),
        ],
    )
    def test_tunable_follows_its_values_key(self, env_key, values_path):
        env = _autoscaler_env(f"{values_path}={OVERRIDE_SECONDS}")

        assert env[env_key] == OVERRIDE_SECONDS


class TestEnvValuesAreStrings:
    """Kubernetes requires env[].value to be a string.

    If a wiring lands unquoted (`value: {{ ... }}` instead of
    `value: "{{ ... }}"`), helm template still emits it happily and YAML
    parses the number as an int -- the manifest is rejected only at apply
    time, in the cluster, which is far too late to be useful.
    """

    @pytest.mark.parametrize("env_key", OWNED_ENV_KEYS)
    def test_value_renders_as_a_yaml_string(self, default_env, env_key):
        assert isinstance(default_env[env_key], str)
