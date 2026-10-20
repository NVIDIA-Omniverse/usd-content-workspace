# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Common fixtures and helpers for REST integration tests.

The client, the raw-body poster, the auth headers and the sample event live one
level up, in ``tests/integration_tests/conftest.py``, because the
entry-point tests there exercise the REST and the gRPC surface from a single
table and need the same fixtures.
"""

import json
from datetime import datetime, timezone

import pytest

from app.config.settings import settings
from app.lib.rabbitmq import publisher


@pytest.fixture
def sample_event_without_resource():
    """Return a sample event without a resource for testing."""
    return {
        "event_type": "test.event",
        "message": {"key": "value"},
        "occurred_at": datetime.now(timezone.utc).isoformat(),
    }


@pytest.fixture
def max_request_bytes():
    """Return the request-body cap the REST app is configured with."""
    return settings.rest_max_request_bytes


@pytest.fixture
def sized_json_body(sample_event):
    """Return a builder for a publish request body of an exact byte length.

    The cap is enforced on the raw request bytes, so the boundary rows need a
    body of a known exact length rather than whatever a serialized fixture
    happens to weigh. The envelope key selects the single-event or the batch
    shape.

    The body is a valid publish request followed by JSON whitespace up to the
    requested length. Padding with whitespace rather than inside the event
    keeps every field within its own limits at any body size, so a body at the
    cap is refused by the cap or published, never rejected as invalid.
    """

    def _build(envelope_key: str, size: int) -> bytes:
        if envelope_key == "events":
            payload = {"events": [sample_event]}
        else:
            payload = {"event": sample_event}
        body = json.dumps(payload).encode()
        assert size >= len(body), "the requested size is smaller than the request"
        return body + b" " * (size - len(body))

    return _build


@pytest.fixture(autouse=True)
def setup_publisher_mocks(mocker):
    """Setup common publisher mocks."""
    mocker.patch.object(publisher, "publish")
    mocker.patch.object(publisher, "batch_publish")
