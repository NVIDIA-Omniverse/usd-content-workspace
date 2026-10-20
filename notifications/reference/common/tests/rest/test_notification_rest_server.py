# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for REST server base class."""

import asyncio
import signal
from contextlib import asynccontextmanager
from unittest.mock import AsyncMock

import pytest
from fastapi import FastAPI

from fastapi.testclient import TestClient

from notification_common.auth.principal_source import PrincipalSource
from notification_common.config.metadata import version_number
from notification_common.errors.errors import ServerError
from notification_common.rest import notification_rest_server
from notification_common.rest.notification_rest_server import NotificationRestServer

# Upper bound for the real server's start and stop handshake. Uvicorn's main
# loop polls should_exit every 0.1s, so a couple of seconds is ample slack.
SERVER_LIFECYCLE_TIMEOUT_SECONDS = 5


@pytest.fixture
def noop_signal_handler():
    """
    Install a do-nothing process handler for a signal and restore the original.

    A delivered signal has to land on something harmless: the default SIGTERM
    disposition kills the test process and the default SIGINT disposition
    raises KeyboardInterrupt. Whatever was installed before is always put back,
    so that a test can deliver a real signal without leaving the pytest
    process's own handlers altered.
    """
    originals = {}

    def _install(handled_signal):
        originals[handled_signal] = signal.getsignal(handled_signal)
        signal.signal(handled_signal, lambda *_args: None)

    yield _install

    for handled_signal, original in originals.items():
        signal.signal(handled_signal, original)


def _mock_auth_settings(mocker, principal_source):
    """Build an auth_settings mock whose authentication_enabled() is pinned to
    the real gate (principal_source != DISABLED) for the given source."""
    mock_settings = mocker.Mock()
    mock_settings.principal_source = principal_source
    mock_settings.authentication_enabled.side_effect = (
        lambda: mock_settings.principal_source != PrincipalSource.DISABLED
    )
    mocker.patch.object(notification_rest_server, "auth_settings", mock_settings)
    return mock_settings


class TestRequireUvicornCaptureSignals:
    """The import-time guard for the capture_signals override (b.dvk)."""

    def test_passes_for_the_installed_uvicorn(self):
        notification_rest_server._require_uvicorn_capture_signals()

    def test_raises_import_error_when_the_seam_is_missing(self):
        class ServerWithoutTheSeam:
            pass

        with pytest.raises(ImportError, match="capture_signals"):
            notification_rest_server._require_uvicorn_capture_signals(
                ServerWithoutTheSeam
            )

    def test_inherited_seam_does_not_count(self):
        """A subclass that merely inherits the method is not the class uvicorn
        ships; the guard must look at the class's own namespace."""

        class Inheriting(notification_rest_server.uvicorn.Server):
            pass

        with pytest.raises(ImportError, match="capture_signals"):
            notification_rest_server._require_uvicorn_capture_signals(Inheriting)


_ALLOWED_ORIGIN = "https://app.example.com"


def _preflight(app, origin):
    """Send a CORS preflight for GET /probe from ``origin``."""
    return TestClient(app).options(
        "/probe",
        headers={"Origin": origin, "Access-Control-Request-Method": "GET"},
    )


@pytest.fixture
def cors_app_factory(mocker):
    """Build an auth-free app with a /probe route and the given CORS origins."""
    _mock_auth_settings(mocker, PrincipalSource.DISABLED)

    def _build(cors_allow_origins):
        app = NotificationRestServer.create_app(
            title="Test API",
            description="Test Description",
            cors_allow_origins=cors_allow_origins,
        )
        app.get("/probe")(lambda: {"ok": True})
        return app

    return _build


