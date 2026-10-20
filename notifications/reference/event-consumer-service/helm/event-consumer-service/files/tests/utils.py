# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Common utility functions for Event Consumer Service smoke tests."""

import base64
import json
import logging
import os
import time
import uuid

import grpc
import requests

logger = logging.getLogger(__name__)


def generate_queue_id() -> str:
    """Build a unique, client-supplied durable queue id for a smoke test.

    CreateDurableQueue is retried on transport failure, and a retry that
    lets the server pick the id would orphan a queue whenever the first
    attempt actually succeeded but its response was lost. Supplying an id
    makes the call ensure-exists, so every attempt after the first is a
    no-op that returns the same queue.

    The id is namespaced rather than a bare uuid because queue ids share
    one deployment-wide namespace, and it matches the server's
    ``^[A-Za-z0-9._-]{1,200}$`` constraint.
    """
    return f"smoke-test-{uuid.uuid4()}"


def _should_skip_tls() -> bool:
    """Check if TLS should be skipped for gRPC based on environment variable."""
    return os.environ.get("SMOKE_TESTS_GRPC_SKIP_TLS", "false").lower() == "true"


def _create_grpc_channel(endpoint: str, skip_tls: bool):
    """Create gRPC channel with or without TLS."""
    if skip_tls:
        return grpc.insecure_channel(endpoint)
    credentials = grpc.ssl_channel_credentials()
    return grpc.secure_channel(endpoint, credentials)


def create_grpc_channel_from_url(grpc_url: str):
    """Create gRPC channel from URL with automatic TLS detection."""
    skip_tls = _should_skip_tls()
    if not skip_tls:
        if grpc_url.startswith("https://"):
            skip_tls = False
        elif grpc_url.startswith("http://"):
            skip_tls = True
        else:
            raise ValueError(f"Invalid endpoint URL: {grpc_url}")
    clean_endpoint = grpc_url.removeprefix("https://").removeprefix("http://")
    return _create_grpc_channel(clean_endpoint, skip_tls)


def timeout_rest_request(request_func, *args, **kwargs):
    """Execute a REST request with timeout."""
    timeout_seconds = float(os.environ.get("SMOKE_TESTS_TIMEOUT", 5))
    if "timeout" not in kwargs:
        kwargs["timeout"] = timeout_seconds
    return request_func(*args, **kwargs)


def timeout_grpc_call(grpc_func, *args, **kwargs):
    """Execute a gRPC call with timeout."""
    timeout_seconds = float(os.environ.get("SMOKE_TESTS_TIMEOUT", 5))
    if "timeout" not in kwargs:
        kwargs["timeout"] = timeout_seconds
    return grpc_func(*args, **kwargs)


def assert_response(response, expected_status, operation_name):
    """Helper for consistent response assertions."""
    actual_status = response.status_code
    if isinstance(expected_status, list):
        status_ok = actual_status in expected_status
    else:
        status_ok = actual_status == expected_status
    assert (
        status_ok
    ), f"{operation_name}: expected {expected_status}, got {actual_status}"


_TOKEN_FILE_WAIT_SECONDS = 30.0
_TOKEN_FILE_POLL_INTERVAL = 1.0

CONFLICTING_AUTH_MESSAGE = (
    "Conflicting auth: both SMOKE_TESTS_ACCESS_TOKEN_FILE_PATH and "
    "SMOKE_TESTS_AUTH_* are set. Configure exactly one auth mode."
)


def has_access_token_file() -> bool:
    """Return True if the access-token-file auth mode is configured."""
    return bool(os.environ.get("SMOKE_TESTS_ACCESS_TOKEN_FILE_PATH"))


def has_client_credentials() -> bool:
    """Return True if the client-credentials OAuth auth mode is configured."""
    return bool(
        os.environ.get("SMOKE_TESTS_AUTH_MICROSOFT_URL")
        and os.environ.get("SMOKE_TESTS_AUTH_CREDENTIALS", "").strip()
        and os.environ.get("SMOKE_TESTS_AUTH_SCOPE")
    )


def _read_access_token_from_file(path: str) -> str:
    """Read the access token from the sidecar-written JSON file."""
    deadline = time.monotonic() + _TOKEN_FILE_WAIT_SECONDS
    last_error: Exception | None = None
    while True:
        try:
            with open(path) as f:
                parsed = json.load(f)
            access_token = parsed["access_token"]
            if not isinstance(access_token, str) or not access_token:
                raise ValueError("access_token missing or not a non-empty string")
            return access_token
        except (OSError, KeyError, ValueError, TypeError) as e:
            last_error = e
            if time.monotonic() >= deadline:
                raise RuntimeError(
                    f"SMOKE_TESTS_ACCESS_TOKEN_FILE_PATH is set ({path}) but the "
                    f"token file could not be read within "
                    f"{_TOKEN_FILE_WAIT_SECONDS:.0f}s"
                ) from last_error
            logger.info("Waiting for token file %s to become readable...", path)
            time.sleep(_TOKEN_FILE_POLL_INTERVAL)


