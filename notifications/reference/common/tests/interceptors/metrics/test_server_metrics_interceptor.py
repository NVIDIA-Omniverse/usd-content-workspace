# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from unittest.mock import MagicMock, ANY
import time

from typing import Any, Optional

from opentelemetry.sdk.metrics.export import InMemoryMetricReader
from grpc import StatusCode
from grpc.aio import AioRpcError

from notification_common.config.observability_config import EXCLUDE_METHODS
from notification_common.interceptors.metrics.server_metrics_interceptor import (
    RPC_SERVER_CALLS,
    RPC_SERVER_ACTIVE_REQUESTS,
    RPC_SERVER_ACTIVE_METHODS,
    RPC_SERVER_ACTIVE_RESPONSES,
)

# Import mock classes from mocks
from tests.interceptors.metrics.conftest import (
    MockHandler,
    MockHandlerWithError,
    MockStreamHandlerWithLateError,
    MockStreamHandlerWithRequestError,
)


def get_metric_by_name(
    metric_reader: InMemoryMetricReader, metric_name: str
) -> Optional[Any]:
    metrics_data = metric_reader.get_metrics_data()
    if not metrics_data or not metrics_data.resource_metrics:
        return None
    for resource_metrics in metrics_data.resource_metrics:
        for scope_metrics in resource_metrics.scope_metrics:
            for metric in scope_metrics.metrics:
                if metric.name == metric_name:
                    return metric
    return None


def assert_success_recorded(
    mock_success, mock_error, method_name, status_code, is_streaming=False
):
    mock_success.assert_called_once_with(
        method_name, status_code, ANY, is_streaming=is_streaming
    )
    mock_error.assert_not_called()


def assert_error_recorded(mock_success, mock_error, method_name, is_streaming=False):
    mock_success.assert_not_called()
    mock_error.assert_called_once_with(
        method_name, StatusCode.UNKNOWN, ANY, is_streaming=is_streaming
    )


def verify_metric_for_method(
    metric_reader, metric_name, method_name, expected_value=None
):
    metric_reader.collect()
    metric = get_metric_by_name(metric_reader, metric_name)
    assert metric is not None, f"Metric {metric_name} not found"

    short_method_name = method_name.split("/")[-1]

    method_data_points = [
        dp
        for dp in metric.data.data_points
        if dict(dp.attributes).get("rpc.method") == short_method_name
    ]
    assert method_data_points, f"No data points for {method_name} in {metric_name}"

    if expected_value is not None:
        assert any(
            dp.sum == expected_value for dp in method_data_points
        ), f"Expected value {expected_value} not found in {metric_name}"

    return method_data_points


def verify_active_metrics_zero(metric_reader, method_name):
    metric_reader.collect()

    short_method_name = method_name.split("/")[-1]
    max_retries = 3
    for _ in range(max_retries):
        all_metrics_zero = True

        for metric_name, metric_full_name in [
            ("methods", RPC_SERVER_ACTIVE_METHODS),
            ("requests", RPC_SERVER_ACTIVE_REQUESTS),
            ("responses", RPC_SERVER_ACTIVE_RESPONSES),
        ]:
            metric = get_metric_by_name(metric_reader, metric_full_name)
            if metric is None:
                continue

            method_data_points = [
                dp
                for dp in metric.data.data_points
                if dict(dp.attributes).get("rpc.method") == short_method_name
            ]

            if method_data_points and not all(
                dp.value == 0 for dp in method_data_points
            ):
                all_metrics_zero = False
                break

        if all_metrics_zero:
            return

        time.sleep(0.01)
        metric_reader.collect()

    for metric_name, metric_full_name in [
        ("methods", RPC_SERVER_ACTIVE_METHODS),
        ("requests", RPC_SERVER_ACTIVE_REQUESTS),
        ("responses", RPC_SERVER_ACTIVE_RESPONSES),
    ]:
        metric = get_metric_by_name(metric_reader, metric_full_name)
        if metric is None:
            continue

        method_data_points = [
            dp
            for dp in metric.data.data_points
            if dict(dp.attributes).get("rpc.method") == short_method_name
        ]

        if method_data_points:
            assert all(
                dp.value == 0 for dp in method_data_points
            ), f"Active {metric_name} counter not reset to zero"


def verify_duration_metrics(metric_reader, method_name, expect_duration):
    short_method_name = method_name.split("/")[-1]
    metric_reader.collect()
    duration_metric = get_metric_by_name(metric_reader, "rpc.server.duration")

    if duration_metric is None:
        assert (
            not expect_duration
        ), f"Expected duration metric for {method_name} but it's missing"
        return

    method_data_points = [
        dp
        for dp in duration_metric.data.data_points
        if dict(dp.attributes).get("rpc.method") == short_method_name
    ]

    if expect_duration:
        assert (
            method_data_points
        ), f"Duration metric should be present for {method_name}"
        assert method_data_points[0].sum > 0, "Duration value should be positive"
    else:
        assert (
            not method_data_points
        ), f"Duration should not be recorded for streaming RPCs: {method_name}"


