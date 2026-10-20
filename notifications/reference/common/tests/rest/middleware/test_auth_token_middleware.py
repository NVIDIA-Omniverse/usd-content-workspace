# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import json
import logging
from contextvars import ContextVar
from unittest.mock import AsyncMock, MagicMock

import pytest
import pytest_asyncio
from fastapi import Request
from notification_common.auth import auth_context
from notification_common.auth import principal_id
from notification_common.auth.principal_source import PrincipalSource
from notification_common.config.auth_settings import auth_settings
from notification_common.errors.errors import AuthenticationError, ServerError

from notification_common.rest.middleware.auth_token_middleware import (
    create_auth_token_middleware,
)

#: The principal a resolvable token resolves to.
A_PRINCIPAL = "a-principal"

#: The client-facing message an unresolvable token is refused with.
REFUSAL_MESSAGE = "Invalid or expired JWT"

#: The paths the middleware bypasses when the caller names none of its own.
#: The docs pages are served to a browser that carries no Authorization
#: header, and each one of these fetches a sibling of its own: Swagger UI
#: fetches /openapi.json and completes an OAuth2 flow through
#: /docs/oauth2-redirect, ReDoc is served from /redoc. A sibling left off this
#: list is a docs page that 401s in a deployment with auth on.
DEFAULT_EXCLUDED_PATHS = [
    "/api/v1/health",
    "/api/v1beta/health",
    "/docs",
    "/docs/oauth2-redirect",
    "/openapi.json",
    "/redoc",
]


def _warnings(caplog):
    """Return the WARNING records the middleware emitted.

    Callers request the ``real_get_logger`` fixture, which puts the stdlib
    ``logging.getLogger`` back for the test -- without it caplog installs its
    handler on a MagicMock and every read here is empty.
    """
    return [
        record
        for record in caplog.records
        if record.levelno == logging.WARNING
        and record.name == "notification_common.rest.middleware.auth_token_middleware"
    ]


def _errors(caplog):
    """Return the ERROR-or-worse records the middleware emitted."""
    return [
        record
        for record in caplog.records
        if record.levelno >= logging.ERROR
        and record.name == "notification_common.rest.middleware.auth_token_middleware"
    ]


@pytest_asyncio.fixture
async def mock_request():
    mock = MagicMock(spec=Request)
    mock.headers = {}
    return mock


@pytest_asyncio.fixture
async def mock_call_next():
    mock = AsyncMock()
    mock.return_value = "response"
    return mock


@pytest.fixture
def resolve_principal(mocker):
    """
    Replace the principal resolution the middleware performs.

    Patched on ``notification_common.auth.principal_id`` -- the module object
    the middleware imported -- so it stays in force however the middleware
    spells the call.
    """

    def _patch(**kwargs):
        return mocker.patch.object(
            principal_id, "principal_id", autospec=True, **kwargs
        )

    return _patch


@pytest.mark.asyncio
class TestValidTokenExtraction:
    @pytest.fixture
    def mock_request_header(self):
        return {"Authorization": "Bearer test_token_123"}

    @pytest.fixture(autouse=True)
    def mock_auth_token_var(self, mocker):
        mock_var = MagicMock(spec=ContextVar)
        mock_var.set = MagicMock()
        mocker.patch.object(
            auth_context,
            "auth_token_var",
            new=mock_var,
        )
        return mock_var

    @pytest.fixture(autouse=True)
    def resolvable_token(self, resolve_principal):
        """
        Resolve every token in this class.

        The fixture above swaps ``auth_context.auth_token_var`` for a
        MagicMock, so the real resolution would read an unset ContextVar and
        raise ``LookupError`` rather than exercising anything. These rows are
        about extraction and the context var; the rows about the resolution
        itself are in ``TestTokenResolution`` below.
        """
        return resolve_principal(return_value=A_PRINCIPAL)

    async def test_sets_context_var(
        self, mock_request, mock_call_next, mock_auth_token_var, mock_request_header
    ):
        mock_request.headers = mock_request_header

        middleware = create_auth_token_middleware()
        await middleware(mock_request, mock_call_next)

        mock_auth_token_var.set.assert_called_once_with("test_token_123")

    async def test_calls_next(self, mock_request, mock_call_next, mock_request_header):
        mock_request.headers = mock_request_header

        middleware = create_auth_token_middleware()
        response = await middleware(mock_request, mock_call_next)

        mock_call_next.assert_called_once_with(mock_request)
        assert response == "response"


