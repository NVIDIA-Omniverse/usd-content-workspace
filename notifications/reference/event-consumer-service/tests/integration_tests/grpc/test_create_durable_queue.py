# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import grpc
import pytest
from grpc.aio import AioRpcError

from app.grpc import event_consumer_servicer
from app.lib.rabbitmq import DurableQueueConsumer
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    CreateDurableQueueRequest,
)
from tests.integration_tests.conftest import filter_group


class TestCreateDurableQueue:
    @pytest.mark.asyncio
    async def test_no_queue_id_returns_server_generated_id_created_true(
        self, event_consumer_stub, metadata, mocker
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=("generated-id", True),
        )

        request = CreateDurableQueueRequest()
        request.filter_groups.append(filter_group())

        grpc_response = await event_consumer_stub.CreateDurableQueue(
            request, metadata=metadata
        )

        assert grpc_response.queue_id == "generated-id"

    @pytest.mark.asyncio
    async def test_no_queue_id_sets_created_true(
        self, event_consumer_stub, metadata, mocker
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=("generated-id", True),
        )

        request = CreateDurableQueueRequest()
        request.filter_groups.append(filter_group())

        grpc_response = await event_consumer_stub.CreateDurableQueue(
            request, metadata=metadata
        )

        assert grpc_response.created is True

    @pytest.mark.asyncio
    async def test_no_queue_id_calls_create_queue_without_client_id(
        self, event_consumer_stub, metadata, mocker
    ):
        mocker.patch.object(
            event_consumer_servicer,
            "convert_to_models",
            return_value="converted-filter-groups",
        )
        spy = mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=("generated-id", True),
        )

        request = CreateDurableQueueRequest()
        request.filter_groups.append(filter_group())

        await event_consumer_stub.CreateDurableQueue(request, metadata=metadata)

        spy.assert_called_once_with("converted-filter-groups", queue_id=None)

    @pytest.mark.asyncio
    async def test_supplied_new_queue_id_returns_that_id(
        self, event_consumer_stub, metadata, mocker
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=("my-app-1", True),
        )

        request = CreateDurableQueueRequest(queue_id="my-app-1")
        request.filter_groups.append(filter_group())

        grpc_response = await event_consumer_stub.CreateDurableQueue(
            request, metadata=metadata
        )

        assert grpc_response.queue_id == "my-app-1"

    @pytest.mark.asyncio
    async def test_supplied_new_queue_id_sets_created_true(
        self, event_consumer_stub, metadata, mocker
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=("my-app-1", True),
        )

        request = CreateDurableQueueRequest(queue_id="my-app-1")
        request.filter_groups.append(filter_group())

        grpc_response = await event_consumer_stub.CreateDurableQueue(
            request, metadata=metadata
        )

        assert grpc_response.created is True

    @pytest.mark.asyncio
    async def test_supplied_queue_id_is_threaded_to_create_queue(
        self, event_consumer_stub, metadata, mocker
    ):
        mocker.patch.object(
            event_consumer_servicer,
            "convert_to_models",
            return_value="converted-filter-groups",
        )
        spy = mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=("my-app-1", True),
        )

        request = CreateDurableQueueRequest(queue_id="my-app-1")
        request.filter_groups.append(filter_group())

        await event_consumer_stub.CreateDurableQueue(request, metadata=metadata)

        spy.assert_called_once_with("converted-filter-groups", queue_id="my-app-1")

    @pytest.mark.asyncio
    async def test_supplied_existing_queue_id_returns_that_id(
        self, event_consumer_stub, metadata, mocker
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=("my-app-1", False),
        )

        request = CreateDurableQueueRequest(queue_id="my-app-1")
        request.filter_groups.append(filter_group())

        grpc_response = await event_consumer_stub.CreateDurableQueue(
            request, metadata=metadata
        )

        assert grpc_response.queue_id == "my-app-1"

    @pytest.mark.asyncio
    async def test_supplied_existing_queue_id_sets_created_false(
        self, event_consumer_stub, metadata, mocker
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "create_queue",
            return_value=("my-app-1", False),
        )

        request = CreateDurableQueueRequest(queue_id="my-app-1")
        request.filter_groups.append(filter_group())

        grpc_response = await event_consumer_stub.CreateDurableQueue(
            request, metadata=metadata
        )

        assert grpc_response.created is False

    @pytest.mark.parametrize(
        "invalid_queue_id",
        [
            "bad:id",
            "x" * 201,
            "abc\n",
        ],
    )
    @pytest.mark.asyncio
    async def test_invalid_queue_id_raises_invalid_argument(
        self, invalid_queue_id, event_consumer_stub, metadata, mocker
    ):
        mocker.patch.object(DurableQueueConsumer, "create_queue")

        request = CreateDurableQueueRequest(queue_id=invalid_queue_id)
        request.filter_groups.append(filter_group())

        with pytest.raises(AioRpcError) as exc_info:
            await event_consumer_stub.CreateDurableQueue(request, metadata=metadata)

        assert exc_info.value.code() == grpc.StatusCode.INVALID_ARGUMENT

    @pytest.mark.parametrize(
        "invalid_queue_id",
        [
            "bad:id",
            "x" * 201,
            "abc\n",
        ],
    )
    @pytest.mark.asyncio
    async def test_invalid_queue_id_does_not_call_create_queue(
        self, invalid_queue_id, event_consumer_stub, metadata, mocker
    ):
        spy = mocker.patch.object(DurableQueueConsumer, "create_queue")

        request = CreateDurableQueueRequest(queue_id=invalid_queue_id)
        request.filter_groups.append(filter_group())

        with pytest.raises(AioRpcError):
            await event_consumer_stub.CreateDurableQueue(request, metadata=metadata)

        spy.assert_not_called()