def assert_active_metrics_incremented(metric_reader, method_name, metrics_to_check):

    short_method_name = method_name.split("/")[-1]
    metric_reader.collect()

    for metric_name in metrics_to_check:
        metric = get_metric_by_name(metric_reader, metric_name)
        assert metric is not None, f"Metric {metric_name} not found"

        method_data_points = [
            dp
            for dp in metric.data.data_points
            if dict(dp.attributes).get("rpc.method") == short_method_name
        ]

        assert method_data_points, f"No data points for {method_name} in {metric_name}"
        assert any(
            dp.value > 0 for dp in method_data_points
        ), f"Expected {metric_name} to be incremented"


# Helper for verifying error attributes in metrics
def verify_error_attributes(
    metric_reader, method_name, status_code, expect_server_error=True
):
    short_method_name = method_name.split("/")[-1]
    metric_reader.collect()

    calls_metric = get_metric_by_name(metric_reader, RPC_SERVER_CALLS)
    assert calls_metric is not None, "Calls metric not found"

    method_data_points = [
        dp
        for dp in calls_metric.data.data_points
        if dict(dp.attributes).get("rpc.method") == short_method_name
    ]
    assert method_data_points, f"No data points for {method_name} in calls metric"

    # Verify attributes are correctly set
    attributes = dict(method_data_points[0].attributes)

    # Status code should match
    assert "rpc.grpc.status_code" in attributes, "Status code attribute missing"
    assert attributes["rpc.grpc.status_code"] == str(
        status_code.value[0]
    ), f"Expected status code {status_code}, got {attributes['rpc.grpc.status_code']}"

    # Server error flag should be set appropriately
    assert "rpc.grpc.server_error" in attributes, "Server error attribute missing"
    expected_error_value = "true" if expect_server_error else "false"
    assert (
        attributes["rpc.grpc.server_error"] == expected_error_value
    ), f"Expected {expected_error_value}, got {attributes['rpc.grpc.server_error']}"


class TestServerMetricsInterceptor:
    @pytest.fixture
    def call_details_factory(self):
        def _create_call_details(method):
            class CallDetails:
                def __init__(self, method):
                    self.method = method

            return CallDetails(method)

        return _create_call_details

    @pytest.fixture
    def mock_continuation(self, mocker, unary_unary_handler):
        return mocker.AsyncMock(return_value=unary_unary_handler)

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "handler_fixture_name, expected_wrapper",
        [
            ("unary_unary_handler", "_wrap_unary_unary"),
            ("unary_stream_handler", "_wrap_unary_stream"),
            ("stream_unary_handler", "_wrap_stream_unary"),
            ("stream_stream_handler", "_wrap_stream_stream"),
        ],
    )
    async def test_intercept_service_returns_correct_handler(
        self,
        interceptor,
        method_name,
        mocker,
        handler_fixture_name,
        expected_wrapper,
        request,
    ):
        # Get the actual handler fixture using pytest's request.getfixturevalue
        handler = request.getfixturevalue(handler_fixture_name)

        # Create a dict of all wrapper methods with mocks
        wrapper_methods = {
            "_wrap_unary_unary": mocker.patch.object(interceptor, "_wrap_unary_unary"),
            "_wrap_unary_stream": mocker.patch.object(
                interceptor, "_wrap_unary_stream"
            ),
            "_wrap_stream_unary": mocker.patch.object(
                interceptor, "_wrap_stream_unary"
            ),
            "_wrap_stream_stream": mocker.patch.object(
                interceptor, "_wrap_stream_stream"
            ),
        }

        async def continuation(_call_details):
            return handler

        await interceptor.intercept_service(continuation, MagicMock(method=method_name))

        for wrapper_name, mock_method in wrapper_methods.items():
            if wrapper_name == expected_wrapper:
                mock_method.assert_called_once()
            else:
                mock_method.assert_not_called()

    @pytest.mark.asyncio
    async def test_none_handler_returns_none_without_errors(self, interceptor):

        async def continuation(_call_details):
            return None

        result = await interceptor.intercept_service(
            continuation, MagicMock(method="/test.Service/NonExistentMethod")
        )
        assert result is None

    @pytest.mark.asyncio
    @pytest.mark.parametrize("excluded_method", EXCLUDE_METHODS)
    async def test_excludes_health_check_and_reflection_endpoints(
        self,
        interceptor,
        meter,
        excluded_method,
        call_details_factory,
        mock_continuation,
    ):
        _, metric_reader = meter

        call_details = call_details_factory(excluded_method)
        await interceptor.intercept_service(mock_continuation, call_details)

        # Verify no metrics were recorded
        metric_reader.collect()
        metric = get_metric_by_name(metric_reader, RPC_SERVER_CALLS)

        assert metric is None

    @pytest.mark.asyncio
    async def test_processes_regular_endpoints(
        self,
        interceptor,
        meter,
        method_name,
        call_details_factory,
        mock_continuation,
        context,
    ):
        _, metric_reader = meter

        call_details = call_details_factory(method_name)
        result = await interceptor.intercept_service(mock_continuation, call_details)

        assert result != mock_continuation.return_value

        await result.unary_unary({"test": "data"}, context)

        verify_metric_for_method(metric_reader, RPC_SERVER_CALLS, method_name)


