# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import grpc
import pytest
from grpc.aio import AioRpcError

from app.grpc import event_consumer_servicer
from app.lib.rabbitmq import DurableQueueConsumer
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    UpdateDurableQueueRequest,
    UpdateDurableQueueResponse,
    FilterGroup,
    ResourceFilter,
)
from tests.integration_tests.conftest import filter_group


@pytest.fixture
def req() -> UpdateDurableQueueRequest:
    request = UpdateDurableQueueRequest(queue_id="test-id")

    current_filters = filter_group()
    new_filters = filter_group()

    request.current_filter_groups.append(current_filters)
    request.new_filter_groups.append(new_filters)

    return request


@pytest.mark.asyncio
async def test_validates_filter_groups(event_consumer_stub, metadata):
    invalid_request = UpdateDurableQueueRequest(queue_id="test-id")
    invalid_request.current_filter_groups.append(filter_group())

    invalid_filter_group = FilterGroup(
        event_type="test-event",
        filters=[
            ResourceFilter(
                filter_type=ResourceFilter.FilterType.FILTER_TYPE_EQ, resource_id=""
            ),
        ],
    )
    invalid_request.new_filter_groups.append(invalid_filter_group)

    with pytest.raises(AioRpcError) as exc_info:
        await event_consumer_stub.UpdateDurableQueue(invalid_request, metadata=metadata)

    assert exc_info.value.code() == grpc.StatusCode.INVALID_ARGUMENT

    details = exc_info.value.details()
    expected_error_msg = "Resource id is required when filter type is EQ"

    assert details is not None and expected_error_msg in details


@pytest.mark.asyncio
async def test_updates_queue_bindings(req, event_consumer_stub, metadata, mocker):
    mocker.patch.object(
        event_consumer_servicer,
        "convert_to_models",
        side_effect=["current-filter-groups", "new-filter-groups"],
    )
    mocker.patch.object(DurableQueueConsumer, "get_queue", return_value="test-queue")

    spy = mocker.patch.object(DurableQueueConsumer, "update_queue")

    await event_consumer_stub.UpdateDurableQueue(req, metadata=metadata)
    spy.assert_called_once_with("test-id", "current-filter-groups", "new-filter-groups")


@pytest.mark.asyncio
async def test_returns_response(req, event_consumer_stub, metadata, mocker):
    mocker.patch.object(DurableQueueConsumer, "get_queue", return_value="test-queue")

    assert isinstance(
        await event_consumer_stub.UpdateDurableQueue(req, metadata=metadata),
        UpdateDurableQueueResponse,
    )
