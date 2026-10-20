# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import MagicMock

import pytest

from app.lib.observability import (
    cache_hydration_metrics as cache_hydration_metrics_module,
)
from app.lib.observability.cache_hydration_metrics import (
    CacheHydrationMetrics,
    cache_hydration_metrics,
)


@pytest.fixture
def mock_source_counter():
    return MagicMock()


@pytest.fixture
def mock_latency_histogram():
    return MagicMock()


@pytest.fixture(autouse=True)
def setup_mock_meter(mock_meter, mock_source_counter, mock_latency_histogram):
    mock_meter.create_counter.return_value = mock_source_counter
    mock_meter.create_histogram.return_value = mock_latency_histogram


def _named_call(calls, name):
    """Find a call by its first positional arg or `name` kwarg."""
    return next(
        c
        for c in calls
        if (c.args and c.args[0] == name) or c.kwargs.get("name") == name
    )


@pytest.fixture
def metrics():
    return CacheHydrationMetrics()


class TestInit:
    def test_creates_source_counter_with_correct_name(self, mock_meter):
        CacheHydrationMetrics()

        assert _named_call(
            mock_meter.create_counter.call_args_list, "cache.hydrate.source"
        )

    def test_source_counter_unit_is_hydration(self, mock_meter):
        CacheHydrationMetrics()

        c = _named_call(
            mock_meter.create_counter.call_args_list, "cache.hydrate.source"
        )
        assert c.kwargs.get("unit") == "{hydration}"

    def test_creates_latency_histogram_with_correct_name(self, mock_meter):
        CacheHydrationMetrics()

        assert _named_call(
            mock_meter.create_histogram.call_args_list, "cache.hydrate.latency"
        )

    def test_latency_histogram_unit_is_ms(self, mock_meter):
        CacheHydrationMetrics()

        c = _named_call(
            mock_meter.create_histogram.call_args_list, "cache.hydrate.latency"
        )
        assert c.kwargs.get("unit") == "ms"

    def test_does_not_create_instruments_when_telemetry_disabled(
        self, telemetry_disabled, mock_meter
    ):
        CacheHydrationMetrics()

        mock_meter.create_counter.assert_not_called()
        mock_meter.create_histogram.assert_not_called()

    def test_does_not_create_instruments_when_only_tracing_enabled(
        self, tracing_only, mock_meter
    ):
        # Metrics must gate on metrics_enabled, not tracing_enabled.
        CacheHydrationMetrics()

        mock_meter.create_counter.assert_not_called()
        mock_meter.create_histogram.assert_not_called()


class TestRecordHydrate:
    def test_redis_source_increments_counter_with_redis_label(
        self, metrics, mock_source_counter
    ):
        metrics.record_hydrate("redis", 1.23)

        mock_source_counter.add.assert_called_once_with(1, {"source": "redis"})

    def test_redis_source_records_histogram_with_redis_label(
        self, metrics, mock_latency_histogram
    ):
        metrics.record_hydrate("redis", 1.23)

        mock_latency_histogram.record.assert_called_once_with(1.23, {"source": "redis"})

    def test_storage_api_source_increments_counter_with_storage_api_label(
        self, metrics, mock_source_counter
    ):
        metrics.record_hydrate("storage_api", 4.56)

        mock_source_counter.add.assert_called_once_with(1, {"source": "storage_api"})

    def test_storage_api_source_records_histogram_with_storage_api_label(
        self, metrics, mock_latency_histogram
    ):
        metrics.record_hydrate("storage_api", 4.56)

        mock_latency_histogram.record.assert_called_once_with(
            4.56, {"source": "storage_api"}
        )

    def test_does_not_emit_when_telemetry_disabled(
        self, telemetry_disabled, mock_source_counter, mock_latency_histogram
    ):
        metrics = CacheHydrationMetrics()

        metrics.record_hydrate("redis", 1.23)

        mock_source_counter.add.assert_not_called()
        mock_latency_histogram.record.assert_not_called()


class TestSingleton:
    def test_module_level_singleton_is_cache_hydration_metrics_instance(self):
        assert isinstance(cache_hydration_metrics, CacheHydrationMetrics)

    def test_singleton_is_exported_from_module(self):
        assert (
            cache_hydration_metrics_module.cache_hydration_metrics
            is cache_hydration_metrics
        )