class TestUnaryUnaryWrapper:
    @pytest.mark.asyncio
    async def test_records_calls_and_duration(
        self,
        interceptor,
        context,
        method_name,
        setup_record_mocks,
        unary_unary_handler,
    ):
        mock_record_success, mock_record_error = setup_record_mocks

        wrapped_handler = interceptor._wrap_unary_unary(
            unary_unary_handler.unary_unary, method_name
        )

        response = await wrapped_handler({"request": "data"}, context)
        assert response == "unary_response"

        assert_success_recorded(
            mock_record_success,
            mock_record_error,
            method_name,
            context.code(),
            is_streaming=False,
        )

    @pytest.mark.asyncio
    async def test_active_metrics_increment_during_call_and_zero_after(
        self, interceptor, meter, context, method_name, mocker
    ):
        mocker.patch.object(interceptor, "_record_success")
        mocker.patch.object(interceptor, "_record_error")

        _, metric_reader = meter

        class TestHandler(MockHandler):
            async def unary_unary(self, request, _context):
                # Check that the active metrics are incremented during processing
                assert_active_metrics_incremented(
                    metric_reader,
                    method_name,
                    [RPC_SERVER_ACTIVE_METHODS, RPC_SERVER_ACTIVE_REQUESTS],
                )
                return "response_data"

        handler = TestHandler()

        # Get the wrapped handler directly
        wrapped_handler = interceptor._wrap_unary_unary(
            handler.unary_unary, method_name
        )

        await wrapped_handler({"test": "data"}, context)

        verify_active_metrics_zero(metric_reader, method_name)

    @pytest.mark.asyncio
    async def test_request_and_response_size_metrics_are_recorded(
        self,
        interceptor,
        meter,
        context,
        method_name,
        unary_unary_handler,
    ):
        _, metric_reader = meter

        wrapped_handler = interceptor._wrap_unary_unary(
            unary_unary_handler.unary_unary, method_name
        )

        await wrapped_handler({"test": "data"}, context)

        verify_metric_for_method(metric_reader, "rpc.server.request.size", method_name)
        verify_metric_for_method(metric_reader, "rpc.server.response.size", method_name)

    @pytest.mark.asyncio
    async def test_metrics_include_correct_labels(
        self, interceptor, meter, context, unary_unary_handler
    ):
        _, metric_reader = meter
        method_name = "/test.Service/LabelsTest"

        wrapped_handler = interceptor._wrap_unary_unary(
            unary_unary_handler.unary_unary, method_name
        )

        await wrapped_handler({"test": "data"}, context)

        metric_reader.collect()
        calls_metric = get_metric_by_name(metric_reader, RPC_SERVER_CALLS)
        assert calls_metric is not None
        assert len(calls_metric.data.data_points) > 0

        data_point = calls_metric.data.data_points[0]
        attributes = dict(data_point.attributes)

        expected_labels = {
            "rpc.service": "Service",
            "rpc.method": "LabelsTest",
            "rpc.system": "grpc",
            "rpc.grpc.status_code": str(StatusCode.OK.value[0]),
        }

        for key, expected_value in expected_labels.items():
            assert key in attributes, f"{key} label missing"
            assert (
                attributes[key] == expected_value
            ), f"Expected {key}={expected_value}, got {attributes[key]}"

    @pytest.mark.asyncio
    async def test_error_handling_records_error_metrics(
        self, interceptor, context, method_name, setup_record_mocks
    ):
        mock_record_success, mock_record_error = setup_record_mocks

        class ErrorHandler(MockHandler):
            def __init__(self):
                super().__init__()

            async def unary_unary(self, request, _context):
                raise ValueError("Test error")

        handler = ErrorHandler()

        wrapped_handler = interceptor._wrap_unary_unary(
            handler.unary_unary, method_name
        )

        with pytest.raises(ValueError):
            await wrapped_handler({"request": "data"}, context)

        assert_error_recorded(
            mock_record_success, mock_record_error, method_name, is_streaming=False
        )

    @pytest.mark.asyncio
    async def test_active_counters_reset_after_error(
        self, interceptor, meter, context, grpc_internal_error, mocker
    ):
        mocker.patch.object(interceptor, "_record_success")
        mocker.patch.object(interceptor, "_record_error")

        _, metric_reader = meter
        method_name = "/test.Service/ResetCountersTest"

        class ErrorHandler(MockHandlerWithError):
            def __init__(self):
                super().__init__(grpc_internal_error)

        handler = ErrorHandler()

        wrapped_handler = interceptor._wrap_unary_unary(
            handler.unary_unary, method_name
        )

        with pytest.raises(Exception):
            await wrapped_handler({"test": "data"}, context)

        verify_active_metrics_zero(metric_reader, method_name)

    @pytest.mark.asyncio
    async def test_handles_rpc_error(
        self, interceptor, context, method_name, mock_rpc_error, mocker
    ):
        mock_rpc_err = mocker.patch.object(interceptor, "_handle_rpc_error")

        class ErrorHandler(MockHandler):
            def __init__(self):
                super().__init__()

            async def unary_unary(self, request, _context):
                raise mock_rpc_error

        handler = ErrorHandler()

        wrapped_handler = interceptor._wrap_unary_unary(
            handler.unary_unary, method_name
        )

        with pytest.raises(AioRpcError):
            await wrapped_handler({"request": "data"}, context)

        mock_rpc_err.assert_called_once()


