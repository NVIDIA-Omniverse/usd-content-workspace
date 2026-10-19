# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Common fixtures for event stream handler tests."""

from unittest.mock import AsyncMock

import pytest

from app.lib.models.event_message import EventMessage
from app.lib.observability.sse_metrics import sse_metrics
from app.lib.rabbitmq import DurableQueueConsumer


class AsyncIterator:
    """Helper class for mocking async iterators"""

    def __init__(self, items=None, error=None):
        self.items = items or []
        self.error = error

    def __aiter__(self):
        return self

    async def __anext__(self):
        if self.error:
            raise self.error
        if not self.items:
            raise StopAsyncIteration
        return self.items.pop(0)


@pytest.fixture
def mock_sse_end_connection(mocker):
    return mocker.patch.object(sse_metrics, "end_connection")


@pytest.fixture
def mock_sse_record_event_sent(mocker):
    return mocker.patch.object(sse_metrics, "record_event_sent")


@pytest.fixture
def event_data():
    return {
        "event_type": "test_event",
        "message": {"test": "data"},
        "occurred_at": "2023-01-01T00:00:00Z",
        "published_at": "2023-01-01T00:01:00Z",
    }


@pytest.fixture
def mock_queue_id():
    return "test-durable-queue-id"


@pytest.fixture
def mock_get_queue(mocker):
    return mocker.patch.object(
        DurableQueueConsumer, "get_queue", return_value=AsyncMock()
    )


@pytest.fixture
def mock_durable_consume(mocker, event_data):
    async def mock_gen():
        yield EventMessage(event_data)

    mocker.patch.object(
        DurableQueueConsumer,
        "consume",
        side_effect=lambda *_args, **_kwargs: mock_gen(),
    )
    return mock_gen
