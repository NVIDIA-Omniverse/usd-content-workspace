# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""FastAPI application for the Event Aggregation service."""

import tomllib
from pathlib import Path

from fastapi.openapi.utils import get_openapi
from notification_common.rest.notification_rest_server import NotificationRestServer

from app.config.settings import settings
from app.rest.middleware.body_size_limit import BodySizeLimitMiddleware
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
    """REST server for the Event Aggregation service."""


# Create and configure the REST server
rest_server = RestServer.create_app(
    title="Event Publishing API",
    description="""
    REST API for publishing events to the notification service.

    Use this API to publish individual events or batch publish multiple events.
    Events are routed to consumers based on their filter subscriptions.
    """,
    # CORS admits only these explicit origins (OMNI_EVENTS_CORS_ALLOW_ORIGINS);
    # empty, the default, adds no CORS middleware, and "*" fails at startup.
    cors_allow_origins=settings.cors_allow_origins,
)

# Added last so it lands outermost: `add_middleware` inserts at index 0 of the
# user middleware list and the stack is built in reverse, so the body cap runs
# ahead of CORS, error handling, auth, and tracing. An oversize request is
# refused before it is authenticated or parsed.
# noinspection PyTypeChecker
rest_server.add_middleware(
    BodySizeLimitMiddleware, max_bytes=settings.rest_max_request_bytes
)

_pyproject = tomllib.loads(Path("pyproject.toml").read_text())
rest_server.version = _pyproject["tool"]["app"]["api_version"]

rest_server.include_router(router, prefix="/api/v1beta")


def custom_openapi():
    """
    Customize the OpenAPI schema for NVIDIA API governance compliance.
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

    # Override FastAPI's ValidationError models with governance-compliant versions
    if "components" in openapi_schema and "schemas" in openapi_schema["components"]:
        openapi_schema["components"]["schemas"].update(VALIDATION_ERROR_SCHEMA)

    rest_server.openapi_schema = openapi_schema
    return rest_server.openapi_schema


rest_server.openapi = custom_openapi