class TestUnaryStreamWrapper:

    @pytest.mark.asyncio
    async def test_records_metrics_without_duration(
        self,
        interceptor,
        context,
        setup_record_mocks,
        unary_stream_handler,
    ):
        mock_record_success, mock_record_error = setup_record_mocks
        method_name = "/test.Service/TestStreamMethod"

        wrapped_handler = interceptor._wrap_unary_stream(
            unary_stream_handler.unary_stream, method_name
        )

        responses = []
        async for response in wrapped_handler({"request": "data"}, context):
            responses.append(response)

        assert responses == ["stream_response_1", "stream_response_2"]

        assert_success_recorded(
            mock_record_success,
            mock_record_error,
            method_name,
            context.code(),
            is_streaming=True,
        )

    @pytest.mark.asyncio
    async def test_active_responses_incremented_during_streaming_and_zeroed_after(
        self, interceptor, meter, context, mocker
    ):
        mocker.patch.object(interceptor, "_record_success")
        mocker.patch.object(interceptor, "_record_error")

        _, metric_reader = meter
        method_name = "/test.Service/ActiveResponsesTest"

        class TestStreamHandler(MockHandler):
            def __init__(self):
                super().__init__(response_streaming=True)

            async def unary_stream(self, request, _context):
                yield "response_1"

                assert_active_metrics_incremented(
                    metric_reader, method_name, [RPC_SERVER_ACTIVE_RESPONSES]
                )

                yield "response_2"

        handler = TestStreamHandler()
        wrapped_handler = interceptor._wrap_unary_stream(
            handler.unary_stream, method_name
        )

        responses = []
        async for response in wrapped_handler({"test": "data"}, context):
            responses.append(response)

        verify_active_metrics_zero(metric_reader, method_name)

    @pytest.mark.asyncio
    async def test_unary_stream_with_responses_then_error(
        self, interceptor, context, grpc_internal_error, setup_record_mocks
    ):
        mock_record_success, mock_record_error = setup_record_mocks
        method_name = "/test.Service/StreamWithLateErrorTest"

        class LateErrorStreamHandler(MockStreamHandlerWithLateError):
            def __init__(self):
                super().__init__(grpc_internal_error)

        handler = LateErrorStreamHandler()

        wrapped_handler = interceptor._wrap_unary_stream(
            handler.unary_stream, method_name
        )

        responses = []
        with pytest.raises(Exception) as exc_info:
            async for response in wrapped_handler({"test": "data"}, context):
                responses.append(response)

        assert exc_info.value == grpc_internal_error
        assert len(responses) == 2

        assert_error_recorded(
            mock_record_success, mock_record_error, method_name, is_streaming=True
        )

    @pytest.mark.asyncio
    async def test_responses_per_rpc_records_exact_counts(
        self, interceptor, meter, context, setup_record_mocks
    ):
        mock_record_success, mock_record_error = setup_record_mocks
        _, metric_reader = meter
        method_name = "/test.Service/ExactResponsesCountTest"

        class ExactResponseHandler(MockHandler):
            def __init__(self):
                super().__init__(response_streaming=True)

            async def unary_stream(self, request, _context):
                yield "response_1"
                yield "response_2"
                yield "response_3"

        handler = ExactResponseHandler()

        wrapped_handler = interceptor._wrap_unary_stream(
            handler.unary_stream, method_name
        )

        responses = []
        async for response in wrapped_handler({"test": "data"}, context):
            responses.append(response)

        assert len(responses) == 3

        verify_metric_for_method(
            metric_reader, "rpc.server.responses_per_rpc", method_name, expected_value=3
        )

        assert_success_recorded(
            mock_record_success,
            mock_record_error,
            method_name,
            context.code(),
            is_streaming=True,
        )

    @pytest.mark.asyncio
    async def test_handles_rpc_error(
        self, interceptor, context, method_name, mock_rpc_error, mocker
    ):
        mock_rpc_err = mocker.patch.object(interceptor, "_handle_rpc_error")

        class ErrorHandler(MockHandler):
            def __init__(self):
                super().__init__(response_streaming=True)

            async def unary_stream(self, request, _context):
                raise mock_rpc_error
                # noinspection PyUnreachableCode
                yield "This will not be reached"

        handler = ErrorHandler()

        wrapped_handler = interceptor._wrap_unary_stream(
            handler.unary_stream, method_name
        )

        with pytest.raises(AioRpcError):
            async for _ in wrapped_handler({"request": "data"}, context):
                pass

        mock_rpc_err.assert_called_once()


