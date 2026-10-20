# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Helper functions for REST API request/response handling."""

from typing import Optional

from notification_common.rabbitmq.omni_event import OmniEvent
from notification_common.rabbitmq.publisher import PublishingResult

from app.rest.models.event_models import Event, EventResource
from app.rest.models.event_models import (
    PublishingResult as RestPublishingResult,
)


async def event_to_omni_event(event: Event, principal: str) -> OmniEvent:
    """
    Convert a REST Event model to an OmniEvent for publishing.

    :param event: The REST Event model
    :param principal: The principal identity from auth context
    :return: An OmniEvent ready for publishing
    """
    resource_id = ""
    if event.resource:
        resource_id = event.resource.resource_id

    # occurred_at is already an ISO 8601 string, just ensure it uses Z
    occurred_at = event.occurred_at.replace("+00:00", "Z")

    return OmniEvent(
        event_type=event.event_type,
        occurred_at=occurred_at,
        resource_id=resource_id,
        message=event.message,
        principal_identity=principal,
    )


def publishing_result_to_rest(
    result: PublishingResult, event: Event
) -> RestPublishingResult:
    """
    Convert a PublishingResult to a REST PublishingResult model.

    :param result: The internal PublishingResult
    :param event: The original REST Event that was published
    :return: A REST PublishingResult model
    """
    return RestPublishingResult(
        event=event,
        success=result.success,
        failure_reason=result.reason if not result.success else None,
    )


def create_event_resource(resource_id: Optional[str]) -> Optional[EventResource]:
    """
    Create an EventResource if resource_id is provided.

    :param resource_id: The resource ID string
    :return: EventResource or None
    """
    if resource_id:
        return EventResource(resource_id=resource_id)
    return None
