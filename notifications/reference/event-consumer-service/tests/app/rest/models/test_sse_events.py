# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from datetime import datetime

from app.rest.models.sse_data import EventData
from app.rest.models.sse_events import ConsumeEvent, ConnectedEvent


class TestSseEvent:
    class TestToDict:
        def test_returns_dict(self, freezer):
            freezer.move_to("2025-05-27T18:13:54.328674")

            event = ConsumeEvent(
                data=EventData(
                    event_type="test-type",
                    principal_identity="test-principal",
                    occurred_at=datetime.now(),
                    published_at=datetime.now(),
                    message={"data": "test-data"},
                ),
            )

            assert event.to_dict() == {
                "data": "{"
                '"event_type":"test-type",'
                '"principal_identity":"test-principal",'
                '"occurred_at":"2025-05-27T18:13:54.328674",'
                '"published_at":"2025-05-27T18:13:54.328674",'
                '"message":{"data":"test-data"}'
                "}",
                "event": "event",
            }


class TestConnectedEvent:
    class TestToDict:
        def test_includes_id_in_dict(self):
            event = ConnectedEvent(id="test-id")

            assert event.to_dict() == {
                "data": "{"
                '"message":"Connection established",'
                '"reconnect_token":"test-id"'
                "}",
                "event": "connected",
                "id": "test-id",
            }


class TestConsumeEvent:
    class TestSimplifiedSchema:
        def test_returns_dict_with_openapi_acceptable_values(self):
            assert ConsumeEvent.simplified_schema() == {
                "properties": {
                    "data": {
                        "description": "Model representing a full " "event payload.",
                        "properties": {
                            "event_type": {
                                "description": (
                                    "Type of event - defined by " "the publisher"
                                ),
                                "maxLength": 1024,
                                "title": "Event " "Type",
                                "type": "string",
                            },
                            "message": {
                                "additionalProperties": True,
                                "description": "Body of event",
                                "title": "Message",
                                "type": "object",
                            },
                            "occurred_at": {
                                "description": ("Time at which event was " "generated"),
                                "format": "date-time",
                                "maxLength": 64,
                                "title": "Occurred " "At",
                                "type": "string",
                            },
                            "principal_identity": {
                                "default": "",
                                "description": ("Principal who published event"),
                                "maxLength": 1024,
                                "title": "Principal " "Identity",
                                "type": "string",
                            },
                            "published_at": {
                                "description": ("Time at which event was " "published"),
                                "format": "date-time",
                                "maxLength": 64,
                                "title": "Published " "At",
                                "type": "string",
                            },
                        },
                        "required": [
                            "event_type",
                            "occurred_at",
                            "published_at",
                            "message",
                        ],
                        "title": "EventData",
                        "type": "object",
                    },
                    "event": {
                        "default": "event",
                        "maxLength": 256,
                        "title": "Event",
                        "type": "string",
                    },
                },
                "required": ["data"],
                "title": "ConsumeEvent",
                "type": "object",
            }