class TestStreamUnaryWrapper:

    @pytest.mark.asyncio
    async def test_records_metrics_and_requests_per_rpc(
        self,
        interceptor,
        context,
        stream_request_iterator,
        setup_record_mocks,
        stream_unary_handler,
    ):
        mock_record_success, mock_record_error = setup_record_mocks
        method_name = "/test.Service/TestStreamInputMethod"

        wrapped_handler = interceptor._wrap_stream_unary(
            stream_unary_handler.stream_unary, method_name
        )

        response = await wrapped_handler(stream_request_iterator, context)
        assert response == "Got 2 requests"

        assert_success_recorded(
            mock_record_success,
            mock_record_error,
            method_name,
            context.code(),
            is_streaming=True,
        )

    @pytest.mark.asyncio
    async def test_active_methods_decremented_after_completion(
        self,
        interceptor,
        meter,
        context,
        stream_request_iterator,
        mocker,
        stream_unary_handler,
    ):
        mocker.patch.object(interceptor, "_record_success")
        mocker.patch.object(interceptor, "_record_error")

        _, metric_reader = meter
        method_name = "/test.Service/ActiveMethodsTest"

        wrapped_handler = interceptor._wrap_stream_unary(
            stream_unary_handler.stream_unary, method_name
        )

        await wrapped_handler(stream_request_iterator, context)
        verify_active_metrics_zero(metric_reader, method_name)

    @pytest.mark.asyncio
    async def test_requests_per_rpc_records_exact_counts(
        self,
        interceptor,
        meter,
        context,
        setup_record_mocks,
        stream_unary_handler,
    ):
        mock_record_success, mock_record_error = setup_record_mocks
        _, metric_reader = meter
        method_name = "/test.Service/ExactRequestsCountTest"

        async def gen():
            yield "request_1"
            yield "request_2"
            yield "request_3"
            yield "request_4"

        stream_request_iterator = gen()

        wrapped_handler = interceptor._wrap_stream_unary(
            stream_unary_handler.stream_unary, method_name
        )

        await wrapped_handler(stream_request_iterator, context)

        verify_metric_for_method(
            metric_reader, "rpc.server.requests_per_rpc", method_name, expected_value=4
        )

        assert_success_recorded(
            mock_record_success,
            mock_record_error,
            method_name,
            context.code(),
            is_streaming=True,
        )

    @pytest.mark.asyncio
    async def test_active_requests_incremented_and_decremented_per_request(
        self, interceptor, meter, context, mocker
    ):
        mocker.patch.object(interceptor, "_record_success")
        mocker.patch.object(interceptor, "_record_error")

        _, metric_reader = meter
        method_name = "/test.Service/ActiveRequestsPerRequestTest"

        active_requests_values = []

        class TestHandler(MockHandler):
            def __init__(self):
                super().__init__(request_streaming=True)

            async def stream_unary(self, request_iterator, _context):
                requests = []
                async for request in request_iterator:
                    requests.append(request)

                    assert_active_metrics_incremented(
                        metric_reader, method_name, [RPC_SERVER_ACTIVE_REQUESTS]
                    )
                    active_requests = get_metric_by_name(
                        metric_reader, RPC_SERVER_ACTIVE_REQUESTS
                    )
                    if active_requests is not None:
                        active_requests_values.append(
                            sum(
                                dp.value
                                for dp in active_requests.data.data_points
                                if dict(dp.attributes).get("rpc.method")
                                == "ActiveRequestsPerRequestTest"
                            )
                        )

                return f"Got {len(requests)} requests"

        handler = TestHandler()

        async def gen():
            yield "request_1"
            yield "request_2"

        stream_request_iterator = gen()

        wrapped_handler = interceptor._wrap_stream_unary(
            handler.stream_unary, method_name
        )

        await wrapped_handler(stream_request_iterator, context)
        assert all(
            value > 0 for value in active_requests_values
        ), "Active requests counter should be positive while processing requests"

        verify_active_metrics_zero(metric_reader, method_name)

    @pytest.mark.asyncio
    async def test_stream_unary_with_error_during_requests(
        self,
        interceptor,
        context,
        stream_request_iterator,
        grpc_invalid_arg_error,
        setup_record_mocks,
    ):
        mock_record_success, mock_record_error = setup_record_mocks
        method_name = "/test.Service/StreamUnaryWithRequestErrorTest"

        class RequestErrorHandler(MockStreamHandlerWithRequestError):
            def __init__(self):
                super().__init__(grpc_invalid_arg_error)

        handler = RequestErrorHandler()

        wrapped_handler = interceptor._wrap_stream_unary(
            handler.stream_unary, method_name
        )

        with pytest.raises(Exception) as exc_info:
            await wrapped_handler(stream_request_iterator, context)

        assert exc_info.value == grpc_invalid_arg_error

        assert_error_recorded(
            mock_record_success, mock_record_error, method_name, is_streaming=True
        )

    @pytest.mark.asyncio
    async def test_handles_rpc_error(
        self,
        interceptor,
        context,
        method_name,
        stream_request_iterator,
        mock_rpc_error,
        mocker,
    ):
        mock_rpc_err = mocker.patch.object(interceptor, "_handle_rpc_error")

        class ErrorHandler(MockHandler):
            def __init__(self):
                super().__init__(request_streaming=True)

            async def stream_unary(self, request_iterator, _context):
                raise mock_rpc_error

        handler = ErrorHandler()

        wrapped_handler = interceptor._wrap_stream_unary(
            handler.stream_unary, method_name
        )

        with pytest.raises(AioRpcError):
            await wrapped_handler(stream_request_iterator, context)

        mock_rpc_err.assert_called_once()


