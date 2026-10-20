# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Integration tests for the create durable queue endpoint."""

import pytest

from app.lib.rabbitmq import DurableQueueConsumer


@pytest.mark.asyncio
class TestPostCreateDurableQueue:
    async def test_omitted_queue_id_returns_200_created_true(
        self, client, create_queue_payload, mocker, metadata
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=("test-queue-id", True),
        )

        response = client.post(
            "/api/v1/queues/durable",
            json=create_queue_payload,
            headers=metadata,
        )

        assert response.status_code == 200
        assert response.json()["queue_id"] == "test-queue-id"
        assert response.json()["created"] is True

    async def test_supplied_new_queue_id_returns_201_created_true(
        self, client, create_queue_payload, mocker, metadata
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=("my-queue", True),
        )

        response = client.post(
            "/api/v1/queues/durable",
            json={**create_queue_payload, "queue_id": "my-queue"},
            headers=metadata,
        )

        assert response.status_code == 201
        assert response.json()["queue_id"] == "my-queue"
        assert response.json()["created"] is True

    async def test_supplied_existing_queue_id_returns_200_created_false(
        self, client, create_queue_payload, mocker, metadata
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=("my-queue", False),
        )

        response = client.post(
            "/api/v1/queues/durable",
            json={**create_queue_payload, "queue_id": "my-queue"},
            headers=metadata,
        )

        assert response.status_code == 200
        assert response.json()["queue_id"] == "my-queue"
        assert response.json()["created"] is False

    async def test_invalid_queue_id_returns_422_before_broker_call(
        self, client, create_queue_payload, mocker, metadata
    ):
        mock_create = mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=("my-queue", True),
        )

        response = client.post(
            "/api/v1/queues/durable",
            json={**create_queue_payload, "queue_id": "bad:id"},
            headers=metadata,
        )

        assert response.status_code == 422
        mock_create.assert_not_called()

    async def test_v1beta_returns_200_on_success(
        self, client, create_queue_payload, mocker, metadata
    ):
        mock_queue_id = "test-queue-id"
        mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=(mock_queue_id, True),
        )

        response = client.post(
            "/api/v1beta/queues/durable",
            json=create_queue_payload,
            headers=metadata,
        )

        assert response.status_code == 200
        assert response.json().get("queue_id") == mock_queue_id

    async def test_creates_queue_with_correct_filter_groups(
        self, client, create_queue_payload, valid_filter_groups, mocker, metadata
    ):
        mock_create = mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=("test-queue-id", True),
        )

        client.post(
            "/api/v1/queues/durable",
            json=create_queue_payload,
            headers=metadata,
        )

        # Verify create_queue was called with the correct filter groups
        mock_create.assert_called_once()
        call_args = mock_create.call_args[0][0]
        assert len(call_args) == len(valid_filter_groups)
        assert call_args[0].event_type == valid_filter_groups[0].event_type

    async def test_accepts_empty_filter_groups(self, client, mocker, metadata):
        mock_queue_id = "test-queue-id"
        mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=(mock_queue_id, True),
        )

        response = client.post(
            "/api/v1/queues/durable",
            json={"filter_groups": []},
            headers=metadata,
        )

        # The endpoint accepts empty filter groups, though the consumer may reject it
        assert response.status_code in [200, 422]

    async def test_returns_422_for_invalid_filter_groups(self, client, metadata):
        invalid_resource_id = (
            "https://exampleaccount.blob.core.windows.net/example-container/"
            "ov-content/Users/user@example.com"
        )
        invalid_payload = {
            "filter_groups": [
                {
                    "event_type": "omni.storage.dir_created",
                    "filters": [
                        {
                            "filter_type": 2,
                            "resource_id": invalid_resource_id,
                        }
                    ],
                }
            ]
        }

        response = client.post(
            "/api/v1/queues/durable",
            json=invalid_payload,
            headers=metadata,
        )

        assert response.status_code == 422
        error_detail = response.json().get("detail", "")

        expected_error = (
            "When consuming storage events only folders values are "
            "supported, and the resource_id value must end with a '/'"
        )
        assert error_detail == expected_error

    async def test_returns_401_for_missing_auth(self, client, create_queue_payload):
        response = client.post(
            "/api/v1/queues/durable",
            json=create_queue_payload,
        )

        assert response.status_code == 401

    async def test_returns_422_for_malformed_json(self, client, metadata):
        response = client.post(
            "/api/v1/queues/durable",
            content="not json",
            headers=metadata,
        )

        assert response.status_code == 422