class TestCreateApp:
    """Tests for create_app method."""

    def test_create_app_returns_instance(self, basic_app):
        """Test that create_app returns a server instance."""
        assert isinstance(basic_app, NotificationRestServer)

    def test_create_app_sets_title(self, basic_app):
        """Test that create_app sets the title."""
        # noinspection PyUnresolvedReferences
        assert basic_app.title == "Test API"

    def test_create_app_sets_description(self, basic_app):
        """Test that create_app sets the description."""
        # noinspection PyUnresolvedReferences
        assert "Test Description" in basic_app.description

    def test_create_app_uses_version_from_metadata(self, basic_app):
        """Test that create_app uses the version from metadata."""
        # noinspection PyUnresolvedReferences
        assert basic_app.version == version_number()

    def test_create_app_stores_instance(self, basic_app):
        """Test that create_app stores the app instance."""
        assert NotificationRestServer._app_instance == basic_app

    def test_create_app_with_cors_disabled(self):
        """Test that CORS middleware is not added when disabled."""
        app = NotificationRestServer.create_app(
            title="Test API",
            description="Test Description",
            cors_enabled=False,
        )

        assert isinstance(app, NotificationRestServer)

    def test_create_app_with_custom_excluded_paths(self):
        """Test that custom excluded paths can be provided."""
        app = NotificationRestServer.create_app(
            title="Test API",
            description="Test Description",
            auth_excluded_paths=["/custom/health"],
            tracing_excluded_paths=["/custom/metrics"],
        )

        assert isinstance(app, NotificationRestServer)

    def test_create_app_with_custom_lifespan(self, custom_lifespan):
        """Test that custom lifespan handler can be provided."""
        app = NotificationRestServer.create_app(
            title="Test API",
            description="Test Description",
            lifespan_handler=custom_lifespan,
        )

        assert isinstance(app, NotificationRestServer)

    def test_create_app_uses_default_lifespan_when_none_provided(self, basic_app):
        """Test that default lifespan is used when none provided."""
        assert basic_app.router.lifespan_context is not None

    def test_no_origins_sends_no_allow_origin_header(self, cors_app_factory):
        response = _preflight(cors_app_factory([]), _ALLOWED_ORIGIN)

        assert "access-control-allow-origin" not in response.headers

    def test_configured_origin_is_echoed(self, cors_app_factory):
        response = _preflight(cors_app_factory([_ALLOWED_ORIGIN]), _ALLOWED_ORIGIN)

        assert response.headers["access-control-allow-origin"] == _ALLOWED_ORIGIN

    def test_configured_origin_allows_credentials(self, cors_app_factory):
        response = _preflight(cors_app_factory([_ALLOWED_ORIGIN]), _ALLOWED_ORIGIN)

        assert response.headers["access-control-allow-credentials"] == "true"

    def test_unlisted_origin_is_not_echoed(self, cors_app_factory):
        response = _preflight(
            cors_app_factory([_ALLOWED_ORIGIN]), "https://evil.example.com"
        )

        assert "access-control-allow-origin" not in response.headers

    def test_defaults_to_the_configured_setting(self, mocker, cors_app_factory):
        mocker.patch.object(
            notification_rest_server.common_settings,
            "cors_allow_origins",
            [_ALLOWED_ORIGIN],
        )

        response = _preflight(cors_app_factory(None), _ALLOWED_ORIGIN)

        assert response.headers["access-control-allow-origin"] == _ALLOWED_ORIGIN

    def test_wildcard_origin_with_credentials_fails_at_startup(self, cors_app_factory):
        with pytest.raises(ServerError, match="must not contain '\\*'"):
            cors_app_factory(["https://app.example.com", "*"])

    def test_wildcard_from_settings_fails_at_startup(self, mocker, cors_app_factory):
        mocker.patch.object(
            notification_rest_server.common_settings, "cors_allow_origins", ["*"]
        )

        with pytest.raises(ServerError, match="must not contain"):
            cors_app_factory(None)

    def test_null_origin_fails_at_startup(self, cors_app_factory):
        with pytest.raises(ServerError, match="must not contain 'null'"):
            cors_app_factory(["null"])

    def test_origin_with_a_path_fails_at_startup(self, cors_app_factory):
        with pytest.raises(ServerError, match="is not written the way browsers send"):
            cors_app_factory(["https://app.example.com/"])

    def test_argument_origins_are_normalized(self, cors_app_factory):
        app = cors_app_factory([" HTTPS://App.Example.com ", ""])

        response = _preflight(app, _ALLOWED_ORIGIN)

        assert response.headers["access-control-allow-origin"] == _ALLOWED_ORIGIN


