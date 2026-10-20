# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Models for batch event publishing."""

from typing import List

from pydantic import BaseModel, Field

from app.lib.limits import MAX_BATCH_EVENTS
from app.rest.models.event_models import Event, PublishingResult


class BatchPublishEventsRequest(BaseModel):
    """
    Request model for publishing multiple events in a batch.

    When multiple events are sent, they are published into the message
    broker in parallel for faster performance.
    """

    events: List[Event] = Field(
        max_length=MAX_BATCH_EVENTS,
        description="The events that should be published",
        examples=[
            [
                {
                    "event_type": "storage.file.created",
                    "message": {"filename": "doc1.pdf"},
                    "occurred_at": "2024-10-23T12:34:56Z",
                    "resource": {"resource_id": "/folder/doc1.pdf"},
                },
                {
                    "event_type": "storage.file.created",
                    "message": {"filename": "doc2.pdf"},
                    "occurred_at": "2024-10-23T12:34:57Z",
                    "resource": {"resource_id": "/folder/doc2.pdf"},
                },
            ]
        ],
    )


class BatchPublishEventsResponse(BaseModel):
    """
    Response model for publishing multiple events.
    """

    results: List[PublishingResult] = Field(
        max_length=MAX_BATCH_EVENTS,
        description=(
            "The results from all of the publishing attempts "
            "for each request/event in the batch"
        ),
    )
