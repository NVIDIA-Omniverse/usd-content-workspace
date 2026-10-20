# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from opentelemetry import trace
from unittest.mock import MagicMock

from notification_common.config.common_settings import common_settings
from notification_common.observability.tracing import (
    add_current_span_attributes,
    add_current_span_event,
    trace_async_function,
    trace_operation,
)


@pytest.fixture
def mock_span(mocker):
    span = MagicMock()
    span.is_recording.return_value = True
    mocker.patch.object(trace, "get_current_span", return_value=span)
    return span


@pytest.fixture
def mock_tracer(mocker, mock_span):
    tracer = MagicMock()
    mocker.patch.object(trace, "get_tracer", return_value=tracer)
    tracer.start_as_current_span.return_value.__enter__.return_value = mock_span
    return tracer


@pytest.fixture(autouse=True)
def enable_telemetry(mocker):
    mocker.patch.object(common_settings, "tracing_enabled", True)


@pytest.fixture
def disable_telemetry(mocker):
    mocker.patch.object(common_settings, "tracing_enabled", False)


@pytest.mark.asyncio
class TestAddCurrentSpanAttributes:
    async def test_adds_attributes_to_current_span(self, mock_span):
        add_current_span_attributes(test_key="test_value", another_key=123)

        mock_span.set_attribute.assert_any_call("test_key", "test_value")
        mock_span.set_attribute.assert_any_call("another_key", 123)

    async def test_skips_if_no_current_span(self, mocker, mock_span):
        mocker.patch.object(trace, "get_current_span", return_value=None)

        add_current_span_attributes(test_key="test_value")

        mock_span.set_attribute.assert_not_called()

    async def test_skips_if_span_not_recording(self, mocker):
        span = MagicMock()
        span.is_recording.return_value = False

        mocker.patch.object(trace, "get_current_span", return_value=span)

        add_current_span_attributes(test_key="test_value")

        span.set_attribute.assert_not_called()


@pytest.mark.asyncio
class TestAddCurrentSpanEvent:
    async def test_adds_event_with_attributes_to_current_span(self, mock_span):
        add_current_span_event("test_event", test_key="test_value", another_key=123)

        mock_span.add_event.assert_called_once_with(
            "test_event", attributes={"test_key": "test_value", "another_key": 123}
        )

    async def test_adds_event_without_attributes(self, mock_span):
        add_current_span_event("simple_event")

        mock_span.add_event.assert_called_once_with("simple_event", attributes={})

    async def test_skips_if_no_current_span(self, mocker, mock_span):
        mocker.patch.object(trace, "get_current_span", return_value=None)

        add_current_span_event("test_event", test_key="test_value")

        mock_span.add_event.assert_not_called()

    async def test_skips_if_span_not_recording(self, mocker):
        span = MagicMock()
        span.is_recording.return_value = False

        mocker.patch.object(trace, "get_current_span", return_value=span)

        add_current_span_event("test_event", test_key="test_value")

        span.add_event.assert_not_called()


@pytest.mark.asyncio
class TestTraceAsyncFunction:
    @trace_async_function(
        span_name="test_function", attributes={"static_attr": "value"}
    )
    async def traced_function(self, arg1: str, arg2: int = 42):
        return arg1, arg2

    async def test_creates_span_with_name_and_attributes(self, mock_tracer, mock_span):
        await self.traced_function("test", 123)

        mock_tracer.start_as_current_span.assert_called_once_with("test_function")
        mock_span.set_attribute.assert_any_call("static_attr", "value")

    async def test_uses_function_name_if_no_span_name(self, mock_tracer):
        @trace_async_function()
        async def unnamed_function():
            pass

        await unnamed_function()
        mock_tracer.start_as_current_span.assert_called_once_with("unnamed_function")

    async def test_records_exceptions(self, mock_tracer, mock_span):
        @trace_async_function()
        async def failing_function():
            raise ValueError("test error")

        with pytest.raises(ValueError):
            await failing_function()

        mock_span.record_exception.assert_called_once()

    async def test_skips_tracing_when_telemetry_disabled(
        self, mock_tracer, disable_telemetry, mock_span
    ):
        result = await self.traced_function("test", 123)
        assert result == ("test", 123)

        mock_tracer.start_as_current_span.assert_not_called()


@pytest.mark.asyncio
class TestTraceOperation:
    async def test_creates_span_with_name_and_attributes(self, mock_tracer, mock_span):
        with trace_operation("test_op", attr1="value1"):
            pass

        mock_tracer.start_as_current_span.assert_called_once_with("test_op")
        mock_span.set_attribute.assert_any_call("attr1", "value1")

    async def test_skips_tracing_when_telemetry_disabled(
        self, mock_tracer, disable_telemetry
    ):
        with trace_operation("test_op"):
            pass

        mock_tracer.start_as_current_span.assert_not_called()