def get_access_token():
    token_file = os.environ.get("SMOKE_TESTS_ACCESS_TOKEN_FILE_PATH")

    if has_access_token_file() and has_client_credentials():
        raise RuntimeError(CONFLICTING_AUTH_MESSAGE)

    if token_file:
        return _read_access_token_from_file(token_file)

    if not has_client_credentials():
        logger.warning("Missing required OAuth environment variables")
        return None
    microsoft_url = os.environ.get("SMOKE_TESTS_AUTH_MICROSOFT_URL")
    login_password = os.environ.get("SMOKE_TESTS_AUTH_CREDENTIALS", "").strip()
    scope = os.environ.get("SMOKE_TESTS_AUTH_SCOPE")
    response = requests.get(microsoft_url)
    response.raise_for_status()
    token_endpoint = response.json()["token_endpoint"]
    auth_header = base64.b64encode(login_password.encode("utf-8")).decode("utf-8")
    headers = {
        "Content-Type": "application/x-www-form-urlencoded",
        "Authorization": f"Basic {auth_header}",
    }
    data = f"grant_type=client_credentials&scope={scope}"
    token_response = requests.post(token_endpoint, headers=headers, data=data)
    token_response.raise_for_status()
    return token_response.json()["access_token"]


# Wall-clock budget for the REST retry helper below, mirroring the gRPC
# side's _UNAVAILABLE_RETRY_SECONDS. The bound is wall-clock rather than
# attempt-count so that one persistently failing call costs about a minute
# whatever SMOKE_TESTS_TIMEOUT is set to: these helpers run inside a Helm
# hook Job with backoffLimit 0, and an attempt-count bound at a raised
# request timeout would overrun the release timeout.
_REST_RETRY_SECONDS = 60
_REST_RETRY_DELAY = 2

# Exceptions worth inspecting for a retry. Kept as a module-level tuple so
# the except clause names one symbol (Black 25 on Python 3.14 strips the
# parentheses from an inline tuple, see issues ticket b.8qg).
_RETRYABLE_REST_EXC = (
    requests.exceptions.ConnectionError,
    requests.exceptions.HTTPError,
    AssertionError,
)


def _is_retryable_rest_error(error) -> bool:
    """Return True for an exception retry_on_503_rest should retry."""
    if isinstance(error, requests.exceptions.ConnectionError):
        return True
    if isinstance(error, requests.exceptions.HTTPError):
        status_code = getattr(getattr(error, "response", None), "status_code", None)
        return status_code == 503
    if isinstance(error, AssertionError):
        return "503" in str(error) or "Service Unavailable" in str(error)
    return False


def retry_on_503_rest(rest_func, *args, **kwargs):
    """Retry a REST call that failed with 503 or a connection failure.

    Tolerates exactly two things, both of which occur transiently while a
    rolling upgrade settles: an HTTP 503 (the proxy has no healthy pod to
    give the request to yet) and a ``requests`` ConnectionError (the proxy
    still routed to a pod whose servers have already stopped). Anything
    else -- any other status, and any other exception -- propagates on the
    first attempt. A bare 5xx in particular is NOT retried, because the
    application may already have processed the request.

    Retrying is safe here because every caller is idempotent, not because
    the request is known to be undelivered. A ConnectionError also wraps
    urllib3's ProtocolError / http.client's RemoteDisconnected, which is
    what a peer closing a pooled keep-alive connection after the request
    bytes were written looks like, so a retried request may in fact have
    been processed. The callers tolerate that: create supplies its own
    queue_id and is therefore ensure-exists, update resolves to RabbitMQ
    bind / unbind calls that are idempotent, and the SSE connect is
    read-only. Do not route a non-idempotent call through this helper.

    Note which timeouts are covered: requests' ConnectTimeout subclasses
    both ConnectionError and Timeout, so a timed-out connect IS retried; a
    read Timeout is only a Timeout and is raised immediately, since the
    request had already been written when it expired.

    The retry budget is wall-clock -- about ``_REST_RETRY_SECONDS`` in
    total, shared by every caller regardless of SMOKE_TESTS_TIMEOUT -- and
    exhausting it raises an AssertionError naming the budget and the last
    failure.
    """
    deadline = time.monotonic() + _REST_RETRY_SECONDS
    last_error = None
    while True:
        try:
            response = rest_func(*args, **kwargs)
            if getattr(response, "status_code", None) != 503:
                return response
            last_error = AssertionError(f"HTTP 503: {response.text}")
        except _RETRYABLE_REST_EXC as e:
            if not _is_retryable_rest_error(e):
                raise
            last_error = e
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise AssertionError(
                f"Consumer REST call did not succeed within the "
                f"{_REST_RETRY_SECONDS}s retry budget. Last failure: {last_error}"
            ) from last_error
        logger.info(
            "Consumer REST call failed with a retryable error (%s). "
            "Retrying, %.0fs of the %ss budget left...",
            last_error,
            remaining,
            _REST_RETRY_SECONDS,
        )
        time.sleep(min(_REST_RETRY_DELAY, remaining))
