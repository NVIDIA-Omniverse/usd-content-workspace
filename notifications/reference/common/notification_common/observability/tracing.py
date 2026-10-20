# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import functools
from typing import Any, Callable
from contextlib import contextmanager

from opentelemetry import trace

from notification_common.config.common_settings import common_settings
from notification_common.config.metadata import project_name


def add_current_span_attributes(**attributes):
    _add_attributes_to_span(trace.get_current_span(), **attributes)


def add_current_span_event(event_name: str, **attributes):
    current_span = trace.get_current_span()
    if current_span and current_span.is_recording():
        current_span.add_event(
            event_name,
            attributes=attributes,
        )


def trace_async_function(
    span_name: str | None = None,
    attributes: dict[str, Any] | None = None,
):
    """
    Decorator for tracing async functions with custom span names and attributes.

    Creates a new span for the decorated function and automatically handles
    exceptions by recording them in the span.
    """

    def decorator(func: Callable):
        @functools.wraps(func)
        async def wrapper(*args, **kwargs):

            if not common_settings.tracing_enabled:
                return await func(*args, **kwargs)

            tracer = trace.get_tracer(project_name())
            name = span_name or f"{func.__name__}"

            with tracer.start_as_current_span(name) as span:
                if attributes:
                    _add_attributes_to_span(span, **attributes)

                try:
                    result = await func(*args, **kwargs)
                    return result
                except Exception as e:
                    span.record_exception(e)
                    raise

        return wrapper

    return decorator


@contextmanager
def trace_operation(span_name: str, **attributes):
    """
    Context manager for tracing a specific operation within a function.

    This creates a child span for the operation and allows adding custom attributes.
    Useful for tracing specific parts of a function or operation.
    """
    if not common_settings.tracing_enabled:
        yield
        return

    tracer = trace.get_tracer(project_name())
    with tracer.start_as_current_span(span_name) as span:
        _add_attributes_to_span(span, **attributes)
        yield span


def _add_attributes_to_span(span: trace.Span, **attributes) -> None:
    if span and span.is_recording():
        for key, value in attributes.items():

            span.set_attribute(key, value)
