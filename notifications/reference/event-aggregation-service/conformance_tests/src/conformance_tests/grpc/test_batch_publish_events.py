# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
gRPC conformance tests for BatchPublishEvents.

Contract under test:
  rpc BatchPublishEvents(BatchPublishEventsRequest) returns (BatchPublishEventsResponse)

Expected behaviour:
  - Batch of one or more valid events returns a response with one
    PublishingResult per input event, all with success == True.
  - The results array length must equal the input events count.
  - An empty events list must return gRPC status INVALID_ARGUMENT.
  - More than 1000 events, or any event over a per-event limit, must return
    INVALID_ARGUMENT for the whole batch.
"""

import grpc
import pytest
from google.protobuf.struct_pb2 import Struct
from google.protobuf.timestamp_pb2 import Timestamp

from conformance_tests.conftest import (
    MAX_BATCH_EVENTS,
    MAX_EVENT_MESSAGE_BYTES,
    expect_invalid_argument,
    make_event_proto,
    message_of_size,
)

pytestmark = pytest.mark.grpc


class TestBatchPublishEvents:
    def test_batch_publish_single_event_returns_one_result(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        BatchPublishEvents with a single event must return a response whose
        results list contains exactly one entry with success == True.
        """
        event = make_event_proto(pb2)
        response = publisher_stub.BatchPublishEvents(
            pb2.BatchPublishEventsRequest(events=[event]),
            metadata=grpc_metadata,
        )
        assert len(response.results) == 1, (
            f"Expected exactly 1 result for a batch of 1 event, "
            f"but got {len(response.results)}."
        )
        assert response.results[0].success is True, (
            f"Expected result[0].success=True, "
            f"but got success={response.results[0].success}. "
            f"Failure reason: {response.results[0].failure_reason!r}"
        )

    def test_batch_publish_multiple_events_returns_matching_count(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        BatchPublishEvents with N events must return exactly N results,
        all with success == True.
        """
        n = 5
        events = []
        for i in range(n):
            msg = Struct()
            msg.update({"index": i})
            events.append(
                pb2.Event(
                    event_type="conformance.batch.test",
                    occurred_at=Timestamp(seconds=1_700_000_000 + i),
                    message=msg,
                )
            )

        response = publisher_stub.BatchPublishEvents(
            pb2.BatchPublishEventsRequest(events=events),
            metadata=grpc_metadata,
        )
        assert len(response.results) == n, (
            f"Expected {n} results for a batch of {n} events, "
            f"but got {len(response.results)}."
        )
        for i, result in enumerate(response.results):
            assert result.success is True, (
                f"Expected result[{i}].success=True, "
                f"but got success={result.success}. "
                f"Failure reason: {result.failure_reason!r}"
            )

    def test_batch_publish_empty_events_returns_invalid_argument(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        BatchPublishEvents with an empty events list must return gRPC status
        INVALID_ARGUMENT.  The service requires at least one event per batch.
        """
        try:
            publisher_stub.BatchPublishEvents(
                pb2.BatchPublishEventsRequest(events=[]),
                metadata=grpc_metadata,
            )
            pytest.fail(
                "Expected BatchPublishEvents with empty events list to raise "
                "a gRPC error (INVALID_ARGUMENT), but the call succeeded."
            )
        except grpc.RpcError as exc:
            assert exc.code() == grpc.StatusCode.INVALID_ARGUMENT, (
                f"Expected INVALID_ARGUMENT for empty events list, "
                f"but got {exc.code().name}. Details: {exc.details()}"
            )

    def test_batch_publish_results_echo_event_types(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        Each PublishingResult in the batch response must echo back the
        event_type of the corresponding input event.
        """
        event_types = ["conformance.type.a", "conformance.type.b"]
        events = []
        for et in event_types:
            msg = Struct()
            msg.update({"type": et})
            events.append(
                pb2.Event(
                    event_type=et,
                    occurred_at=Timestamp(seconds=1_700_000_001),
                    message=msg,
                )
            )

        response = publisher_stub.BatchPublishEvents(
            pb2.BatchPublishEventsRequest(events=events),
            metadata=grpc_metadata,
        )
        for i, (result, expected_type) in enumerate(zip(response.results, event_types)):
            assert result.event.event_type == expected_type, (
                f"Expected result[{i}].event.event_type == '{expected_type}', "
                f"but got '{result.event.event_type}'."
            )

    def test_batch_events_exceeding_max_returns_invalid_argument(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        BatchPublishEvents with more than 1000 events must return gRPC status
        INVALID_ARGUMENT naming the limit.
        """
        events = [make_event_proto(pb2) for _ in range(MAX_BATCH_EVENTS + 1)]
        expect_invalid_argument(
            lambda: publisher_stub.BatchPublishEvents(
                pb2.BatchPublishEventsRequest(events=events), metadata=grpc_metadata
            ),
            what="more than 1000 events",
            field=str(MAX_BATCH_EVENTS),
        )

    def test_batch_events_at_max_returns_matching_count(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        BatchPublishEvents with exactly 1000 events must succeed with 1000
        results: the limit is inclusive.
        """
        events = [make_event_proto(pb2) for _ in range(MAX_BATCH_EVENTS)]
        response = publisher_stub.BatchPublishEvents(
            pb2.BatchPublishEventsRequest(events=events), metadata=grpc_metadata
        )
        assert len(response.results) == MAX_BATCH_EVENTS, (
            f"Expected {MAX_BATCH_EVENTS} results for a batch of exactly "
            f"{MAX_BATCH_EVENTS} events, but got {len(response.results)}."
        )

    def test_batch_with_oversize_message_returns_invalid_argument(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        BatchPublishEvents where one event's message encodes to more than 65536
        JSON bytes must return gRPC status INVALID_ARGUMENT for the whole batch,
        naming the message.
        """
        oversize = make_event_proto(pb2)
        oversize.message.Clear()
        oversize.message.update(message_of_size(MAX_EVENT_MESSAGE_BYTES + 1))
        events = [make_event_proto(pb2), oversize, make_event_proto(pb2)]
        expect_invalid_argument(
            lambda: publisher_stub.BatchPublishEvents(
                pb2.BatchPublishEventsRequest(events=events), metadata=grpc_metadata
            ),
            what="an oversize message in the batch",
            field="message",
        )
