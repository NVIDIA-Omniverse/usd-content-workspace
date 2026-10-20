# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Common fixtures for REST tests."""

import asyncio
from contextlib import asynccontextmanager
from unittest.mock import AsyncMock, MagicMock

import pytest
import uvicorn
from fastapi import FastAPI
from opentelemetry import trace

from notification_common.rest import notification_rest_server
from notification_common.rest.notification_rest_server import NotificationRestServer


@pytest.fixture
def mock_tracer(mocker):
    """Mock OpenTelemetry tracer."""
    mock = MagicMock()
    mocker.patch.object(trace, "get_tracer", return_value=mock)
    return mock


@pytest.fixture
def mock_span():
    """Mock OpenTelemetry span."""
    return MagicMock()


@pytest.fixture
def basic_app():
    """Create a basic NotificationRestServer app with standard test parameters."""
    return NotificationRestServer.create_app(
        title="Test API",
        description="Test Description",
    )


@pytest.fixture
def mock_common_settings(mocker):
    """Mock common settings with standard test values."""
    from notification_common.config.common_settings import common_settings

    mocker.patch.object(common_settings, "rest_port", 8001)
    mocker.patch.object(common_settings, "log_level", "INFO")
    return common_settings


@pytest.fixture
def mock_uvicorn_components(mocker):
    """
    Mock the uvicorn Config and the server class that start_server instantiates.

    The server class has to be patched at its definition site -- the
    notification_rest_server module -- and not as uvicorn.Server: start_server
    calls the _NoSignalUvicornServer subclass, which was bound to the real
    uvicorn.Server at import time, so rebinding uvicorn.Server here would leave
    a real server to be constructed and served.
    """
    config_spy = mocker.patch.object(uvicorn, "Config")

    mock_server = MagicMock()
    mock_server.serve = AsyncMock()
    # A real server starts out with should_exit False, so that the tests can
    # tell "not asked to stop yet" from "asked to stop".
    mock_server.should_exit = False
    server_spy = mocker.patch.object(
        notification_rest_server,
        "_NoSignalUvicornServer",
        return_value=mock_server,
    )

    return {
        "config_spy": config_spy,
        "server_spy": server_spy,
        "mock_server": mock_server,
    }


class UnboundRestServer:
    """A handle on the real uvicorn server that start_server is running."""

    def __init__(self):
        self.started = asyncio.Event()
        self.server = None


@pytest.fixture
def unbound_rest_server(mocker):
    """
    Let start_server run the real _NoSignalUvicornServer without a socket.

    Only ``startup()`` is patched out, so no port is bound and no lifespan
    runs, while ``serve()`` -- and therefore ``capture_signals()``, the method
    the no-signal subclass overrides -- executes for real. The handle's
    ``started`` event is set once the server has reached its startup phase,
    which is strictly after ``capture_signals()`` has been entered, and its
    ``server`` attribute is the running instance.
    """
    handle = UnboundRestServer()

    async def _startup(server, *_args, **_kwargs):
        handle.server = server
        handle.started.set()

    mocker.patch.object(
        notification_rest_server._NoSignalUvicornServer,
        "startup",
        autospec=True,
        side_effect=_startup,
    )
    return handle


@pytest.fixture
def shutdown_event():
    """Create a shutdown event for testing server lifecycle."""
    return asyncio.Event()


@pytest.fixture
def immediate_shutdown_event():
    """Create a shutdown event that is immediately set for testing."""
    event = asyncio.Event()
    event.set()
    return event


@pytest.fixture
def mock_lifespan_app():
    """Create a mock FastAPI app for lifespan testing."""
    return MagicMock(spec=FastAPI)


@pytest.fixture
def lifespan_function():
    """Get the default lifespan function for testing."""
    # noinspection PyProtectedMember
    return NotificationRestServer._default_lifespan()


@pytest.fixture
def custom_lifespan():
    """Create a custom lifespan handler for testing."""

    @asynccontextmanager
    async def custom_lifespan(_app: FastAPI):
        yield

    return custom_lifespan


@pytest.fixture
def cors_middleware_spy(mocker):
    """Create a spy for FastAPI add_middleware calls and CORS checking helper."""
    spy = mocker.spy(FastAPI, "add_middleware")

    def check_cors_added():
        """Check if CORS middleware was added to the spy calls."""
        from fastapi.middleware.cors import CORSMiddleware

        calls = spy.call_args_list
        return any(CORSMiddleware in (call[0] if call[0] else []) for call in calls)

    spy.check_cors_added = check_cors_added
    return spy
