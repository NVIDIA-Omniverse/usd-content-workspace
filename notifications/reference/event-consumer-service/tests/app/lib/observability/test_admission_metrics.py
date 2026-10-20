# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from unittest.mock import MagicMock, call

from app.lib.observability.admission_metrics import AdmissionMetrics


@pytest.fixture
def mock_admitted():
    return MagicMock()


@pytest.fixture
def mock_shed():
    return MagicMock()


@pytest.fixture(autouse=True)
def setup_mock_meter(mock_meter, mock_admitted, mock_shed):
    mock_meter.create_counter.side_effect = [mock_admitted, mock_shed]


class TestInit:

    def test_initializes_counter_metrics(self, mock_meter):
        AdmissionMetrics()

        mock_meter.create_counter.assert_has_calls(
            [
                call(
                    "channel_pool.admission.admitted_total",
                    unit="{stream}",
                    description=(
                        "Non-durable consume streams admitted by the capacity guard"
                    ),
                ),
                call(
                    "channel_pool.admission.shed_total",
                    unit="{stream}",
                    description=(
                        "Non-durable consume streams shed by the capacity guard"
                    ),
                ),
            ]
        )

    def test_does_not_initialize_metrics_when_telemetry_disabled(
        self, telemetry_disabled, mock_meter
    ):
        AdmissionMetrics()

        mock_meter.create_counter.assert_not_called()

    def test_does_not_initialize_metrics_when_only_tracing_enabled(
        self, tracing_only, mock_meter
    ):
        # Metrics must gate on metrics_enabled, not tracing_enabled.
        AdmissionMetrics()

        mock_meter.create_counter.assert_not_called()


class TestRecordAdmitted:

    def test_records_re_routable_class(self, mock_admitted):
        metrics = AdmissionMetrics()
        metrics.record_admitted(re_routable=True)

        mock_admitted.add.assert_called_once_with(1, {"class": "re_routable"})

    def test_records_pinned_class(self, mock_admitted):
        metrics = AdmissionMetrics()
        metrics.record_admitted(re_routable=False)

        mock_admitted.add.assert_called_once_with(1, {"class": "pinned"})

    def test_does_not_record_when_telemetry_disabled(
        self, telemetry_disabled, mock_admitted
    ):
        metrics = AdmissionMetrics()
        metrics.record_admitted(re_routable=True)

        mock_admitted.add.assert_not_called()


class TestRecordShed:

    def test_records_reason_with_default_any_class(self, mock_shed):
        metrics = AdmissionMetrics()
        metrics.record_shed(reason="memory_cap")

        mock_shed.add.assert_called_once_with(
            1, {"reason": "memory_cap", "class": "any"}
        )

    def test_records_reason_and_explicit_class(self, mock_shed):
        metrics = AdmissionMetrics()
        metrics.record_shed(reason="shed_threshold", stream_class="re_routable")

        mock_shed.add.assert_called_once_with(
            1, {"reason": "shed_threshold", "class": "re_routable"}
        )

    def test_does_not_record_when_telemetry_disabled(
        self, telemetry_disabled, mock_shed
    ):
        metrics = AdmissionMetrics()
        metrics.record_shed(reason="draining")

        mock_shed.add.assert_not_called()
