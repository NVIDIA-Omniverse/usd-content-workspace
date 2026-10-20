# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import AsyncMock, MagicMock

import pytest
from fastapi import Request, Response
from starlette.datastructures import URL
from notification_common.config.common_settings import common_settings
from notification_common.rest.middleware.tracing_middleware import (
    create_tracing_middleware,
)


@pytest.fixture(autouse=True)
def mock_telemetry_enabled(mocker):
    return mocker.patch.object(common_settings, "tracing_enabled", True)


@pytest.fixture
def make_request():
    """
    A request whose ``url`` is a real ``starlette.datastructures.URL``.

    Not a MagicMock: the middleware calls ``url.replace(query="")`` to drop
    the query string, and a mock answers that with another mock -- so every
    ``http.url`` assertion would pass whether or not the query was actually
    removed.
    """

    def _make_request(path="/api/v1/resource", method="GET", query=""):
        req = MagicMock(spec=Request)
        req.method = method
        # noinspection HttpUrlsUsage
        req.url = URL(f"http://testserver{path}{query}")
        return req

    return _make_request


@pytest.mark.asyncio
async def test_no_tracing_when_telemetry_disabled(make_request, mock_tracer, mocker):
    mocker.patch.object(common_settings, "tracing_enabled", False)

    req = make_request()
    call_next = AsyncMock(return_value=Response("ok", status_code=200))

    middleware = create_tracing_middleware()
    await middleware(req, call_next)

    mock_tracer.start_as_current_span.assert_not_called()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "path", ["/api/v1/health", "/api/v1beta/health", "/docs", "/openapi.json", "/redoc"]
)
async def test_no_tracing_for_health_and_docs_endpoints(
    mocker, make_request, mock_tracer, path
):
    mocker.patch.object(common_settings, "tracing_enabled", False)

    req = make_request(path=path)
    call_next = AsyncMock(return_value=Response("ok", status_code=200))

    middleware = create_tracing_middleware()
    await middleware(req, call_next)

    mock_tracer.start_as_current_span.assert_not_called()


@pytest.mark.asyncio
async def test_tracing_span_created_with_attributes(
    mock_telemetry_enabled, make_request, mock_tracer, mock_span
):
    req = make_request(path="/api/v1/resource", method="POST")
    call_next = AsyncMock(return_value=Response("ok", status_code=201))

    mock_tracer.start_as_current_span.return_value.__enter__.return_value = mock_span
    mock_tracer.start_as_current_span.return_value.__exit__.return_value = None

    middleware = create_tracing_middleware()
    await middleware(req, call_next)

    mock_tracer.start_as_current_span.assert_called_once_with(
        "HTTP POST /api/v1/resource"
    )

    mock_span.set_attribute.assert_any_call("http.method", "POST")
    mock_span.set_attribute.assert_any_call(
        "http.url", "http://testserver/api/v1/resource"
    )
    mock_span.set_attribute.assert_any_call("http.route", "/api/v1/resource")
    mock_span.set_attribute.assert_any_call("http.status_code", 201)


@pytest.fixture
def traced_query_request(make_request, mock_tracer, mock_span):
    """
    Run the middleware over a request carrying a query string and hand
    back the span it recorded attributes on.

    The query deliberately carries a credential-shaped parameter: this
    middleware is shared, it cannot know what any one service puts in a
    query, and a query routinely carries a bearer token, a signed-URL
    signature, or a reconnect token.
    """

    async def _run():
        req = make_request(path="/api/v1/resource", query="?reconnect_token=s3cr3t&x=1")
        call_next = AsyncMock(return_value=Response("ok", status_code=200))

        mock_tracer.start_as_current_span.return_value.__enter__.return_value = (
            mock_span
        )
        mock_tracer.start_as_current_span.return_value.__exit__.return_value = None

        middleware = create_tracing_middleware()
        await middleware(req, call_next)

        return mock_span

    return _run


@pytest.mark.asyncio
async def test_query_string_is_dropped_from_the_url_attribute(traced_query_request):
    span = await traced_query_request()

    span.set_attribute.assert_any_call("http.url", "http://testserver/api/v1/resource")


@pytest.mark.asyncio
async def test_no_query_value_reaches_the_span(traced_query_request):
    span = await traced_query_request()

    assert "s3cr3t" not in str(span.set_attribute.call_args_list)


@pytest.mark.asyncio
async def test_route_is_unchanged_by_the_query_string(traced_query_request):
    span = await traced_query_request()

    span.set_attribute.assert_any_call("http.route", "/api/v1/resource")


@pytest.mark.asyncio
async def test_method_is_unchanged_by_the_query_string(traced_query_request):
    span = await traced_query_request()

    span.set_attribute.assert_any_call("http.method", "GET")


@pytest.mark.asyncio
async def test_status_code_is_unchanged_by_the_query_string(traced_query_request):
    span = await traced_query_request()

    span.set_attribute.assert_any_call("http.status_code", 200)


@pytest.mark.asyncio
async def test_exception_is_recorded_and_reraised(
    mock_telemetry_enabled, make_request, mock_tracer, mock_span
):
    req = make_request(path="/api/v1/resource")
    test_exc = RuntimeError("fail")

    async def call_next(_):
        raise test_exc

    mock_tracer.start_as_current_span.return_value.__enter__.return_value = mock_span
    mock_tracer.start_as_current_span.return_value.__exit__.return_value = None

    middleware = create_tracing_middleware()
    with pytest.raises(RuntimeError, match="fail"):
        await middleware(req, call_next)

    mock_span.record_exception.assert_called_once_with(test_exc)


@pytest.mark.asyncio
async def test_custom_excluded_paths(
    mock_telemetry_enabled, make_request, mock_tracer, mock_span
):
    """Test that custom excluded paths work."""
    req = make_request(path="/custom/exclude")
    call_next = AsyncMock(return_value=Response("ok", status_code=200))

    middleware = create_tracing_middleware(excluded_paths=["/custom/exclude"])
    await middleware(req, call_next)

    # Should not create span for an excluded path
    mock_tracer.start_as_current_span.assert_not_called()
