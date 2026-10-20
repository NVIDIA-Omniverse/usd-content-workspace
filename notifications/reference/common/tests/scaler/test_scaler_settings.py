# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest

from notification_common.scaler.scaler_settings import ScalerSettings


class TestScalerSettings:
    """
    Shipped defaults and field validators for the autoscaler.

    ``scaler_min_replicas`` is 3 and ``scaler_scale_down_cooldown_seconds`` is
    1800 because of the 2026-08-11 sapi-bravo incident: the fleet spent roughly
    an hour oscillating between 2 and 4 replicas against a real need of about 9
    pods, so every burst was paid for by re-climbing from a hole the previous
    scale-down had just dug. A 3-pod floor stops the fleet digging that hole,
    and a 30-minute scale-down memory stops it re-digging one inside the same
    load phase. Source of record: the service repo's
    docs/runbooks/incident-2026-08-11-sapi-bravo-channel-exhaustion.md, and SDD
    SR-5.5.

    ``scaler_min_available_capacity`` is deliberately NOT retuned alongside
    them: it is denominated in channel slots, and Epic t1.tj2.wi owns
    re-expressing and re-sizing it. A future change to THAT number is that
    Epic's intentional update.

    A failure on any of these values is expected from the Epic that owns it and
    must be an intentional test update, not silent drift.
    """

    def test_default_values(self):
        settings = ScalerSettings()

        assert settings.scaler_target_deployment == ""
        assert settings.scaler_target_namespace == "default"
        assert settings.scaler_metrics_port == 8000
        assert settings.scaler_metrics_path == "/api/v1beta/metrics/channel-pool"
        assert settings.scaler_min_available_capacity == 30
        assert settings.scaler_min_replicas == 3
        assert settings.scaler_max_replicas == 10
        assert settings.scaler_poll_interval_hot_seconds == 2.0
        assert settings.scaler_poll_interval_cold_seconds == 15.0
        assert settings.scaler_scale_up_cooldown_seconds == 10
        assert settings.scaler_scale_down_cooldown_seconds == 1800

    def test_rejects_zero_min_available_capacity(self):
        with pytest.raises(ValueError) as exc:
            ScalerSettings(scaler_min_available_capacity=0)

        assert "scaler_min_available_capacity must be at least 1" in str(exc.value)

    def test_rejects_negative_min_available_capacity(self):
        with pytest.raises(ValueError) as exc:
            ScalerSettings(scaler_min_available_capacity=-10)

        assert "scaler_min_available_capacity must be at least 1" in str(exc.value)

    @pytest.mark.parametrize(
        "field",
        [
            "scaler_scale_down_window_seconds",
            "scaler_post_scale_up_stabilization_seconds",
        ],
    )
    def test_rejects_a_negative_scale_down_suppressing_duration(self, field):
        # Both durations are operator-settable through OMNI_EVENTS_*. A
        # negative one would make the guard it sizes meaningless, and the
        # window's own cutoff would run ahead of "now"; it is refused at
        # construction rather than absorbed at the decision.
        with pytest.raises(ValueError) as exc:
            ScalerSettings(**{field: -1})

        assert f"{field} must not be negative" in str(exc.value)

    @pytest.mark.parametrize(
        "field",
        [
            "scaler_scale_down_window_seconds",
            "scaler_post_scale_up_stabilization_seconds",
        ],
    )
    def test_accepts_zero_as_the_disabling_value(self, field):
        # 0 is the documented escape hatch for each: no smoothing, and no
        # post-scale-up stabilization respectively.
        settings = ScalerSettings(**{field: 0})

        assert getattr(settings, field) == 0


class TestMinReplicasFloorValidator:
    """
    ``scaler_min_replicas`` is refused below 1, at construction.

    The rejection is about the off switch rather than about capacity. Scaling
    the deployment to zero replicas is the documented operator off switch, and
    the scaler never reverts it: it acts only on iterations that observe a
    running, reporting fleet, and a fleet at zero offers nothing to observe. A
    floor of 0 leaves the upward branch inert -- every replica count is already
    at or above it -- so nothing stands between the fleet and that
    unrecoverable state. Refusing the value at load closes the
    ``scaler_min_replicas`` route to it.

    This is a single-field bound, so it is enforced the same way
    ``scaler_min_available_capacity`` already is. The min > max relationship
    next door is deliberately NOT enforced this way -- it is reported as a
    startup WARNING instead, see
    ``TestLogStartupConfig::test_a_floor_above_the_ceiling_is_reported_once``
    in tests/scaler/test_autoscaler.py -- because a cross-field failure would
    take the scaler down rather than degrade it. The stabilization-vs-window
    pair below is a second instance of that same policy, pinned by
    ``test_stabilization_default_is_at_least_the_window_default``.
    """

    def test_rejects_a_zero_replica_floor(self):
        with pytest.raises(ValueError) as exc:
            ScalerSettings(scaler_min_replicas=0)

        assert "scaler_min_replicas must be at least 1" in str(exc.value)

    def test_rejects_a_negative_replica_floor(self):
        with pytest.raises(ValueError) as exc:
            ScalerSettings(scaler_min_replicas=-1)

        assert "scaler_min_replicas must be at least 1" in str(exc.value)

    def test_accepts_a_floor_of_one(self):
        # The boundary the validator admits: one pod is a fleet the scaler can
        # still observe and climb from, so it is a legitimate floor even though
        # the shipped default is 3.
        settings = ScalerSettings(scaler_min_replicas=1)

        assert settings.scaler_min_replicas == 1

    def test_rejects_a_zero_replica_floor_from_the_env_var(self, monkeypatch):
        # The env var is how a deployment actually sets this (the chart maps
        # onto OMNI_EVENTS_*), so the bound has to bind on that path and not
        # only on a keyword argument no operator ever passes.
        monkeypatch.setenv("OMNI_EVENTS_SCALER_MIN_REPLICAS", "0")

        with pytest.raises(ValueError) as exc:
            ScalerSettings()

        assert "scaler_min_replicas must be at least 1" in str(exc.value)


