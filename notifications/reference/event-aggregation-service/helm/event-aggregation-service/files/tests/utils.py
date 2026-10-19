# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Common utility functions for Event Aggregation Service smoke tests."""

import base64
import json
import logging
import os
import time

import grpc
import requests

logger = logging.getLogger(__name__)


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
    """Create gRPC channel from URL with automatic TLS detection.

    Args:
        grpc_url: Full URL with scheme (e.g. "https://example.com:443")

    Returns:
        gRPC channel (caller is responsible for closing it)
    """
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
    if (
        os.environ.get("SMOKE_TESTS_REST_SKIP_TLS", "false").lower() == "true"
        and "verify" not in kwargs
    ):
        kwargs["verify"] = False

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
