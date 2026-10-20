# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from opentelemetry import metrics
from notification_common.config.common_settings import common_settings
from notification_common.config.metadata import project_name

from opentelemetry.metrics import Counter, Histogram
from app.lib.models.event_message import EventMessage


class ConsumerMetrics:

    def __init__(self):
        if not common_settings.metrics_enabled:
            return

        self.meter = metrics.get_meter_provider().get_meter(project_name())

        self._events_by_type: Counter = self.meter.create_counter(
            "events_by_type_total", description="Events processed by event type"
        )

        self._events_processed_successfully: Counter = self.meter.create_counter(
            "events_processed_successfully_total",
            description="Events successfully processed",
        )

        self._events_processing_failed: Counter = self.meter.create_counter(
            "events_processing_failed_total",
            description="Events that failed during processing",
        )

        self._event_processing_duration: Histogram = self.meter.create_histogram(
            "event_processing_duration_ms",
            description="Time spent processing each event (business logic only)",
        )

        self._client_nacks: Counter = self.meter.create_counter(
            "client_nacks_total",
            description=(
                "Messages negatively acknowledged by a client on the durable "
                "consume-with-acks stream and requeued for redelivery"
            ),
        )

    def record_event_processing_success(self, event_msg: EventMessage) -> None:
        if not common_settings.metrics_enabled:
            return

        self._events_by_type.add(1, {"event_type": event_msg.event_type})
        self._events_processed_successfully.add(1)
        self._event_processing_duration.record(event_msg.processing_duration_ms)

    def record_event_processing_failure(self, event_msg: EventMessage) -> None:
        if not common_settings.metrics_enabled:
            return

        self._events_by_type.add(1, {"event_type": event_msg.event_type})
        self._events_processing_failed.add(1)
        self._event_processing_duration.record(event_msg.processing_duration_ms)

    def record_client_nack(self) -> None:
        """
        Count one client-requested nack on the durable consume-with-acks
        stream. Nacks always requeue, so a sustained rate here is a
        client-side processing-failure signal and a leading indicator of
        dead-lettering: every nack advances the message's cumulative
        delivery count toward the deployment's delivery limit.
        """
        if not common_settings.metrics_enabled:
            return

        self._client_nacks.add(1)


consumer_metrics = ConsumerMetrics()