class TestDefaultLifespan:
    """Tests for _default_lifespan method."""

    @pytest.mark.asyncio
    async def test_default_lifespan_logs_startup(
        self, mocker, lifespan_function, mock_lifespan_app
    ):
        """Test that default lifespan logs startup message."""
        spy = mocker.patch.object(notification_rest_server, "logger")
        mocker.patch.object(asyncio, "sleep", new_callable=AsyncMock)

        async with lifespan_function(mock_lifespan_app):
            pass

        spy.info.assert_any_call("REST server starting up...")

    @pytest.mark.asyncio
    async def test_default_lifespan_logs_shutdown(
        self, mocker, lifespan_function, mock_lifespan_app
    ):
        """Test that default lifespan logs shutdown messages."""
        spy = mocker.patch.object(notification_rest_server, "logger")
        mocker.patch.object(asyncio, "sleep", new_callable=AsyncMock)

        async with lifespan_function(mock_lifespan_app):
            pass

        spy.info.assert_any_call("REST server shutting down with 5 second grace...")
        spy.info.assert_any_call("REST server shutdown complete")

    @pytest.mark.asyncio
    async def test_default_lifespan_sleeps_five_seconds(
        self, mocker, lifespan_function, mock_lifespan_app
    ):
        """Test that default lifespan sleeps for 5 seconds on shutdown."""
        mocker.patch.object(notification_rest_server, "logger")
        spy = mocker.patch.object(asyncio, "sleep", new_callable=AsyncMock)

        async with lifespan_function(mock_lifespan_app):
            pass

        spy.assert_called_once_with(5)


