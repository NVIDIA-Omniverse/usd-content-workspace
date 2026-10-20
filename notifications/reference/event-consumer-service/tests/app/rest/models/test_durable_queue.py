# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for durable queue models."""

import pytest
from pydantic import ValidationError

from app.lib.models.filter_group import FilterGroup
from app.lib.models.filter_type import FilterType
from app.lib.models.resource_filter import ResourceFilter
from app.rest.models.durable_queue_models import (
    CreateDurableQueueRequest,
    CreateDurableQueueResponse,
    DeleteDurableQueueResponse,
    UpdateDurableQueueRequest,
    UpdateDurableQueueResponse,
)
from tests.fixtures.queue_id_bounds import (
    MALFORMED_QUEUE_IDS,
    MALFORMED_QUEUE_ID_IDS,
    MAX_QUEUE_ID_LENGTH,
)


@pytest.fixture
def valid_filter_group():
    return FilterGroup(
        event_type="test.event",
        filters=(
            ResourceFilter(filter_type=FilterType.EQ, resource_id="test-resource"),
        ),
    )


class TestCreateDurableQueueRequest:
    def test_creates_with_empty_filter_groups_by_default(self):
        request = CreateDurableQueueRequest()
        assert request.filter_groups == []

    def test_creates_with_provided_filter_groups(self, valid_filter_group):
        request = CreateDurableQueueRequest(filter_groups=[valid_filter_group])
        assert len(request.filter_groups) == 1
        assert request.filter_groups[0].event_type == "test.event"

    def test_serializes_to_dict(self, valid_filter_group):
        request = CreateDurableQueueRequest(filter_groups=[valid_filter_group])
        result = request.model_dump()
        assert "filter_groups" in result
        assert len(result["filter_groups"]) == 1

    def test_deserializes_from_dict(self):
        data = {
            "filter_groups": [
                {
                    "event_type": "test.event",
                    "filters": [{"filter_type": "eq", "resource_id": "test-resource"}],
                }
            ]
        }
        request = CreateDurableQueueRequest(**data)
        assert len(request.filter_groups) == 1
        assert request.filter_groups[0].event_type == "test.event"

    def test_deserializes_from_dict_with_uppercase_filter_type(self):
        """Test backward compatibility: old uppercase values still work."""
        data = {
            "filter_groups": [
                {
                    "event_type": "test.event",
                    "filters": [{"filter_type": "EQ", "resource_id": "test-resource"}],
                }
            ]
        }
        request = CreateDurableQueueRequest(**data)
        assert len(request.filter_groups) == 1
        assert request.filter_groups[0].event_type == "test.event"
        # Verify it serializes back to lowercase
        serialized = request.model_dump(mode="json")
        filter_type = serialized["filter_groups"][0]["filters"][0]["filter_type"]
        assert filter_type == "eq"

    def test_accepts_empty_filter_groups_list(self):
        request = CreateDurableQueueRequest(filter_groups=[])
        assert request.filter_groups == []

    def test_accepts_valid_queue_id(self):
        request = CreateDurableQueueRequest(queue_id="my-app-1")
        assert request.queue_id == "my-app-1"

    def test_rejects_invalid_queue_id(self):
        with pytest.raises(ValidationError) as exc_info:
            CreateDurableQueueRequest(queue_id="bad:id")
        assert "queue_id" in str(exc_info.value)


class TestCreateDurableQueueResponse:
    def test_creates_with_queue_id(self):
        response = CreateDurableQueueResponse(queue_id="test-queue-id", created=True)
        assert response.queue_id == "test-queue-id"

    # noinspection PyArgumentList
    def test_requires_queue_id(self):
        with pytest.raises(ValidationError) as exc_info:
            CreateDurableQueueResponse(created=True)
        assert "queue_id" in str(exc_info.value)

    # noinspection PyArgumentList
    def test_requires_created(self):
        with pytest.raises(ValidationError) as exc_info:
            CreateDurableQueueResponse(queue_id="test-queue-id")
        assert "created" in str(exc_info.value)

    def test_serializes_to_dict(self):
        response = CreateDurableQueueResponse(queue_id="test-queue-id", created=True)
        result = response.model_dump()
        assert result == {"queue_id": "test-queue-id", "created": True}

    def test_deserializes_from_dict(self):
        data = {"queue_id": "test-queue-id", "created": True}
        response = CreateDurableQueueResponse(**data)
        assert response.queue_id == "test-queue-id"


class TestDeleteDurableQueueResponse:
    def test_creates_with_default_success_true(self):
        response = DeleteDurableQueueResponse()
        assert response.success is True

    def test_creates_with_explicit_success_value(self):
        response = DeleteDurableQueueResponse(success=False)
        assert response.success is False

    def test_serializes_to_dict(self):
        response = DeleteDurableQueueResponse()
        result = response.model_dump()
        assert result == {"success": True}

    def test_deserializes_from_dict(self):
        data = {"success": True}
        response = DeleteDurableQueueResponse(**data)
        assert response.success is True


