# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Integration tests for the health endpoint."""

import pytest
from fastapi.testclient import TestClient

from app.rest.rest_server import rest_server


@pytest.fixture
def client():
    """Return a FastAPI test client."""
    return TestClient(rest_server)


@pytest.mark.asyncio
class TestHealthEndpoint:
    async def test_returns_200(self, client):
        response = client.get("/api/v1/health")
        assert response.status_code == 200
        assert response.json() == {"status": "healthy"}

    async def test_v1beta_returns_200(self, client):
        response = client.get("/api/v1beta/health")
        assert response.status_code == 200
        assert response.json() == {"status": "healthy"}
