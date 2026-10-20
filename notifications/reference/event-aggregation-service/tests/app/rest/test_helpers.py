# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for REST helpers."""

import pytest
from notification_common.rabbitmq.publisher import (
    SuccessfulPublishingResult,
    FailedPublishingResult,
)

from app.rest.helpers import (
    event_to_omni_event,
    publishing_result_to_rest,
    create_event_resource,
)
from app.rest.models.event_models import Event, EventResource


class TestEventToOmniEvent:
    """Tests for event_to_omni_event function."""

    @pytest.mark.asyncio
    async def test_converts_event_with_resource(self):
        """Test converting Event with resource to OmniEvent."""
        event = Event(
            event_type="test.event",
            message={"key": "value"},
            occurred_at="2024-10-23T12:34:56Z",
            resource=EventResource(resource_id="/test/resource"),
        )

        omni_event = await event_to_omni_event(event, "test_principal")

        assert omni_event.event_type == "test.event"
        assert omni_event.message == {"key": "value"}
        assert omni_event.occurred_at == "2024-10-23T12:34:56Z"
        assert omni_event.resource_id == "/test/resource"
        assert omni_event.principal_identity == "test_principal"

    @pytest.mark.asyncio
    async def test_converts_event_without_resource(self):
        """Test converting Event without resource to OmniEvent."""
        event = Event(
            event_type="test.event",
            message={"key": "value"},
            occurred_at="2024-10-23T12:34:56Z",
        )

        omni_event = await event_to_omni_event(event, "test_principal")

        assert omni_event.event_type == "test.event"
        assert omni_event.resource_id == ""

    @pytest.mark.asyncio
    async def test_handles_complex_message(self):
        """Test converting Event with a complex message."""
        event = Event(
            event_type="test.event",
            message={"nested": {"key": "value"}, "list": [1, 2, 3]},
            occurred_at="2024-10-23T12:34:56Z",
        )

        omni_event = await event_to_omni_event(event, "test_principal")

        assert omni_event.message == {"nested": {"key": "value"}, "list": [1, 2, 3]}


class TestPublishingResultToRest:
    """Tests for publishing_result_to_rest function."""

    def test_converts_successful_result(self, mocked_omni_event):
        """Test converting successful PublishingResult."""
        event = Event(
            event_type="test.event",
            message={"key": "value"},
            occurred_at="2024-10-23T12:34:56Z",
        )
        result = SuccessfulPublishingResult(event=mocked_omni_event)

        rest_result = publishing_result_to_rest(result, event)

        assert rest_result.event == event
        assert rest_result.success is True
        assert rest_result.failure_reason is None

    def test_converts_failed_result(self, mocked_omni_event):
        """Test converting failed PublishingResult."""
        event = Event(
            event_type="test.event",
            message={"key": "value"},
            occurred_at="2024-10-23T12:34:56Z",
        )
        result = FailedPublishingResult(event=mocked_omni_event, reason="Test failure")

        rest_result = publishing_result_to_rest(result, event)

        assert rest_result.event == event
        assert rest_result.success is False
        assert rest_result.failure_reason == "Test failure"


class TestCreateEventResource:
    """Tests for create_event_resource function."""

    def test_creates_resource_with_id(self):
        """Test creating EventResource with resource_id."""
        resource = create_event_resource("/test/resource")

        assert resource is not None
        assert resource.resource_id == "/test/resource"

    def test_returns_none_for_empty_string(self):
        """Test returns None for empty string."""
        resource = create_event_resource("")

        assert resource is None

    def test_returns_none_for_none(self):
        """Test returns None for None."""
        resource = create_event_resource(None)

        assert resource is None