class TestStreamStreamWrapper:
    @pytest.mark.asyncio
    async def test_records_metrics_and_message_counts(
        self,
        interceptor,
        context,
        stream_request_iterator,
        setup_record_mocks,
        stream_stream_handler,
    ):
        mock_record_success, mock_record_error = setup_record_mocks
        method_name = "/test.Service/TestBidiStreamMethod"

        wrapped_handler = interceptor._wrap_stream_stream(
            stream_stream_handler.stream_stream, method_name
        )

        responses = []
        async for response in wrapped_handler(stream_request_iterator, context):
            responses.append(response)

        assert responses == [
            "Response for stream_request_1",
            "Response for stream_request_2",
        ]

        assert_success_recorded(
            mock_record_success,
            mock_record_error,
            method_name,
            context.code(),
            is_streaming=True,
        )

    @pytest.mark.asyncio
    async def test_message_count_metrics_record_exact_counts(
        self,
        interceptor,
        meter,
        context,
        stream_request_iterator,
        setup_record_mocks,
        stream_stream_handler,
    ):
        mock_record_success, mock_record_error = setup_record_mocks
        _, metric_reader = meter
        method_name = "/test.Service/MessageCountTest"

        wrapped_handler = interceptor._wrap_stream_stream(
            stream_stream_handler.stream_stream, method_name
        )

        _ = []
        async for response in wrapped_handler(stream_request_iterator, context):
            _.append(response)

        verify_metric_for_method(
            metric_reader, "rpc.server.requests_per_rpc", method_name, expected_value=2
        )
        verify_metric_for_method(
            metric_reader, "rpc.server.responses_per_rpc", method_name, expected_value=2
        )

        assert_success_recorded(
            mock_record_success,
            mock_record_error,
            method_name,
            context.code(),
            is_streaming=True,
        )

    @pytest.mark.asyncio
    async def test_active_metrics_decremented_after_completion(
        self, interceptor, meter, context, stream_request_iterator, mocker
    ):
        mocker.patch.object(interceptor, "_record_success")
        mocker.patch.object(interceptor, "_record_error")

        _, metric_reader = meter
        method_name = "/test.Service/ActiveMetricsDecrementTest"

        class TestHandler(MockHandler):
            async def stream_stream(self, request_iterator, _context):
                async for request in request_iterator:
                    assert_active_metrics_incremented(
                        metric_reader,
                        method_name,
                        [
                            RPC_SERVER_ACTIVE_METHODS,
                            RPC_SERVER_ACTIVE_REQUESTS,
                            RPC_SERVER_ACTIVE_RESPONSES,
                        ],
                    )
                    yield f"Response for {request}"

        handler = TestHandler()
        wrapped_handler = interceptor._wrap_stream_stream(
            handler.stream_stream, method_name
        )

        responses = []
        async for response in wrapped_handler(stream_request_iterator, context):
            responses.append(response)

        verify_active_metrics_zero(metric_reader, method_name)

    @pytest.mark.asyncio
    async def test_stream_stream_with_early_error(
        self,
        interceptor,
        context,
        stream_request_iterator,
        grpc_internal_error,
        setup_record_mocks,
    ):
        mock_record_success, mock_record_error = setup_record_mocks
        method_name = "/test.Service/StreamStreamEarlyErrorTest"

        class EarlyErrorHandler(MockHandlerWithError):
            def __init__(self):
                super().__init__(
                    grpc_internal_error, request_streaming=True, response_streaming=True
                )

        handler = EarlyErrorHandler()
        wrapped_handler = interceptor._wrap_stream_stream(
            handler.stream_stream, method_name
        )

        # Call the handler expecting an error
        with pytest.raises(Exception) as exc_info:
            async for _ in wrapped_handler(stream_request_iterator, context):
                pass

        assert exc_info.value == grpc_internal_error

        # Verify recording methods were called correctly
        assert_error_recorded(
            mock_record_success, mock_record_error, method_name, is_streaming=True
        )

    @pytest.mark.asyncio
    async def test_stream_stream_with_error_in_response_iterator(
        self,
        interceptor,
        context,
        stream_request_iterator,
        grpc_internal_error,
        setup_record_mocks,
    ):
        mock_record_success, mock_record_error = setup_record_mocks
        method_name = "/test.Service/StreamStreamResponseErrorTest"

        class ResponseErrorHandler(MockHandler):
            def __init__(self):
                super().__init__(request_streaming=True, response_streaming=True)

            async def stream_stream(self, request_iterator, _context):
                requests = []
                async for request in request_iterator:
                    requests.append(request)

                yield "First response"
                raise grpc_internal_error

        handler = ResponseErrorHandler()
        wrapped_handler = interceptor._wrap_stream_stream(
            handler.stream_stream, method_name
        )

        responses = []
        with pytest.raises(Exception) as exc_info:
            async for response in wrapped_handler(stream_request_iterator, context):
                responses.append(response)

        assert exc_info.value == grpc_internal_error
        assert len(responses) == 1

        assert_error_recorded(
            mock_record_success, mock_record_error, method_name, is_streaming=True
        )

    @pytest.mark.asyncio
    async def test_handles_rpc_error(
        self,
        interceptor,
        context,
        method_name,
        stream_request_iterator,
        mock_rpc_error,
        mocker,
    ):
        mock_rpc_err = mocker.patch.object(interceptor, "_handle_rpc_error")

        class ErrorHandler(MockHandler):
            def __init__(self):
                super().__init__(request_streaming=True, response_streaming=True)

            async def stream_stream(self, request_iterator, _context):
                raise mock_rpc_error
                # noinspection PyUnreachableCode
                yield "This will not be reached"

        handler = ErrorHandler()

        wrapped_handler = interceptor._wrap_stream_stream(
            handler.stream_stream, method_name
        )

        with pytest.raises(AioRpcError):
            async for _ in wrapped_handler(stream_request_iterator, context):
                pass

        mock_rpc_err.assert_called_once()

    @pytest.mark.asyncio
    async def test_metrics_recorded_in_finally_block(
        self,
        interceptor,
        meter,
        context,
        stream_request_iterator,
        mocker,
        stream_stream_handler,
    ):
        mocker.patch.object(interceptor, "_record_success")
        mock_record_error = mocker.patch.object(interceptor, "_record_error")
        mock_record_error.side_effect = RuntimeError("Error in _record_error")

        _, metric_reader = meter
        method_name = "/test.Service/FinallyBlockTest"

        custom_error = RuntimeError("Error that bypasses normal metric recording")

        failing_context = MagicMock()
        failing_context.code.side_effect = custom_error

        wrapped_handler = interceptor._wrap_stream_stream(
            stream_stream_handler.stream_stream, method_name
        )

        with pytest.raises(RuntimeError):
            async for _ in wrapped_handler(stream_request_iterator, failing_context):
                pass

        verify_metric_for_method(
            metric_reader, "rpc.server.requests_per_rpc", method_name
        )


