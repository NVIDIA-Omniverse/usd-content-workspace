# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from unittest.mock import MagicMock, call

from notification_common.config.common_settings import common_settings
from opentelemetry import trace

from app.lib.observability.tracing import (
    trace_sse_event_stream,
    trace_grpc_event_stream,
    _trace_message_stream,
)


@pytest.fixture
def mock_trace_message_stream(mocker):
    return mocker.patch("app.lib.observability.tracing._trace_message_stream")


@pytest.fixture
def sample_async_func():
    async def test_func():
        pass

    return test_func


@pytest.fixture
def sample_attributes():
    return {"key1": "value1", "key2": 123}


class AnyCallable:
    def __eq__(self, other):
        return callable(other)


class TestSseEventStreamDecorator:

    def test_calls_trace_message_stream_with_correct_args(
        self, mock_trace_message_stream, sample_async_func, sample_attributes
    ):
        decorator = trace_sse_event_stream(
            stream_name="custom_name", attributes=sample_attributes
        )
        decorator(sample_async_func)

        mock_trace_message_stream.assert_called_once_with(
            sample_async_func, "custom_name", sample_attributes, AnyCallable()
        )

    def test_decorator_uses_function_name_if_no_stream_name_provided(
        self, mock_trace_message_stream, sample_async_func
    ):
        decorator = trace_sse_event_stream()
        decorator(sample_async_func)

        mock_trace_message_stream.assert_called_once_with(
            sample_async_func, "test_func_stream", None, AnyCallable()
        )

    @pytest.mark.parametrize(
        "item,expected",
        [
            (
                {"event": "test_event", "data": "test_data"},
                {
                    "event_type": "test_event",
                    "event_content": "{'event': 'test_event', 'data': 'test_data'}",
                },
            ),
            (
                {"data": "test_data"},
                {"event_type": "unknown", "event_content": "{'data': 'test_data'}"},
            ),
            ("test_string", {"event_content": "test_string"}),
        ],
    )
    def test_event_handler_returns_expected_attributes(
        self, mock_trace_message_stream, sample_async_func, item, expected
    ):
        decorator = trace_sse_event_stream()
        decorator(sample_async_func)

        call_args = mock_trace_message_stream.call_args[0]
        event_handler = call_args[3]
        result = event_handler(item)
        assert result == expected


class TestGrpcEventStreamDecorator:

    def test_calls_trace_message_stream_with_correct_args(
        self, mock_trace_message_stream, sample_async_func, sample_attributes
    ):
        decorator = trace_grpc_event_stream(
            stream_name="custom_name", attributes=sample_attributes
        )
        decorator(sample_async_func)

        mock_trace_message_stream.assert_called_once_with(
            sample_async_func, "custom_name", sample_attributes, AnyCallable()
        )

    def test_uses_function_name_if_no_stream_name_provided(
        self, mock_trace_message_stream, sample_async_func
    ):
        decorator = trace_grpc_event_stream()
        decorator(sample_async_func)

        mock_trace_message_stream.assert_called_once_with(
            sample_async_func, "grpc.test_func", None, AnyCallable()
        )

    @pytest.mark.parametrize(
        "response_setup,expected",
        [
            (
                lambda: MagicMock(events=["event1", "event2", "event3"]),
                {"event_count": 3, "events_content": ["event1", "event2", "event3"]},
            ),
            (lambda: MagicMock(events=[]), {"event_count": 0}),
            (lambda: MagicMock(spec=[]), {"event_count": 0}),  # No events attribute
        ],
    )
    def test_event_handler_returns_expected_attributes(
        self, mock_trace_message_stream, sample_async_func, response_setup, expected
    ):
        decorator = trace_grpc_event_stream()
        decorator(sample_async_func)

        call_args = mock_trace_message_stream.call_args[0]
        event_handler = call_args[3]
        response = response_setup()
        result = event_handler(response)
        assert result == expected