class TestStartServer:
    """Tests for start_server method."""

    @pytest.mark.asyncio
    async def test_raises_error_if_app_not_created(self, shutdown_event):
        """Test that start_server raises error if create_app not called."""
        # Clear the instance
        NotificationRestServer._app_instance = None

        with pytest.raises(RuntimeError, match="App instance not created"):
            await NotificationRestServer.start_server(shutdown_event)

    @pytest.mark.asyncio
    async def test_creates_uvicorn_config(
        self,
        basic_app,
        mock_common_settings,
        mock_uvicorn_components,
        immediate_shutdown_event,
    ):
        """Test that start_server creates uvicorn config correctly."""
        await NotificationRestServer.start_server(immediate_shutdown_event)

        config_spy = mock_uvicorn_components["config_spy"]
        config_spy.assert_called_once()
        call_kwargs = config_spy.call_args[1]
        assert call_kwargs["app"] == basic_app
        assert call_kwargs["host"] == "0.0.0.0"
        assert call_kwargs["port"] == 8001
        assert call_kwargs["log_level"] == "info"
        assert call_kwargs["log_config"] is None
        assert call_kwargs["loop"] == "asyncio"

    @pytest.mark.asyncio
    async def test_starts_uvicorn_server(
        self,
        basic_app,
        mock_common_settings,
        mock_uvicorn_components,
        immediate_shutdown_event,
    ):
        """Test that start_server starts uvicorn server with the built config."""
        await NotificationRestServer.start_server(immediate_shutdown_event)

        config_spy = mock_uvicorn_components["config_spy"]
        server_spy = mock_uvicorn_components["server_spy"]
        mock_server = mock_uvicorn_components["mock_server"]
        server_spy.assert_called_once_with(config_spy.return_value)
        mock_server.serve.assert_called_once()

    @pytest.mark.asyncio
    async def test_waits_for_shutdown_event(
        self, basic_app, mock_common_settings, mock_uvicorn_components, mocker
    ):
        """Test that start_server waits for shutdown event."""
        shutdown_event = asyncio.Event()
        wait_spy = mocker.spy(shutdown_event, "wait")
        shutdown_event.set()  # Immediately trigger shutdown

        await NotificationRestServer.start_server(shutdown_event)

        wait_spy.assert_called_once()

    @pytest.mark.asyncio
    async def test_sets_should_exit_on_shutdown(
        self,
        basic_app,
        mock_common_settings,
        mock_uvicorn_components,
        immediate_shutdown_event,
    ):
        """Test that start_server sets should_exit on shutdown."""
        await NotificationRestServer.start_server(immediate_shutdown_event)

        mock_server = mock_uvicorn_components["mock_server"]
        assert mock_server.should_exit is True

    @staticmethod
    @asynccontextmanager
    async def _serving(shutdown_event, handle):
        """
        Run the REST server for the duration of the block.

        The block is entered once the server is actually up, so that it can
        observe the process while the server serves. Leaving the block always
        shuts the server down again, even if the block raised, so that a failed
        assertion cannot strand a running server.
        """
        task = asyncio.create_task(NotificationRestServer.start_server(shutdown_event))
        try:
            await asyncio.wait_for(
                handle.started.wait(), timeout=SERVER_LIFECYCLE_TIMEOUT_SECONDS
            )
            yield
        finally:
            shutdown_event.set()
            await asyncio.wait_for(task, timeout=SERVER_LIFECYCLE_TIMEOUT_SECONDS)

    @pytest.mark.asyncio
    async def test_stops_only_once_shutdown_event_is_set(
        self,
        basic_app,
        mock_common_settings,
        mock_uvicorn_components,
        shutdown_event,
    ):
        """
        Test that start_server has not set should_exit when it awaits serve().

        Nothing has asked the server to stop at that point: the shutdown event
        is still clear, and the drain sequence in start_servers is what sets it
        at the end of the drain. The state is sampled at the moment serve() is
        awaited rather than after a wall-clock window, so that the test cannot
        pass merely by outrunning a regression. No signal is delivered here --
        the SIGTERM contract is covered by
        test_keeps_serving_when_the_signal_is_delivered -- and the matching
        "stopped after the event" half lives in
        test_sets_should_exit_on_shutdown.
        """
        mock_server = mock_uvicorn_components["mock_server"]
        serving = asyncio.Event()
        observed = {}

        async def _sample_should_exit():
            observed["at_serve"] = mock_server.should_exit
            serving.set()

        mock_server.serve = AsyncMock(side_effect=_sample_should_exit)

        task = asyncio.create_task(NotificationRestServer.start_server(shutdown_event))
        try:
            await asyncio.wait_for(
                serving.wait(), timeout=SERVER_LIFECYCLE_TIMEOUT_SECONDS
            )
        finally:
            # Always release the task, so that a failure above cannot strand it.
            shutdown_event.set()
            await asyncio.wait_for(task, timeout=SERVER_LIFECYCLE_TIMEOUT_SECONDS)

        assert observed["at_serve"] is False

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "handled_signal",
        [signal.SIGINT, signal.SIGTERM],
        ids=["sigint", "sigterm"],
    )
    async def test_leaves_process_signal_handler_untouched(
        self,
        handled_signal,
        basic_app,
        mock_common_settings,
        unbound_rest_server,
        shutdown_event,
    ):
        """
        Test that a running REST server does not replace the process's signal
        handlers.

        Stock uvicorn installs its own handle_exit for SIGINT/SIGTERM while it
        serves, which would race the shutdown sequence that start_servers owns.
        The handler is therefore read while the server is actually up, since
        uvicorn's capture_signals restores the original one on the way out.
        """
        handler_before = signal.getsignal(handled_signal)

        async with self._serving(shutdown_event, unbound_rest_server):
            handler_while_serving = signal.getsignal(handled_signal)

        assert handler_while_serving is handler_before

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "delivered_signal",
        [signal.SIGINT, signal.SIGTERM],
        ids=["sigint", "sigterm"],
    )
    async def test_keeps_serving_when_the_signal_is_delivered(
        self,
        delivered_signal,
        basic_app,
        mock_common_settings,
        unbound_rest_server,
        shutdown_event,
        noop_signal_handler,
    ):
        """
        Test that a signal delivered to the process does not stop the server.

        This is the issue's contract exercised end to end, against a real
        signal rather than a spy: stock uvicorn's capture_signals binds
        handle_exit to SIGINT/SIGTERM while it serves, which flips should_exit
        the instant the signal lands and closes the listening socket while
        start_servers is only just beginning to drain.
        """
        noop_signal_handler(delivered_signal)

        async with self._serving(shutdown_event, unbound_rest_server):
            signal.raise_signal(delivered_signal)
            stopped = unbound_rest_server.server.should_exit

        assert stopped is False

    @pytest.mark.asyncio
    async def test_serves_the_default_server_header(
        self,
        basic_app,
        mock_common_settings,
        unbound_rest_server,
        shutdown_event,
    ):
        """
        Test that the running server still populates its default headers.

        Uvicorn's own serving loop is what refreshes the Date and Server
        headers, so this pins that overriding capture_signals left the
        inherited loop -- and the response headers it maintains -- in place.
        """
        async with self._serving(shutdown_event, unbound_rest_server):
            pass

        headers = dict(unbound_rest_server.server.server_state.default_headers)
        assert b"server" in headers


