# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Root conftest for the Event Consumer Service conformance tests.

Configuration is driven entirely by environment variables so the same test
container can be aimed at any running service instance:

  CONFORMANCE_GRPC_ENDPOINT   - host:port for gRPC (default: localhost:50052)
  CONFORMANCE_REST_ENDPOINT   - base URL for REST (default: http://localhost:8000)
  CONFORMANCE_AUTH_TOKEN      - Bearer token to include (default: empty = no auth header)
  CONFORMANCE_AUTH_REQUIRED   - set to "true" to run auth-error tests
                                (requires the service to have auth enabled)
  CONFORMANCE_STREAM_TIMEOUT  - seconds to wait for the first SSE event (default: 10.0)
  CONFORMANCE_REQUEST_TIMEOUT - seconds for non-streaming HTTP requests (default: 30.0)
"""

import json
import os
import threading
from typing import Optional

import grpc
import httpx
import pytest
from nvidia.omniverse.notifications.consumer.v1beta import (
    event_consumer_pb2_grpc,
)

# ---------------------------------------------------------------------------
# Configuration (read once at import time so fixtures can reference them)
# ---------------------------------------------------------------------------

GRPC_ENDPOINT: str = os.environ.get("CONFORMANCE_GRPC_ENDPOINT", "localhost:50052")
REST_ENDPOINT: str = os.environ.get(
    "CONFORMANCE_REST_ENDPOINT", "http://localhost:8000"
)
AUTH_TOKEN: str = os.environ.get("CONFORMANCE_AUTH_TOKEN", "")
AUTH_REQUIRED: bool = (
    os.environ.get("CONFORMANCE_AUTH_REQUIRED", "false").lower() == "true"
)
STREAM_TIMEOUT: float = float(os.environ.get("CONFORMANCE_STREAM_TIMEOUT", "10.0"))
REQUEST_TIMEOUT: float = float(os.environ.get("CONFORMANCE_REQUEST_TIMEOUT", "30.0"))


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
def consumer_stub(grpc_channel):
    """gRPC stub for the EventConsumerService."""
    return event_consumer_pb2_grpc.EventConsumerServiceStub(grpc_channel)


@pytest.fixture(scope="session")
def grpc_metadata():
    """gRPC call metadata (auth header when a token is configured)."""
    if AUTH_TOKEN:
        return [("authorization", f"Bearer {AUTH_TOKEN}")]
    return []


# ---------------------------------------------------------------------------
# REST fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def http_client():
    """httpx Client for non-streaming REST requests."""
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


def assert_grpc_ok(call_fn):
    """Call a gRPC unary RPC and return the response. Fails with a clear message on error."""
    try:
        return call_fn()
    except grpc.RpcError as exc:
        pytest.fail(
            f"Expected gRPC call to succeed (OK) but got status "
            f"{exc.code().name}: {exc.details()}"
        )


def assert_grpc_error(
    call_fn, expected_code: grpc.StatusCode, detail_contains: Optional[str] = None
):
    """
    Call a gRPC RPC (unary or streaming) and assert that it fails with the expected
    status code. Returns the RpcError for further inspection.

    For streaming calls, pass a callable that iterates the stream:
        assert_grpc_error(lambda: list(stub.ConsumeX(req)), grpc.StatusCode.NOT_FOUND)
    """
    try:
        call_fn()
        pytest.fail(
            f"Expected gRPC call to fail with {expected_code.name} "
            f"but it succeeded."
        )
    except grpc.RpcError as exc:
        assert exc.code() == expected_code, (
            f"Expected gRPC status {expected_code.name} "
            f"but got {exc.code().name}.\n"
            f"Error details: {exc.details()}"
        )
        if detail_contains is not None:
            details = exc.details() or ""
            assert detail_contains in details, (
                f"Expected error details to contain '{detail_contains}' "
                f"but got: '{details}'"
            )
        return exc


def read_grpc_stream_with_timeout(response_iter, timeout: float = STREAM_TIMEOUT):
    """
    Drain a gRPC server-streaming call up to *timeout* seconds.
    Cancels the call after the timeout and returns collected messages.

    Returns (messages, error) where error is None if the stream closed cleanly
    or an RpcError if the stream ended with a gRPC error status.
    """
    messages = []
    error_holder = [None]
    done = threading.Event()

    def _read():
        try:
            for msg in response_iter:
                messages.append(msg)
        except grpc.RpcError as exc:
            error_holder[0] = exc
        except Exception as exc:
            error_holder[0] = exc
        finally:
            done.set()

    thread = threading.Thread(target=_read, daemon=True)
    thread.start()
    timed_out = not done.wait(timeout=timeout)

    if timed_out:
        try:
            response_iter.cancel()
        except Exception:
            pass

    return messages, error_holder[0]


# ---------------------------------------------------------------------------
# SSE helpers
# ---------------------------------------------------------------------------


def parse_sse_events(text: str) -> list[dict]:
    """Parse SSE text into a list of event dicts with 'event' and 'data' keys."""
    events = []
    current: dict = {}
    for line in text.split("\n"):
        line = line.rstrip("\r")
        if not line:
            if current:
                events.append(current)
                current = {}
            continue
        if line.startswith("event:"):
            current["event"] = line[6:].strip()
        elif line.startswith("data:"):
            raw = line[5:].strip()
            try:
                current["data"] = json.loads(raw)
            except json.JSONDecodeError:
                current["data"] = raw
    if current:
        events.append(current)
    return events


def read_sse_first_event(
    path: str,
    *,
    extra_headers: Optional[dict] = None,
    timeout: float = STREAM_TIMEOUT,
) -> tuple[int, str, Optional[dict]]:
    """
    Connect to an SSE endpoint and read the first complete event.

    Returns (status_code, content_type, first_event_or_body) where:
    - For non-200 responses, the third element is the JSON response body (dict).
    - For 200 responses, the third element is the parsed first SSE event dict,
      or None if the stream closed before sending any event.

    The httpx Timeout bounds both connect and read; once the first SSE event
    arrives we break out of the read loop and the context managers close the
    connection synchronously.
    """
    headers: dict = {}
    if AUTH_TOKEN:
        headers["Authorization"] = f"Bearer {AUTH_TOKEN}"
    if extra_headers:
        headers.update(extra_headers)

    url = f"{REST_ENDPOINT}{path}"

    try:
        with httpx.Client(timeout=httpx.Timeout(timeout, connect=10.0)) as client:
            with client.stream("GET", url, headers=headers) as response:
                status_code = response.status_code
                content_type = response.headers.get("content-type", "")

                if status_code != 200:
                    response.read()
                    try:
                        body: object = response.json()
                    except Exception:
                        body = {"raw": response.text}
                    return status_code, content_type, body

                # For 200, read until we have the first complete SSE event.
                # The service terminates events with either \n\n (bare LF) or
                # \r\n\r\n (CRLF, as used by many SSE implementations).
                buffer = ""
                try:
                    for chunk in response.iter_text():
                        buffer += chunk
                        if "\n\n" in buffer or "\r\n\r\n" in buffer:
                            break
                except httpx.ReadTimeout:
                    pass  # Timeout is OK — return whatever we accumulated

                events = parse_sse_events(buffer)
                return status_code, content_type, events[0] if events else None
    except httpx.ConnectError as exc:
        pytest.fail(
            f"Could not connect to {url}. Is the service running?\n"
            f"Set CONFORMANCE_REST_ENDPOINT to override. Error: {exc}"
        )
