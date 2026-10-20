# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Base REST server implementation for notification services."""

import asyncio
import logging
from contextlib import asynccontextmanager, contextmanager
from typing import Callable, Iterator, List, Optional, Dict, Any, Sequence

import uvicorn
from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel, Field

from notification_common.config.auth_settings import auth_settings
from notification_common.config.common_settings import (
    common_settings,
    validate_cors_origins,
)
from notification_common.config.metadata import version_number
from notification_common.rest.middleware.auth_token_middleware import (
    create_auth_token_middleware,
)
from notification_common.rest.middleware.error_handling_middleware import (
    error_handling_middleware,
)
from notification_common.rest.middleware.tracing_middleware import (
    create_tracing_middleware,
)
from notification_common.servers.server_base import ServerBase

logger = logging.getLogger(__name__)


class ErrorResponse(BaseModel):
    """Standard error response model for API governance."""

    detail: str = Field(max_length=1024, description="Error details")


def _require_uvicorn_capture_signals(server_cls: type = uvicorn.Server) -> None:
    """
    Fail at import if uvicorn no longer exposes the seam this module overrides.

    ``_NoSignalUvicornServer`` keeps uvicorn from installing its own SIGTERM
    and SIGINT handlers by overriding ``Server.capture_signals()``. That method
    is undocumented, and the library declares no direct uvicorn bound, so a
    future uvicorn that renames it or installs handlers elsewhere would import
    cleanly and silently restore the defect this override exists to fix (see
    notification_common issue b.cpi, follow-up b.dvk). Raising here turns that
    silent regression into a loud one at process start. The repository's own
    tests additionally pin the observable behaviour (a delivered SIGTERM must
    not stop the server), which is the check this guard cannot perform.
    """
    if "capture_signals" not in vars(server_cls):
        raise ImportError(
            "uvicorn.Server does not define capture_signals(); "
            "notification_common overrides it to keep uvicorn from installing "
            "process signal handlers (b.cpi). This uvicorn version needs its "
            "signal handling re-verified before it can be used."
        )


_require_uvicorn_capture_signals()


class _NoSignalUvicornServer(uvicorn.Server):
    """
    A uvicorn server that installs no process signal handlers.

    Stock ``uvicorn.Server.serve()`` wraps ``_serve()`` in
    ``capture_signals()``, which calls ``signal.signal(SIGTERM,
    self.handle_exit)`` on the main thread (verified against uvicorn 0.42.0,
    ``uvicorn/server.py:322-339``). That handler runs alongside the asyncio
    handler that ``start_servers`` registers, so uvicorn used to close its
    listening socket the moment SIGTERM arrived, while the library's drain
    sequence was only just beginning. REST calls routed to the pod then failed
    for the whole drain window, even though the gRPC server kept serving.

    ``start_servers`` owns SIGINT and SIGTERM for the whole process, so this
    subclass neuters the one method that installs the handlers and inherits
    everything else -- ``serve()``, ``_serve()``, ``main_loop()`` and
    ``shutdown()`` are all unchanged. The server therefore still stops the way
    it always did, by way of ``should_exit``, but now only when the library's
    drain sequence sets the shutdown event.

    Note that this also gives up uvicorn's repeated-SIGINT force-quit escape,
    because nothing sets ``force_exit`` any more.
    """

    @contextmanager
    def capture_signals(self) -> Iterator[None]:
        """Install no signal handlers; start_servers owns the signals."""
        yield


