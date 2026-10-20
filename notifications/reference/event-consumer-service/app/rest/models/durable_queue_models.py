# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Models for durable queue management.

Only the shapes a route actually declares live here. The delete and durable
consume endpoints take their ``queue_id`` as a query parameter rather than a
request body, so they carry the shared ``QUEUE_ID_PATTERN`` on the ``Query``
declaration in ``app.rest.routes`` and need no model of their own.
"""

from pydantic import BaseModel, Field

from app.lib.models.filter_group import FilterGroup
from app.lib.rabbitmq.rabbitmq_base import QUEUE_ID_PATTERN


class CreateDurableQueueRequest(BaseModel):
    """
    Request model for creating a durable queue.
    """

    filter_groups: list[FilterGroup] = Field(
        max_length=100,
        description=(
            "Filter groups that define the sets of filters to apply to events based "
            "on the resource id of the messages. If omitted or empty, all events "
            "for an event type are added to the queue for later consumption."
        ),
        default_factory=list,
    )
    queue_id: str | None = Field(
        default=None,
        pattern=QUEUE_ID_PATTERN,
        max_length=200,
        description=(
            "Optional client-specified id for the durable queue. If supplied, it "
            "must match the shared character and length constraints (1-200 "
            "characters, each one of A-Z, a-z, 0-9, '.', '_', or '-'). If omitted, "
            "the service generates a queue_id.\n\n"
            "A supplied queue_id must be unique across the whole deployment. Queue "
            "ids share a single namespace: this endpoint attaches to an existing "
            "queue with the supplied id regardless of who created it or which "
            "filter_groups are passed, returning created=false and that queue with "
            "its existing filters. Two callers that pick the same id therefore "
            "share one queue - each receives the other's events, and either can "
            "delete it. Use a UUID, or a namespaced convention such as "
            "<service-name>-<purpose>; never a bare generic word like 'events'.\n\n"
            "Per-queue isolation is not enforced. The three durable-queue "
            "permissions are deployment-wide rather than per-queue: a principal "
            "granted 'consume-durable-queues' can consume from any durable queue "
            "whose id it knows, one granted 'delete-durable-queues' can delete any "
            "of them, and 'create-durable-queues' authorizes both creating a queue "
            "and updating the filters of any existing one. Operators should grant "
            "these permissions deliberately - to service accounts rather than to "
            "broad user groups."
        ),
    )


class CreateDurableQueueResponse(BaseModel):
    """
    Response model for creating a durable queue.
    """

    queue_id: str = Field(
        max_length=256,
        description=(
            "The queue_id identifies the durable queue. This ID must be persisted "
            "somewhere and sent on all ConsumeDurableEvents calls."
        ),
    )
    created: bool = Field(
        description=(
            "Indicates whether the durable queue was newly created (true) versus "
            "already existed (false)."
        ),
    )


class DeleteDurableQueueResponse(BaseModel):
    """
    Response model for deleting a durable queue.
    """

    success: bool = Field(
        default=True,
        description="Indicates whether the queue was successfully deleted",
    )


class UpdateDurableQueueRequest(BaseModel):
    """
    Request model for updating a durable queue's filters.
    """

    queue_id: str = Field(
        pattern=QUEUE_ID_PATTERN,
        max_length=200,
        description=(
            "The id of the durable queue to update. It must match the shared "
            "character and length constraints (1-200 characters, each one of "
            "A-Z, a-z, 0-9, '.', '_', or '-').\n\n"
            "queue_ids share one deployment-wide namespace, so this must be the "
            "id of a queue that was created by (or for) your client; see the "
            "queue_id field of POST /queues/durable."
        ),
    )
    current_filter_groups: list[FilterGroup] = Field(
        max_length=100,
        description=(
            "The current filters in use on the existing durable queue. "
            "These will be replaced if new_filter_groups is provided."
        ),
    )
    new_filter_groups: list[FilterGroup] = Field(
        max_length=100,
        description=(
            "The new filters to use on the existing durable queue. "
            "They will replace the existing filters."
        ),
    )


class UpdateDurableQueueResponse(BaseModel):
    """
    Response model for updating a durable queue.
    """

    success: bool = Field(
        default=True,
        description="Indicates whether the queue was successfully updated",
    )
