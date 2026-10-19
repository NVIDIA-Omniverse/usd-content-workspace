# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest

from app.lib.rabbitmq import DurableQueueConsumer
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    DeleteDurableQueueRequest,
)


@pytest.mark.asyncio
async def test_creates_durable_queue(event_consumer_stub, metadata, mocker):
    """
    Done this way to prevent warning
    of aio-pika code running with no event loop.
    """
    spy = mocker.patch.object(DurableQueueConsumer, "delete_queue")

    grpc_response = await event_consumer_stub.DeleteDurableQueue(
        DeleteDurableQueueRequest(queue_id="test-id"), metadata=metadata
    )

    assert grpc_response
    spy.assert_called_once_with("test-id")