class TestRecordFunctions:

    @pytest.mark.asyncio
    async def test_record_success_adds_calls_and_duration(self, interceptor, meter):
        _, metric_reader = meter
        method_name = "/test.Service/SuccessTest"
        start_time = time.time() - 1  # 1 second ago

        interceptor._record_success(
            method_name, StatusCode.OK, start_time, is_streaming=False
        )

        verify_metric_for_method(metric_reader, RPC_SERVER_CALLS, method_name)
        verify_duration_metrics(metric_reader, method_name, expect_duration=True)

    @pytest.mark.asyncio
    async def test_record_success_no_duration_for_streaming(self, interceptor, meter):
        _, metric_reader = meter
        method_name = "/test.Service/StreamingSuccessTest"
        start_time = time.time() - 0.5  # 0.5 seconds ago

        interceptor._record_success(
            method_name, StatusCode.OK, start_time, is_streaming=True
        )

        verify_metric_for_method(metric_reader, RPC_SERVER_CALLS, method_name)
        verify_duration_metrics(metric_reader, method_name, expect_duration=False)

    @pytest.mark.asyncio
    async def test_record_error_sets_error_attribute(self, interceptor, meter):
        _, metric_reader = meter
        method_name = "/test.Service/ErrorTest"
        start_time = time.time() - 0.1

        interceptor._record_error(
            method_name, StatusCode.INTERNAL, start_time, is_streaming=False
        )

        # Use the helper to verify error attributes
        verify_error_attributes(
            metric_reader, method_name, StatusCode.INTERNAL, expect_server_error=True
        )

    @pytest.mark.asyncio
    async def test_record_error_sets_status_code(self, interceptor, meter):
        _, metric_reader = meter
        method_name = "/test.Service/InvalidArgTest"
        start_time = time.time() - 0.2

        interceptor._record_error(
            method_name, StatusCode.INVALID_ARGUMENT, start_time, is_streaming=False
        )

        verify_error_attributes(
            metric_reader,
            method_name,
            StatusCode.INVALID_ARGUMENT,
            expect_server_error=False,
        )


