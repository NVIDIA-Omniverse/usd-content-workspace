# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Integration tests for the update durable queue endpoint."""

import pytest
from notification_common.errors.errors import QueueNotFoundError

from app.lib.models.filter_group import FilterGroup
from app.lib.models.filter_type import FilterType
from app.lib.models.resource_filter import ResourceFilter
from app.lib.rabbitmq import DurableQueueConsumer
from tests.integration_tests.rest.conftest import serialize_filter_groups


@pytest.fixture
def mock_queue_id():
    """Return a test queue ID."""
    return "test-queue-id"


@pytest.fixture
def current_filter_group():
    """Create a current filter group for testing."""
    return FilterGroup(
        event_type="test_event",
        filters=(
            ResourceFilter(filter_type=FilterType.EQ, resource_id="test_resource"),
        ),
    )


@pytest.fixture
def new_filter_group():
    """Create a new filter group for testing."""
    return FilterGroup(
        event_type="new_test_event",
        filters=(
            ResourceFilter(
                filter_type=FilterType.STARTS_WITH_GREEDY, resource_id="new_resource/"
            ),
        ),
    )


@pytest.fixture
def current_filter_groups(current_filter_group):
    """Return a list with a current filter group."""
    return [current_filter_group]


@pytest.fixture
def new_filter_groups(new_filter_group):
    """Return a list with a new filter group."""
    return [new_filter_group]


@pytest.fixture
def update_queue_payload(mock_queue_id, current_filter_groups, new_filter_groups):
    """Create a valid payload for updating a durable queue."""
    return {
        "queue_id": mock_queue_id,
        "current_filter_groups": serialize_filter_groups(current_filter_groups),
        "new_filter_groups": serialize_filter_groups(new_filter_groups),
    }


@pytest.mark.asyncio
class TestPatchUpdateDurableQueue:
    async def test_returns_200_on_success(
        self, client, update_queue_payload, mocker, metadata
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "update_queue",
            return_value=None,
        )

        response = client.patch(
            "/api/v1/queues/durable",
            json=update_queue_payload,
            headers=metadata,
        )

        assert response.status_code == 200
        assert response.json().get("success") is True

    async def test_v1beta_returns_200_on_success(
        self, client, update_queue_payload, mocker, metadata
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "update_queue",
            return_value=None,
        )

        response = client.patch(
            "/api/v1beta/queues/durable",
            json=update_queue_payload,
            headers=metadata,
        )

        assert response.status_code == 200
        assert response.json().get("success") is True

    async def test_calls_update_with_correct_parameters(
        self,
        client,
        update_queue_payload,
        mock_queue_id,
        current_filter_groups,
        new_filter_groups,
        mocker,
        metadata,
    ):
        mock_update = mocker.patch.object(
            DurableQueueConsumer,
            "update_queue",
            return_value=None,
        )

        client.patch(
            "/api/v1/queues/durable",
            json=update_queue_payload,
            headers=metadata,
        )

        # Verify update_queue was called with the correct parameters
        mock_update.assert_called_once()
        call_args = mock_update.call_args[0]
        assert call_args[0] == mock_queue_id
        assert len(call_args[1]) == len(current_filter_groups)
        assert len(call_args[2]) == len(new_filter_groups)
        assert call_args[1][0].event_type == current_filter_groups[0].event_type
        assert call_args[2][0].event_type == new_filter_groups[0].event_type

    async def test_returns_404_for_nonexistent_queue(
        self, client, update_queue_payload, mocker, metadata
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "update_queue",
            side_effect=QueueNotFoundError("test-queue-id"),
        )

        response = client.patch(
            "/api/v1/queues/durable",
            json=update_queue_payload,
            headers=metadata,
        )

        assert response.status_code == 404

    async def test_returns_401_for_missing_auth(self, client, update_queue_payload):
        response = client.patch(
            "/api/v1/queues/durable",
            json=update_queue_payload,
        )

        assert response.status_code == 401

    async def test_returns_422_for_missing_queue_id(
        self, client, current_filter_groups, new_filter_groups, metadata
    ):
        response = client.patch(
            "/api/v1/queues/durable",
            json={
                "current_filter_groups": serialize_filter_groups(current_filter_groups),
                "new_filter_groups": serialize_filter_groups(new_filter_groups),
            },
            headers=metadata,
        )

        assert response.status_code == 422

    async def test_returns_422_for_missing_current_filter_groups(
        self, client, mock_queue_id, new_filter_groups, metadata
    ):
        response = client.patch(
            "/api/v1/queues/durable",
            json={
                "queue_id": mock_queue_id,
                "new_filter_groups": serialize_filter_groups(new_filter_groups),
            },
            headers=metadata,
        )

        assert response.status_code == 422

    async def test_returns_422_for_missing_new_filter_groups(
        self, client, mock_queue_id, current_filter_groups, metadata
    ):
        response = client.patch(
            "/api/v1/queues/durable",
            json={
                "queue_id": mock_queue_id,
                "current_filter_groups": serialize_filter_groups(current_filter_groups),
            },
            headers=metadata,
        )

        assert response.status_code == 422

    async def test_returns_422_for_malformed_json(self, client, metadata):
        response = client.patch(
            "/api/v1/queues/durable",
            content="not json",
            headers=metadata,
        )

        assert response.status_code == 422

    async def test_returns_422_for_invalid_new_filter_groups(
        self, client, mock_queue_id, current_filter_groups, metadata
    ):
        invalid_resource_id = (
            "https://exampleaccount.blob.core.windows.net/example-container/"
            "ov-content/Users/user@example.com"
        )
        invalid_payload = {
            "queue_id": mock_queue_id,
            "current_filter_groups": serialize_filter_groups(current_filter_groups),
            "new_filter_groups": [
                {
                    "event_type": "omni.storage.dir_created",
                    "filters": [
                        {
                            "filter_type": 2,
                            "resource_id": invalid_resource_id,
                        }
                    ],
                }
            ],
        }

        response = client.patch(
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
