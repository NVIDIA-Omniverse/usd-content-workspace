# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from pydantic import ValidationInfo, field_validator

from notification_common.config.notification_base_settings import (
    NotificationBaseSettings,
)


class ScalerSettings(NotificationBaseSettings):
    """
    Settings for the autoscaler.

    All environment variables are prefixed with "OMNI_EVENTS_".
    """

    # Target deployment configuration
    scaler_target_deployment: str = ""
    scaler_target_namespace: str = "default"
    scaler_target_label_selector: str = ""

    # Metrics endpoint configuration
    scaler_metrics_port: int = 8000
    scaler_metrics_path: str = "/api/v1beta/metrics/channel-pool"
    scaler_metrics_timeout_seconds: float = 5.0

    # Scaling threshold - minimum available client connection slots across all pods.
    # Scale up when total_available < this value.
    # Scale down when total_available >= this value + one pod's capacity.
    scaler_min_available_capacity: int = 30

    # noinspection PyNestedDecorators
    @field_validator("scaler_min_available_capacity")
    @classmethod
    def validate_min_available_capacity(cls, v: int) -> int:
        """Validate that the minimum available capacity is at least 1."""
        if v < 1:
            raise ValueError("scaler_min_available_capacity must be at least 1")
        return v

    # Replica limits
    # The floor is 3 rather than 1 because of the 2026-08-11 incident profile:
    # violently bursty per-client fan-out across many concurrent clients. A
    # fleet that has dug below 3 has to re-climb from cold the moment the next
    # burst lands, and the flapping band measured during that incident was 2-4
    # replicas. Holding three pods is cheap insurance against paying that climb
    # on every burst.
    #
    # scaler_min_replicas is a floor in both directions, not merely a lower
    # bound on scale-down. The scale-down clamp refuses to shrink below it, and
    # a fleet found below it -- a fresh Deployment Kubernetes created at 1
    # replica, or one hand-scaled under the minimum -- is raised toward it one
    # replica per poll, whatever the load. That climb is subject to the same
    # gates as any other scale-up (no pod still starting, the scale-up
    # cooldown, and a cap at scaler_max_replicas), and it happens only on
    # iterations that observe a running, reporting fleet, because the absence
    # of an observation is not an observation.
    #
    # With scaler_min_replicas above scaler_max_replicas, upward enforcement is
    # capped at scaler_max_replicas and a WARNING at startup names the
    # misconfiguration. That pair is deliberately not rejected by a cross-field
    # validator, for the reason recorded further down in this file. The line
    # runs between the two kinds of bound: a single-field bound, as
    # validate_min_available_capacity already is, is rejected at load because
    # there is no degraded behavior worth running with an out-of-range single
    # value, whereas a cross-field relationship does have one, so a stale pair
    # degrades the scaler instead of taking it down.
    #
    # A floor of 0 is rejected at load, and the rejection is about the off
    # switch rather than about capacity. Scaling the deployment to zero
    # replicas is the documented operator off switch, and the scaler
    # deliberately never reverts it: it acts only on iterations that observe a
    # running, reporting fleet, because the absence of an observation is not an
    # observation. A fleet at zero replicas offers nothing to observe, so it is
    # unrecoverable by the scaler and stays down until an operator scales it
    # back up. The claim holds against the effective floor the upward branch
    # actually enforces, min(scaler_min_replicas, scaler_max_replicas): at an
    # effective floor of 0 that branch is inert -- every replica count is
    # already at or above the floor -- and nothing then stands between the
    # fleet and that unrecoverable state. This validator closes the
    # scaler_min_replicas route to such a floor. The scaler_max_replicas route
    # stays open: a maximum at or below zero is not rejected at load, and is
    # surfaced instead by the startup min > max WARNING. The scale-down clamp
    # reads the raw scaler_min_replicas and so refuses to shrink the fleet
    # below the configured minimum, which leaves it holding wherever it already
    # sits within [scaler_max_replicas, scaler_min_replicas].
    #
    # The scale-down clamp on its own cannot drive a steady one-pod fleet to
    # zero even with the floor at 0: scale-down requires the windowed available
    # slots to reach scaler_min_available_capacity + pod_capacity, and a single
    # reporter's available slots never exceed its own capacity, so the
    # comparison could only pass with scaler_min_available_capacity at 0 or
    # below, which that setting's own validator forbids. It becomes reachable
    # only where spec.replicas and the reporting pods disagree -- an external
    # hand-scale, an orphaned pod -- and two reporters' summed availability
    # clears a single pod's deadband while spec.replicas already reads 1. The
    # constraint therefore guards the floor's semantics and any future route to
    # zero, not a drain the clamp performs in steady state.
    scaler_min_replicas: int = 3
    scaler_max_replicas: int = 10

    # noinspection PyNestedDecorators
    @field_validator("scaler_min_replicas")
    @classmethod
    def validate_min_replicas(cls, v: int) -> int:
        """Validate that the minimum replica floor is at least 1."""
        if v < 1:
            raise ValueError("scaler_min_replicas must be at least 1")
        return v

    # Polling configuration
    scaler_poll_interval_hot_seconds: float = 2.0
    scaler_poll_interval_cold_seconds: float = 15.0

    # Cooldown configuration
    # Only the scale-DOWN side is slow. Scale-UP deliberately stays at 10
    # seconds so an under-capacity fleet still recovers fast. 1800 gives
    # scale-down a 30-minute memory, sized for the 2026-08-11 incident profile
    # (violently bursty per-client fan-out across many concurrent clients),
    # whose fleet spent roughly an hour oscillating between 2 and 4 replicas as
    # the load cliffed and returned.
    scaler_scale_up_cooldown_seconds: int = 10
    scaler_scale_down_cooldown_seconds: int = 1800

    # Trailing window over which the scale-down decision takes the MINIMUM
    # observed available capacity, so that only sustained excess capacity can
    # give a pod back. Measured client behaviour is why the default is minutes
    # rather than seconds: per-client stream counts collapse in bursts (an
    # observed drop from 140 to 10 in under 15 seconds) and then return within
    # the same load phase, so a few seconds of apparent excess capacity is not
    # evidence that the load has gone away. 300 seconds demands five sustained
    # minutes instead, and matches the value Kubernetes itself defaults to for
    # the same failure mode (kube-controller-manager's
    # --horizontal-pod-autoscaler-downscale-stabilization is 5m0s, and the HPA
    # behavior.scaleDown.stabilizationWindowSeconds default is 300).
    # 0 disables smoothing entirely, leaving the decision on the instantaneous
    # reading; it exists as an operator escape hatch.
    scaler_scale_down_window_seconds: int = 300

    # How long after a scale-up actually executes that scale-down stays
    # suppressed. This is not the same guard as the scale-down cooldown, which
    # keys off the last scale-DOWN and so does not bind at all when a fleet
    # enters a burst after a quiet period -- precisely the entry condition of
    # the flap this exists to stop. Nor is it the same as the trailing window
    # above: a scale-up steps total_available up by a full pod's capacity, and
    # once the burst evaporates that step simply ages into the window and
    # permits a scale-down exactly one window later.
    #
    # The default is 2x the smoothing window so that pod-step jump cannot age
    # in and reverse the very scale-up that produced it. The fleet has just
    # demonstrated it needed the capacity, and holding one extra pod for ten
    # minutes costs trivially less than re-entering the flap that cost roughly
    # 78 minutes of availability. This is a sized-for-profile default, not a
    # measured optimum; the stream-unit work may retune it.
    #
    # Keep this at or above scaler_scale_down_window_seconds, or the smoothing
    # window largely subsumes it. That relationship is deliberately not
    # enforced by a cross-field validator: a cross-field failure would raise at
    # import time and crash the scaler rather than degrade it.
    # 0 disables the stabilization window entirely.
    scaler_post_scale_up_stabilization_seconds: int = 600

    # noinspection PyNestedDecorators
    @field_validator(
        "scaler_scale_down_window_seconds",
        "scaler_post_scale_up_stabilization_seconds",
    )
    @classmethod
    def validate_non_negative_seconds(cls, v: int, info: ValidationInfo) -> int:
        """Validate that a scale-down-suppressing duration is not negative."""
        if v < 0:
            raise ValueError(f"{info.field_name} must not be negative")
        return v

    # Logging level for the scaler
    scaler_log_level: str = "INFO"

    @property
    def label_selector(self) -> str:
        """
        Returns the label selector for querying pods.
        If not explicitly set, defaults to app.kubernetes.io/name=<deployment>.
        """
        if self.scaler_target_label_selector:
            return self.scaler_target_label_selector
        return f"app.kubernetes.io/name={self.scaler_target_deployment}"


scaler_settings = ScalerSettings()
