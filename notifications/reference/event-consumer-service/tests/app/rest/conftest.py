# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Common fixtures for HTTP tests."""

from unittest.mock import AsyncMock, MagicMock

import pytest
from fastapi import Request
from fastapi.testclient import TestClient

from app.lib.models.filter_group import FilterGroup
from app.lib.models.filter_type import FilterType
from app.lib.models.resource_filter import ResourceFilter
from app.rest.rest_server import rest_server


@pytest.fixture
def client():
    """Return a FastAPI test client."""
    return TestClient(rest_server)


@pytest.fixture
def valid_filter_group():
    """Create a valid filter group for testing."""
    return FilterGroup(
        event_type="test_event",
        filters=(
            ResourceFilter(filter_type=FilterType.EQ, resource_id="test_resource"),
        ),
    )


@pytest.fixture
def mock_request():
    """Create a mock FastAPI request object."""
    request = MagicMock(spec=Request)
    request.client = MagicMock()
    request.client.host = "127.0.0.1"
    request.is_disconnected = AsyncMock(return_value=False)
    return request


@pytest.fixture(autouse=True)
def reset_sse_starlette_event():
    """
    Reset the sse_starlette AppStatus.should_exit_event before each test.
    This prevents 'RuntimeError: is bound to a different event loop' when
    running tests in sequence with different event loops.

    https://github.com/sysid/sse-starlette/issues/59
    """
    try:
        from sse_starlette.sse import AppStatus

        AppStatus.should_exit_event = None
    except ImportError:
        pass
