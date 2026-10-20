# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Specialized tracing decorators for the event consumer service.

These decorators are tailored for specific streaming patterns used in this service:
- trace_sse_event_stream: For SSE and other message streaming scenarios
- trace_grpc_event_stream: For gRPC streaming methods
"""

import functools
from typing import Any, AsyncGenerator, Callable

from notification_common.config.common_settings import common_settings
from notification_common.config.metadata import project_name
from opentelemetry import trace


def trace_sse_event_stream(
    stream_name: str | None = None,
    attributes: dict[str, Any] | None = None,
):
    """
    Decorator for tracing SSE message streams.

    Creates a span for the entire stream lifecycle and adds events for each message.
    """

    def decorator(func: Callable):
        name = stream_name or f"{func.__name__}_stream"

        def sse_event_handler(item):
            if isinstance(item, dict):
                return {
                    "event_type": item.get("event", "unknown"),
                    "event_content": str(item),
                }
            return {"event_content": str(item)}

        return _trace_message_stream(func, name, attributes, sse_event_handler)

    return decorator


def trace_grpc_event_stream(
    stream_name: str | None = None,
    attributes: dict[str, Any] | None = None,
):
    """
    Decorator for tracing gRPC streaming methods.

    Creates a span for the entire stream lifecycle and adds events for each response.
    """

    def decorator(func: Callable):
        name = stream_name or f"grpc.{func.__name__}"

        def grpc_event_handler(response):
            count = len(response.events) if hasattr(response, "events") else 0
            event_attrs: dict[str, Any] = {"event_count": count}

            if hasattr(response, "events") and response.events:
                events_content = [str(event) for event in response.events]
                event_attrs["events_content"] = events_content

            return event_attrs

        return _trace_message_stream(func, name, attributes, grpc_event_handler)

    return decorator


def _trace_message_stream(
    func: Callable,
    stream_name: str,
    attributes: dict[str, Any] | None,
    event_handler: Callable[[Any], dict[str, Any]],
) -> Callable:

    @functools.wraps(func)
    async def wrapper(*args, **kwargs) -> AsyncGenerator:
        if not common_settings.tracing_enabled:
            async for item in func(*args, **kwargs):
                yield item
            return

        tracer = trace.get_tracer(project_name())

        with tracer.start_as_current_span(stream_name) as span:
            if attributes:
                for key, value in attributes.items():
                    span.set_attribute(key, str(value))

            count = 0
            try:
                async for item in func(*args, **kwargs):
                    count += 1
                    event_attributes = event_handler(item)
                    span.add_event("message_streamed", event_attributes)
                    yield item

            except Exception as e:
                span.record_exception(e)
                raise
            finally:
                span.set_attribute("message_count", count)

    return wrapper
