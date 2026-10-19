# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
gRPC conformance tests for UpdateDurableQueue.

Contract under test:
  rpc UpdateDurableQueue(UpdateDurableQueueRequest) returns (UpdateDurableQueueResponse)

Expected behaviour:
  - Updating an existing queue with valid filter groups succeeds (response.success == True).
  - Updating a non-existent queue returns NOT_FOUND.
  - Providing an empty queue_id returns INVALID_ARGUMENT or NOT_FOUND.
  - A ResourceFilter with FILTER_TYPE_EQ and an empty resource_id returns INVALID_ARGUMENT.
"""

import grpc
import pytest
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2 as pb2

pytestmark = pytest.mark.grpc

_NON_EXISTENT_QUEUE_ID = "00000000-0000-0000-0000-000000000000"


class TestUpdateDurableQueue:
    def _make_filter_group(self, event_type: str = "conformance.event.test"):
        return pb2.FilterGroup(
            event_type=event_type,
            filters=[
                pb2.ResourceFilter(
                    filter_type=pb2.ResourceFilter.FilterType.FILTER_TYPE_EQ,
                    resource_id="test-resource/",
                )
            ],
        )

    def test_update_existing_queue_succeeds(self, consumer_stub, grpc_metadata):
        """
        UpdateDurableQueue on an existing queue must return success=True.
        """
        # Create a queue first
        create_resp = consumer_stub.CreateDurableQueue(
            pb2.CreateDurableQueueRequest(
                filter_groups=[self._make_filter_group("conformance.original")]
            ),
            metadata=grpc_metadata,
        )
        queue_id = create_resp.queue_id
        assert queue_id, "Prerequisite: CreateDurableQueue returned empty queue_id."

        try:
            request = pb2.UpdateDurableQueueRequest(queue_id=queue_id)
            request.current_filter_groups.append(
                self._make_filter_group("conformance.original")
            )
            request.new_filter_groups.append(
                self._make_filter_group("conformance.updated")
            )
            # UpdateDurableQueueResponse is an empty message;
            # completing without raising a gRPC error signals success.
            consumer_stub.UpdateDurableQueue(request, metadata=grpc_metadata)
        finally:
            # Clean up
            try:
                consumer_stub.DeleteDurableQueue(
                    pb2.DeleteDurableQueueRequest(queue_id=queue_id),
                    metadata=grpc_metadata,
                )
            except grpc.RpcError:
                pass

    def test_update_nonexistent_queue_returns_not_found(
        self, consumer_stub, grpc_metadata
    ):
        """
        UpdateDurableQueue on a queue that does not exist must return NOT_FOUND.
        """
        request = pb2.UpdateDurableQueueRequest(queue_id=_NON_EXISTENT_QUEUE_ID)
        request.current_filter_groups.append(self._make_filter_group("conformance.old"))
        request.new_filter_groups.append(self._make_filter_group("conformance.new"))

        try:
            consumer_stub.UpdateDurableQueue(request, metadata=grpc_metadata)
            pytest.fail(
                f"Expected UpdateDurableQueue for non-existent queue "
                f"'{_NON_EXISTENT_QUEUE_ID}' to raise NOT_FOUND, but it succeeded."
            )
        except grpc.RpcError as exc:
            assert exc.code() == grpc.StatusCode.NOT_FOUND, (
                f"Expected NOT_FOUND when updating non-existent queue "
                f"'{_NON_EXISTENT_QUEUE_ID}', but got {exc.code().name}.\n"
                f"Error details: {exc.details()}"
            )

    def test_update_with_empty_queue_id_returns_error(
        self, consumer_stub, grpc_metadata
    ):
        """
        Providing an empty queue_id must return an error (INVALID_ARGUMENT or NOT_FOUND).
        """
        request = pb2.UpdateDurableQueueRequest(queue_id="")
        request.current_filter_groups.append(self._make_filter_group())
        request.new_filter_groups.append(self._make_filter_group())

        try:
            consumer_stub.UpdateDurableQueue(request, metadata=grpc_metadata)
            pytest.fail(
                "Expected UpdateDurableQueue with empty queue_id to return a "
                "gRPC error, but the call succeeded."
            )
        except grpc.RpcError as exc:
            assert exc.code() in (
                grpc.StatusCode.INVALID_ARGUMENT,
                grpc.StatusCode.NOT_FOUND,
            ), (
                f"Expected INVALID_ARGUMENT or NOT_FOUND for empty queue_id, "
                f"but got {exc.code().name}. Error: {exc.details()}"
            )

    def test_update_with_eq_filter_and_empty_resource_id_returns_invalid_argument(
        self, consumer_stub, grpc_metadata
    ):
        """
        A ResourceFilter with FILTER_TYPE_EQ and an empty resource_id is invalid.
        The service must return INVALID_ARGUMENT with a message explaining the problem.
        """
        # Create a queue to update
        create_resp = consumer_stub.CreateDurableQueue(
            pb2.CreateDurableQueueRequest(
                filter_groups=[self._make_filter_group("conformance.event.eq")]
            ),
            metadata=grpc_metadata,
        )
        queue_id = create_resp.queue_id
        assert queue_id, "Prerequisite: CreateDurableQueue returned empty queue_id."

        try:
            invalid_request = pb2.UpdateDurableQueueRequest(queue_id=queue_id)
            invalid_request.current_filter_groups.append(
                self._make_filter_group("conformance.event.eq")
            )
            # Invalid: EQ filter with empty resource_id
            invalid_fg = pb2.FilterGroup(
                event_type="conformance.event.new",
                filters=[
                    pb2.ResourceFilter(
                        filter_type=pb2.ResourceFilter.FilterType.FILTER_TYPE_EQ,
                        resource_id="",  # empty is invalid for EQ
                    )
                ],
            )
            invalid_request.new_filter_groups.append(invalid_fg)

            consumer_stub.UpdateDurableQueue(invalid_request, metadata=grpc_metadata)
            pytest.fail(
                "Expected UpdateDurableQueue with FILTER_TYPE_EQ and empty "
                "resource_id to return INVALID_ARGUMENT, but it succeeded."
            )
        except grpc.RpcError as exc:
            assert exc.code() == grpc.StatusCode.INVALID_ARGUMENT, (
                f"Expected INVALID_ARGUMENT for EQ filter with empty resource_id, "
                f"but got {exc.code().name}. Error: {exc.details()}"
            )
            expected_fragment = "Resource id is required when filter type is EQ"
            details = exc.details() or ""
            assert expected_fragment in details, (
                f"Expected error details to contain '{expected_fragment}', "
                f"but got: '{details}'"
            )
        finally:
            try:
                consumer_stub.DeleteDurableQueue(
                    pb2.DeleteDurableQueueRequest(queue_id=queue_id),
                    metadata=grpc_metadata,
                )
            except grpc.RpcError:
                pass
