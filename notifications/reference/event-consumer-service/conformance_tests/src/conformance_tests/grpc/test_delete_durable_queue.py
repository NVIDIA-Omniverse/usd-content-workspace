# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
gRPC conformance tests for DeleteDurableQueue.

Contract under test:
  rpc DeleteDurableQueue(DeleteDurableQueueRequest) returns (DeleteDurableQueueResponse)

Expected behaviour:
  - Deleting an existing queue succeeds (response.success == True).
  - Deleting a non-existent queue returns NOT_FOUND.
  - Providing an empty queue_id returns an error (INVALID_ARGUMENT or similar).
"""

import grpc
import pytest
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2 as pb2

pytestmark = pytest.mark.grpc

_NON_EXISTENT_QUEUE_ID = "00000000-0000-0000-0000-000000000000"


class TestDeleteDurableQueue:
    def test_delete_existing_queue_succeeds(self, consumer_stub, grpc_metadata):
        """
        Deleting a queue that was just created must succeed.
        The response must indicate success.
        """
        # First create a queue to delete
        create_resp = consumer_stub.CreateDurableQueue(
            pb2.CreateDurableQueueRequest(
                filter_groups=[pb2.FilterGroup(event_type="conformance.delete.test")]
            ),
            metadata=grpc_metadata,
        )
        queue_id = create_resp.queue_id
        assert (
            queue_id
        ), "Prerequisite failed: CreateDurableQueue returned an empty queue_id."

        # Now delete it — DeleteDurableQueueResponse is an empty message;
        # the call completing without raising a gRPC error signals success.
        consumer_stub.DeleteDurableQueue(
            pb2.DeleteDurableQueueRequest(queue_id=queue_id),
            metadata=grpc_metadata,
        )

    def test_delete_nonexistent_queue_returns_not_found(
        self, consumer_stub, grpc_metadata
    ):
        """
        Deleting a queue that does not exist must return gRPC status NOT_FOUND.
        """
        try:
            consumer_stub.DeleteDurableQueue(
                pb2.DeleteDurableQueueRequest(queue_id=_NON_EXISTENT_QUEUE_ID),
                metadata=grpc_metadata,
            )
            pytest.fail(
                f"Expected DeleteDurableQueue for non-existent queue "
                f"'{_NON_EXISTENT_QUEUE_ID}' to raise a gRPC error with "
                f"status NOT_FOUND, but the call succeeded."
            )
        except grpc.RpcError as exc:
            assert exc.code() == grpc.StatusCode.NOT_FOUND, (
                f"Expected gRPC status NOT_FOUND when deleting non-existent queue "
                f"'{_NON_EXISTENT_QUEUE_ID}', but got {exc.code().name}.\n"
                f"Error details: {exc.details()}"
            )

    def test_delete_with_empty_queue_id_returns_error(
        self, consumer_stub, grpc_metadata
    ):
        """
        Providing an empty queue_id must return an error status (INVALID_ARGUMENT or
        NOT_FOUND are both acceptable — the key constraint is that it must NOT succeed).
        """
        try:
            consumer_stub.DeleteDurableQueue(
                pb2.DeleteDurableQueueRequest(queue_id=""),
                metadata=grpc_metadata,
            )
            pytest.fail(
                "Expected DeleteDurableQueue with an empty queue_id to return a "
                "gRPC error, but the call succeeded. An empty queue_id is not valid."
            )
        except grpc.RpcError as exc:
            assert exc.code() in (
                grpc.StatusCode.INVALID_ARGUMENT,
                grpc.StatusCode.NOT_FOUND,
            ), (
                f"Expected an error status for empty queue_id "
                f"(INVALID_ARGUMENT or NOT_FOUND), but got {exc.code().name}.\n"
                f"Error details: {exc.details()}"
            )

    def test_delete_same_queue_twice_returns_not_found_on_second_call(
        self, consumer_stub, grpc_metadata
    ):
        """
        Deleting the same queue twice must return NOT_FOUND on the second call.
        Queues are one-shot resources; once deleted they no longer exist.
        """
        create_resp = consumer_stub.CreateDurableQueue(
            pb2.CreateDurableQueueRequest(
                filter_groups=[pb2.FilterGroup(event_type="conformance.double.delete")]
            ),
            metadata=grpc_metadata,
        )
        queue_id = create_resp.queue_id

        # First delete — should succeed (empty response means success)
        consumer_stub.DeleteDurableQueue(
            pb2.DeleteDurableQueueRequest(queue_id=queue_id),
            metadata=grpc_metadata,
        )

        # Second delete — must fail with NOT_FOUND
        try:
            consumer_stub.DeleteDurableQueue(
                pb2.DeleteDurableQueueRequest(queue_id=queue_id),
                metadata=grpc_metadata,
            )
            pytest.fail(
                f"Expected second DeleteDurableQueue for queue '{queue_id}' "
                f"to return NOT_FOUND, but it succeeded."
            )
        except grpc.RpcError as exc:
            assert exc.code() == grpc.StatusCode.NOT_FOUND, (
                f"Expected NOT_FOUND on second delete of queue '{queue_id}', "
                f"but got {exc.code().name}. Error: {exc.details()}"
            )
