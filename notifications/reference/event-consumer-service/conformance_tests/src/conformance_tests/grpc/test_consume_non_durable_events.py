# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
gRPC conformance tests for ConsumeNonDurableEvents.

Contract under test:
  rpc ConsumeNonDurableEvents(stream ConsumeNonDurableEventsRequest)
      returns (stream ConsumeNonDurableEventsResponse)

This is a bidirectional streaming RPC. The client sends a stream of requests
(to set or update filter groups) and the server streams matching events back.
The first server response contains a reconnect_token and no events.

Expected behaviour:
  - Sending a valid initial request opens the stream; the first response
    includes a reconnect_token.
  - Sending a request with an empty event_type in any FilterGroup returns
    INVALID_ARGUMENT immediately.
  - The stream supports updating filters by sending subsequent requests.

Filter-update acknowledgements are OPTIONAL, and the responses say so per
connection. A server that sets "supports_filter_update_acks" sends one
response carrying a "filter_update_ack" for each filter update it processes;
a server that leaves the flag unset applies updates without reporting them,
which is still conformant. The acknowledgement case below is therefore
asserted only when the server under test advertises the capability, so this
suite passes against a reimplementation or an older deployment that does not
offer it.
"""

import grpc
import pytest
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2 as pb2

from conformance_tests.conftest import read_grpc_stream_with_timeout

pytestmark = pytest.mark.grpc


def _make_request_generator(filter_groups):
    """Yield a single ConsumeNonDurableEventsRequest with the given filter groups."""
    request = pb2.ConsumeNonDurableEventsRequest()
    for fg in filter_groups:
        request.filter_groups.append(fg)
    yield request


def _open_then_update_generator(initial_group, updated_group):
    """
    Open a stream with one FilterGroup, then replace it with another.

    The update names the set it replaces in previous_filter_groups, which is
    what the contract requires of every mid-stream filter change.
    """
    yield pb2.ConsumeNonDurableEventsRequest(filter_groups=[initial_group])
    yield pb2.ConsumeNonDurableEventsRequest(
        filter_groups=[updated_group],
        previous_filter_groups=[initial_group],
    )


class TestConsumeNonDurableEvents:
    def test_consume_with_valid_filter_opens_stream_and_returns_reconnect_token(
        self, consumer_stub, grpc_metadata
    ):
        """
        ConsumeNonDurableEvents with a valid initial FilterGroup must open a
        stream. The first response must contain a non-empty reconnect_token and
        no events (it is the "connected" acknowledgement).
        """
        filter_group = pb2.FilterGroup(event_type="conformance.test.event")
        stream = consumer_stub.ConsumeNonDurableEvents(
            _make_request_generator([filter_group]),
            metadata=grpc_metadata,
        )

        messages, error = read_grpc_stream_with_timeout(stream, timeout=5.0)

        # A compliant service must send the connected-ack first response
        # before the client times out.
        assert messages, (
            "Expected ConsumeNonDurableEvents to send a first response with a "
            "reconnect_token before the read timeout, but received none. "
            "The contract requires the server to acknowledge the stream "
            "immediately on open."
        )
        first = messages[0]
        assert first.reconnect_token, (
            f"Expected the first ConsumeNonDurableEvents response to contain "
            f"a non-empty reconnect_token, but got: '{first.reconnect_token}'."
        )

        # Any error that occurred must not be a hard failure (only CANCELLED from
        # timeout is acceptable here since no events are being published)
        if error is not None:
            assert isinstance(
                error, grpc.RpcError
            ), f"Expected grpc.RpcError but got {type(error).__name__}: {error}"
            assert error.code() in (
                grpc.StatusCode.CANCELLED,
                grpc.StatusCode.OK,
            ), (
                f"ConsumeNonDurableEvents returned unexpected error "
                f"{error.code().name}: {error.details()}"
            )

    def test_a_server_that_advertises_acks_acknowledges_the_opening_set(
        self, consumer_stub, grpc_metadata
    ):
        """
        A server that sets "supports_filter_update_acks" must acknowledge the
        filter set the opening request named, on the first response: that set
        is applied before the stream yields anything, so it never reaches the
        mid-stream acknowledgement path and this is a client's only
        confirmation of it.

        Skipped against a server that does not advertise the capability --
        acknowledgements are optional, and such a server is still conformant.
        """
        filter_group = pb2.FilterGroup(event_type="conformance.ack.opening")
        stream = consumer_stub.ConsumeNonDurableEvents(
            _make_request_generator([filter_group]),
            metadata=grpc_metadata,
        )

        messages, _ = read_grpc_stream_with_timeout(stream, timeout=5.0)

        assert messages, (
            "Expected ConsumeNonDurableEvents to send a first response before "
            "the read timeout, but received none."
        )
        if not messages[0].supports_filter_update_acks:
            pytest.skip(
                "Server does not set supports_filter_update_acks; filter-update "
                "acknowledgements are an optional capability and this case does "
                "not apply. Remove the skip only if the capability becomes "
                "mandatory in the contract."
            )

        assert list(messages[0].filter_update_ack.applied_filter_groups) == [
            filter_group
        ], (
            "A server that advertises filter-update acknowledgements must echo "
            "the opening request's filter_groups on its first response, but got: "
            f"{list(messages[0].filter_update_ack.applied_filter_groups)}"
        )

    def test_a_server_that_advertises_acks_acknowledges_a_mid_stream_update(
        self, consumer_stub, grpc_metadata
    ):
        """
        A server that sets "supports_filter_update_acks" must send exactly one
        response acknowledging each mid-stream filter update, echoing that
        update's own filter_groups.

        Skipped against a server that does not advertise the capability --
        acknowledgements are optional, and such a server is still conformant.
        """
        initial_group = pb2.FilterGroup(event_type="conformance.ack.initial")
        updated_group = pb2.FilterGroup(event_type="conformance.ack.updated")
        stream = consumer_stub.ConsumeNonDurableEvents(
            _open_then_update_generator(initial_group, updated_group),
            metadata=grpc_metadata,
        )

        messages, _ = read_grpc_stream_with_timeout(stream, timeout=5.0)

        assert messages, (
            "Expected ConsumeNonDurableEvents to send a first response before "
            "the read timeout, but received none."
        )
        if not messages[0].supports_filter_update_acks:
            pytest.skip(
                "Server does not set supports_filter_update_acks; filter-update "
                "acknowledgements are an optional capability and this case does "
                "not apply. Remove the skip only if the capability becomes "
                "mandatory in the contract."
            )

        # Selected by the echoed set rather than by position: a server may
        # interleave event responses with acknowledgements in any order, and
        # the opening response carries an acknowledgement of its own.
        acknowledgements = [
            message
            for message in messages
            if message.HasField("filter_update_ack")
            and list(message.filter_update_ack.applied_filter_groups) == [updated_group]
        ]
        assert len(acknowledgements) == 1, (
            "Expected exactly one response acknowledging the mid-stream filter "
            f"update and echoing its filter_groups, but got "
            f"{len(acknowledgements)}."
        )

    def test_consume_without_filter_groups_returns_invalid_argument(
        self, consumer_stub, grpc_metadata
    ):
        """
        ConsumeNonDurableEvents with no filter_groups must return INVALID_ARGUMENT.
        At least one filter group with an event_type is required.
        """
        stream = consumer_stub.ConsumeNonDurableEvents(
            _make_request_generator([]),
            metadata=grpc_metadata,
        )

        messages, error = read_grpc_stream_with_timeout(stream, timeout=5.0)

        assert error is not None, (
            "Expected ConsumeNonDurableEvents with empty filter_groups to return "
            "INVALID_ARGUMENT, but the stream opened without error."
        )
        assert isinstance(error, grpc.RpcError)
        assert error.code() == grpc.StatusCode.INVALID_ARGUMENT, (
            f"Expected INVALID_ARGUMENT for empty filter_groups, "
            f"but got {error.code().name}: {error.details()}"
        )

    def test_consume_with_empty_event_type_returns_invalid_argument(
        self, consumer_stub, grpc_metadata
    ):
        """
        A FilterGroup with an empty event_type is invalid.
        The service must return INVALID_ARGUMENT immediately.
        """
        invalid_fg = pb2.FilterGroup(event_type="")  # empty event_type is invalid
        stream = consumer_stub.ConsumeNonDurableEvents(
            _make_request_generator([invalid_fg]),
            metadata=grpc_metadata,
        )

        messages, error = read_grpc_stream_with_timeout(stream, timeout=5.0)

        assert error is not None, (
            "Expected ConsumeNonDurableEvents with empty event_type to return "
            "INVALID_ARGUMENT, but the stream opened without error."
        )
        assert isinstance(
            error, grpc.RpcError
        ), f"Expected grpc.RpcError but got {type(error).__name__}: {error}"
        assert error.code() == grpc.StatusCode.INVALID_ARGUMENT, (
            f"Expected INVALID_ARGUMENT for empty event_type FilterGroup, "
            f"but got {error.code().name}. Error: {error.details()}"
        )

    def test_consume_with_reconnect_token_opens_stream(
        self, consumer_stub, grpc_metadata
    ):
        """
        ConsumeNonDurableEvents with a reconnect_token must be accepted.
        A reconnect allows a client to resume consumption from the same
        non-durable queue after a disconnect.
        """
        # First, open a stream to get a reconnect token
        filter_group = pb2.FilterGroup(event_type="conformance.reconnect.test")
        initial_stream = consumer_stub.ConsumeNonDurableEvents(
            _make_request_generator([filter_group]),
            metadata=grpc_metadata,
        )
        messages, _ = read_grpc_stream_with_timeout(initial_stream, timeout=5.0)

        assert messages, (
            "Expected the initial ConsumeNonDurableEvents stream to send a "
            "first response with a reconnect_token, but received none. "
            "A compliant service must acknowledge the stream on open so that "
            "clients can later reconnect."
        )
        reconnect_token = messages[0].reconnect_token
        assert reconnect_token, (
            f"Expected the initial response to contain a non-empty "
            f"reconnect_token, but got: '{reconnect_token}'."
        )

        # Now reconnect using the token
        def reconnect_request_gen():
            req = pb2.ConsumeNonDurableEventsRequest(reconnect_token=reconnect_token)
            yield req

        reconnect_stream = consumer_stub.ConsumeNonDurableEvents(
            reconnect_request_gen(),
            metadata=grpc_metadata,
        )
        reconnect_messages, reconnect_error = read_grpc_stream_with_timeout(
            reconnect_stream, timeout=5.0
        )

        # Reconnecting to an existing (or recently expired) queue should not
        # return a hard error immediately.
        if reconnect_error is not None:
            assert isinstance(reconnect_error, grpc.RpcError)
            # NOT_FOUND is acceptable if the queue TTL has expired
            assert reconnect_error.code() in (
                grpc.StatusCode.CANCELLED,
                grpc.StatusCode.OK,
                grpc.StatusCode.NOT_FOUND,
            ), (
                f"Expected reconnect to succeed or return NOT_FOUND (TTL expired), "
                f"but got {reconnect_error.code().name}: {reconnect_error.details()}"
            )
