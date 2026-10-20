# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import MagicMock, call
import pytest
import time

from app.lib.observability.sse_metrics import SseMetrics


@pytest.fixture
def mock_active_connections():
    return MagicMock()


@pytest.fixture
def mock_total_connections():
    return MagicMock()


@pytest.fixture
def mock_connection_duration():
    return MagicMock()


@pytest.fixture
def mock_events_sent():
    return MagicMock()


@pytest.fixture
def mock_events_per_connection():
    return MagicMock()


@pytest.fixture
def mock_connection_errors():
    return MagicMock()


@pytest.fixture
def base_labels():
    return {
        "endpoint": "/api/v1/events/stream",
    }


@pytest.fixture(autouse=True)
def setup_mock_meter(
    mock_meter,
    mock_active_connections,
    mock_total_connections,
    mock_connection_duration,
    mock_events_sent,
    mock_events_per_connection,
    mock_connection_errors,
):
    mock_meter.create_up_down_counter.return_value = mock_active_connections
    mock_meter.create_counter.side_effect = [
        mock_total_connections,
        mock_events_sent,
        mock_connection_errors,
    ]
    mock_meter.create_histogram.side_effect = [
        mock_connection_duration,
        mock_events_per_connection,
    ]


@pytest.fixture
def sse_metrics():
    return SseMetrics()


class TestInit:
    def test_initializes_up_down_counter_metrics(self, mock_meter, sse_metrics):
        mock_meter.create_up_down_counter.assert_called_once_with(
            "sse.server.active_connections",
            unit="{connection}",
            description="Number of active SSE connections",
        )

    def test_initializes_counter_metrics(self, mock_meter, sse_metrics):
        assert mock_meter.create_counter.call_count == 3
        mock_meter.create_counter.assert_has_calls(
            [
                call(
                    "sse.server.connections_total",
                    unit="{connection}",
                    description="Total number of SSE connections established",
                ),
                call(
                    "sse.server.events_sent_total",
                    unit="{event}",
                    description="Total number of events sent via SSE",
                ),
                call(
                    "sse.server.connection_errors_total",
                    unit="{error}",
                    description="Total number of SSE connection errors",
                ),
            ]
        )

    def test_initializes_histogram_metrics(self, mock_meter, sse_metrics):
        assert mock_meter.create_histogram.call_count == 2
        mock_meter.create_histogram.assert_has_calls(
            [
                call(
                    "sse.server.connection_duration",
                    unit="ms",
                    description="Duration of SSE connections",
                ),
                call(
                    "sse.server.events_per_connection",
                    unit="{event}",
                    description="Number of events sent per SSE connection",
                ),
            ]
        )

    def test_does_not_initialize_metrics_when_telemetry_disabled(
        self, telemetry_disabled, mock_meter, sse_metrics
    ):
        mock_meter.assert_not_called()

    def test_does_not_initialize_metrics_when_only_tracing_enabled(
        self, tracing_only, mock_meter, sse_metrics
    ):
        # Metrics must gate on metrics_enabled, not tracing_enabled.
        mock_meter.assert_not_called()


class TestStartConnection:

    def test_records_active_connections_metric(
        self, mock_active_connections, connection_metadata, sse_metrics, base_labels
    ):
        sse_metrics.start_connection(connection_metadata)
        mock_active_connections.add.assert_called_once_with(1, base_labels)

    def test_records_total_connections_metric(
        self, mock_total_connections, connection_metadata, sse_metrics, base_labels
    ):
        sse_metrics.start_connection(connection_metadata)
        mock_total_connections.add.assert_called_once_with(1, base_labels)

    def test_does_not_record_metrics_when_telemetry_disabled(
        self,
        telemetry_disabled,
        mock_active_connections,
        mock_total_connections,
        connection_metadata,
        sse_metrics,
    ):
        sse_metrics.start_connection(connection_metadata)

        mock_active_connections.add.assert_not_called()
        mock_total_connections.add.assert_not_called()


class TestEndConnection:

    def test_records_active_connections_decrement(
        self, mock_active_connections, connection_metadata, sse_metrics, base_labels
    ):
        sse_metrics.end_connection(connection_metadata)
        mock_active_connections.add.assert_called_once_with(-1, base_labels)

    def test_records_connection_duration(
        self,
        mock_connection_duration,
        connection_metadata,
        sse_metrics,
        base_labels,
        mocker,
    ):
        mock_time = mocker.patch.object(time, "time")
        mock_time.return_value = 1000.5

        connection_metadata.start_time = 1000.0

        sse_metrics.end_connection(connection_metadata)
        mock_connection_duration.record.assert_called_once_with(500.0, base_labels)

    def test_records_events_per_connection(
        self, mock_events_per_connection, connection_metadata, sse_metrics, base_labels
    ):
        connection_metadata.events_sent_count = 10

        sse_metrics.end_connection(connection_metadata)
        mock_events_per_connection.record.assert_called_once_with(10, base_labels)

    def test_records_connection_error_when_error_disconnect(
        self, mock_connection_errors, connection_metadata, sse_metrics, base_labels
    ):
        sse_metrics.end_connection(connection_metadata, True)
        mock_connection_errors.add.assert_called_once_with(1, base_labels)

    def test_does_not_record_metrics_when_telemetry_disabled(
        self,
        telemetry_disabled,
        mock_active_connections,
        mock_connection_duration,
        mock_events_per_connection,
        mock_connection_errors,
        connection_metadata,
        sse_metrics,
    ):
        sse_metrics.end_connection(connection_metadata, True)

        mock_active_connections.add.assert_not_called()
        mock_connection_duration.record.assert_not_called()
        mock_events_per_connection.record.assert_not_called()
        mock_connection_errors.add.assert_not_called()


class TestRecordEventSent:

    def test_records_events_sent_metric(
        self, mock_events_sent, connection_metadata, sse_metrics, base_labels
    ):
        sse_metrics.record_event_sent(connection_metadata, "storage_create")

        expected_labels = {**base_labels, "event_type": "storage_create"}
        mock_events_sent.add.assert_called_once_with(1, expected_labels)

    def test_increments_events_sent_count_in_metadata(
        self, mock_events_sent, connection_metadata, sse_metrics
    ):
        assert connection_metadata.events_sent_count == 0

        sse_metrics.record_event_sent(connection_metadata, "storage_create")
        assert connection_metadata.events_sent_count == 1

    def test_does_not_record_metrics_when_telemetry_disabled(
        self, telemetry_disabled, mock_events_sent, connection_metadata, sse_metrics
    ):
        sse_metrics.record_event_sent(connection_metadata, "error")

        mock_events_sent.add.assert_not_called()

    def test_increments_count_when_telemetry_disabled(
        self, telemetry_disabled, connection_metadata, sse_metrics
    ):
        assert connection_metadata.events_sent_count == 0

        sse_metrics.record_event_sent(connection_metadata, "storage_create")
        assert connection_metadata.events_sent_count == 1