@pytest.mark.asyncio
class TestTokenResolution:
    """
    Extracting the token proves nothing about it, so the middleware also
    resolves it -- through the configured principal source -- before the
    request reaches a route. An unresolvable token is the caller's fault and
    is refused; anything else is ours and must not be reported to the caller
    as a bad token.
    """

    @pytest.fixture(autouse=True)
    def authorized_request(self, mock_request):
        mock_request.headers = {"Authorization": "Bearer test_token_123"}
        return mock_request

    @pytest.fixture(autouse=True)
    def mock_auth_token_var(self, mocker):
        mock_var = MagicMock(spec=ContextVar)
        mock_var.set = MagicMock()
        mocker.patch.object(auth_context, "auth_token_var", new=mock_var)
        return mock_var

    @pytest.fixture
    def middleware(self):
        return create_auth_token_middleware()

    async def test_a_resolvable_token_reaches_the_route(
        self, middleware, mock_request, mock_call_next, resolve_principal
    ):
        resolve_principal(return_value=A_PRINCIPAL)

        await middleware(mock_request, mock_call_next)

        mock_call_next.assert_awaited_once_with(mock_request)

    class TestWhenTheTokenIsUnresolvable:
        @pytest.fixture(autouse=True)
        def unresolvable_token(self, resolve_principal):
            return resolve_principal(side_effect=AuthenticationError(REFUSAL_MESSAGE))

        async def test_returns_401(self, middleware, mock_request, mock_call_next):
            response = await middleware(mock_request, mock_call_next)

            assert response.status_code == 401

        async def test_returns_the_refusal_message(
            self, middleware, mock_request, mock_call_next
        ):
            response = await middleware(mock_request, mock_call_next)

            assert json.loads(response.body) == {"detail": REFUSAL_MESSAGE}

        async def test_never_reaches_the_route(
            self, middleware, mock_request, mock_call_next
        ):
            # The ordering pin: refusing after the route ran would still
            # answer 401, having already done the work.
            await middleware(mock_request, mock_call_next)

            mock_call_next.assert_not_called()

        async def test_logs_one_warning(
            self, middleware, mock_request, mock_call_next, real_get_logger, caplog
        ):
            await middleware(mock_request, mock_call_next)

            assert len(_warnings(caplog)) == 1

        async def test_logs_no_traceback(
            self, middleware, mock_request, mock_call_next, real_get_logger, caplog
        ):
            # A refused token is routine traffic, not a service fault:
            # logger.exception here would put a stack trace in the log for
            # every caller who sent a stale token.
            await middleware(mock_request, mock_call_next)

            assert _warnings(caplog)[0].exc_info is None

    class TestWhenTheResolutionFails:
        """A ServerError is an outage of ours, not a bad token."""

        @pytest.fixture(autouse=True)
        def failing_resolution(self, resolve_principal):
            return resolve_principal(
                side_effect=ServerError("identity provider unreachable")
            )

        async def test_propagates_the_server_error(
            self, middleware, mock_request, mock_call_next
        ):
            # It reaches the error handling middleware, which answers 500 --
            # rather than being flattened into a 401 that invites the caller
            # to fetch a new token, which cannot help.
            with pytest.raises(ServerError, match="Internal Server Error"):
                await middleware(mock_request, mock_call_next)

        async def test_never_reaches_the_route(
            self, middleware, mock_request, mock_call_next
        ):
            with pytest.raises(ServerError):
                await middleware(mock_request, mock_call_next)

            mock_call_next.assert_not_called()


