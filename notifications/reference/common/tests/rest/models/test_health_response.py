# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for HealthResponse model."""

import pytest
from pydantic import ValidationError

from notification_common.rest.models.health_response import HealthResponse


class TestHealthResponse:
    """Tests for HealthResponse model."""

    def test_creates_with_default_status(self):
        """Test that HealthResponse creates with default status."""
        response = HealthResponse()

        assert response.status == "healthy"

    def test_creates_with_custom_status(self):
        """Test that HealthResponse can be created with custom status."""
        response = HealthResponse(status="ok")

        assert response.status == "ok"

    def test_accepts_status_at_max_length(self):
        """Test that status accepts string at max length."""
        response = HealthResponse(status="x" * 10)

        assert response.status == "x" * 10

    def test_rejects_status_exceeding_max_length(self):
        """Test that status rejects string exceeding max length."""
        with pytest.raises(ValidationError, match="String should have at most"):
            HealthResponse(status="x" * 11)

    def test_serializes_to_dict(self):
        """Test that HealthResponse serializes to dict correctly."""
        response = HealthResponse()

        assert response.model_dump() == {"status": "healthy"}

    def test_deserializes_from_dict(self):
        """Test that HealthResponse deserializes from dict correctly."""
        data = {"status": "healthy"}

        response = HealthResponse.model_validate(data)

        assert response.status == "healthy"
