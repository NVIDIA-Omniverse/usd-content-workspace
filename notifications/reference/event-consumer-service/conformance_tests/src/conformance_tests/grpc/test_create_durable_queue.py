# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
gRPC conformance tests for CreateDurableQueue.

Contract under test:
  rpc CreateDurableQueue(CreateDurableQueueRequest) returns (CreateDurableQueueResponse)

Expected behaviour:
  - Returns a non-empty, unique queue_id on success.
  - Accepts any combination of filter_groups (including none).
  - Successive calls return different queue_ids.
"""

import grpc
import pytest
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2 as pb2

pytestmark = pytest.mark.grpc


def _delete_queue(consumer_stub, grpc_metadata, queue_id: str) -> None:
    """Best-effort cleanup to avoid leaking queues in RabbitMQ across runs."""
    if not queue_id:
        return
    try:
        consumer_stub.DeleteDurableQueue(
            pb2.DeleteDurableQueueRequest(queue_id=queue_id),
            metadata=grpc_metadata,
        )
    except grpc.RpcError:
        pass


class TestCreateDurableQueue:
    def test_create_with_event_type_filter_returns_queue_id(
        self, consumer_stub, grpc_metadata
    ):
        """
        CreateDurableQueue with a valid FilterGroup must return a non-empty queue_id.
        """
        request = pb2.CreateDurableQueueRequest(
            filter_groups=[
                pb2.FilterGroup(
                    event_type="conformance.test.created",
                    filters=[
                        pb2.ResourceFilter(
                            filter_type=pb2.ResourceFilter.FilterType.FILTER_TYPE_EQ,
                            resource_id="test-resource/",
                        )
                    ],
                )
            ]
        )
        response = consumer_stub.CreateDurableQueue(request, metadata=grpc_metadata)

        try:
            assert response.queue_id, (
                "Expected CreateDurableQueue to return a non-empty queue_id, "
                "but got an empty string. "
                "The service must assign a unique identifier to each created queue."
            )
        finally:
            _delete_queue(consumer_stub, grpc_metadata, response.queue_id)

    def test_create_with_empty_filter_groups_returns_invalid_argument(
        self, consumer_stub, grpc_metadata
    ):
        """
        CreateDurableQueue with no filter_groups must return INVALID_ARGUMENT.
        At least one filter group with an event_type is required.
        """
        request = pb2.CreateDurableQueueRequest(filter_groups=[])
        try:
            consumer_stub.CreateDurableQueue(request, metadata=grpc_metadata)
            pytest.fail(
                "Expected CreateDurableQueue with empty filter_groups to return "
                "INVALID_ARGUMENT, but it succeeded."
            )
        except grpc.RpcError as exc:
            assert exc.code() == grpc.StatusCode.INVALID_ARGUMENT, (
                f"Expected INVALID_ARGUMENT for empty filter_groups, "
                f"but got {exc.code().name}. Error: {exc.details()}"
            )

    def test_create_with_multiple_filter_groups_returns_queue_id(
        self, consumer_stub, grpc_metadata
    ):
        """
        CreateDurableQueue with multiple FilterGroups must return a queue_id.
        Multiple groups express an OR relationship between event types.
        """
        request = pb2.CreateDurableQueueRequest(
            filter_groups=[
                pb2.FilterGroup(event_type="conformance.event.alpha"),
                pb2.FilterGroup(event_type="conformance.event.beta"),
            ]
        )
        response = consumer_stub.CreateDurableQueue(request, metadata=grpc_metadata)

        try:
            assert response.queue_id, (
                "Expected CreateDurableQueue with multiple filter_groups to return "
                "a non-empty queue_id."
            )
        finally:
            _delete_queue(consumer_stub, grpc_metadata, response.queue_id)

    def test_create_returns_unique_queue_ids_for_each_call(
        self, consumer_stub, grpc_metadata
    ):
        """
        Each call to CreateDurableQueue must return a distinct queue_id.
        """
        request = pb2.CreateDurableQueueRequest(
            filter_groups=[pb2.FilterGroup(event_type="conformance.test.unique")]
        )
        resp1 = consumer_stub.CreateDurableQueue(request, metadata=grpc_metadata)
        resp2 = consumer_stub.CreateDurableQueue(request, metadata=grpc_metadata)

        try:
            assert resp1.queue_id != resp2.queue_id, (
                f"Expected two separate CreateDurableQueue calls to return different "
                f"queue_ids, but both returned '{resp1.queue_id}'. "
                f"Each queue must have a unique identifier."
            )
        finally:
            _delete_queue(consumer_stub, grpc_metadata, resp1.queue_id)
            _delete_queue(consumer_stub, grpc_metadata, resp2.queue_id)

    def test_create_with_starts_with_greedy_filter_type(
        self, consumer_stub, grpc_metadata
    ):
        """
        CreateDurableQueue with STARTS_WITH_GREEDY filter type must succeed.
        """
        request = pb2.CreateDurableQueueRequest(
            filter_groups=[
                pb2.FilterGroup(
                    event_type="conformance.test.greedy",
                    filters=[
                        pb2.ResourceFilter(
                            filter_type=pb2.ResourceFilter.FilterType.FILTER_TYPE_STARTS_WITH_GREEDY,
                            resource_id="prefix/path/",
                        )
                    ],
                )
            ]
        )
        response = consumer_stub.CreateDurableQueue(request, metadata=grpc_metadata)

        try:
            assert response.queue_id, (
                "Expected CreateDurableQueue with STARTS_WITH_GREEDY filter to return "
                "a non-empty queue_id."
            )
        finally:
            _delete_queue(consumer_stub, grpc_metadata, response.queue_id)

    def test_create_with_starts_with_lazy_filter_type(
        self, consumer_stub, grpc_metadata
    ):
        """
        CreateDurableQueue with STARTS_WITH_LAZY filter type must succeed.
        """
        request = pb2.CreateDurableQueueRequest(
            filter_groups=[
                pb2.FilterGroup(
                    event_type="conformance.test.lazy",
                    filters=[
                        pb2.ResourceFilter(
                            filter_type=pb2.ResourceFilter.FilterType.FILTER_TYPE_STARTS_WITH_LAZY,
                            resource_id="prefix/path/",
                        )
                    ],
                )
            ]
        )
        response = consumer_stub.CreateDurableQueue(request, metadata=grpc_metadata)

        try:
            assert response.queue_id, (
                "Expected CreateDurableQueue with STARTS_WITH_LAZY filter to return "
                "a non-empty queue_id."
            )
        finally:
            _delete_queue(consumer_stub, grpc_metadata, response.queue_id)
