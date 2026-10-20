# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Models for single event publishing."""

from pydantic import BaseModel, Field

from app.rest.models.event_models import Event, PublishingResult


class PublishEventRequest(BaseModel):
    """
    Request model for publishing a single event.
    """

    event: Event = Field(
        description="The event to publish",
        examples=[
            {
                "event_type": "storage.file.created",
                "message": {"filename": "document.pdf", "size": 1024},
                "occurred_at": "2024-10-23T12:34:56Z",
                "resource": {"resource_id": "/folder/document.pdf"},
            }
        ],
    )


class PublishEventResponse(BaseModel):
    """
    Response model for publishing a single event.
    """

    result: PublishingResult = Field(description="The result from the publish attempt")