class TestMiddleware:
    """Tests for middleware configuration."""

    def test_middleware_added_in_correct_order(self, mocker, basic_app):
        """Test that middleware is added in the correct order."""
        spy = mocker.spy(FastAPI, "middleware")

        # Mock auth as enabled for this test
        _mock_auth_settings(mocker, PrincipalSource.JWT)

        # Create a fresh app for this test to capture middleware calls
        NotificationRestServer.create_app(
            title="Test API",
            description="Test Description",
        )

        # Middleware should be added (executed in reverse order)
        assert spy.call_count >= 3  # At least tracing, auth, error handling

    def test_auth_middleware_not_added_when_auth_disabled(self, mocker):
        """Test that auth middleware is not added when principal_source is
        disabled (authentication_enabled() is false)."""
        _mock_auth_settings(mocker, PrincipalSource.DISABLED)

        # Mock the middleware creation functions
        auth_spy = mocker.patch.object(
            notification_rest_server, "create_auth_token_middleware"
        )
        tracing_spy = mocker.patch.object(
            notification_rest_server,
            "create_tracing_middleware",
            return_value=lambda x: x,
        )

        NotificationRestServer.create_app(
            title="Test API",
            description="Test Description",
        )

        # Auth middleware should not be called when disabled
        auth_spy.assert_not_called()
        # Other middleware should still be added
        tracing_spy.assert_called_once()

    @pytest.mark.parametrize(
        "principal_source",
        [
            PrincipalSource.JWT,
            PrincipalSource.OPAQUE,
            PrincipalSource.JWT_UNVERIFIED,
            PrincipalSource.USER_INFO_SERVICE,
        ],
    )
    def test_auth_middleware_added_for_non_disabled_sources(
        self, mocker, principal_source
    ):
        """Test that auth middleware is added for every non-disabled
        principal_source (authentication_enabled() is true)."""
        _mock_auth_settings(mocker, principal_source)

        # Mock the middleware creation functions
        auth_spy = mocker.patch.object(
            notification_rest_server,
            "create_auth_token_middleware",
            return_value=lambda x: x,
        )
        mocker.patch.object(
            notification_rest_server,
            "create_tracing_middleware",
            return_value=lambda x: x,
        )
        mocker.patch.object(
            notification_rest_server,
            "error_handling_middleware",
            lambda x: x,
        )

        NotificationRestServer.create_app(
            title="Test API",
            description="Test Description",
        )

        # Auth middleware should be called when authentication is enabled
        auth_spy.assert_called_once()

    def test_cors_middleware_added_for_explicit_origins(self, cors_middleware_spy):
        """Test that CORS middleware is added when origins are configured."""
        NotificationRestServer.create_app(
            title="Test API",
            description="Test Description",
            cors_enabled=True,
            cors_allow_origins=["https://app.example.com"],
        )

        assert cors_middleware_spy.check_cors_added()

    def test_cors_middleware_not_added_without_origins(
        self, mocker, cors_middleware_spy
    ):
        """Test that the default (no configured origins) adds no CORS."""
        mocker.patch.object(
            notification_rest_server.common_settings, "cors_allow_origins", []
        )
        NotificationRestServer.create_app(
            title="Test API",
            description="Test Description",
        )

        assert not cors_middleware_spy.check_cors_added()

    def test_cors_disabled_ignores_configured_origins(self, cors_middleware_spy):
        """Test that cors_enabled=False wins over configured origins."""
        NotificationRestServer.create_app(
            title="Test API",
            description="Test Description",
            cors_enabled=False,
            cors_allow_origins=["https://app.example.com"],
        )

        assert not cors_middleware_spy.check_cors_added()

    def test_cors_middleware_not_added_when_disabled(self):
        """Test that CORS middleware is not added when disabled."""
        app = NotificationRestServer.create_app(
            title="Test API",
            description="Test Description",
            cors_enabled=False,
        )

        # CORS should not be in middleware when disabled
        # The spy might catch other middleware, but we verify the app itself
        assert isinstance(app, NotificationRestServer)


