# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import time

from opentelemetry import metrics
from opentelemetry.metrics import Counter, Histogram, UpDownCounter

from notification_common.config.common_settings import common_settings
from notification_common.config.metadata import project_name
from app.rest.models.connection_metadata import ConnectionMetadata


class SseMetrics:
    """
    Metrics collection for SSE streaming endpoints.
    """

    def __init__(self):
        if not common_settings.metrics_enabled:
            return

        self.meter = metrics.get_meter_provider().get_meter(project_name())

        self.active_connections: UpDownCounter = self.meter.create_up_down_counter(
            "sse.server.active_connections",
            unit="{connection}",
            description="Number of active SSE connections",
        )

        self.total_connections: Counter = self.meter.create_counter(
            "sse.server.connections_total",
            unit="{connection}",
            description="Total number of SSE connections established",
        )

        self.connection_duration: Histogram = self.meter.create_histogram(
            "sse.server.connection_duration",
            unit="ms",
            description="Duration of SSE connections",
        )

        self.events_sent: Counter = self.meter.create_counter(
            "sse.server.events_sent_total",
            unit="{event}",
            description="Total number of events sent via SSE",
        )

        self.events_per_connection: Histogram = self.meter.create_histogram(
            "sse.server.events_per_connection",
            unit="{event}",
            description="Number of events sent per SSE connection",
        )

        self.connection_errors: Counter = self.meter.create_counter(
            "sse.server.connection_errors_total",
            unit="{error}",
            description="Total number of SSE connection errors",
        )

    def _get_base_labels(self, metadata: ConnectionMetadata) -> dict[str, str]:
        return {
            "endpoint": metadata.endpoint,
        }

    def _get_event_stream_labels(
        self, metadata: ConnectionMetadata, event_type: str
    ) -> dict[str, str]:
        base_labels = self._get_base_labels(metadata)
        return {**base_labels, "event_type": event_type}

    def start_connection(self, metadata: ConnectionMetadata) -> None:
        """
        Record metrics for the start of an SSE connection.
        """
        if not common_settings.metrics_enabled:
            return

        labels = self._get_base_labels(metadata)
        self.active_connections.add(1, labels)
        self.total_connections.add(1, labels)

    def end_connection(
        self,
        metadata: ConnectionMetadata,
        is_error_disconnect: bool = False,
    ) -> None:
        """
        Record metrics for the end of an SSE connection.
        """
        if not common_settings.metrics_enabled:
            return

        labels = self._get_base_labels(metadata)

        self.active_connections.add(-1, labels)

        duration_ms = (time.time() - metadata.start_time) * 1000
        self.connection_duration.record(duration_ms, labels)

        self.events_per_connection.record(metadata.events_sent_count, labels)

        if is_error_disconnect:
            self.connection_errors.add(1, labels)

    def record_event_sent(self, metadata: ConnectionMetadata, event_type: str) -> None:
        """
        Record an event being streamed
        """
        metadata.events_sent_count += 1

        if not common_settings.metrics_enabled:
            return

        event_labels = self._get_event_stream_labels(metadata, event_type)
        self.events_sent.add(1, event_labels)


sse_metrics = SseMetrics()
