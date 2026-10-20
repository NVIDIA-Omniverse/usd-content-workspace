# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Core event models for the REST API."""

from typing import Optional

from pydantic import BaseModel, Field, field_validator

from app.lib.limits import (
    MAX_EVENT_MESSAGE_BYTES,
    MAX_EVENT_TYPE_LENGTH,
    MAX_OCCURRED_AT_LENGTH,
    MAX_RESOURCE_ID_LENGTH,
    message_size_violation,
)


class EventResource(BaseModel):
    """
    The resource that an event is concerned with.

    A path-like string (slash separated) that consumers can use for
    filtering events. Each word separated by a slash is something that
    a consumer could filter by.
    """

    resource_id: str = Field(
        max_length=MAX_RESOURCE_ID_LENGTH,
        description=(
            "A path-like string (slash separated) that consumers can use for "
            "filtering events. For example, in the case of a Storage API, this "
            "could be a file path like '/folder/subfolder/file.txt'. "
            f"At most {MAX_RESOURCE_ID_LENGTH} characters (Unicode code points)."
        ),
        examples=["/folder/file.txt", "/projects/project1/"],
    )


class Event(BaseModel):
    """
    An event object that encapsulates all event data.
    """

    event_type: str = Field(
        max_length=MAX_EVENT_TYPE_LENGTH,
        pattern=r"^[a-zA-Z0-9._-]+$",
        description=(
            "A string that defines the event type. Services that use "
            "the notification aggregation service will typically define "
            "their own event types. "
            f"At most {MAX_EVENT_TYPE_LENGTH} characters (Unicode code points)."
        ),
        examples=["storage.file.created", "project.updated"],
    )

    message: dict = Field(
        description=(
            "JSON event message containing the event payload. "
            "The structure of this message is determined by the event type. "
            f"Encoded as JSON, it must be at most {MAX_EVENT_MESSAGE_BYTES} bytes, "
            "measured as Python's json.dumps encodes it with its defaults: "
            "', ' and ': ' separators, every non-ASCII character escaped as "
            "\\uXXXX (six bytes; twelve as a surrogate pair outside the Basic "
            "Multilingual Plane), plus JSON's usual escapes for quotes, "
            "backslashes, and control characters. Leave headroom rather than "
            "sizing a message to the exact byte."
        ),
        examples=[{"action": "created", "user": "john@example.com"}],
    )

    occurred_at: str = Field(
        max_length=MAX_OCCURRED_AT_LENGTH,
        pattern=r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$",
        description=(
            "Timestamp for when the event occurred. Must be provided "
            "in ISO 8601 format (e.g., '2024-10-23T12:34:56Z')."
        ),
        examples=["2024-10-23T12:34:56Z"],
    )

    resource: Optional[EventResource] = Field(
        default=None,
        description=(
            "Optional resource identifier that the event is concerned with. "
            "Used by consumer clients for filtering messages beyond just "
            "filtering by event_type. If omitted, it is equivalent to "
            "passing a resource with resource_id=''."
        ),
    )

    @field_validator("message")
    @classmethod
    def _message_within_size_limit(cls, value: dict) -> dict:
        violation = message_size_violation(value)
        if violation:
            raise ValueError(violation)
        return value


class PublishingResult(BaseModel):
    """
    The result from when an event publish is attempted.
    """

    event: Event = Field(
        description="The event that was published or attempted to be published"
    )

    success: bool = Field(description="Whether or not publishing was successful")

    failure_reason: Optional[str] = Field(
        default=None,
        max_length=1024,
        description="If publishing failed, this contains the reason for failure",
    )
