# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from typing import Literal

from opentelemetry import metrics
from opentelemetry.metrics import Counter, Histogram

from notification_common.config.common_settings import common_settings
from notification_common.config.metadata import project_name

HydrationSource = Literal["redis", "storage_api"]


class CacheHydrationMetrics:
    """
    Metrics for the read-side hydration path in `_fetch_dir_contents`.

    Two instruments, both labelled with `source` ∈ {`redis`, `storage_api`}:
    a counter of hydration events and a histogram of wall-clock latency.
    Pairing the labels in a single `record_hydrate` call prevents callers
    from emitting label-mismatched samples between the two series.
    """

    def __init__(self):
        if not common_settings.metrics_enabled:
            return

        self.meter = metrics.get_meter_provider().get_meter(project_name())

        self.source: Counter = self.meter.create_counter(
            "cache.hydrate.source",
            unit="{hydration}",
            description="Cache hydrations by source",
        )

        self.latency: Histogram = self.meter.create_histogram(
            "cache.hydrate.latency",
            unit="ms",
            description="Latency to hydrate a directory's contents on in-memory miss",
        )

    def record_hydrate(self, source: HydrationSource, latency_ms: float) -> None:
        if not common_settings.metrics_enabled:
            return

        attrs = {"source": source}
        self.source.add(1, attrs)
        self.latency.record(latency_ms, attrs)


cache_hydration_metrics = CacheHydrationMetrics()
