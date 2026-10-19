# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging
import os
import random
import string
from functools import cache
from typing import Dict
from urllib.parse import urlparse, urlunparse

import pytest
from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2_grpc
from utils import (
    CONFLICTING_AUTH_MESSAGE,
    create_grpc_channel_from_url,
    get_access_token,
    has_access_token_file,
    has_client_credentials,
)

logging.basicConfig(
    level=logging.INFO, format="%(filename)s - %(levelname)s: %(message)s"
)
logger = logging.getLogger(__name__)


class ConfigurationError(RuntimeError):
    """Raised when required configuration is missing or invalid."""

    pass


@cache
def _get_notification_endpoints() -> Dict[str, str]:
    """Get notification service endpoints from environment variables.

    Returns:
        Dict with endpoint URLs keyed by service type

    Raises:
        ConfigurationError: If no endpoints are configured
    """
    env_to_key = {
        "SMOKE_TESTS_GRPC_AGGREGATION_ENDPOINT": "grpc_notification_aggregation",
        "SMOKE_TESTS_REST_AGGREGATION_ENDPOINT": "http_notification_aggregation",
    }

    endpoints = {}
    for env_var, key in env_to_key.items():
        value = os.getenv(env_var)
        if value:
            endpoint = _add_port(value.rstrip("/"))
            endpoints[key] = endpoint
            logger.info(f"Using {key}: {endpoint}")

    if not endpoints:
        raise ConfigurationError(
            "No notification endpoints configured. Set at least one of:\n"
            "  SMOKE_TESTS_GRPC_AGGREGATION_ENDPOINT\n"
            "  SMOKE_TESTS_REST_AGGREGATION_ENDPOINT"
        )

    logger.info(f"✅ Using {len(endpoints)} notification endpoint(s)")
    return endpoints


def pytest_configure(config):
    """Validate environment variables before running tests."""
    logger.info(f"\n{'=' * 70}")
    logger.info("🚀 PREFLIGHT CHECKS")
    logger.info(f"{'=' * 70}\n")

    if has_access_token_file() and has_client_credentials():
        logger.error(CONFLICTING_AUTH_MESSAGE)
        pytest.exit("Conflicting auth configuration", returncode=1)

    _get_notification_endpoints()

    logger.info(f"\n{'=' * 70}")
    logger.info("✅ All preflight checks passed!")
    logger.info(f"{'=' * 70}\n")


@pytest.fixture(scope="session")
def http_headers():
    """Get common headers for API requests."""
    token = get_access_token()
    if token:
        return {
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        }
    return {"Content-Type": "application/json"}


@pytest.fixture(scope="session")
def resource_base_url() -> str:
    """Generate a unique resource base URL for testing (in-test placeholder only)."""
    base_url = "https://test-storage/"
    random_suffix = "".join(random.choices(string.ascii_lowercase + string.digits, k=8))
    return f"{base_url}test-{random_suffix}/"


@pytest.fixture(scope="session")
def service_endpoints(http_headers):
    """Session-scoped fixture for service endpoints."""
    # http_headers is requested only for fixture ordering (token fetched first).
    return _get_notification_endpoints()


@pytest.fixture(scope="session")
def require_notification_rest_endpoint(service_endpoints):
    """Fixture that skips test if notification REST endpoint is not configured."""
    if not service_endpoints.get("http_notification_aggregation"):
        pytest.skip("Notification REST endpoint not configured")


# ============================================================================
# gRPC Fixtures
# ============================================================================


@pytest.fixture(scope="session")
def grpc_metadata():
    """Create gRPC metadata with authorization token."""
    token = get_access_token()
    return [("authorization", f"Bearer {token}")] if token else []


def _create_service_channel(service_endpoints: Dict[str, str], endpoint_key: str):
    """Helper to create a gRPC channel for a service."""
    grpc_endpoint = service_endpoints.get(endpoint_key)
    if not grpc_endpoint:
        pytest.skip(f"'{endpoint_key}' endpoint not configured")

    channel = create_grpc_channel_from_url(grpc_endpoint)
    yield channel
    channel.close()


@pytest.fixture(scope="session")
def notification_aggregation_grpc_channel(service_endpoints: Dict[str, str]):
    """Create and manage gRPC channel for notification aggregation service."""
    yield from _create_service_channel(
        service_endpoints, "grpc_notification_aggregation"
    )


@pytest.fixture(scope="session")
def notification_aggregation_grpc_stub(notification_aggregation_grpc_channel):
    """Create a gRPC EventPublishingService stub."""
    return event_publisher_pb2_grpc.EventPublishingServiceStub(
        notification_aggregation_grpc_channel
    )


def _add_port(endpoint: str) -> str:
    """Ensure endpoint has correct port based on scheme."""
    parsed = urlparse(endpoint)
    if parsed.hostname is None:
        if endpoint.startswith("grpc://"):
            base = endpoint.removeprefix("grpc://")
            return f"https://{base}" if parsed.port else f"https://{base}:443"
        return endpoint if parsed.port else f"{endpoint}:443"
    default_ports = {"http": 80, "https": 443, "grpc": 443}
    scheme = "https" if parsed.scheme == "grpc" else parsed.scheme or "https"
    port = (
        parsed.port
        if parsed.port is not None
        else default_ports.get(parsed.scheme, 443)
    )
    netloc = f"{parsed.hostname}:{port}"
    path = parsed.path if parsed.path else "/"
    return urlunparse(
        (scheme, netloc, path, parsed.params, parsed.query, parsed.fragment)
    )
