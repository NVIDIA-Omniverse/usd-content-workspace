# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from unittest.mock import MagicMock, call

from app.lib.observability.consumer_metrics import ConsumerMetrics
from app.lib.models.event_message import EventMessage


@pytest.fixture
def mock_event_message():
    event_msg = MagicMock(spec=EventMessage)
    event_msg.event_type = "test_event_type"
    event_msg.processing_duration_ms = 123.45
    return event_msg


@pytest.fixture
def mock_events_by_type():
    return MagicMock()


@pytest.fixture
def mock_events_processed_successfully():
    return MagicMock()


@pytest.fixture
def mock_events_processing_failed():
    return MagicMock()


@pytest.fixture
def mock_event_processing_duration():
    return MagicMock()


@pytest.fixture
def mock_client_nacks():
    return MagicMock()


@pytest.fixture(autouse=True)
def setup_mock_meter(
    mock_meter,
    mock_events_by_type,
    mock_events_processed_successfully,
    mock_events_processing_failed,
    mock_event_processing_duration,
    mock_client_nacks,
):
    # Ordered to match the create_counter call sequence in
    # ConsumerMetrics.__init__; append new counters at the end.
    mock_meter.create_counter.side_effect = [
        mock_events_by_type,
        mock_events_processed_successfully,
        mock_events_processing_failed,
        mock_client_nacks,
    ]
    mock_meter.create_histogram.return_value = mock_event_processing_duration


class TestInit:

    def test_initializes_counter_metrics(self, mock_meter):
        ConsumerMetrics()

        mock_meter.create_counter.assert_has_calls(
            [
                call(
                    "events_by_type_total", description="Events processed by event type"
                ),
                call(
                    "events_processed_successfully_total",
                    description="Events successfully processed",
                ),
                call(
                    "events_processing_failed_total",
                    description="Events that failed during processing",
                ),
            ]
        )

    def test_initializes_client_nacks_counter(self, mock_meter):
        # The client-nack rate is the leading indicator of dead-lettering, so
        # the counter has to exist under its published name -- a rename here
        # silently breaks every dashboard and alert built on it.
        ConsumerMetrics()

        mock_meter.create_counter.assert_any_call(
            "client_nacks_total",
            description=(
                "Messages negatively acknowledged by a client on the durable "
                "consume-with-acks stream and requeued for redelivery"
            ),
        )

    def test_initializes_histogram_metrics(self, mock_meter):
        ConsumerMetrics()

        mock_meter.create_histogram.assert_called_once_with(
            "event_processing_duration_ms",
            description="Time spent processing each event (business logic only)",
        )

    def test_does_not_initialize_metrics_when_telemetry_disabled(
        self, telemetry_disabled, mock_meter
    ):
        ConsumerMetrics()

        mock_meter.create_counter.assert_not_called()
        mock_meter.create_histogram.assert_not_called()

    def test_does_not_initialize_metrics_when_only_tracing_enabled(
        self, tracing_only, mock_meter
    ):
        # Metrics must gate on metrics_enabled, not tracing_enabled.
        ConsumerMetrics()

        mock_meter.create_counter.assert_not_called()
        mock_meter.create_histogram.assert_not_called()


class TestRecordEventProcessingSuccess:

    def test_records_events_by_type(self, mock_event_message, mock_events_by_type):
        metrics = ConsumerMetrics()
        metrics.record_event_processing_success(mock_event_message)

        mock_events_by_type.add.assert_called_once_with(
            1, {"event_type": "test_event_type"}
        )

    def test_records_events_processed_successfully(
        self, mock_event_message, mock_events_processed_successfully
    ):
        metrics = ConsumerMetrics()
        metrics.record_event_processing_success(mock_event_message)

        mock_events_processed_successfully.add.assert_called_once_with(1)

    def test_records_event_processing_duration(
        self, mock_event_message, mock_event_processing_duration
    ):
        metrics = ConsumerMetrics()
        metrics.record_event_processing_success(mock_event_message)

        mock_event_processing_duration.record.assert_called_once_with(123.45)

    def test_does_not_record_metrics_when_telemetry_disabled(
        self,
        telemetry_disabled,
        mock_event_message,
        mock_events_by_type,
        mock_events_processed_successfully,
        mock_event_processing_duration,
    ):
        metrics = ConsumerMetrics()

        metrics.record_event_processing_success(mock_event_message)

        mock_events_by_type.add.assert_not_called()
        mock_events_processed_successfully.add.assert_not_called()
        mock_event_processing_duration.record.assert_not_called()


class TestRecordEventProcessingFailure:

    def test_records_events_by_type(self, mock_event_message, mock_events_by_type):
        metrics = ConsumerMetrics()
        metrics.record_event_processing_failure(mock_event_message)

        mock_events_by_type.add.assert_called_once_with(
            1, {"event_type": "test_event_type"}
        )

    def test_records_events_processing_failed(
        self, mock_event_message, mock_events_processing_failed
    ):
        metrics = ConsumerMetrics()
        metrics.record_event_processing_failure(mock_event_message)

        mock_events_processing_failed.add.assert_called_once_with(1)

    def test_does_not_record_metrics_when_telemetry_disabled(
        self,
        telemetry_disabled,
        mock_event_message,
        mock_events_by_type,
        mock_events_processing_failed,
    ):
        metrics = ConsumerMetrics()

        metrics.record_event_processing_failure(mock_event_message)

        mock_events_by_type.add.assert_not_called()
        mock_events_processing_failed.add.assert_not_called()


class TestRecordClientNack:

    def test_increments_client_nacks(self, mock_client_nacks):
        metrics = ConsumerMetrics()
        metrics.record_client_nack()

        mock_client_nacks.add.assert_called_once_with(1)

    def test_counts_each_nack_separately(self, mock_client_nacks):
        # One add per nack, not one per request: the servicer calls this once
        # per successfully settled tag, and a batch of nacks in a single
        # request must move the counter by the number of messages.
        metrics = ConsumerMetrics()

        metrics.record_client_nack()
        metrics.record_client_nack()

        assert mock_client_nacks.add.call_count == 2

    def test_does_not_record_when_telemetry_disabled(
        self, telemetry_disabled, mock_client_nacks
    ):
        # With metrics off, __init__ never creates the counter, so an
        # unguarded add would raise AttributeError rather than merely
        # miscount.
        metrics = ConsumerMetrics()

        metrics.record_client_nack()

        mock_client_nacks.add.assert_not_called()

    def test_does_not_record_when_only_tracing_enabled(
        self, tracing_only, mock_client_nacks
    ):
        # The nack counter gates on metrics_enabled, not tracing_enabled.
        metrics = ConsumerMetrics()

        metrics.record_client_nack()

        mock_client_nacks.add.assert_not_called()
