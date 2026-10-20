# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""FastAPI routes for the event aggregation service."""

import logging

from fastapi import APIRouter, Depends
from fastapi.security import HTTPBearer
from notification_common.errors.errors import BadRequestError
from notification_common.rest.models.health_response import HealthResponse

from app.lib.auth.authenticate import authenticate
from app.lib.rabbitmq import publisher
from app.rest.helpers import event_to_omni_event, publishing_result_to_rest
from app.rest.models.batch_publish_models import (
    BatchPublishEventsRequest,
    BatchPublishEventsResponse,
)
from app.rest.models.publish_event_models import (
    PublishEventRequest,
    PublishEventResponse,
)

# Security dependency for protected endpoints
http_bearer = HTTPBearer(auto_error=False)

logger = logging.getLogger(__name__)

router = APIRouter()

# Shared response definition for an oversize request body
PAYLOAD_TOO_LARGE_RESPONSE = {
    413: {
        "description": (
            "Content Too Large - The request body exceeds the configured "
            "OMNI_EVENTS_REST_MAX_REQUEST_BYTES limit (4 MiB by default). "
            "The request is refused before it is authenticated or parsed, so "
            "no event is published."
        )
    }
}

# Shared response definitions for authorization errors
AUTH_ERROR_RESPONSES = {
    401: {
        "description": (
            "Unauthorized - The request requires valid authentication "
            "credentials. This typically indicates a missing, invalid, "
            "or expired authentication token."
        )
    },
    403: {
        "description": (
            "Forbidden - The authenticated user does not have permission "
            "to access this resource or perform this operation."
        )
    },
}


@router.post(
    "/events",
    description=(
        "Publishes a single event to the notification service. "
        "The event will be routed to all appropriate consumers based "
        "on their filter subscriptions."
    ),
    response_model=PublishEventResponse,
    dependencies=[Depends(http_bearer)],
    responses={
        200: {"description": "Event successfully published"},
        400: {
            "description": (
                "Bad Request - Invalid event data. "
                "Check that all required fields are present and valid."
            )
        },
        422: {"description": "Unprocessable Entity - Validation error"},
        500: {"description": "Internal Server Error"},
        **AUTH_ERROR_RESPONSES,
        **PAYLOAD_TOO_LARGE_RESPONSE,
    },
)
async def publish_event(
    body: PublishEventRequest, principal: str = Depends(authenticate)
) -> PublishEventResponse:
    """
    Publish a single event to the notification service.

    The event will be routed to all appropriate consumers based on their
    filter subscriptions. This endpoint returns after the event has been
    successfully published to the message broker.

    :param body: The event to publish
    :param principal: The calling principal, resolved by ``authenticate``
        before the request reaches this handler
    :return: A PublishEventResponse containing the publishing result
    """
    logger.debug(f"Received publish request: {body}")

    omni_event = await event_to_omni_event(body.event, principal)
    result = await publisher.publish(omni_event)
    rest_result = publishing_result_to_rest(result, body.event)

    return PublishEventResponse(result=rest_result)


@router.post(
    "/events/batch",
    description=(
        "Publishes multiple events to the notification service in parallel. "
        "Each event will be routed to all appropriate consumers based on "
        "their filter subscriptions. This endpoint is more efficient than "
        "making multiple individual publish requests."
    ),
    response_model=BatchPublishEventsResponse,
    dependencies=[Depends(http_bearer)],
    responses={
        200: {
            "description": (
                "Events processed. Check the results list for "
                "individual publish outcomes."
            )
        },
        400: {
            "description": (
                "Bad Request - Invalid request. "
                "The request must contain at least one event."
            )
        },
        422: {"description": "Unprocessable Entity - Validation error"},
        429: {"description": "Too Many Requests - Rate limit exceeded"},
        500: {"description": "Internal Server Error"},
        **AUTH_ERROR_RESPONSES,
        **PAYLOAD_TOO_LARGE_RESPONSE,
    },
)
async def batch_publish_events(
    body: BatchPublishEventsRequest, principal: str = Depends(authenticate)
) -> BatchPublishEventsResponse:
    """
    Publish multiple events to the notification service in parallel.

    When multiple events are sent, they are published into the message
    broker in parallel for faster performance. Each event will be routed
    to all appropriate consumers based on their filter subscriptions.

    :param body: The batch of events to publish
    :param principal: The calling principal, resolved by ``authenticate``
        before the request reaches this handler
    :return: BatchPublishEventsResponse with results for each event
    """
    logger.debug(f"Received batch publish request with {len(body.events)} events")

    if not body.events:
        raise BadRequestError("Publish request must contain at least one event")

    omni_events = [await event_to_omni_event(event, principal) for event in body.events]
    results = await publisher.batch_publish(omni_events)

    rest_results = [
        publishing_result_to_rest(result, event)
        for result, event in zip(results, body.events)
    ]

    return BatchPublishEventsResponse(results=rest_results)


@router.get(
    "/health",
    description="Health check endpoint for the API",
    response_model=HealthResponse,
)
async def health_check() -> HealthResponse:
    """Health check endpoint."""
    return HealthResponse()
