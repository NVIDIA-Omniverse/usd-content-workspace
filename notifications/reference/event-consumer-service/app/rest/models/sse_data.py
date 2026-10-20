# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from datetime import datetime
from typing import Any

from pydantic import BaseModel, Field


class EventData(BaseModel):
    """
    Model representing a full event payload.
    """

    event_type: str = Field(
        max_length=1024, description="Type of event - defined by the publisher"
    )
    principal_identity: str = Field(
        default="",
        max_length=1024,
        description="Principal who published event",
    )
    occurred_at: datetime = Field(
        json_schema_extra={"maxLength": 64},
        description="Time at which event was generated",
    )
    published_at: datetime = Field(
        json_schema_extra={"maxLength": 64},
        description="Time at which event was published",
    )
    message: dict[str, Any] = Field(description="Body of event")


class ConnectedMessageData(BaseModel):
    message: str = Field(max_length=1024, description="Connection status message")
    reconnect_token: str = Field(
        max_length=1024,
        description=(
            "Token for reconnecting to the same queue after disconnect, or for "
            "consuming from the same queue in parallel from multiple clients."
        ),
    )
