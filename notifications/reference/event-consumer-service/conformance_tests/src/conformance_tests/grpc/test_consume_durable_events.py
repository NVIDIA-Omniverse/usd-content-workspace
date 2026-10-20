# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
gRPC conformance tests for consuming events from a durable queue.

Contracts under test:
  rpc ConsumeDurableEvents(ConsumeDurableEventsRequest)
      returns (stream ConsumeDurableEventsResponse)
  rpc ConsumeDurableEventsWithAcks(stream ConsumeDurableEventsWithAcksRequest)
      returns (stream ConsumeDurableEventsWithAcksResponse)

Expected behaviour — ConsumeDurableEvents:
  - Connecting to an existing queue opens a stream without error.
    The stream blocks waiting for events; when no events are being published
    it remains open (the test cancels after a short timeout).
  - Connecting with a non-existent queue_id returns NOT_FOUND immediately.
  - Connecting with an empty queue_id returns an error with a message indicating
    that the queue_id is missing.

Expected behaviour — ConsumeDurableEventsWithAcks:
  - The client settles delivered messages by sending their delivery tags back
    in ack_delivery_tags (acknowledge and drop) or nack_delivery_tags
    (negatively acknowledge and requeue for redelivery). One request may carry
    both fields; acks are processed first, so a tag in both is acknowledged.
  - Unknown or already-settled tags — in either field — are skipped without
    terminating the stream.

