# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import pytest
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2
from consumer_grpc_helpers import (
    add_filters,
    create_durable_queue_grpc,
    delete_durable_queue_grpc,
    retry_on_resource_exhausted_grpc,
    update_durable_queue_grpc,
)
from utils import timeout_grpc_call

logger = logging.getLogger(__name__)


@pytest.mark.grpc
def test_create_consume_and_delete_durable_queue_grpc(
    consumer_grpc_stub,
    grpc_metadata,
    resource_base_url,
):
    """Create a durable queue, connect via bidi stream with acks, then delete."""
    queue_id = None
    try:
        queue_id = create_durable_queue_grpc(
            consumer_grpc_stub, grpc_metadata, resource_base_url
        )
        assert queue_id, "Queue ID should not be empty"
        logger.info("CreateDurableQueue succeeded - Queue ID: %s", queue_id)

        # Verify the bidi ConsumeDurableEventsWithAcks stream connects
        def request_generator():
            yield event_consumer_pb2.ConsumeDurableEventsWithAcksRequest(
                queue_id=queue_id
            )

        def _consume():
            return timeout_grpc_call(
                consumer_grpc_stub.ConsumeDurableEventsWithAcks,
                request_generator(),
                metadata=grpc_metadata,
            )

        stream = retry_on_resource_exhausted_grpc(_consume)
        stream.cancel()
        logger.info("ConsumeDurableEventsWithAcks bidi stream connected")

        delete_durable_queue_grpc(grpc_metadata, consumer_grpc_stub, queue_id)
        logger.info("DeleteDurableQueue succeeded")
        queue_id = None
    finally:
        if queue_id:
            try:
                delete_durable_queue_grpc(grpc_metadata, consumer_grpc_stub, queue_id)
            except Exception:
                pass


@pytest.mark.grpc
def test_update_durable_queue_grpc(
    consumer_grpc_stub,
    grpc_metadata,
    resource_base_url,
):
    """Create durable queue, update filters via gRPC, then delete (consumer-only)."""
    queue_id = None
    try:
        queue_id = create_durable_queue_grpc(
            consumer_grpc_stub, grpc_metadata, resource_base_url
        )
        assert queue_id, "Queue ID should not be empty"

        update_durable_queue_grpc(
            consumer_grpc_stub, grpc_metadata, queue_id, resource_base_url
        )
        logger.info("UpdateDurableQueue succeeded")

        delete_durable_queue_grpc(grpc_metadata, consumer_grpc_stub, queue_id)
        queue_id = None
    finally:
        if queue_id:
            try:
                delete_durable_queue_grpc(grpc_metadata, consumer_grpc_stub, queue_id)
            except Exception:
                pass


@pytest.mark.grpc
def test_non_durable_stream_reconnect_token_grpc(
    consumer_grpc_stub,
    grpc_metadata,
    resource_base_url,
):
    """Connect to ConsumeNonDurableEvents and verify we get a reconnect token."""
    consume_request = event_consumer_pb2.ConsumeNonDurableEventsRequest()
    add_filters(consume_request, resource_base_url)

    def request_generator():
        yield consume_request

    def _consume():
        return timeout_grpc_call(
            consumer_grpc_stub.ConsumeNonDurableEvents,
            request_generator(),
            metadata=grpc_metadata,
        )

    stream = retry_on_resource_exhausted_grpc(_consume)
    reconnect_token = None
    for response in stream:
        reconnect_token = response.reconnect_token
        break
    stream.cancel()

    assert reconnect_token, "Failed to get reconnect token from gRPC stream"