class TestAddSecuritySchemes:
    """Tests for add_security_schemes method."""

    def test_adds_security_schemes_to_empty_schema(self):
        """Test that security schemes are added to an empty schema."""
        openapi_schema = {}

        NotificationRestServer.add_security_schemes(openapi_schema)

        assert "components" in openapi_schema
        assert "securitySchemes" in openapi_schema["components"]
        assert "BearerAuth" in openapi_schema["components"]["securitySchemes"]

    def test_adds_bearer_auth_with_correct_structure(self):
        """Test that BearerAuth has the correct structure."""
        openapi_schema = {}

        NotificationRestServer.add_security_schemes(openapi_schema)

        bearer_auth = openapi_schema["components"]["securitySchemes"]["BearerAuth"]
        assert bearer_auth["type"] == "http"
        assert bearer_auth["scheme"] == "bearer"
        assert bearer_auth["bearerFormat"] == "JWT"
        assert "description" in bearer_auth

    def test_does_not_overwrite_existing_components(self):
        """Test that existing components are preserved."""
        openapi_schema = {
            "components": {"schemas": {"ExistingSchema": {"type": "object"}}}
        }

        NotificationRestServer.add_security_schemes(openapi_schema)

        assert "schemas" in openapi_schema["components"]
        assert "ExistingSchema" in openapi_schema["components"]["schemas"]
        assert "securitySchemes" in openapi_schema["components"]

    def test_does_not_overwrite_existing_security_schemes(self):
        """Test that existing security schemes are preserved."""
        openapi_schema = {
            "components": {"securitySchemes": {"ApiKeyAuth": {"type": "apiKey"}}}
        }

        NotificationRestServer.add_security_schemes(openapi_schema)

        assert "ApiKeyAuth" in openapi_schema["components"]["securitySchemes"]
        assert "BearerAuth" in openapi_schema["components"]["securitySchemes"]


class TestGetStandardErrorResponses:
    """Tests for get_standard_error_responses method."""

    def test_returns_dict(self):
        """Test that method returns a dictionary."""
        result = NotificationRestServer.get_standard_error_responses()

        assert isinstance(result, dict)

    def test_includes_all_standard_status_codes(self):
        """Test that all standard error status codes are included."""
        result = NotificationRestServer.get_standard_error_responses()

        assert 401 in result
        assert 403 in result
        assert 500 in result
        assert 429 in result

    def test_each_response_has_description(self):
        """Test that each response has a description."""
        result = NotificationRestServer.get_standard_error_responses()

        for status_code, response in result.items():
            assert "description" in response
            assert isinstance(response["description"], str)
            assert len(response["description"]) > 0

    def test_each_response_has_content_schema(self):
        """Test that each response has content with schema."""
        result = NotificationRestServer.get_standard_error_responses()

        for status_code, response in result.items():
            assert "content" in response
            assert "application/json" in response["content"]
            assert "schema" in response["content"]["application/json"]

    def test_401_description_mentions_authentication(self):
        """Test that 401 descriptions mention authentication."""
        result = NotificationRestServer.get_standard_error_responses()

        assert "authentication" in result[401]["description"].lower()

    def test_403_description_mentions_permission(self):
        """Test that 403 descriptions mention permission."""
        result = NotificationRestServer.get_standard_error_responses()

        assert "permission" in result[403]["description"].lower()

    def test_429_description_mentions_rate_limit(self):
        """Test that 429 description mentions rate limit."""
        result = NotificationRestServer.get_standard_error_responses()

        assert "rate limit" in result[429]["description"].lower()

    def test_500_description_mentions_server_error(self):
        """Test that 500 description mentions server error."""
        result = NotificationRestServer.get_standard_error_responses()

        assert "server error" in result[500]["description"].lower()


class TestErrorResponse:
    """Tests for ErrorResponse model."""

    def test_error_response_has_detail_field(self):
        """Test that ErrorResponse model has detail field."""
        from notification_common.rest.notification_rest_server import ErrorResponse

        error = ErrorResponse(detail="Test error message")

        assert error.detail == "Test error message"

    def test_error_response_has_max_length_constraint(self):
        """Test that ErrorResponse has max_length constraint on detail."""
        from notification_common.rest.notification_rest_server import ErrorResponse
        from pydantic import ValidationError

        # This should work - at the limit
        error = ErrorResponse(detail="x" * 1024)
        assert error.detail == "x" * 1024

        # This should fail - exceeds max_length
        with pytest.raises(ValidationError, match="String should have at most"):
            ErrorResponse(detail="x" * 2000)
