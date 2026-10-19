# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from opentelemetry import metrics
from opentelemetry.metrics import Counter

from notification_common.config.common_settings import common_settings
from notification_common.config.metadata import project_name


class AdmissionMetrics:
    """
    Per-class admission telemetry for the non-durable capacity guard.

    Two counters, mirroring the guard's existing admit/shed decision points:
    ``admitted`` on the success path and ``shed`` beside each refusal log line,
    labelled with the decision ``reason`` and, where the decision is
    class-aware, the stream ``class`` (``re_routable`` vs ``pinned``). Together
    they give an operator tuning the re-routable shed threshold (SDD SR-3.7) the
    per-class admitted/shed rates that previously had to be reconstructed by
    hand-counting Loki log rows.

    ``CapacityGuard.raise_if_at_capacity`` is called exactly ONCE per stream open
    (at the top of each consume RPC), never per message, so incrementing these
    O(1) counters there adds no per-message or per-event work and cannot regress
    the hot consume path.
    """

    def __init__(self):
        if not common_settings.metrics_enabled:
            return

        self.meter = metrics.get_meter_provider().get_meter(project_name())

        self._admitted: Counter = self.meter.create_counter(
            "channel_pool.admission.admitted_total",
            unit="{stream}",
            description="Non-durable consume streams admitted by the capacity guard",
        )
        self._shed: Counter = self.meter.create_counter(
            "channel_pool.admission.shed_total",
            unit="{stream}",
            description="Non-durable consume streams shed by the capacity guard",
        )

    def record_admitted(self, re_routable: bool) -> None:
        """Record one admitted non-durable stream, labelled by class."""
        if not common_settings.metrics_enabled:
            return

        stream_class = "re_routable" if re_routable else "pinned"
        self._admitted.add(1, {"class": stream_class})

    def record_shed(self, reason: str, stream_class: str = "any") -> None:
        """
        Record one shed non-durable stream.

        :param reason: The refusal decision -- ``memory_cap``, ``pool_exhausted``,
            ``shed_threshold``, or ``draining`` -- mirroring the guard's log
            fingerprints.
        :param stream_class: ``re_routable`` for the class-aware shed-threshold
            refusal; ``any`` for the class-blind refusals (memory cap, pool
            exhaustion, draining), which refuse every class.
        """
        if not common_settings.metrics_enabled:
            return

        self._shed.add(1, {"reason": reason, "class": stream_class})


admission_metrics = AdmissionMetrics()
