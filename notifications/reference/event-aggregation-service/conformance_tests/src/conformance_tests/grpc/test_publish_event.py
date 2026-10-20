# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
gRPC conformance tests for PublishEvent.

Contract under test:
  rpc PublishEvent(PublishEventRequest) returns (PublishEventResponse)

Expected behaviour:
  - Publishing a valid event returns a response with result.success == True.
  - Publishing an event without a resource also succeeds.
  - Publishing an event with occurred_at not set (seconds == 0) must return
    gRPC status INVALID_ARGUMENT with a message about occurred_at.
  - An event whose message encodes to more than 65536 JSON bytes, whose
    event_type exceeds 256 characters, or whose resource_id exceeds 2048
    characters must return INVALID_ARGUMENT.
"""

import grpc
import pytest
from google.protobuf.struct_pb2 import Struct
from google.protobuf.timestamp_pb2 import Timestamp

from conformance_tests.conftest import (
    MAX_EVENT_MESSAGE_BYTES,
    MAX_EVENT_TYPE_LENGTH,
    MAX_RESOURCE_ID_LENGTH,
    expect_invalid_argument,
    make_event_proto,
    message_of_size,
)

pytestmark = pytest.mark.grpc


class TestPublishEvent:
    def test_publish_valid_event_returns_success(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        PublishEvent with a fully-populated event (event_type, message,
        occurred_at, resource) must return a response with result.success == True.
        """
        event = make_event_proto(pb2, with_resource=True)
        response = publisher_stub.PublishEvent(
            pb2.PublishEventRequest(event=event),
            metadata=grpc_metadata,
        )
        assert response.result.success is True, (
            f"Expected PublishEvent to return result.success=True, "
            f"but got success={response.result.success}. "
            f"Failure reason: {response.result.failure_reason!r}"
        )

    def test_publish_event_without_resource_returns_success(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        PublishEvent without a resource field must also succeed.
        resource is optional — omitting it is equivalent to resource_id=''.
        """
        event = make_event_proto(pb2, with_resource=False)
        response = publisher_stub.PublishEvent(
            pb2.PublishEventRequest(event=event),
            metadata=grpc_metadata,
        )
        assert response.result.success is True, (
            f"Expected PublishEvent without resource to return result.success=True, "
            f"but got success={response.result.success}. "
            f"Failure reason: {response.result.failure_reason!r}"
        )

    def test_publish_event_returns_echoed_event_type(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        The PublishingResult must echo back the submitted event.
        The result.event.event_type must match what was sent.
        """
        event_type = "conformance.echo.test"
        msg = Struct()
        msg.update({"check": "echo"})
        event = pb2.Event(
            event_type=event_type,
            occurred_at=Timestamp(seconds=1_700_000_000),
            message=msg,
        )
        response = publisher_stub.PublishEvent(
            pb2.PublishEventRequest(event=event),
            metadata=grpc_metadata,
        )
        assert response.result.event.event_type == event_type, (
            f"Expected result.event.event_type == '{event_type}', "
            f"but got '{response.result.event.event_type}'."
        )

    def test_publish_event_with_zero_occurred_at_returns_invalid_argument(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        An event whose occurred_at is not set (Timestamp seconds == 0) must
        return gRPC status INVALID_ARGUMENT.  The service requires occurred_at
        to be explicitly specified.
        """
        msg = Struct()
        msg.update({"key": "value"})
        event = pb2.Event(
            event_type="conformance.missing.ts",
            message=msg,
            # occurred_at intentionally omitted — defaults to Timestamp(seconds=0)
        )
        try:
            publisher_stub.PublishEvent(
                pb2.PublishEventRequest(event=event),
                metadata=grpc_metadata,
            )
            pytest.fail(
                "Expected PublishEvent with occurred_at not set to raise a "
                "gRPC error (INVALID_ARGUMENT), but the call succeeded."
            )
        except grpc.RpcError as exc:
            assert exc.code() == grpc.StatusCode.INVALID_ARGUMENT, (
                f"Expected INVALID_ARGUMENT for missing occurred_at, "
                f"but got {exc.code().name}. Details: {exc.details()}"
            )
            assert "occurred_at" in (exc.details() or "").lower(), (
                f"Expected error details to mention 'occurred_at', "
                f"but got: '{exc.details()}'"
            )

    def test_publish_message_exceeding_max_size_returns_invalid_argument(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        PublishEvent with a message over 65536 JSON bytes must return gRPC
        status INVALID_ARGUMENT naming the message.
        """
        event = make_event_proto(pb2)
        event.message.Clear()
        event.message.update(message_of_size(MAX_EVENT_MESSAGE_BYTES + 1))
        expect_invalid_argument(
            lambda: publisher_stub.PublishEvent(
                pb2.PublishEventRequest(event=event), metadata=grpc_metadata
            ),
            what="a message over 65536 JSON bytes",
            field="message",
        )

    def test_publish_message_at_max_size_returns_success(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        PublishEvent with a message of exactly 65536 JSON bytes must succeed:
        the limit is inclusive.
        """
        event = make_event_proto(pb2)
        event.message.Clear()
        event.message.update(message_of_size(MAX_EVENT_MESSAGE_BYTES))
        response = publisher_stub.PublishEvent(
            pb2.PublishEventRequest(event=event), metadata=grpc_metadata
        )
        assert response.result.success is True, (
            f"Expected success for a message of exactly 65536 JSON bytes, "
            f"but got failure_reason={response.result.failure_reason!r}"
        )

    def test_publish_event_type_exceeding_max_length_returns_invalid_argument(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        PublishEvent with an event_type over 256 characters must return gRPC
        status INVALID_ARGUMENT naming event_type.
        """
        event = make_event_proto(pb2)
        event.event_type = "a" * (MAX_EVENT_TYPE_LENGTH + 1)
        expect_invalid_argument(
            lambda: publisher_stub.PublishEvent(
                pb2.PublishEventRequest(event=event), metadata=grpc_metadata
            ),
            what="an event_type over 256 characters",
            field="event_type",
        )

    def test_publish_event_type_at_max_length_returns_success(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        PublishEvent with an event_type of exactly 256 characters must succeed.
        """
        event = make_event_proto(pb2)
        event.event_type = "a" * MAX_EVENT_TYPE_LENGTH
        response = publisher_stub.PublishEvent(
            pb2.PublishEventRequest(event=event), metadata=grpc_metadata
        )
        assert response.result.success is True, (
            f"Expected success for an event_type of exactly 256 characters, "
            f"but got failure_reason={response.result.failure_reason!r}"
        )

    def test_publish_resource_id_exceeding_max_length_returns_invalid_argument(
        self, publisher_stub, grpc_metadata, pb2
    ):
        """
        PublishEvent with a resource_id over 2048 characters must return gRPC
        status INVALID_ARGUMENT naming resource_id.
        """
        event = make_event_proto(pb2)
        event.resource.resource_id = "a" * (MAX_RESOURCE_ID_LENGTH + 1)
        expect_invalid_argument(
            lambda: publisher_stub.PublishEvent(
                pb2.PublishEventRequest(event=event), metadata=grpc_metadata
            ),
            what="a resource_id over 2048 characters",
            field="resource_id",
        )
