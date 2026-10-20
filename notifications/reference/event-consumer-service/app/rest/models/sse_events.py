# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from notification_common.errors.error_details import ErrorDetailsModel
from pydantic import BaseModel, field_validator, computed_field, Field

from app.lib.models.schema_model import SchemaModel
from app.rest.models.sse_data import ConnectedMessageData, EventData


class SseEvent(BaseModel):
    def to_dict(self):
        """
        Convert an SSEEvent model to a dictionary to be streamed back to a client.
        Pydantic models are not automatically serialized for SSE
        """
        return {
            "event": self.event,
            "data": self.data.model_dump_json(exclude_none=True),  # type: ignore
        }


class ConnectedEvent(SseEvent):
    # The id field is used for reconnecting.
    # When the browser automatically tries reconnecting,
    # it will include the ID value in the "Last-Event-ID" request header.
    id: str = Field(max_length=1024)
    event: str = Field(default="connected", max_length=256)

    @computed_field
    @property
    def data(self) -> ConnectedMessageData:
        return ConnectedMessageData(
            message="Connection established",
            reconnect_token=self.id,
        )

    def to_dict(self):
        """
        Convert an SSEEvent model to a dictionary to be streamed back to a client.
        Pydantic models are not automatically serialized for SSE
        """
        data = super().to_dict()
        data["id"] = self.id
        return data


class ConsumeEvent(SseEvent, SchemaModel):
    event: str = Field(default="event", max_length=256)
    data: EventData

    # noinspection PyNestedDecorators
    @field_validator("data", mode="before")
    @classmethod
    def validate_data(cls, v):
        if isinstance(v, dict):
            return EventData.model_validate(v)
        return v


class ErrorEvent(SseEvent):
    event: str = Field(default="error", max_length=256)
    data: ErrorDetailsModel
