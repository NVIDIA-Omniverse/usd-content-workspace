# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Root conftest for the Event Aggregation Service conformance tests.

Configuration is driven entirely by environment variables so the same test
container can be aimed at any running service instance:

  CONFORMANCE_GRPC_ENDPOINT   - host:port for gRPC (default: localhost:50051)
  CONFORMANCE_REST_ENDPOINT   - base URL for REST (default: http://localhost:8001)
  CONFORMANCE_AUTH_TOKEN      - Bearer token to include (default: empty = no auth header)
  CONFORMANCE_AUTH_REQUIRED   - set to "true" to run auth-error tests
                                (requires the service to have auth enabled)
  CONFORMANCE_REQUEST_TIMEOUT - seconds for HTTP requests (default: 30.0)
"""

import json
import os
import time

import grpc
import httpx
import pytest
from google.protobuf.struct_pb2 import Struct
from google.protobuf.timestamp_pb2 import Timestamp
from nvidia.omniverse.notifications.publisher.v1beta import (
    event_publisher_pb2,
    event_publisher_pb2_grpc,
)

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

GRPC_ENDPOINT: str = os.environ.get("CONFORMANCE_GRPC_ENDPOINT", "localhost:50051")
REST_ENDPOINT: str = os.environ.get(
    "CONFORMANCE_REST_ENDPOINT", "http://localhost:8001"
)
AUTH_TOKEN: str = os.environ.get("CONFORMANCE_AUTH_TOKEN", "")
AUTH_REQUIRED: bool = (
    os.environ.get("CONFORMANCE_AUTH_REQUIRED", "false").lower() == "true"
)
REQUEST_TIMEOUT: float = float(os.environ.get("CONFORMANCE_REQUEST_TIMEOUT", "30.0"))


# Request limits the service documents (see the reimplementor guide): the
# batch size, the per-event string lengths, and the message size.
MAX_BATCH_EVENTS = 1000
MAX_EVENT_TYPE_LENGTH = 256
MAX_RESOURCE_ID_LENGTH = 2048
MAX_EVENT_MESSAGE_BYTES = 65536


def message_of_size(size: int) -> dict:
    """Return a message whose JSON encoding is exactly ``size`` bytes.

    Measured as the service measures it (Python ``json.dumps`` defaults); the
    padding is ASCII, so the encoding is the same on REST and gRPC.
    """
    return {"k": "x" * (size - len(json.dumps({"k": ""})))}


# ---------------------------------------------------------------------------
# gRPC fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def grpc_channel():
    """Insecure gRPC channel to the service under test."""
    channel = grpc.insecure_channel(GRPC_ENDPOINT)
    yield channel
    channel.close()


@pytest.fixture(scope="session")
def publisher_stub(grpc_channel):
    """gRPC stub for EventPublishingService."""
    return event_publisher_pb2_grpc.EventPublishingServiceStub(grpc_channel)


@pytest.fixture(scope="session")
def grpc_metadata():
    """gRPC call metadata (auth header when a token is configured)."""
    if AUTH_TOKEN:
        return [("authorization", f"Bearer {AUTH_TOKEN}")]
    return []


@pytest.fixture(scope="session")
def pb2():
    """Protobuf message classes for the Event Publisher API."""
    return event_publisher_pb2


# ---------------------------------------------------------------------------
# REST fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def http_client():
    """httpx Client for REST requests."""
    headers = {}
    if AUTH_TOKEN:
        headers["Authorization"] = f"Bearer {AUTH_TOKEN}"
    with httpx.Client(
        base_url=REST_ENDPOINT,
        headers=headers,
        timeout=REQUEST_TIMEOUT,
    ) as client:
        yield client


@pytest.fixture(scope="session")
def rest_base_url():
    return REST_ENDPOINT


# ---------------------------------------------------------------------------
# gRPC helpers
# ---------------------------------------------------------------------------


def make_timestamp() -> Timestamp:
    """Return a Timestamp for the current time (seconds > 0)."""
    return Timestamp(seconds=int(time.time()))


def make_event_proto(pb2_module, *, with_resource: bool = True):
    """
    Build a minimal valid Event proto suitable for PublishEvent tests.

    All three required fields (event_type, message, occurred_at) are set.
    """
    msg = Struct()
    msg.update({"conformance": "test", "index": 1})

    event = pb2_module.Event(
        event_type="conformance.test.event",
        occurred_at=make_timestamp(),
        message=msg,
    )
    if with_resource:
        event.resource.CopyFrom(
            pb2_module.EventResource(resource_id="conformance/test/resource/")
        )
    return event


# ---------------------------------------------------------------------------
# REST helpers
# ---------------------------------------------------------------------------


def make_rest_event(*, with_resource: bool = True) -> dict:
    """Return a minimal valid REST event payload dict."""
    from datetime import datetime, timezone

    occurred_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    event: dict = {
        "event_type": "conformance.test.event",
        "message": {"conformance": "test"},
        "occurred_at": occurred_at,
    }
    if with_resource:
        event["resource"] = {"resource_id": "conformance/test/resource/"}
    return event


def expect_invalid_argument(call, *, what: str, field: str) -> None:
    """Assert that ``call()`` fails with INVALID_ARGUMENT naming ``field``.

    ``what`` describes the request for the failure messages.
    """
    try:
        call()
    except grpc.RpcError as exc:
        assert exc.code() == grpc.StatusCode.INVALID_ARGUMENT, (
            f"Expected INVALID_ARGUMENT for {what}, "
            f"but got {exc.code().name}. Details: {exc.details()}"
        )
        assert field in (exc.details() or ""), (
            f"Expected error details for {what} to name '{field}', "
            f"but got: '{exc.details()}'"
        )
    else:
        pytest.fail(
            f"Expected a gRPC error (INVALID_ARGUMENT) for {what}, "
            f"but the call succeeded."
        )
