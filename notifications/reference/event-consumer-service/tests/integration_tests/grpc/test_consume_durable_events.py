# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio

import pytest
import grpc
from google.protobuf import json_format
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ConsumeDurableEventsRequest,
    ConsumeDurableEventsWithAcksRequest,
)


def _request_stream(queue_id="test-queue-id"):
    """Generate a request stream for ConsumeDurableEventsWithAcks."""

    def requests():
        yield ConsumeDurableEventsWithAcksRequest(queue_id=queue_id)

    return requests


async def _settle_after_deliveries(
    event_consumer_stub,
    metadata,
    deliveries,
    build_settle_request,
    queue_id="test-queue-id",
):
    """
    Drive a with-acks stream that settles only tags it has already been given.

    The client waits for ``deliveries`` responses, hands their delivery tags
    to ``build_settle_request``, and sends the single request it returns.
    That ordering is what removes the settle race these tests used to carry:
    a request sent before the server issued the tag would take the
    unknown-tag skip, and every assertion about the settle having applied
    would pass vacuously.

    :param deliveries: How many responses to wait for before settling.
    :param build_settle_request: Called with the delivery tags of those
        responses, in order; returns the request to send.
    :return: Every response the stream produced, in order.
    """
    settle_tags: asyncio.Queue = asyncio.Queue()

    async def requests():
        yield ConsumeDurableEventsWithAcksRequest(queue_id=queue_id)
        yield build_settle_request(await settle_tags.get())

    responses = []
    async for grpc_response in event_consumer_stub.ConsumeDurableEventsWithAcks(
        requests(), metadata=metadata
    ):
        responses.append(grpc_response)
        if len(responses) == deliveries:
            settle_tags.put_nowait([r.delivery_tag for r in responses])

    return responses


@pytest.mark.asyncio
async def test_consumes_all_events(event_consumer_stub, metadata, mock_record_success):
    responses = []

    async for grpc_response in event_consumer_stub.ConsumeDurableEvents(
        ConsumeDurableEventsRequest(queue_id="test-queue-id"),
        metadata=metadata,
    ):
        responses.append(grpc_response)

    assert len(responses) == 3
    assert mock_record_success.call_count == 3


@pytest.mark.asyncio
async def test_raises_error_if_validation_fails(event_consumer_stub, metadata):
    with pytest.raises(
        grpc.aio.AioRpcError,
        match=".*Queue ID is missing from consume durable request.*",
    ):
        async for _ in event_consumer_stub.ConsumeDurableEvents(
            ConsumeDurableEventsRequest(queue_id=""),
            metadata=metadata,
        ):
            pass


@pytest.mark.asyncio
async def test_with_acks_consumes_all_events(
    event_consumer_stub, metadata, mock_record_success
):
    responses = []

    async for grpc_response in event_consumer_stub.ConsumeDurableEventsWithAcks(
        _request_stream()(), metadata=metadata
    ):
        responses.append(grpc_response)

    assert len(responses) == 3
    assert mock_record_success.call_count == 3


@pytest.mark.asyncio
async def test_with_acks_responses_include_delivery_tags(event_consumer_stub, metadata):
    responses = []

    async for grpc_response in event_consumer_stub.ConsumeDurableEventsWithAcks(
        _request_stream()(), metadata=metadata
    ):
        responses.append(grpc_response)

    assert len(responses) == 3
    # Each response should have a non-zero delivery tag
    for resp in responses:
        response_dict = json_format.MessageToDict(
            resp, preserving_proto_field_name=True
        )
        assert "delivery_tag" in response_dict
        assert isinstance(response_dict["delivery_tag"], str)


@pytest.mark.asyncio
async def test_with_acks_client_can_send_acks(
    event_consumer_stub, metadata, mock_queue_iterator
):
    # Asserted at the broker boundary, not on the response count: an ack
    # that fell through the unknown-tag skip would leave the same three
    # responses behind. Every delivered message is acked and none nacked.
    await _settle_after_deliveries(
        event_consumer_stub,
        metadata,
        deliveries=3,
        build_settle_request=lambda tags: ConsumeDurableEventsWithAcksRequest(
            ack_delivery_tags=tags
        ),
    )

    assert len(mock_queue_iterator.messages) == 3
    for message in mock_queue_iterator.messages:
        message.ack.assert_awaited_once()
        message.nack.assert_not_awaited()


