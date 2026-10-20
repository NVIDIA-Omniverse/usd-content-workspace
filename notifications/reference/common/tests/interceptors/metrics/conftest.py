# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
import pytest_asyncio
from unittest.mock import MagicMock

from opentelemetry.sdk.metrics import MeterProvider
from opentelemetry.sdk.metrics.export import InMemoryMetricReader
from grpc import StatusCode
from grpc.aio import AioRpcError

from notification_common.interceptors.metrics import server_metrics_interceptor


@pytest.fixture
def meter():
    """Fixture to create an OpenTelemetry meter with in-memory metrics."""
    metric_reader = InMemoryMetricReader()
    meter_provider = MeterProvider(metric_readers=[metric_reader])
    yield meter_provider.get_meter("test_meter"), metric_reader
    metric_reader.shutdown()


@pytest.fixture
def grpc_cancelled_error():
    """Fixture for a gRPC cancelled error."""
    return MockRpcError(StatusCode.CANCELLED, "Operation was cancelled by the client")


@pytest.fixture
def grpc_invalid_arg_error():
    """Fixture for a gRPC invalid argument error."""
    return MockRpcError(StatusCode.INVALID_ARGUMENT, "Invalid argument")


@pytest.fixture
def grpc_internal_error():
    """Fixture for a gRPC internal server error."""
    return MockRpcError(StatusCode.INTERNAL, "Internal server error")


@pytest.fixture
def mock_rpc_error():
    """Fixture for a gRPC AioRpcError with empty metadata tuples."""
    return AioRpcError(StatusCode.INTERNAL, initial_metadata=(), trailing_metadata=())


@pytest_asyncio.fixture
async def stream_request_iterator():
    """Mock stream request iterator."""

    async def gen():
        yield "stream_request_1"
        yield "stream_request_2"

    return gen()


@pytest.fixture
def context():
    """Fixture for gRPC context with status code."""
    context = MagicMock()
    context.code.return_value = StatusCode.OK
    return context


@pytest_asyncio.fixture
async def interceptor(meter):
    """Fixture for the metrics interceptor."""
    meter_obj, _ = meter
    return server_metrics_interceptor.ServerMetricsInterceptor(meter_obj)


@pytest.fixture
def method_name():
    """Default method name for tests."""
    return "/test.Service/TestMethod"


@pytest.fixture
def setup_record_mocks(interceptor, mocker):
    """Setup mocks for _record_success and _record_error methods."""
    mock_record_success = mocker.patch.object(interceptor, "_record_success")
    mock_record_error = mocker.patch.object(interceptor, "_record_error")
    return mock_record_success, mock_record_error


class MockRpcError(Exception):
    def __init__(self, code, details="Error details"):
        self._code = code
        self._details = details
        super().__init__(f"Mock Error with code {code}: {details}")

    def code(self):
        return self._code

    def details(self):
        return self._details

    async def initial_metadata(self):
        return {}

    async def trailing_metadata(self):
        return {}

    def debug_error_string(self):
        return f"Error {self._code}: {self._details}"


class MockHandler:
    def __init__(
        self,
        request_streaming=False,
        response_streaming=False,
        request_deserializer=None,
        response_serializer=None,
    ):
        self.request_streaming = request_streaming
        self.response_streaming = response_streaming
        self.request_deserializer = request_deserializer
        self.response_serializer = response_serializer

    # Default implementations that can be overridden by subclasses
    async def unary_unary(self, request, context):
        return "unary_response"

    async def unary_stream(self, request, context):
        yield "stream_response_1"
        yield "stream_response_2"

    async def stream_unary(self, request_iterator, context):
        requests = []
        async for request in request_iterator:
            requests.append(request)
        return f"Got {len(requests)} requests"

    async def stream_stream(self, request_iterator, context):
        requests = []
        async for request in request_iterator:
            requests.append(request)
            yield f"Response for {request}"


class MockHandlerWithError(MockHandler):
    """Mock handler that raises an exception based on handler type."""

    def __init__(self, exception, **kwargs):
        super().__init__(**kwargs)
        self.exception = exception

    async def unary_unary(self, request, context):
        raise self.exception

    async def unary_stream(self, request, context):
        if self.response_streaming:
            # For streaming, yield one item to enter the generator before raising
            yield "dummy_to_trigger_generator"
            raise self.exception
        else:
            raise self.exception

    async def stream_unary(self, request_iterator, context):
        # Consume the iterator first to catch any iterator errors
        async for _ in request_iterator:
            pass
        raise self.exception

    async def stream_stream(self, request_iterator, context):
        if self.response_streaming:
            # Process one request, then raise
            async for _ in request_iterator:
                yield "dummy_to_trigger_generator"
                # Raise after the first item
                raise self.exception
                # We won't get here, but this ensures we break after first yield
                break
        else:
            raise self.exception


class MockStreamHandlerWithLateError(MockHandler):
    """Mock stream handler that yields some responses before raising an exception."""

    def __init__(self, exception, **kwargs):
        kwargs["response_streaming"] = True  # Force response streaming
        super().__init__(**kwargs)
        self.exception = exception

    async def unary_stream(self, request, context):
        # Yield some responses then raise the exception
        yield "first_response"
        yield "second_response"
        raise self.exception


class MockStreamHandlerWithRequestError(MockHandler):
    """Mock handler that yields an error during request processing."""

    def __init__(self, exception, **kwargs):
        kwargs["request_streaming"] = True  # Force request streaming
        super().__init__(**kwargs)
        self.exception = exception

    async def stream_unary(self, request_iterator, context):
        # This simulates an error while processing the request iterator
        count = 0
        async for _ in request_iterator:
            count += 1
            if count > 1:
                raise self.exception
        return f"Processed {count} requests before error"


# Basic handler fixtures for different RPC patterns
@pytest.fixture
def unary_unary_handler():
    """Fixture for a unary-unary RPC handler."""
    return MockHandler()


@pytest.fixture
def unary_stream_handler():
    """Fixture for a unary-stream RPC handler."""
    return MockHandler(response_streaming=True)


@pytest.fixture
def stream_unary_handler():
    """Fixture for a stream-unary RPC handler."""
    return MockHandler(request_streaming=True)


@pytest.fixture
def stream_stream_handler():
    """Fixture for a stream-stream RPC handler."""
    return MockHandler(request_streaming=True, response_streaming=True)
