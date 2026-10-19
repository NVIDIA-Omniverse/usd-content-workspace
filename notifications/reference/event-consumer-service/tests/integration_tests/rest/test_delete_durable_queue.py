# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Integration tests for the delete durable queue endpoint."""

import pytest
from fastapi.testclient import TestClient
from notification_common.errors.errors import QueueNotFoundError

from app.lib.rabbitmq import DurableQueueConsumer
from app.rest.rest_server import rest_server


@pytest.fixture
def client():
    """Return a FastAPI test client."""
    return TestClient(rest_server)


@pytest.fixture
def mock_queue_id():
    """Return a test queue ID."""
    return "test-queue-id"


@pytest.mark.asyncio
class TestDeleteDurableQueue:
    async def test_returns_200_on_success(
        self, client, mock_queue_id, mocker, metadata
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "delete_queue",
            return_value=None,
        )

        response = client.delete(
            f"/api/v1/queues/durable?queue_id={mock_queue_id}",
            headers=metadata,
        )

        assert response.status_code == 200
        assert response.json().get("success") is True

    async def test_v1beta_returns_200_on_success(
        self, client, mock_queue_id, mocker, metadata
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "delete_queue",
            return_value=None,
        )

        response = client.delete(
            f"/api/v1beta/queues/durable?queue_id={mock_queue_id}",
            headers=metadata,
        )

        assert response.status_code == 200
        assert response.json().get("success") is True

    async def test_calls_delete_with_correct_queue_id(
        self, client, mock_queue_id, mocker, metadata
    ):
        mock_delete = mocker.patch.object(
            DurableQueueConsumer,
            "delete_queue",
            return_value=None,
        )

        client.delete(
            f"/api/v1/queues/durable?queue_id={mock_queue_id}",
            headers=metadata,
        )

        # Verify delete_queue was called with the correct queue ID
        mock_delete.assert_called_once_with(mock_queue_id)

    async def test_returns_404_for_nonexistent_queue(
        self, client, mock_queue_id, mocker, metadata
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "delete_queue",
            side_effect=QueueNotFoundError("test-queue-id"),
        )

        response = client.delete(
            f"/api/v1/queues/durable?queue_id={mock_queue_id}",
            headers=metadata,
        )

        assert response.status_code == 404

    async def test_returns_401_for_missing_auth(self, client, mock_queue_id):
        response = client.delete(
            f"/api/v1/queues/durable?queue_id={mock_queue_id}",
        )

        assert response.status_code == 401

    async def test_returns_422_for_missing_queue_id(self, client, metadata):
        response = client.delete(
            "/api/v1/queues/durable",
            headers=metadata,
        )

        assert response.status_code == 422
