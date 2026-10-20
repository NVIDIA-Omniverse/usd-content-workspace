# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for event models."""

import pytest
from pydantic import ValidationError

from app.lib.limits import MAX_EVENT_MESSAGE_BYTES
from app.rest.models.event_models import Event, EventResource, PublishingResult


class TestEventResource:
    """Tests for EventResource model."""

    def test_creates_with_resource_id(self):
        """Test creating EventResource with resource_id."""
        resource = EventResource(resource_id="/folder/file.txt")
        assert resource.resource_id == "/folder/file.txt"

    def test_validates_resource_id_required(self):
        """Test that resource_id is required."""
        with pytest.raises(ValidationError):
            EventResource()


class TestEvent:
    """Tests for Event model."""

    def test_creates_with_all_fields(self):
        """Test creating Event with all fields."""
        occurred_at = "2024-10-23T12:34:56Z"
        resource = EventResource(resource_id="/test")

        event = Event(
            event_type="test.event",
            message={"key": "value"},
            occurred_at=occurred_at,
            resource=resource,
        )

        assert event.event_type == "test.event"
        assert event.message == {"key": "value"}
        assert event.occurred_at == occurred_at
        assert event.resource == resource

    def test_creates_without_resource(self):
        """Test creating Event without resource."""
        occurred_at = "2024-10-23T12:34:56Z"

        event = Event(
            event_type="test.event",
            message={"key": "value"},
            occurred_at=occurred_at,
        )

        assert event.event_type == "test.event"
        assert event.resource is None

    def test_validates_event_type_required(self):
        """Test that event_type is required."""
        with pytest.raises(ValidationError):
            Event(
                message={"key": "value"},
                occurred_at="2024-10-23T12:34:56Z",
            )

    def test_validates_message_required(self):
        """Test that message is required."""
        with pytest.raises(ValidationError):
            Event(
                event_type="test.event",
                occurred_at="2024-10-23T12:34:56Z",
            )

    def test_validates_occurred_at_required(self):
        """Test that occurred_at is required."""
        with pytest.raises(ValidationError):
            Event(
                event_type="test.event",
                message={"key": "value"},
            )

    def test_parses_iso_datetime_string(self):
        """Test parsing ISO 8601 datetime string."""
        event = Event(
            event_type="test.event",
            message={"key": "value"},
            occurred_at="2024-10-23T12:34:56Z",
        )

        assert isinstance(event.occurred_at, str)
        assert event.occurred_at == "2024-10-23T12:34:56Z"


class TestPublishingResult:
    """Tests for PublishingResult model."""

    def test_creates_successful_result(self):
        """Test creating successful PublishingResult."""
        event = Event(
            event_type="test.event",
            message={"key": "value"},
            occurred_at="2024-10-23T12:34:56Z",
        )

        result = PublishingResult(
            event=event,
            success=True,
        )

        assert result.event == event
        assert result.success is True
        assert result.failure_reason is None

    def test_creates_failed_result(self):
        """Test creating failed PublishingResult."""
        event = Event(
            event_type="test.event",
            message={"key": "value"},
            occurred_at="2024-10-23T12:34:56Z",
        )

        result = PublishingResult(
            event=event,
            success=False,
            failure_reason="Test failure",
        )

        assert result.event == event
        assert result.success is False
        assert result.failure_reason == "Test failure"

    def test_validates_event_required(self):
        """Test that event is required."""
        with pytest.raises(ValidationError):
            PublishingResult(success=True)

    def test_validates_success_required(self):
        """Test that success is required."""
        event = Event(
            event_type="test.event",
            message={"key": "value"},
            occurred_at="2024-10-23T12:34:56Z",
        )

        with pytest.raises(ValidationError):
            PublishingResult(event=event)


class TestEventMessageSizeLimit:
    """Tests for the size limit on the Event message field."""

    @pytest.fixture
    def event_with_message(self):
        def _build(message: dict) -> Event:
            return Event(
                event_type="test.event",
                message=message,
                occurred_at="2024-10-23T12:34:56Z",
            )

        return _build

    def test_accepts_message_at_the_limit(self, event_with_message, message_of_size):
        message = message_of_size(MAX_EVENT_MESSAGE_BYTES)

        assert event_with_message(message).message == message

    def test_rejects_message_over_the_limit(self, event_with_message, message_of_size):
        with pytest.raises(
            ValidationError, match=f"at most {MAX_EVENT_MESSAGE_BYTES} bytes"
        ):
            event_with_message(message_of_size(MAX_EVENT_MESSAGE_BYTES + 1))
