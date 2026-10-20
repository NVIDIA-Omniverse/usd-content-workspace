# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import datetime
import logging

import grpc
from google.protobuf.json_format import MessageToDict
from notification_common.errors.errors import BadRequestError
from notification_common.rabbitmq.omni_event import OmniEvent

from app.grpc.grpc_publishing_results import (
    grpc_publishing_results,
    grpc_publishing_result,
)
from app.lib.auth.authenticate import authenticate
from app.lib.limits import (
    MAX_BATCH_EVENTS,
    MAX_EVENT_TYPE_LENGTH,
    MAX_RESOURCE_ID_LENGTH,
    length_violation,
    message_size_violation,
)
from app.lib.rabbitmq import publisher

from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2_grpc

from nvidia.omniverse.notifications.publisher.v1beta.event_publisher_pb2 import (
    PublishEventRequest,
    PublishEventResponse,
    BatchPublishEventsRequest,
    Event,
    BatchPublishEventsResponse,
)

logger = logging.getLogger(__name__)


class EventAggregationServicer(event_publisher_pb2_grpc.EventPublishingServiceServicer):
    async def PublishEvent(
        self, request: PublishEventRequest, _context: grpc.ServicerContext
    ):
        principal = await authenticate()

        # Log a summary, not the request: formatting a request that the limits
        # below are about to refuse would cost as much as the request is large.
        logger.debug(
            "Received publish request for event_type %r", request.event.event_type[:64]
        )

        publishing_result = await publisher.publish(
            await self._omni_event(request.event, principal, "event")
        )

        return PublishEventResponse(
            result=grpc_publishing_result(publishing_result, request.event)
        )

    async def BatchPublishEvents(
        self, request: BatchPublishEventsRequest, _context: grpc.ServicerContext
    ):
        principal = await authenticate()

        # Local so that aio-pika code doesn't raise
        # warning about no existing event loop.
        from app.lib.rabbitmq import publisher

        logger.debug(
            "Received batch publish request with %d events", len(request.events)
        )

        if not request.events:
            raise BadRequestError("Publish request must contain at least one event")

        if len(request.events) > MAX_BATCH_EVENTS:
            raise BadRequestError(
                f"Publish request must contain at most {MAX_BATCH_EVENTS} events, "
                f"but contains {len(request.events)}"
            )

        omni_events = [
            await self._omni_event(event, principal, f"events[{index}]")
            for index, event in enumerate(request.events)
        ]

        results = await publisher.batch_publish(omni_events)

        return BatchPublishEventsResponse(
            results=grpc_publishing_results(results, request.events),
        )

    @staticmethod
    async def _omni_event(event: Event, principal: str, location: str) -> OmniEvent:
        """Build the event to publish, applying the shared request limits.

        ``location`` names the event in the request (``event`` or
        ``events[<index>]``), so a limit violation says which field of which
        event is over, as a REST validation error's ``loc`` does. The REST
        models' ``occurred_at`` length bound has no gRPC counterpart because a
        protobuf ``Timestamp`` is fixed-size.
        """
        if not event.occurred_at.seconds:
            raise BadRequestError(f"{location}.occurred_at must be specified")

        violation = length_violation(event.event_type, MAX_EVENT_TYPE_LENGTH)
        if violation:
            raise BadRequestError(f"{location}.event_type {violation}")

        resource_id = "" if not event.resource else event.resource.resource_id
        violation = length_violation(resource_id, MAX_RESOURCE_ID_LENGTH)
        if violation:
            raise BadRequestError(f"{location}.resource.resource_id {violation}")

        message = MessageToDict(event.message)
        violation = message_size_violation(message)
        if violation:
            raise BadRequestError(f"{location}.message {violation}")

        occurred_at = (
            datetime.datetime.fromtimestamp(event.occurred_at.seconds, datetime.UTC)
            .isoformat()
            .replace("+00:00", "Z")
        )

        return OmniEvent(
            event_type=event.event_type,
            occurred_at=occurred_at,
            resource_id=resource_id,
            message=message,
            principal_identity=principal,
        )
