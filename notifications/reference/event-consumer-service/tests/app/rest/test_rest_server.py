# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for REST server."""

import runpy
import sys

import pytest
from fastapi.middleware.cors import CORSMiddleware
from notification_common.rest.notification_rest_server import NotificationRestServer

from app.rest.rest_server import rest_server, RestServer


class TestRestServer:
    """Tests for RestServer class."""

    def test_rest_server_extends_notification_rest_server(self):
        """Test that RestServer extends NotificationRestServer."""
        assert issubclass(RestServer, NotificationRestServer)

    def test_rest_server_is_instance_of_rest_server_class(self):
        """Test that rest_server is an instance of RestServer."""
        assert isinstance(rest_server, RestServer)

    def test_rest_server_has_title(self):
        """Test that rest_server has a title."""
        assert rest_server.title == "Event Consumer API"

    def test_rest_server_has_version_from_metadata(self):
        """Test that rest_server version comes from metadata."""
        # Version is dynamically loaded from pyproject.toml
        assert rest_server.version is not None
        assert len(rest_server.version) > 0

    def test_rest_server_has_routes(self):
        """Test that rest_server has routes registered."""
        # Routers are attached via include_router, which FastAPI stores as
        # _IncludedRouter entries in app.routes rather than flattened Route
        # objects, so assert against the OpenAPI path set (the routing surface
        # clients actually see).
        paths = rest_server.openapi().get("paths", {})
        assert "/api/v1beta/events/stream" in paths
        assert "/api/v1beta/queues/durable" in paths
        assert "/api/v1beta/health" in paths

    @pytest.mark.asyncio
    async def test_start_server_method_exists(self):
        """Test that start_server method exists."""
        assert hasattr(RestServer, "start_server")
        assert callable(RestServer.start_server)

    def test_custom_openapi_function_exists(self):
        """Test that custom_openapi function exists."""
        assert hasattr(rest_server, "openapi")
        assert callable(rest_server.openapi)


class TestCreateApp:
    """The module's create_app call takes its CORS origins from the settings."""

    def test_default_settings_add_no_cors_middleware(self):
        assert not any(m.cls is CORSMiddleware for m in rest_server.user_middleware)

    @pytest.mark.filterwarnings("ignore:'app.rest.rest_server' found in sys.modules")
    def test_configured_origins_reach_the_cors_middleware(self, mocker):
        # Patch the settings object that the module imports now. Tests that
        # reload app.config.settings replace its `settings` instance, so the
        # one imported when this file was collected may be stale.
        current_settings = sys.modules["app.config.settings"].settings
        origins = ["https://a.example.com"]
        mocker.patch.object(current_settings, "cors_allow_origins", origins)

        # Execute the module afresh without replacing the imported one, so the
        # RestServer class other tests hold stays the same object.
        module_globals = runpy.run_module("app.rest.rest_server")

        allowed = [
            m.kwargs["allow_origins"]
            for m in module_globals["rest_server"].user_middleware
            if m.cls is CORSMiddleware
        ]
        assert allowed == [origins]


class TestCustomOpenapi:
    """Tests for the custom_openapi function."""

    def test_openapi_schema_has_no_nspect_extension(self):
        """Test that the OpenAPI document carries no nSpect extension."""
        schema = rest_server.openapi()
        assert "x-nspect" not in schema["info"]

    def test_openapi_schema_has_no_nspect_tag(self):
        """Test that the nSpect ID is no longer published as a tag."""
        schema = rest_server.openapi()
        assert "nspect_id" not in [tag["name"] for tag in schema.get("tags", [])]

    def test_openapi_schema_has_security_schemes(self):
        """Test that the OpenAPI schema includes security schemes."""
        schema = rest_server.openapi()
        assert "components" in schema
        assert "securitySchemes" in schema["components"]

    def test_openapi_schema_has_filter_group_model(self):
        """Test that the OpenAPI schema includes FilterGroup model."""
        schema = rest_server.openapi()
        assert "components" in schema
        assert "schemas" in schema["components"]
        assert "FilterGroup" in schema["components"]["schemas"]

    def test_openapi_schema_has_resource_filter_model(self):
        """Test that the OpenAPI schema includes ResourceFilter model."""
        schema = rest_server.openapi()
        assert "components" in schema
        assert "schemas" in schema["components"]
        assert "ResourceFilter" in schema["components"]["schemas"]

    def test_openapi_schema_has_governance_compliant_validation_error(self):
        """Test that the OpenAPI schema has governance-compliant errors."""
        schema = rest_server.openapi()
        assert "components" in schema
        assert "schemas" in schema["components"]
        assert "ValidationError" in schema["components"]["schemas"]
        validation_error = schema["components"]["schemas"]["ValidationError"]
        assert "properties" in validation_error
        assert "loc" in validation_error["properties"]
        assert "maxItems" in validation_error["properties"]["loc"]
