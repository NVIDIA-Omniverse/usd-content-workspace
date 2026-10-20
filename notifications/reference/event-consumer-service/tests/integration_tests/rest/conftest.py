# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Common fixtures and helpers for REST integration tests."""

import json
from typing import List, Dict, Any

import pytest
from fastapi.testclient import TestClient

from app.lib.models.filter_group import FilterGroup
from app.lib.models.filter_type import FilterType
from app.lib.models.resource_filter import ResourceFilter
from app.rest.rest_server import rest_server


def parse_sse_response(content: str) -> List[Dict[str, Any]]:
    """
    Parse Server-Sent Events response into a list of event objects.
    """
    events = []
    current_event = {}

    # Process the content line by line for more reliable parsing
    for line in content.splitlines():
        line = line.strip()

        # Empty line indicates the end of an event
        if not line and current_event:
            events.append(current_event)
            current_event = {}
            continue

        # Skip empty lines otherwise
        if not line:
            continue

        # Parse event field
        if line.startswith("event:"):
            current_event["event"] = line.replace("event:", "").strip()

        # Parse data field
        elif line.startswith("data:"):
            data_str = line.replace("data:", "").strip()
            try:
                # Handle Python-style quotes
                current_event["data"] = json.loads(data_str.replace("'", '"'))
            except json.JSONDecodeError:
                current_event["data"] = data_str

    # Add the last event if there is one
    if current_event:
        events.append(current_event)

    return events


async def rabbitmq_event_stream_test(mock_event, stream_response):
    content_str = stream_response.content.decode("utf-8")
    events = parse_sse_response(content_str)
    # Verify we have at least 2 events
    assert len(events) >= 2, f"Expected at least 2 events, got: {events}"
    # The second event should be our test event
    test_event_response = events[1]
    assert test_event_response.get("event") == "event"
    # Verify the test event data
    event_data = test_event_response.get("data", {})
    assert event_data.get("event_type") == mock_event.get("event_type")
    assert event_data.get("message") == mock_event.get("message")
    assert event_data.get("occurred_at") == mock_event.get("occurred_at")
    assert event_data.get("published_at") == mock_event.get("published_at")


async def stream_endpoint_test(
    client, mock_event, setup_mock_rabbitmq, stream_endpoint_url_v1beta
):
    setup_mock_rabbitmq([mock_event])
    response = client.get(
        stream_endpoint_url_v1beta, headers={"Authorization": "Bearer test-token"}
    )
    assert response.status_code == 200
    assert response.headers.get("content-type", "").startswith("text/event-stream")
    content_str = response.content.decode("utf-8")
    events = parse_sse_response(content_str)
    # Verify we have the connected event
    assert len(events) >= 1
    connected_event = events[0]
    assert connected_event.get("event") == "connected"


def serialize_filter_groups(filter_groups):
    """Convert filter groups to JSON-serializable format."""
    serializable_groups = []

    for fg in filter_groups:
        serializable_filters = []
        for f in fg.filters:
            serializable_filters.append(
                {
                    "filter_type": f.filter_type.value,
                    "resource_id": f.resource_id,
                }
            )
        serializable_groups.append(
            {"event_type": fg.event_type, "filters": serializable_filters}
        )

    return serializable_groups


@pytest.fixture
def client():
    """Return a FastAPI test client."""
    return TestClient(rest_server)


@pytest.fixture
def valid_filter_group():
    """Create a valid filter group for testing."""
    return FilterGroup(
        event_type="test_event",
        filters=(
            ResourceFilter(filter_type=FilterType.EQ, resource_id="test_resource"),
        ),
    )


@pytest.fixture
def valid_filter_groups(valid_filter_group):
    """Return a list with a valid filter group."""
    return [valid_filter_group]


@pytest.fixture
def create_queue_payload(valid_filter_groups):
    """Create a valid payload for creating a durable queue."""
    return {"filter_groups": serialize_filter_groups(valid_filter_groups)}


@pytest.fixture
def valid_filter_groups_json(valid_filter_groups):
    """Create JSON representation of filter groups for HTTP request parameters."""
    return json.dumps(serialize_filter_groups(valid_filter_groups))


@pytest.fixture(autouse=True)
def reset_sse_starlette_event():
    """
    Reset the sse_starlette AppStatus.should_exit_event before each test.
    This prevents 'RuntimeError: is bound to a different event loop' when
    running tests in sequence with different event loops.

    https://github.com/sysid/sse-starlette/issues/59
    """
    try:
        from sse_starlette.sse import AppStatus

        AppStatus.should_exit_event = None
    except ImportError:
        pass