Scope note: settlement OUTCOMES (an acked message is not redelivered; a nacked
message is; a tag in both fields is acknowledged rather than requeued) are
deliberately NOT asserted here, because this suite has no way
to put a message into a durable queue. Events reach a durable queue only by
being published to the broker by the Event Aggregation Service; the Event
Consumer Service exposes no publish RPC or endpoint, and this suite is a
black-box client of that service alone (its dependencies are grpcio, protobuf
and httpx — deliberately no AMQP client, since a reimplementor need not use
RabbitMQ). What IS conformance-observable without a producer is the settlement
protocol: that both settle fields are accepted on the wire and that a stale tag
does not take the stream down. The redelivery outcomes are covered against a
live broker in the service's own integration suite
(tests/integration_tests/rabbitmq/test_client_nack_redelivery.py).
"""

import grpc
import pytest
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2 as pb2

from conformance_tests.conftest import read_grpc_stream_with_timeout

pytestmark = pytest.mark.grpc

_NON_EXISTENT_QUEUE_ID = "00000000-0000-0000-0000-000000000000"

# A delivery tag no session will have issued. Delivery tags are per-session and
# start at 1, so this stands in for both "never issued" and "already settled" —
# the two cases the contract folds together as "skipped without terminating the
# stream".
_UNKNOWN_DELIVERY_TAG = 999_999_999

# How long to hold a with-acks stream open after the settle request before
# concluding it survived. Long enough for the server to have consumed the
# request and reacted to it, short enough to keep the suite quick.
_STREAM_SURVIVAL_TIMEOUT_S = 3.0


class TestConsumeDurableEvents:
    def test_consume_from_existing_queue_opens_stream(
        self, consumer_stub, grpc_metadata
    ):
        """
        ConsumeDurableEvents on an existing queue must open a stream without
        raising an immediate error.  Because no events are being published in
        this test the stream will be empty, but it must stay open (the service
        returns an error only if the queue does not exist or is invalid).
        """
        create_resp = consumer_stub.CreateDurableQueue(
            pb2.CreateDurableQueueRequest(
                filter_groups=[pb2.FilterGroup(event_type="conformance.consume.test")]
            ),
            metadata=grpc_metadata,
        )
        queue_id = create_resp.queue_id
        assert queue_id, "Prerequisite: CreateDurableQueue returned empty queue_id."

        stream = consumer_stub.ConsumeDurableEvents(
            pb2.ConsumeDurableEventsRequest(queue_id=queue_id),
            metadata=grpc_metadata,
        )

        messages, error = read_grpc_stream_with_timeout(stream, timeout=3.0)

        # No immediate error means the queue exists and the stream is healthy.
        # It's expected to be empty because nothing is publishing events.
        assert error is None or (
            isinstance(error, grpc.RpcError)
            and error.code() == grpc.StatusCode.CANCELLED
        ), (
            f"ConsumeDurableEvents for queue '{queue_id}' returned an unexpected "
            f"error: {error}. Expected either no error or CANCELLED (from timeout)."
        )

        # Cleanup
        try:
            consumer_stub.DeleteDurableQueue(
                pb2.DeleteDurableQueueRequest(queue_id=queue_id),
                metadata=grpc_metadata,
            )
        except grpc.RpcError:
            pass

    def test_consume_from_nonexistent_queue_returns_not_found(
        self, consumer_stub, grpc_metadata
    ):
        """
        ConsumeDurableEvents with a queue_id that does not exist must return
        gRPC status NOT_FOUND immediately (before streaming any messages).
        """
        stream = consumer_stub.ConsumeDurableEvents(
            pb2.ConsumeDurableEventsRequest(queue_id=_NON_EXISTENT_QUEUE_ID),
            metadata=grpc_metadata,
        )

        messages, error = read_grpc_stream_with_timeout(stream, timeout=5.0)

        assert error is not None, (
            f"Expected ConsumeDurableEvents for non-existent queue "
            f"'{_NON_EXISTENT_QUEUE_ID}' to raise a gRPC error with status "
            f"NOT_FOUND, but the stream completed without error (messages={messages})."
        )
        assert isinstance(
            error, grpc.RpcError
        ), f"Expected a grpc.RpcError but got {type(error).__name__}: {error}"
        assert error.code() == grpc.StatusCode.NOT_FOUND, (
            f"Expected NOT_FOUND for non-existent queue "
            f"'{_NON_EXISTENT_QUEUE_ID}', but got {error.code().name}.\n"
            f"Error details: {error.details()}"
        )

    def test_consume_with_empty_queue_id_returns_invalid_argument(
        self, consumer_stub, grpc_metadata
    ):
        """
        ConsumeDurableEvents with an empty queue_id must return
        INVALID_ARGUMENT with a message containing "Queue ID is missing".
        """
        stream = consumer_stub.ConsumeDurableEvents(
            pb2.ConsumeDurableEventsRequest(queue_id=""),
            metadata=grpc_metadata,
        )

        messages, error = read_grpc_stream_with_timeout(stream, timeout=5.0)

        assert error is not None, (
            "Expected ConsumeDurableEvents with empty queue_id to return a "
            "gRPC error, but the stream completed without error."
        )
        assert isinstance(
            error, grpc.RpcError
        ), f"Expected a grpc.RpcError but got {type(error).__name__}: {error}"
        assert error.code() == grpc.StatusCode.INVALID_ARGUMENT, (
            f"Expected INVALID_ARGUMENT for empty queue_id, "
            f"but got {error.code().name}. Error: {error.details()}"
        )
        expected_fragment = "Queue ID is missing"
        details = error.details() or ""
        assert expected_fragment in details, (
            f"Expected error details to contain '{expected_fragment}' "
            f"when queue_id is empty, but got: '{details}'"
        )


def _settle_request_generator(queue_id, ack_tags=(), nack_tags=()):
    """
    Yield the two requests of a settle-only with-acks session: the opening
    request that names the queue, then one request carrying the delivery tags
    to settle.
    """
    yield pb2.ConsumeDurableEventsWithAcksRequest(queue_id=queue_id)
    yield pb2.ConsumeDurableEventsWithAcksRequest(
        ack_delivery_tags=list(ack_tags),
        nack_delivery_tags=list(nack_tags),
    )


class TestConsumeDurableEventsWithAcks:
    def _create_queue(self, consumer_stub, grpc_metadata, event_type) -> str:
        """Helper: create a durable queue and return its queue_id."""
        create_resp = consumer_stub.CreateDurableQueue(
            pb2.CreateDurableQueueRequest(
                filter_groups=[pb2.FilterGroup(event_type=event_type)]
            ),
            metadata=grpc_metadata,
        )
        queue_id = create_resp.queue_id
        assert queue_id, "Prerequisite: CreateDurableQueue returned empty queue_id."
        return queue_id

    def _delete_queue(self, consumer_stub, grpc_metadata, queue_id):
        """Helper: delete a queue (best-effort cleanup)."""
        try:
            consumer_stub.DeleteDurableQueue(
                pb2.DeleteDurableQueueRequest(queue_id=queue_id),
                metadata=grpc_metadata,
            )
        except grpc.RpcError:
            pass

    def _assert_stream_survived(self, error, queue_id, settled_description):
        """
        A with-acks stream that is still healthy ends only because this client
        stopped reading. Anything else means the settle request took the stream
        down, which the contract forbids for unknown tags.
        """
        assert error is None or (
            isinstance(error, grpc.RpcError)
            and error.code() == grpc.StatusCode.CANCELLED
        ), (
            f"ConsumeDurableEventsWithAcks on queue '{queue_id}' ended with "
            f"{error} after the client sent {settled_description}. The contract "
            f"requires unknown or already-settled delivery tags to be skipped "
            f"without terminating the stream; only no error or CANCELLED (from "
            f"the client's own read timeout) is acceptable."
        )

    @pytest.mark.parametrize(
        "event_type_suffix, ack_tags, nack_tags, settled_description",
        [
            (
                "unknown.ack",
                [_UNKNOWN_DELIVERY_TAG],
                [],
                f"ack_delivery_tags=[{_UNKNOWN_DELIVERY_TAG}]",
            ),
            (
                "unknown.nack",
                [],
                [_UNKNOWN_DELIVERY_TAG],
                f"nack_delivery_tags=[{_UNKNOWN_DELIVERY_TAG}]",
            ),
            (
                "both.fields",
                [_UNKNOWN_DELIVERY_TAG],
                [_UNKNOWN_DELIVERY_TAG],
                "the same delivery tag in both ack_delivery_tags and "
                "nack_delivery_tags",
            ),
        ],
        ids=["ack_field", "nack_field", "both_fields"],
    )
    def test_a_stale_delivery_tag_does_not_terminate_the_stream(
        self,
        consumer_stub,
        grpc_metadata,
        event_type_suffix,
        ack_tags,
        nack_tags,
        settled_description,
    ):
        """
        A delivery tag the session never issued must be skipped, in either
        settle field and in both at once, leaving the stream open.

        A client that reconnects after a disconnect can hold stale tags, and
        one of them must not cost it the stream. The three cases also probe
        the WIRE surface from three directions: a service that had not
        implemented ack_delivery_tags, or nack_delivery_tags, or the
        combination of both on one request, would have to either ignore the
        field (these pass) or reject the request (these fail).

        What these cases can NOT detect, and deliberately do not claim:

        - Whether a settle field is implemented or merely ignored. Both
          answers look identical from here, because an unknown tag is
          skipped either way. Only a settle that ACTS on a real tag could
          tell them apart, and this suite cannot obtain one (see the module
          docstring: no producer).
        - The ack-first precedence for a tag in both fields. Neither verb can
          act on a tag the session never issued, so the both-fields case
          exercises no precedence at all — it establishes only that the
          combination is accepted rather than rejected as contradictory.
          Precedence is covered against a real tracker in the service's own
          unit suite (``TestListenForDurableAcks``).
        """
        queue_id = self._create_queue(
            consumer_stub,
            grpc_metadata,
            f"conformance.consume.acks.{event_type_suffix}",
        )
        try:
            stream = consumer_stub.ConsumeDurableEventsWithAcks(
                _settle_request_generator(
                    queue_id, ack_tags=ack_tags, nack_tags=nack_tags
                ),
                metadata=grpc_metadata,
            )

            _, error = read_grpc_stream_with_timeout(
                stream, timeout=_STREAM_SURVIVAL_TIMEOUT_S
            )

            self._assert_stream_survived(error, queue_id, settled_description)
        finally:
            self._delete_queue(consumer_stub, grpc_metadata, queue_id)

    def test_nack_on_nonexistent_queue_returns_not_found(
        self, consumer_stub, grpc_metadata
    ):
        """
        Settling tags does not change how the RPC reports a missing queue:
        the queue is resolved from the opening request, so a nack-carrying
        session against a queue that does not exist must still return
        NOT_FOUND rather than being swallowed by the settle path.
        """
        stream = consumer_stub.ConsumeDurableEventsWithAcks(
            _settle_request_generator(
                _NON_EXISTENT_QUEUE_ID, nack_tags=[_UNKNOWN_DELIVERY_TAG]
            ),
            metadata=grpc_metadata,
        )

        messages, error = read_grpc_stream_with_timeout(stream, timeout=5.0)

        assert error is not None, (
            f"Expected ConsumeDurableEventsWithAcks for non-existent queue "
            f"'{_NON_EXISTENT_QUEUE_ID}' to raise a gRPC error with status "
            f"NOT_FOUND, but the stream completed without error "
            f"(messages={messages})."
        )
        assert isinstance(
            error, grpc.RpcError
        ), f"Expected a grpc.RpcError but got {type(error).__name__}: {error}"
        assert error.code() == grpc.StatusCode.NOT_FOUND, (
            f"Expected NOT_FOUND for non-existent queue "
            f"'{_NON_EXISTENT_QUEUE_ID}', but got {error.code().name}.\n"
            f"Error details: {error.details()}"
        )
