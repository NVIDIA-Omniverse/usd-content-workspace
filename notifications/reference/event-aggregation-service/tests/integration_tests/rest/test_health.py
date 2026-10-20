# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Integration tests for the health endpoint."""

import pytest


@pytest.mark.asyncio
class TestHealthEndpoint:
    """Tests for the health endpoint."""

    async def test_returns_200(self, client):
        """Test that health endpoint returns 200."""
        response = client.get("/api/v1beta/health")
        assert response.status_code == 200
        assert response.json() == {"status": "healthy"}
