# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Custom tracing middleware for HTTP requests."""

from typing import Callable, List

from fastapi import Request, Response
from opentelemetry import trace
from notification_common.config.common_settings import common_settings
from notification_common.config.metadata import project_name


def create_tracing_middleware(
    excluded_paths: List[str] | None = None,
):
    """
    Create a tracing middleware with configurable excluded paths.

    :param excluded_paths: List of paths to exclude from tracing
    :return: Middleware function
    """
    if excluded_paths is None:
        excluded_paths = [
            "/api/v1/health",
            "/api/v1beta/health",
            "/docs",
            "/openapi.json",
            "/redoc",
        ]

    async def tracing_middleware(request: Request, call_next: Callable) -> Response:
        """
        Custom tracing middleware that creates spans for HTTP requests.
        """
        # Skip tracing if tracing is disabled
        if not common_settings.tracing_enabled:
            return await call_next(request)

        # Skip tracing entirely for excluded paths - no spans created
        if request.url.path in excluded_paths:
            return await call_next(request)

        tracer = trace.get_tracer(project_name())

        span_name = f"HTTP {request.method} {request.url.path}"

        with tracer.start_as_current_span(span_name) as span:
            span.set_attribute("http.method", request.method)
            # Scheme, host and path only. A query string routinely carries
            # credentials -- a bearer token, a signed-URL signature, a
            # reconnect token -- and OpenTelemetry's own semantic conventions
            # warn that `http.url`/`url.full` may hold sensitive values, so a
            # shared middleware that cannot know what any one service puts in
            # a query defaults to leaving it out rather than exporting it to
            # the trace backend. `replace(query="")` drops the `?` with it.
            span.set_attribute("http.url", str(request.url.replace(query="")))
            span.set_attribute("http.route", request.url.path)

            try:
                response = await call_next(request)

                span.set_attribute("http.status_code", response.status_code)

                return response

            except Exception as e:
                span.record_exception(e)
                raise

    return tracing_middleware