@pytest.mark.asyncio
async def test_with_acks_client_can_send_a_batch_of_nacks(
    event_consumer_stub, metadata, mock_redelivering_durable_queue
):
    # Every tag in one nack_delivery_tags field is settled, not just the
    # first. Asserted at the broker boundary the servicer actually calls:
    # the previous version of this test only counted the canned events, so
    # it passed identically whether the nacks applied or fell through the
    # unknown-tag skip.
    await _settle_after_deliveries(
        event_consumer_stub,
        metadata,
        deliveries=3,
        build_settle_request=lambda tags: ConsumeDurableEventsWithAcksRequest(
            nack_delivery_tags=tags
        ),
    )

    for message in mock_redelivering_durable_queue.messages[:3]:
        message.nack.assert_awaited_once_with(requeue=True)


@pytest.mark.asyncio
async def test_with_acks_client_can_send_acks_and_nacks_in_one_request(
    event_consumer_stub, metadata, mock_redelivering_durable_queue
):
    # Both repeated fields populated on the same request must be accepted on
    # the wire AND applied to the right messages: the acked tag is acked and
    # the nacked ones are nacked, rather than one field being silently
    # dropped or the two being crossed.
    await _settle_after_deliveries(
        event_consumer_stub,
        metadata,
        deliveries=3,
        build_settle_request=lambda tags: ConsumeDurableEventsWithAcksRequest(
            ack_delivery_tags=tags[:1],
            nack_delivery_tags=tags[1:],
        ),
    )

    acked, *nacked = mock_redelivering_durable_queue.messages[:3]
    acked.ack.assert_awaited_once()
    acked.nack.assert_not_awaited()
    for message in nacked:
        message.nack.assert_awaited_once_with(requeue=True)
        message.ack.assert_not_awaited()


@pytest.mark.asyncio
async def test_with_acks_unknown_nack_tag_does_not_stop_the_tags_after_it(
    event_consumer_stub, metadata, mock_redelivering_durable_queue
):
    # A stale tag from a client that is behind is skipped, not fatal. Pinned
    # on the REAL tag that follows it in the same field: the redelivery it
    # earns is proof the server consumed the request and carried on past the
    # unknown tag, which counting the canned events could not show (the
    # stream would have delivered them all even if the request was never
    # read).
    responses = await _settle_after_deliveries(
        event_consumer_stub,
        metadata,
        deliveries=1,
        build_settle_request=lambda tags: ConsumeDurableEventsWithAcksRequest(
            nack_delivery_tags=[999_999, *tags]
        ),
    )

    mock_redelivering_durable_queue.messages[0].nack.assert_awaited_once_with(
        requeue=True
    )
    assert len(responses) == 4


@pytest.mark.asyncio
async def test_with_acks_nacked_message_is_redelivered_on_the_same_stream(
    event_consumer_stub, metadata, mock_redelivering_durable_queue
):
    # The end-to-end nack contract: the tag goes out over the real bidi
    # stream in nack_delivery_tags, the servicer settles it through the real
    # AckTracker, and the requeued message comes back on THIS stream under a
    # new delivery tag.
    responses = await _settle_after_deliveries(
        event_consumer_stub,
        metadata,
        deliveries=1,
        build_settle_request=lambda tags: ConsumeDurableEventsWithAcksRequest(
            nack_delivery_tags=tags
        ),
    )

    assert len(responses) == 4
    redelivered = responses[-1]
    assert redelivered.events[0].event_type == responses[0].events[0].event_type
    assert redelivered.delivery_tag != responses[0].delivery_tag


@pytest.mark.asyncio
async def test_with_acks_nack_requeues_the_underlying_message(
    event_consumer_stub, metadata, mock_redelivering_durable_queue
):
    # requeue=True is what makes the redelivery above a requeue rather than
    # a discard, asserted at the broker boundary the servicer actually calls.
    await _settle_after_deliveries(
        event_consumer_stub,
        metadata,
        deliveries=1,
        build_settle_request=lambda tags: ConsumeDurableEventsWithAcksRequest(
            nack_delivery_tags=tags
        ),
    )

    first_message = mock_redelivering_durable_queue.messages[0]
    first_message.nack.assert_awaited_once_with(requeue=True)


@pytest.mark.asyncio
async def test_with_acks_raises_error_if_validation_fails(
    event_consumer_stub, metadata
):
    with pytest.raises(
        grpc.aio.AioRpcError,
        match=".*Queue ID is missing from consume durable request.*",
    ):
        async for _ in event_consumer_stub.ConsumeDurableEventsWithAcks(
            _request_stream(queue_id="")(), metadata=metadata
        ):
            pass