class TestTraceMessageStream:
    @pytest.fixture
    def event_handler(self):
        return lambda x: {"content": str(x)}

    @pytest.fixture
    def make_generator(self):
        def _make_generator(items):
            async def gen():
                for item in items:
                    yield item

            return gen

        return _make_generator

    @pytest.fixture
    def wrap_stream(self):
        def _wrap_stream(
            generator, stream_name="test_stream", attributes=None, event_handler=None
        ):
            return _trace_message_stream(
                generator, stream_name, attributes, event_handler
            )

        return _wrap_stream

    async def collect_results(self, async_gen):
        results = []

        async for item in async_gen():
            results.append(item)

        return results

    @pytest.fixture
    def setup_span_context(self, mock_tracer, mock_span):
        mock_tracer.start_as_current_span.return_value.__enter__.return_value = (
            mock_span
        )
        mock_tracer.start_as_current_span.return_value.__exit__.return_value = None

        return mock_span

    @pytest.mark.asyncio
    async def test_no_tracing_when_telemetry_disabled(
        self, mock_tracer, make_generator, event_handler, wrap_stream, mocker
    ):
        mocker.patch.object(common_settings, "tracing_enabled", False)

        generator = make_generator(["item1", "item2"])
        wrapped_func = wrap_stream(generator, event_handler=event_handler)

        results = await self.collect_results(wrapped_func)

        assert results == ["item1", "item2"]

        mock_tracer.start_as_current_span.assert_not_called()

    @pytest.mark.asyncio
    async def test_tracer_initialized_with_service_name(
        self,
        mock_telemetry_enabled,
        mock_tracer,
        make_generator,
        event_handler,
        wrap_stream,
    ):
        generator = make_generator(["item1"])
        wrapped_func = wrap_stream(generator, event_handler=event_handler)

        async for _ in wrapped_func():
            pass

        trace.get_tracer.assert_called_once_with("event-consumer-service")

    @pytest.mark.asyncio
    async def test_telemetry_enabled_creates_span(
        self,
        mock_telemetry_enabled,
        mock_tracer,
        setup_span_context,
        make_generator,
        event_handler,
        wrap_stream,
    ):
        generator = make_generator(["item1"])
        wrapped_func = wrap_stream(generator, event_handler=event_handler)

        async for _ in wrapped_func():
            pass

        mock_tracer.start_as_current_span.assert_called_once_with("test_stream")

    @pytest.mark.asyncio
    async def test_span_attributes_are_set(
        self,
        mock_telemetry_enabled,
        mock_tracer,
        setup_span_context,
        sample_attributes,
        make_generator,
        event_handler,
        wrap_stream,
    ):
        generator = make_generator(["item1"])
        wrapped_func = wrap_stream(
            generator, attributes=sample_attributes, event_handler=event_handler
        )

        async for _ in wrapped_func():
            pass

        expected_calls = [
            call("key1", "value1"),
            call("key2", "123"),
        ]

        setup_span_context.set_attribute.assert_has_calls(expected_calls)

    @pytest.mark.asyncio
    async def test_message_events_are_added(
        self,
        mock_telemetry_enabled,
        mock_tracer,
        setup_span_context,
        make_generator,
        event_handler,
        wrap_stream,
    ):
        generator = make_generator(["item1", "item2"])
        wrapped_func = wrap_stream(generator, event_handler=event_handler)

        async for _ in wrapped_func():
            pass

        expected_event_calls = [
            call("message_streamed", {"content": "item1"}),
            call("message_streamed", {"content": "item2"}),
        ]

        setup_span_context.add_event.assert_has_calls(expected_event_calls)

    @pytest.mark.asyncio
    async def test_count_is_tracked(
        self,
        mock_telemetry_enabled,
        mock_tracer,
        setup_span_context,
        make_generator,
        event_handler,
        wrap_stream,
    ):
        generator = make_generator(["item1", "item2"])
        wrapped_func = wrap_stream(generator, event_handler=event_handler)

        async for _ in wrapped_func():
            pass

        setup_span_context.set_attribute.assert_any_call("message_count", 2)

    @pytest.mark.asyncio
    async def test_records_exceptions_in_span(
        self,
        mock_telemetry_enabled,
        mock_tracer,
        setup_span_context,
        make_generator,
        event_handler,
        wrap_stream,
    ):
        test_exception = ValueError("Test error")

        async def gen():
            yield "item1"
            yield "item2"
            raise test_exception

        wrapped_func = wrap_stream(gen, event_handler=event_handler)

        with pytest.raises(ValueError, match="Test error"):
            async for _ in wrapped_func():
                pass

        setup_span_context.record_exception.assert_called_once_with(test_exception)
