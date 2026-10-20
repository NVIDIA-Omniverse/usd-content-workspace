# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest

from nvidia.omniverse.notifications.publisher.v1beta.event_publisher_pb2 import (
    Event,
)


@pytest.fixture
def event_with(mocked_event):
    """Return a factory for a copy of ``mocked_event`` with fields overridden.

    ``message`` replaces the event's message; ``event_type`` and
    ``resource_id`` replace those fields.
    """

    def _build(message=None, event_type=None, resource_id=None) -> Event:
        event = Event()
        event.CopyFrom(mocked_event)
        if message is not None:
            event.message.Clear()
            event.message.update(message)
        if event_type is not None:
            event.event_type = event_type
        if resource_id is not None:
            event.resource.resource_id = resource_id
        return event

    return _build