class TestRetunedAntiFlappingDefaults:
    """
    OMNI_EVENTS_* tunability for the two anti-flapping values retuned above.

    The field names are the operator-facing contract: a deployment that has
    tuned either one sets OMNI_EVENTS_<FIELD>. pydantic-settings silently drops
    an env var with no matching field -- the prefixed name is never surfaced,
    so extra='forbid' cannot catch it either -- which means a rename or a units
    change here would quietly revert every tuned deployment to the shipped
    default with no error anywhere. These tests are the pin that makes such a
    change loud.

    Each override value differs from both the pre-retune default and the
    current one, so neither test can pass by accident.
    """

    def test_env_var_override_sets_min_replicas(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_SCALER_MIN_REPLICAS", "7")

        assert ScalerSettings().scaler_min_replicas == 7

    def test_env_var_override_sets_scale_down_cooldown(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_SCALER_SCALE_DOWN_COOLDOWN_SECONDS", "2700")

        assert ScalerSettings().scaler_scale_down_cooldown_seconds == 2700


class TestScaleDownSmoothingSettings:
    """
    Shipped defaults and OMNI_EVENTS_* tunability for the two scale-down
    smoothing durations.

    ``scaler_scale_down_window_seconds`` is 300 because measured client
    behaviour makes a short window worthless: per-client stream counts collapse
    in bursts (an observed drop from 140 to 10 in under 15 seconds) and return
    inside the same load phase, so only minutes of sustained excess capacity
    are evidence that the load actually went away. 300 is also what Kubernetes
    defaults to for the same failure mode (the HPA
    behavior.scaleDown.stabilizationWindowSeconds default, and
    --horizontal-pod-autoscaler-downscale-stabilization at 5m0s).

    ``scaler_post_scale_up_stabilization_seconds`` is 600 -- deliberately twice
    the window -- so that the pod-sized step a scale-up adds to total_available
    cannot age into the trailing window and reverse the very scale-up that
    produced it.

    Both are sized-for-the-profile defaults, not measured optima: Epic
    t1.tj2.wi owns any retune. A failure here from that work is expected and
    must be an intentional test update, not silent drift.

    The field names are themselves the operator-facing contract. The service
    chart's channelPoolAutoscaling values map onto OMNI_EVENTS_<FIELD>, and
    pydantic-settings drops an unknown env var without an error, so a rename or
    a units change would silently strand every tuned deployment. The env-var
    tests below are the pin that makes that loud.
    """

    def test_scale_down_window_defaults_to_300_seconds(self):
        assert ScalerSettings().scaler_scale_down_window_seconds == 300

    def test_scale_down_window_is_a_whole_number_of_seconds(self):
        # Pinned by type, not by equality: 300 == 300.0 cannot discriminate an
        # int from a float, and the sibling poll intervals next door in the
        # same class are floats.
        assert isinstance(ScalerSettings().scaler_scale_down_window_seconds, int)

    def test_env_var_override_sets_scale_down_window(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_SCALER_SCALE_DOWN_WINDOW_SECONDS", "900")

        assert ScalerSettings().scaler_scale_down_window_seconds == 900

    def test_post_scale_up_stabilization_defaults_to_600_seconds(self):
        assert ScalerSettings().scaler_post_scale_up_stabilization_seconds == 600

    def test_post_scale_up_stabilization_is_a_whole_number_of_seconds(self):
        settings = ScalerSettings()

        assert isinstance(settings.scaler_post_scale_up_stabilization_seconds, int)

    def test_env_var_override_sets_post_scale_up_stabilization(self, monkeypatch):
        monkeypatch.setenv(
            "OMNI_EVENTS_SCALER_POST_SCALE_UP_STABILIZATION_SECONDS", "1200"
        )

        assert ScalerSettings().scaler_post_scale_up_stabilization_seconds == 1200

    def test_stabilization_default_is_at_least_the_window_default(self):
        # The stabilization window is documented as needing to sit at or above
        # the smoothing window, or the window subsumes it. That relationship is
        # deliberately not enforced by a cross-field validator (it would raise
        # at import time and take the scaler down rather than degrade it), so
        # the shipped pair is pinned here instead.
        settings = ScalerSettings()

        assert (
            settings.scaler_post_scale_up_stabilization_seconds
            >= settings.scaler_scale_down_window_seconds
        )


class TestLabelSelector:
    def test_returns_explicit_label_selector_if_set(self):
        settings = ScalerSettings(
            scaler_target_deployment="my-service",
            scaler_target_label_selector="custom=label",
        )

        assert settings.label_selector == "custom=label"

    def test_returns_default_label_selector_from_deployment_name(self):
        settings = ScalerSettings(
            scaler_target_deployment="my-service",
        )

        assert settings.label_selector == "app.kubernetes.io/name=my-service"

    def test_returns_default_label_selector_when_empty(self):
        settings = ScalerSettings(
            scaler_target_deployment="event-consumer-service",
            scaler_target_label_selector="",
        )

        assert (
            settings.label_selector == "app.kubernetes.io/name=event-consumer-service"
        )
