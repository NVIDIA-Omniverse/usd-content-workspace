# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Common test functions for Event Aggregation Service gRPC smoke tests."""

import logging
import random
import string
from datetime import datetime, timezone

from google.protobuf import struct_pb2, timestamp_pb2
from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2
from utils import timeout_grpc_call

logger = logging.getLogger(__name__)


def run_publish_event_grpc(
    resource_base_url, notification_aggregation_grpc_stub, grpc_metadata
):
    """Publish a storage created event via gRPC."""
    occurred_at = timestamp_pb2.Timestamp()
    occurred_at.GetCurrentTime()

    file_name = "".join(random.choices(string.ascii_letters + string.digits, k=5))
    resource_address = f"{resource_base_url}{file_name}"

    message = struct_pb2.Struct()
    message["action"] = "created"
    message["resource_address"] = resource_address
    message["timestamp"] = datetime.now(timezone.utc).isoformat()

    event = event_publisher_pb2.Event(
        event_type="omni.storage.created",
        message=message,
        occurred_at=occurred_at,
        resource=event_publisher_pb2.EventResource(resource_id=resource_address),
    )

    publish_request = event_publisher_pb2.PublishEventRequest(event=event)

    publish_response = timeout_grpc_call(
        notification_aggregation_grpc_stub.PublishEvent,
        publish_request,
        metadata=grpc_metadata,
    )

    assert publish_response.result.success, "Publish failed: {}".format(
        publish_response.result.failure_reason
    )


def run_batch_publish_events_grpc(
    resource_base_url, notification_aggregation_grpc_stub, grpc_metadata, count=3
):
    """Batch publish multiple events via gRPC."""
    occurred_at = timestamp_pb2.Timestamp()
    occurred_at.GetCurrentTime()

    events = []
    for i in range(count):
        file_name = "".join(random.choices(string.ascii_letters + string.digits, k=5))
        resource_address = f"{resource_base_url}{file_name}"

        message = struct_pb2.Struct()
        message["resource_address"] = resource_address
        message["event_index"] = i
        message["timestamp"] = datetime.now(timezone.utc).isoformat()

        event = event_publisher_pb2.Event(
            event_type="test.batch.publish",
            message=message,
            occurred_at=occurred_at,
            resource=event_publisher_pb2.EventResource(resource_id=resource_address),
        )
        events.append(event)

    batch_request = event_publisher_pb2.BatchPublishEventsRequest(events=events)

    batch_response = timeout_grpc_call(
        notification_aggregation_grpc_stub.BatchPublishEvents,
        batch_request,
        metadata=grpc_metadata,
    )

    results = batch_response.results
    results_count = len(results)

    assert results_count == count, f"Expected {count} results, got {results_count}"

    failed_count = 0
    for i, result in enumerate(results):
        if not result.success:
            failure_reason = result.failure_reason or "No failure reason provided"
            logger.error(f"  Result {i} failed: {failure_reason}")
            failed_count += 1

    assert failed_count == 0, f"{failed_count} out of {results_count} events failed"

    logger.info(f"✅ All {results_count} events published successfully")
