# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from unittest.mock import MagicMock
from opentelemetry import metrics

from notification_common.config.common_settings import common_settings


@pytest.fixture(autouse=True)
def mock_telemetry_enabled(mocker):
    # Enable both signals; metrics modules read metrics_enabled and the tracing
    # module reads tracing_enabled (they are gated independently since 6.1.0).
    mocker.patch.object(common_settings, "metrics_enabled", True)
    return mocker.patch.object(common_settings, "tracing_enabled", True)


@pytest.fixture
def telemetry_disabled(mocker):
    mocker.patch.object(common_settings, "metrics_enabled", False)
    mocker.patch.object(common_settings, "tracing_enabled", False)


@pytest.fixture
def tracing_only(mocker):
    # Metrics off, tracing on: proves the metrics modules gate specifically on
    # metrics_enabled, independently of tracing (guards the per-signal split).
    mocker.patch.object(common_settings, "metrics_enabled", False)
    mocker.patch.object(common_settings, "tracing_enabled", True)


@pytest.fixture
def mock_meter(mocker):
    mock_meter = MagicMock()
    mock_meter_provider = MagicMock()
    mock_meter_provider.get_meter.return_value = mock_meter

    mocker.patch.object(metrics, "get_meter_provider", return_value=mock_meter_provider)
    return mock_meter