@pytest.mark.asyncio
class TestWhenAuthenticationIsDisabled:
    """
    With ``principal_source`` disabled the resolution is a no-op, so a request
    that carries a token is neither resolved nor refused.

    The app only installs this middleware when authentication is enabled, so
    the combination cannot arise through ``create_app``. It is pinned by
    constructing the middleware directly because that is the whole point: the
    refusal must be gated on the setting inside the resolution, not on the
    middleware being left out of the stack. Nothing here is patched over
    ``principal_id`` -- the real resolver runs and answers None off the
    disabled source, which is the behaviour under test.
    """

    @pytest.fixture(autouse=True)
    def authentication_disabled(self, mocker):
        mocker.patch.object(auth_settings, "principal_source", PrincipalSource.DISABLED)

    async def test_a_bearer_token_reaches_the_route(self, mock_request, mock_call_next):
        # `call_next`'s own return value is the assertion: a refusal would
        # have returned a 401 JSONResponse without ever calling it.
        mock_request.headers = {"Authorization": "Bearer test_token_123"}

        middleware = create_auth_token_middleware()
        response = await middleware(mock_request, mock_call_next)

        assert response == "response"


@pytest.mark.asyncio
class TestInvalidAuthHeader:

    async def test_missing_auth_header_returns_401(self, mock_request, mock_call_next):
        mock_request.headers = {}

        middleware = create_auth_token_middleware()
        result = await middleware(mock_request, mock_call_next)

        assert result.status_code == 401

    async def test_missing_bearer_prefix_logs_one_warning(
        self, mock_request, mock_call_next, real_get_logger, caplog
    ):
        # A caller's malformed header is routine traffic, not a fault of
        # ours: at ERROR it pages whoever alerts on the error rate for
        # something no operator can act on.
        mock_request.headers = {"Authorization": "Basic dXNlcjpwYXNz"}

        middleware = create_auth_token_middleware()
        await middleware(mock_request, mock_call_next)

        assert len(_warnings(caplog)) == 1

    async def test_missing_bearer_prefix_logs_no_error(
        self, mock_request, mock_call_next, real_get_logger, caplog
    ):
        mock_request.headers = {"Authorization": "Basic dXNlcjpwYXNz"}

        middleware = create_auth_token_middleware()
        await middleware(mock_request, mock_call_next)

        # A negative assertion also passes when nothing is captured, so it
        # depends on real_get_logger; the WARNING tests above prove capture.
        assert _errors(caplog) == []

    @pytest.mark.asyncio
    async def test_invalid_auth_header_format_returns_401(
        self, mock_request, mock_call_next
    ):
        mock_request.headers = {"Authorization": "Basic dXNlcjpwYXNz"}

        middleware = create_auth_token_middleware()
        result = await middleware(mock_request, mock_call_next)

        assert result.status_code == 401


@pytest.mark.asyncio
class TestHealthAndDocsEndpointsBypass:
    @pytest.mark.parametrize("path", DEFAULT_EXCLUDED_PATHS)
    async def test_bypasses_auth_for_whitelisted_paths(
        self, mock_request, mock_call_next, path
    ):
        mock_request.url = MagicMock()
        mock_request.url.path = path
        mock_request.headers = {}

        middleware = create_auth_token_middleware()
        response = await middleware(mock_request, mock_call_next)

        mock_call_next.assert_called_once_with(mock_request)
        assert response == "response"

    @pytest.mark.parametrize("path", DEFAULT_EXCLUDED_PATHS)
    async def test_resolves_no_token_for_whitelisted_paths(
        self, mock_request, mock_call_next, resolve_principal, path
    ):
        # A bypassed path must not reach the resolution either: these requests
        # carry no token, so resolving would raise on the way to a page that
        # is meant to be public.
        resolve = resolve_principal(return_value=A_PRINCIPAL)
        mock_request.url = MagicMock()
        mock_request.url.path = path
        mock_request.headers = {}

        middleware = create_auth_token_middleware()
        await middleware(mock_request, mock_call_next)

        resolve.assert_not_called()

    async def test_custom_excluded_paths(self, mock_request, mock_call_next):
        """Test that custom excluded paths work."""
        mock_request.url = MagicMock()
        mock_request.url.path = "/custom/path"
        mock_request.headers = {}

        middleware = create_auth_token_middleware(excluded_paths=["/custom/path"])
        response = await middleware(mock_request, mock_call_next)

        mock_call_next.assert_called_once_with(mock_request)
        assert response == "response"
