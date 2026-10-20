# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import time

from app.lib.models.event_message import EventMessage


def test_initializes_with_message_and_current_time(mocker):
    message = {"event_type": "test_event", "data": "test"}

    mocker.patch.object(time, "perf_counter", return_value=100.0)
    event_msg = EventMessage(message)

    assert event_msg.message == message
    assert event_msg.processing_started_at == 100.0


def test_initializes_with_custom_start_time():
    message = {"event_type": "test_event"}
    custom_start_time = 50.0

    event_msg = EventMessage(message, processing_started_at=custom_start_time)

    assert event_msg.message == message
    assert event_msg.processing_started_at == custom_start_time


class TestEventType:

    def test_returns_event_type_from_message(self):
        message = {"event_type": "user_created", "data": "test"}
        event_msg = EventMessage(message)

        assert event_msg.event_type == "user_created"

    def test_returns_unknown_when_event_type_missing(self):
        message = {"data": "test"}
        event_msg = EventMessage(message)

        assert event_msg.event_type == "unknown"

    def test_returns_unknown_when_event_type_is_none(self):
        message = {"event_type": None, "data": "test"}
        event_msg = EventMessage(message)

        assert event_msg.event_type == "unknown"

    def test_returns_unknown_when_event_type_is_empty_string(self):
        message = {"event_type": "", "data": "test"}
        event_msg = EventMessage(message)

        assert event_msg.event_type == "unknown"


class TestProcessingDurationMs:

    def test_calculates_duration_in_milliseconds(self, mocker):
        message = {"event_type": "test_event"}

        mocker.patch.object(time, "perf_counter", side_effect=[100.0, 100.5])
        event_msg = EventMessage(message)
        duration = event_msg.processing_duration_ms

        assert duration == 500.0
