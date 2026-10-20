# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import time
import grpc
from typing import Any, AsyncIterator, Dict

from grpc.aio import ServerInterceptor, ServicerContext

from opentelemetry.metrics import Counter, Histogram, UpDownCounter
from grpc import StatusCode

from notification_common.config.observability_config import EXCLUDE_METHODS
from notification_common.interceptors.metrics.labels import (
    ATTR_RPC_GRPC_SERVER_ERROR,
    create_labels,
    create_status_labels,
    SERVER_ERROR_CODES,
)
from notification_common.interceptors.metrics.message_size import get_message_size
from opentelemetry.semconv._incubating.metrics.rpc_metrics import (
    create_rpc_server_request_size,
    create_rpc_server_requests_per_rpc,
    create_rpc_server_response_size,
    create_rpc_server_responses_per_rpc,
    create_rpc_server_duration,
)

RPC_SERVER_ACTIVE_REQUESTS = "rpc.server.active_requests"
RPC_SERVER_CALLS = "rpc.server.calls"
RPC_SERVER_ACTIVE_METHODS = "rpc.server.active_methods"
RPC_SERVER_ACTIVE_RESPONSES = "rpc.server.active_responses"


class ServerMetricsInterceptor(ServerInterceptor):
    def __init__(self, meter):
        self.meter = meter

        # Request metrics
        self.request_size: Histogram = create_rpc_server_request_size(meter)
        self.requests_per_rpc: Histogram = create_rpc_server_requests_per_rpc(meter)
        self.active_requests: UpDownCounter = meter.create_up_down_counter(
            RPC_SERVER_ACTIVE_REQUESTS,
            unit="{request}",
            description="In-flight requests",
        )

        # Response metrics
        self.response_size: Histogram = create_rpc_server_response_size(meter)
        self.responses_per_rpc: Histogram = create_rpc_server_responses_per_rpc(meter)
        self.calls: Counter = meter.create_counter(
            RPC_SERVER_CALLS, unit="{count}", description="Total gRPC calls"
        )
        self.active_methods: UpDownCounter = meter.create_up_down_counter(
            RPC_SERVER_ACTIVE_METHODS, unit="{method}", description="In-flight methods"
        )
        self.active_responses: UpDownCounter = meter.create_up_down_counter(
            RPC_SERVER_ACTIVE_RESPONSES,
            unit="{response}",
            description="In-flight responses",
        )
        self.duration: Histogram = create_rpc_server_duration(meter)

    async def intercept_service(self, continuation, call_details):
        method_name = call_details.method

        if method_name in EXCLUDE_METHODS:
            return await continuation(call_details)

        handler = await continuation(call_details)
        if handler is None:
            return None

        if handler.request_streaming and handler.response_streaming:
            return grpc.stream_stream_rpc_method_handler(
                self._wrap_stream_stream(handler.stream_stream, method_name),
                handler.request_deserializer,
                handler.response_serializer,
            )
        elif handler.request_streaming:
            return grpc.stream_unary_rpc_method_handler(
                self._wrap_stream_unary(handler.stream_unary, method_name),
                handler.request_deserializer,
                handler.response_serializer,
            )
        elif handler.response_streaming:
            return grpc.unary_stream_rpc_method_handler(
                self._wrap_unary_stream(handler.unary_stream, method_name),
                handler.request_deserializer,
                handler.response_serializer,
            )
        else:
            return grpc.unary_unary_rpc_method_handler(
                self._wrap_unary_unary(handler.unary_unary, method_name),
                handler.request_deserializer,
                handler.response_serializer,
            )

    def _wrap_unary_unary(self, handler, method_name: str):
        async def wrapper(request, context: ServicerContext):
            labels = create_labels(method_name)
            start = time.time()
            self.active_methods.add(1, labels)
            self.active_requests.add(1, labels)

            try:
                self._record_request_size(request, labels)
                response = await handler(request, context)
                self._record_response_size(response, labels)
                self.responses_per_rpc.record(1, labels)
                self._record_success(
                    method_name, context.code(), start, is_streaming=False
                )
                return response
            except grpc.aio.AioRpcError as e:
                self._handle_rpc_error(e, method_name, start, is_streaming=False)
                raise
            except Exception:
                self._record_error(
                    method_name, StatusCode.UNKNOWN, start, is_streaming=False
                )
                raise
            finally:
                self.active_methods.add(-1, labels)
                self.active_requests.add(-1, labels)

        return wrapper

    def _wrap_unary_stream(self, handler, method_name: str):
        async def wrapper(request, context: ServicerContext):
            labels = create_labels(method_name)
            start = time.time()
            self.active_methods.add(1, labels)
            self.active_requests.add(1, labels)
            self.active_responses.add(1, labels)

            try:
                self._record_request_size(request, labels)
                response_count = 0
                async for response in handler(request, context):
                    response_count += 1
                    self._record_response_size(response, labels)
                    yield response

                self.responses_per_rpc.record(response_count, labels)
                self._record_success(
                    method_name, context.code(), start, is_streaming=True
                )
            except grpc.aio.AioRpcError as e:
                self._handle_rpc_error(
                    e,
                    method_name,
                    start,
                    is_streaming=True,
                    response_count=response_count,
                )
                raise
            except Exception:
                self._record_error(
                    method_name, StatusCode.UNKNOWN, start, is_streaming=True
                )
                raise
            finally:
                self.active_methods.add(-1, labels)
                self.active_requests.add(-1, labels)
                self.active_responses.add(-1, labels)

        return wrapper

    def _wrap_stream_unary(self, handler, method_name: str):
        async def wrapper(request_iterator: AsyncIterator, context: ServicerContext):
            labels = create_labels(method_name)
            start = time.time()
            self.active_methods.add(1, labels)

            try:
                request_count = 0

                async def instrumented_iterator():
                    nonlocal request_count
                    async for request in request_iterator:
                        request_count += 1
                        self._record_request_size(request, labels)
                        self.active_requests.add(1, labels)
                        yield request
                        self.active_requests.add(-1, labels)

                response = await handler(instrumented_iterator(), context)
                self._record_response_size(response, labels)

                self.responses_per_rpc.record(1, labels)
                self.requests_per_rpc.record(request_count, labels)
                self._record_success(
                    method_name, context.code(), start, is_streaming=True
                )
                return response
            except grpc.aio.AioRpcError as e:
                self._handle_rpc_error(
                    e,
                    method_name,
                    start,
                    is_streaming=True,
                    request_count=request_count,
                )
                raise
            except Exception:
                self._record_error(
                    method_name, StatusCode.UNKNOWN, start, is_streaming=True
                )
                raise
            finally:
                self.active_methods.add(-1, labels)

        return wrapper

    def _wrap_stream_stream(self, handler, method_name: str):
        async def wrapper(request_iterator: AsyncIterator, context: ServicerContext):
            labels = create_labels(method_name)
            start = time.time()
            self.active_methods.add(1, labels)
            self.active_responses.add(1, labels)

            request_count = 0
            response_count = 0
            metrics_recorded = False

            try:

                async def instrumented_requests():
                    nonlocal request_count
                    async for request in request_iterator:
                        request_count += 1
                        self._record_request_size(request, labels)
                        self.active_requests.add(1, labels)
                        yield request
                        self.active_requests.add(-1, labels)

                async for response in handler(instrumented_requests(), context):
                    response_count += 1
                    self._record_response_size(response, labels)
                    yield response

                self.requests_per_rpc.record(request_count, labels)
                self.responses_per_rpc.record(response_count, labels)
                self._record_success(
                    method_name, context.code(), start, is_streaming=True
                )
                metrics_recorded = True
            except grpc.aio.AioRpcError as e:
                self._handle_rpc_error(
                    e,
                    method_name,
                    start,
                    is_streaming=True,
                    request_count=request_count,
                    response_count=response_count,
                )
                metrics_recorded = True
                raise
            except Exception:
                self._record_error(
                    method_name, StatusCode.UNKNOWN, start, is_streaming=True
                )
                metrics_recorded = True
                raise
            finally:
                if request_count > 0 and not metrics_recorded:
                    # this is reached when the stream is cancelled
                    self.requests_per_rpc.record(request_count, labels)
                    self.responses_per_rpc.record(response_count, labels)
                    self._record_success(
                        method_name, StatusCode.CANCELLED, start, is_streaming=True
                    )

                self.active_methods.add(-1, labels)
                self.active_responses.add(-1, labels)

        return wrapper

    def _handle_rpc_error(
        self,
        error: grpc.aio.AioRpcError,
        method_name: str,
        start_time: float,
        is_streaming: bool = False,
        request_count: int = 0,
        response_count: int = 0,
    ):
        status_code = error.code()

        # Record counters for streaming RPCs if applicable
        labels = create_labels(method_name)
        if is_streaming and request_count > 0:
            self.requests_per_rpc.record(request_count, labels)

        if is_streaming and response_count > 0:
            self.responses_per_rpc.record(response_count, labels)

        if status_code == StatusCode.CANCELLED:
            self._record_success(method_name, status_code, start_time, is_streaming)
        else:
            self._record_error(method_name, status_code, start_time, is_streaming)

    def _record_request_size(self, request: Any, labels: Dict[str, str]):
        size = get_message_size(request)
        self.request_size.record(size, labels)

    def _record_response_size(self, response: Any, labels: Dict[str, str]):
        size = get_message_size(response)
        self.response_size.record(size, labels)

    def _record_success(
        self,
        method_name: str,
        code: StatusCode,
        start_time: float,
        is_streaming: bool = False,
    ):
        if code is None:
            code = StatusCode.OK

        labels = create_status_labels(method_name, code)
        self.calls.add(1, labels)
        if not is_streaming:
            duration = (time.time() - start_time) * 1000
            self.duration.record(duration, labels)

    def _record_error(
        self,
        method_name: str,
        status_code: StatusCode,
        start_time: float,
        is_streaming: bool = False,
    ):
        if status_code is None:
            status_code = StatusCode.UNKNOWN

        labels = create_status_labels(method_name, status_code)
        labels[ATTR_RPC_GRPC_SERVER_ERROR] = (
            "true"
            if status_code in SERVER_ERROR_CODES or status_code == StatusCode.UNKNOWN
            else "false"
        )

        self.calls.add(1, labels)
        if not is_streaming:
            duration = (time.time() - start_time) * 1000
            self.duration.record(duration, labels)
