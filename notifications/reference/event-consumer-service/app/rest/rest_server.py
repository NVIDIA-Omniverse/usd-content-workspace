# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""FastAPI application for the EventConsumer service."""

from fastapi.openapi.utils import get_openapi
from notification_common.rest.notification_rest_server import NotificationRestServer

from app.config.app_metadata import api_path_prefix, api_version
from app.config.settings import settings
from app.lib.models.filter_group import FilterGroup
from app.lib.models.resource_filter import ResourceFilter
from app.lib.observability.access_log_redaction import install_access_log_redaction
from app.rest.routes import router

# Custom ValidationError schema for API governance compliance
VALIDATION_ERROR_SCHEMA = {
    "ValidationError": {
        "properties": {
            "loc": {
                "items": {
                    "anyOf": [
                        {"type": "string", "maxLength": 256},
                        {
                            "type": "integer",
                            "format": "int32",
                            "minimum": 0,
                            "maximum": 2147483647,
                        },
                    ]
                },
                "type": "array",
                "maxItems": 100,
                "title": "Location",
            },
            "msg": {"type": "string", "maxLength": 1024, "title": "Message"},
            "type": {"type": "string", "maxLength": 256, "title": "Error Type"},
        },
        "type": "object",
        "required": ["loc", "msg", "type"],
        "title": "ValidationError",
    },
    "HTTPValidationError": {
        "properties": {
            "detail": {
                "items": {"$ref": "#/components/schemas/ValidationError"},
                "type": "array",
                "maxItems": 100,
                "title": "Detail",
            }
        },
        "type": "object",
        "title": "HTTPValidationError",
    },
}


class RestServer(NotificationRestServer):
    """REST server for the Event Consumer service."""


# Paths that don't require authentication (internal/health and generated-docs
# endpoints).
#
# This list is deliberately explicit and is passed to `create_app` below, so
# this service does NOT inherit the library's default exclusions: whatever
# `create_auth_token_middleware` defaults to, only the paths named here are
# served without a bearer token. A path the library adds to its defaults later
# has to be added here too, which is the trade made on purpose -- the two
# metrics paths below exist only here, and taking the library defaults instead
# would drop them.
#
# Members that exist only here:
#   - /api/v1/metrics/channel-pool
#   - /api/v1beta/metrics/channel-pool
# The channel-pool autoscaler scrapes the v1beta one (its configured
# `scaler_metrics_path`) and presents no bearer token, so excluding these is a
# live dependency, not a convenience.
#
# Every other member mirrors a library default. /redoc and
# /docs/oauth2-redirect are here for parity: /redoc renders the same document
# /openapi.json already serves unauthenticated, and /docs/oauth2-redirect is
# the static OAuth callback shim /docs uses, so neither exposes anything /docs
# and /openapi.json do not already expose.
AUTH_EXCLUDED_PATHS = [
    "/api/v1/health",
    "/api/v1beta/health",
    "/api/v1/metrics/channel-pool",
    "/api/v1beta/metrics/channel-pool",
    "/docs",
    "/docs/oauth2-redirect",
    "/openapi.json",
    "/redoc",
]

# The stream endpoint accepts its reconnect token as a query parameter, and
# uvicorn's access log records the query string verbatim. Install the
# redacting filter here, at construction: this module is imported to get the
# app, so the filter is in place before uvicorn is handed that app and long
# before it can log a request. See `access_log_redaction` for why the access
# log is filtered rather than turned off.
install_access_log_redaction()

# Create and configure the REST server
rest_server = RestServer.create_app(
    title="Event Consumer API",
    description="""
    REST API for streaming events from the notification service.
    Use this API to stream events in real-time using Server-Sent Events (SSE)
    """,
    auth_excluded_paths=AUTH_EXCLUDED_PATHS,
    # CORS admits only these explicit origins (OMNI_EVENTS_CORS_ALLOW_ORIGINS);
    # empty, the default, adds no CORS middleware, and "*" fails at startup.
    cors_allow_origins=settings.cors_allow_origins,
)

rest_server.version = api_version()

rest_server.include_router(router, prefix="/api/v1", include_in_schema=False)
rest_server.include_router(router, prefix=api_path_prefix())


def custom_openapi():
    """
    Customize the OpenAPI schema to include FilterGroup, ResourceFilter,
    and FilterType models. These models are used in the filter_groups
    JSON string parameter but need to be documented.
    """
    if rest_server.openapi_schema:
        return rest_server.openapi_schema

    # noinspection PyUnresolvedReferences
    openapi_schema = get_openapi(
        title=rest_server.title,
        version=rest_server.version,
        description=rest_server.description,
        routes=rest_server.routes,
    )

    # Add security schemes for API governance
    NotificationRestServer.add_security_schemes(openapi_schema)

    # Add the filter-related models to the schema
    FilterGroup.add_to_openapi_schema(openapi_schema)
    ResourceFilter.add_to_openapi_schema(openapi_schema)

    # Override FastAPI's ValidationError models with governance-compliant versions
    if "components" in openapi_schema and "schemas" in openapi_schema["components"]:
        openapi_schema["components"]["schemas"].update(VALIDATION_ERROR_SCHEMA)

    rest_server.openapi_schema = openapi_schema
    return rest_server.openapi_schema


rest_server.openapi = custom_openapi