class TestUpdateDurableQueueRequest:
    def test_creates_with_all_required_fields(self, valid_filter_group):
        request = UpdateDurableQueueRequest(
            queue_id="test-queue-id",
            current_filter_groups=[valid_filter_group],
            new_filter_groups=[valid_filter_group],
        )
        assert request.queue_id == "test-queue-id"
        assert len(request.current_filter_groups) == 1
        assert len(request.new_filter_groups) == 1

    # noinspection PyArgumentList
    def test_requires_queue_id(self, valid_filter_group):
        with pytest.raises(ValidationError) as exc_info:
            UpdateDurableQueueRequest(
                current_filter_groups=[valid_filter_group],
                new_filter_groups=[valid_filter_group],
            )
        assert "queue_id" in str(exc_info.value)

    @pytest.mark.parametrize(
        "queue_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    def test_rejects_malformed_queue_id(self, queue_id, valid_filter_group):
        with pytest.raises(ValidationError) as exc_info:
            UpdateDurableQueueRequest(
                queue_id=queue_id,
                current_filter_groups=[valid_filter_group],
                new_filter_groups=[valid_filter_group],
            )
        assert "queue_id" in str(exc_info.value)

    def test_accepts_queue_id_at_the_length_bound(self, valid_filter_group):
        queue_id = "a" * MAX_QUEUE_ID_LENGTH

        request = UpdateDurableQueueRequest(
            queue_id=queue_id,
            current_filter_groups=[valid_filter_group],
            new_filter_groups=[valid_filter_group],
        )

        assert request.queue_id == queue_id

    # noinspection PyArgumentList
    def test_requires_current_filter_groups(self, valid_filter_group):
        with pytest.raises(ValidationError) as exc_info:
            UpdateDurableQueueRequest(
                queue_id="test-queue-id",
                new_filter_groups=[valid_filter_group],
            )
        assert "current_filter_groups" in str(exc_info.value)

    # noinspection PyArgumentList
    def test_requires_new_filter_groups(self, valid_filter_group):
        with pytest.raises(ValidationError) as exc_info:
            UpdateDurableQueueRequest(
                queue_id="test-queue-id",
                current_filter_groups=[valid_filter_group],
            )
        assert "new_filter_groups" in str(exc_info.value)

    def test_serializes_to_dict(self, valid_filter_group):
        request = UpdateDurableQueueRequest(
            queue_id="test-queue-id",
            current_filter_groups=[valid_filter_group],
            new_filter_groups=[valid_filter_group],
        )
        result = request.model_dump()
        assert "queue_id" in result
        assert "current_filter_groups" in result
        assert "new_filter_groups" in result
        assert result["queue_id"] == "test-queue-id"

    def test_deserializes_from_dict(self):
        data = {
            "queue_id": "test-queue-id",
            "current_filter_groups": [
                {
                    "event_type": "test.event",
                    "filters": [{"filter_type": "eq", "resource_id": "test-resource"}],
                }
            ],
            "new_filter_groups": [
                {
                    "event_type": "new.event",
                    "filters": [
                        {"filter_type": "starts_with_greedy", "resource_id": "new/"}
                    ],
                }
            ],
        }
        request = UpdateDurableQueueRequest(**data)
        assert request.queue_id == "test-queue-id"
        assert len(request.current_filter_groups) == 1
        assert len(request.new_filter_groups) == 1
        assert request.current_filter_groups[0].event_type == "test.event"
        assert request.new_filter_groups[0].event_type == "new.event"

    def test_deserializes_uppercase_filter_types_for_backward_compatibility(self):
        """Test backward compatibility: old uppercase values still work."""
        data = {
            "queue_id": "test-queue-id",
            "current_filter_groups": [
                {
                    "event_type": "test.event",
                    "filters": [{"filter_type": "EQ", "resource_id": "test-resource"}],
                }
            ],
            "new_filter_groups": [
                {
                    "event_type": "new.event",
                    "filters": [
                        {"filter_type": "STARTS_WITH_GREEDY", "resource_id": "new/"}
                    ],
                }
            ],
        }
        request = UpdateDurableQueueRequest(**data)
        assert request.queue_id == "test-queue-id"
        # Verify it serializes back to lowercase
        serialized = request.model_dump(mode="json")
        current_filter_type = serialized["current_filter_groups"][0]["filters"][0][
            "filter_type"
        ]
        new_filter_type = serialized["new_filter_groups"][0]["filters"][0][
            "filter_type"
        ]
        assert current_filter_type == "eq"
        assert new_filter_type == "starts_with_greedy"

    def test_accepts_empty_filter_groups_lists(self):
        request = UpdateDurableQueueRequest(
            queue_id="test-queue-id",
            current_filter_groups=[],
            new_filter_groups=[],
        )
        assert request.current_filter_groups == []
        assert request.new_filter_groups == []


class TestUpdateDurableQueueResponse:
    def test_creates_with_default_success_true(self):
        response = UpdateDurableQueueResponse()
        assert response.success is True

    def test_creates_with_explicit_success_value(self):
        response = UpdateDurableQueueResponse(success=False)
        assert response.success is False

    def test_serializes_to_dict(self):
        response = UpdateDurableQueueResponse()
        result = response.model_dump()
        assert result == {"success": True}

    def test_deserializes_from_dict(self):
        data = {"success": False}
        response = UpdateDurableQueueResponse(**data)
        assert response.success is False
