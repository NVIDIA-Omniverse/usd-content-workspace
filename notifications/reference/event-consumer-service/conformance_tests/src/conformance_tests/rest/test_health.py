# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REST conformance tests for the health endpoint.

Contract under test:
  GET /api/v1beta/health

Expected behaviour:
  - Returns HTTP 200.
  - Response body contains {"status": "healthy"}.
"""

import pytest

pytestmark = pytest.mark.rest


class TestHealth:
    def test_health_returns_200(self, http_client):
        """GET /api/v1beta/health must return HTTP 200."""
        response = http_client.get("/api/v1beta/health")
        assert response.status_code == 200, (
            f"Expected GET /api/v1beta/health to return HTTP 200, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_health_returns_healthy_status(self, http_client):
        """GET /api/v1beta/health must return a JSON body with status='healthy'."""
        response = http_client.get("/api/v1beta/health")
        assert response.status_code == 200

        body = response.json()
        assert (
            body.get("status") == "healthy"
        ), f'Expected {{"status": "healthy"}} but got: {body}'