class NotificationRestServer(ServerBase, FastAPI):
    """
    Base REST server for notification services.

    This class provides a common implementation for REST servers across
    the notification service ecosystem. It handles:
    - Uvicorn server lifecycle
    - Standard middleware setup (tracing, auth, error handling)
    - CORS configuration (explicit origins only; off unless configured)
    - Graceful shutdown
    """

    _app_instance: Optional["NotificationRestServer"] = None

    @classmethod
    def create_app(
        cls,
        title: str,
        description: str,
        lifespan_handler: Optional[Callable] = None,
        cors_enabled: bool = True,
        auth_excluded_paths: Optional[List[str]] = None,
        tracing_excluded_paths: Optional[List[str]] = None,
        cors_allow_origins: Optional[Sequence[str]] = None,
    ) -> "NotificationRestServer":
        """
        Create a REST server application instance.

        :param title: The API title
        :param description: The API description
        :param lifespan_handler: Optional custom lifespan context manager
        :param cors_enabled: Whether CORS may be enabled (default True). CORS
            middleware is added only when this is true AND at least one origin
            is configured.
        :param auth_excluded_paths: Custom paths to exclude from auth
        :param tracing_excluded_paths: Custom paths to exclude from tracing
        :param cors_allow_origins: Browser origins admitted by CORS, each
            answered with Access-Control-Allow-Credentials: true. None (the
            default) uses ``common_settings.cors_allow_origins``
            (OMNI_EVENTS_CORS_ALLOW_ORIGINS); an empty list adds no CORS
            middleware.
        :raises ServerError: if an origin is "*" or "null", which credentialed
            CORS must never admit, or is not a bare scheme://host[:port]
            origin (see validate_cors_origins).
        :return: Configured REST server instance
        """
        origins = cls._cors_origins(cors_enabled, cors_allow_origins)

        lifespan = lifespan_handler or cls._default_lifespan()

        app = cls(
            title=title,
            description=description,
            version=version_number(),
            lifespan=lifespan,
        )

        # Add middleware in the correct order (executed in reverse)
        app.middleware("http")(create_tracing_middleware(tracing_excluded_paths))

        # Only add auth middleware if authentication is enabled
        if auth_settings.authentication_enabled():
            app.middleware("http")(create_auth_token_middleware(auth_excluded_paths))

        app.middleware("http")(error_handling_middleware)

        # Add CORS middleware only for an explicit set of origins
        if origins:
            # noinspection PyTypeChecker
            app.add_middleware(
                CORSMiddleware,
                allow_origins=origins,
                allow_credentials=True,
                allow_methods=["*"],
                allow_headers=["*"],
            )

        cls._app_instance = app
        return app

    @staticmethod
    def _cors_origins(
        cors_enabled: bool, cors_allow_origins: Optional[Sequence[str]]
    ) -> List[str]:
        """
        Return the origins CORS admits, or an empty list for no CORS at all.

        Credentials are always allowed, so a "*" or "null" origin would let
        pages the operator does not control make credentialed requests on a
        signed-in user's behalf; validate_cors_origins refuses them here, at
        startup, rather than serving them. The argument is checked as well as
        the setting, because it does not pass through the settings validator.
        """
        if not cors_enabled:
            return []
        if cors_allow_origins is None:
            cors_allow_origins = common_settings.cors_allow_origins
        return validate_cors_origins(cors_allow_origins)

    @classmethod
    def _default_lifespan(cls):
        """Default lifespan context manager."""

        @asynccontextmanager
        async def lifespan(_app: FastAPI):
            # Startup
            logger.info("REST server starting up...")
            yield
            # Shutdown
            logger.info("REST server shutting down with 5 second grace...")
            await asyncio.sleep(5)
            logger.info("REST server shutdown complete")

        return lifespan

    @classmethod
    def add_security_schemes(cls, openapi_schema: Dict[str, Any]) -> None:
        """
        Add the Bearer authentication security scheme to the OpenAPI schema.

        This method should be called from custom_openapi() implementations
        to add security definitions for API governance compliance.

        :param openapi_schema: The OpenAPI schema dictionary to modify
        """
        if "components" not in openapi_schema:
            openapi_schema["components"] = {}
        if "securitySchemes" not in openapi_schema["components"]:
            openapi_schema["components"]["securitySchemes"] = {}

        openapi_schema["components"]["securitySchemes"]["BearerAuth"] = {
            "type": "http",
            "scheme": "bearer",
            "bearerFormat": "JWT",
            "description": (
                "Bearer token authentication using JWT. "
                "Implements security best practices from RFC8725."
            ),
        }

    @classmethod
    def get_standard_error_responses(cls) -> Dict[int, Dict[str, Any]]:
        """
        Get standard error response definitions for OpenAPI spec.

        Returns a dict of HTTP status codes to response definitions
        that include proper content schemas for API governance compliance.

        :return: Dictionary of standard error responses
        """
        return {
            401: {
                "description": (
                    "Unauthorized - The request requires valid authentication "
                    "credentials. This typically indicates a missing, invalid, "
                    "or expired authentication token."
                ),
                "content": {
                    "application/json": {"schema": ErrorResponse.model_json_schema()}
                },
            },
            403: {
                "description": (
                    "Forbidden - The authenticated user does not have permission "
                    "to access this resource or perform this operation."
                ),
                "content": {
                    "application/json": {"schema": ErrorResponse.model_json_schema()}
                },
            },
            500: {
                "description": "Server error",
                "content": {
                    "application/json": {"schema": ErrorResponse.model_json_schema()}
                },
            },
            429: {
                "description": "Too Many Requests - Rate limit exceeded",
                "content": {
                    "application/json": {"schema": ErrorResponse.model_json_schema()}
                },
            },
        }

    @classmethod
    async def start_server(cls, shutdown_event: asyncio.Event):
        """
        Start the REST server with uvicorn.

        This method is called by the start_servers infrastructure.

        The server runs until shutdown_event is set, which the graceful-shutdown
        sequence in start_servers does at the end of the drain -- not when
        SIGTERM arrives. uvicorn installs no signal handlers of its own here
        (see _NoSignalUvicornServer), so the whole shutdown ordering for both
        the gRPC and the REST server belongs to start_servers.

        :param shutdown_event: Event to signal shutdown
        """
        if not cls._app_instance:
            raise RuntimeError(
                "App instance not created. " "Call create_app() before starting server."
            )

        # Import settings here to avoid circular imports
        from notification_common.config.common_settings import common_settings

        config = uvicorn.Config(
            app=cls._app_instance,
            host="0.0.0.0",
            port=common_settings.rest_port,
            log_level=common_settings.log_level.lower(),
            log_config=None,
            loop="asyncio",
        )
        server = _NoSignalUvicornServer(config)
        server_task = asyncio.create_task(server.serve())

        # Wait until the shutdown is triggered
        await shutdown_event.wait()

        server.should_exit = True
        await server_task