class TestHandleRpcError:

    @pytest.mark.asyncio
    async def test_handle_cancelled_error_as_success(self, interceptor, mocker):
        mock_record_success = mocker.patch.object(interceptor, "_record_success")
        mock_record_error = mocker.patch.object(interceptor, "_record_error")

        error = MagicMock(spec=AioRpcError)
        error.code.return_value = StatusCode.CANCELLED

        method_name = "/test.Service/CancelledTest"
        start_time = time.time() - 0.5

        interceptor._handle_rpc_error(
            error=error,
            method_name=method_name,
            start_time=start_time,
            is_streaming=False,
        )

        mock_record_success.assert_called_with(
            method_name, StatusCode.CANCELLED, start_time, False
        )
        mock_record_error.assert_not_called()

    @pytest.mark.asyncio
    async def test_handles_rpc_errors_as_error(self, interceptor, mocker):
        mock_record_success = mocker.patch.object(interceptor, "_record_success")
        mock_record_error = mocker.patch.object(interceptor, "_record_error")

        error = MagicMock(spec=AioRpcError)
        error.code.return_value = StatusCode.INTERNAL

        method_name = "/test.Service/ErrorTest"
        start_time = time.time() - 0.5

        interceptor._handle_rpc_error(
            error=error,
            method_name=method_name,
            start_time=start_time,
            is_streaming=False,
        )

        mock_record_error.assert_called_with(
            method_name, StatusCode.INTERNAL, start_time, False
        )
        mock_record_success.assert_not_called()

    @pytest.mark.asyncio
    async def test_streaming_metrics_recorded(self, interceptor, mocker):
        mock_record_error = mocker.patch.object(interceptor, "_record_error")

        mock_requests_per_rpc = mocker.patch.object(interceptor, "requests_per_rpc")
        mock_responses_per_rpc = mocker.patch.object(interceptor, "responses_per_rpc")

        error = MagicMock(spec=AioRpcError)
        error.code.return_value = StatusCode.DEADLINE_EXCEEDED

        method_name = "/test.Service/StreamingErrorTest"
        start_time = time.time() - 0.5

        interceptor._handle_rpc_error(
            error=error,
            method_name=method_name,
            start_time=start_time,
            is_streaming=True,
            request_count=3,
            response_count=2,
        )

        mock_requests_per_rpc.record.assert_called_with(3, ANY)
        mock_responses_per_rpc.record.assert_called_with(2, ANY)

        mock_record_error.assert_called_with(
            method_name, StatusCode.DEADLINE_EXCEEDED, start_time, True
        )

    @pytest.mark.asyncio
    async def test_no_streaming_metrics_for_zero_counts(self, interceptor, mocker):
        mock_record_error = mocker.patch.object(interceptor, "_record_error")

        mock_requests_per_rpc = mocker.patch.object(interceptor, "requests_per_rpc")
        mock_responses_per_rpc = mocker.patch.object(interceptor, "responses_per_rpc")

        error = MagicMock(spec=AioRpcError)
        error.code.return_value = StatusCode.FAILED_PRECONDITION

        method_name = "/test.Service/EarlyErrorTest"
        start_time = time.time() - 0.1

        interceptor._handle_rpc_error(
            error=error,
            method_name=method_name,
            start_time=start_time,
            is_streaming=True,
            request_count=0,
            response_count=0,
        )

        mock_requests_per_rpc.record.assert_not_called()
        mock_responses_per_rpc.record.assert_not_called()

        mock_record_error.assert_called_with(
            method_name, StatusCode.FAILED_PRECONDITION, start_time, True
        )
