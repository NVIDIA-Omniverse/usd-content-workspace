# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from notification_common.rabbitmq.publisher import PublishingResult

from nvidia.omniverse.notifications.publisher.v1beta.event_publisher_pb2 import (
    PublishingResult as GrpcPublishingResult,
    Event,
)


def grpc_publishing_result(
    publishing_result: PublishingResult, grpc_event: Event
) -> GrpcPublishingResult:
    result = GrpcPublishingResult(success=publishing_result.success, event=grpc_event)

    if publishing_result.success:
        return result

    result.failure_reason = publishing_result.reason

    return result


def grpc_publishing_results(
    publishing_results: list[PublishingResult], grpc_events: list[Event]
) -> list[GrpcPublishingResult]:
    return [
        grpc_publishing_result(result, grpc_events[i])
        for i, result in enumerate(publishing_results)
    ]
