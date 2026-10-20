# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging
import os
import random
import string

import pytest
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


def _get_rest_consumer_endpoint() -> str:
    """Get Event Consumer Service REST endpoint from environment."""
    endpoint = os.getenv("SMOKE_TESTS_REST_CONSUMER_ENDPOINT", "").strip().rstrip("/")
    if not endpoint:
        raise ConfigurationError(
            "Event Consumer REST endpoint not configured. "
            "Set SMOKE_TESTS_REST_CONSUMER_ENDPOINT (e.g. http://localhost:8000)."
        )
    logger.info("Using REST consumer endpoint: %s", endpoint)
    return endpoint


def _get_grpc_consumer_endpoint():
    """Get optional Event Consumer gRPC endpoint from environment."""
    endpoint = os.getenv("SMOKE_TESTS_GRPC_CONSUMER_ENDPOINT", "").strip().rstrip("/")
    if endpoint:
        logger.info("Using gRPC consumer endpoint: %s", endpoint)
    return endpoint if endpoint else None


def pytest_configure(config):
    """Validate environment variables before running tests."""
    logger.info("\n%s", "=" * 70)
    logger.info("PREFLIGHT CHECKS")
    logger.info("%s\n", "=" * 70)

    if has_access_token_file() and has_client_credentials():
        logger.error(CONFLICTING_AUTH_MESSAGE)
        pytest.exit("Conflicting auth configuration", returncode=1)

    _get_rest_consumer_endpoint()

    logger.info("\n%s", "=" * 70)
    logger.info("All preflight checks passed!")
    logger.info("%s\n", "=" * 70)


@pytest.fixture(scope="session")
def rest_consumer_endpoint() -> str:
    """Session-scoped fixture for the Event Consumer REST base URL."""
    return _get_rest_consumer_endpoint()


@pytest.fixture(scope="session")
def http_headers():
    """Get common headers for API requests (with optional Bearer token)."""
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
    base_url = "https://test-resource/"
    random_suffix = "".join(random.choices(string.ascii_lowercase + string.digits, k=8))
    return f"{base_url}test-{random_suffix}/"


@pytest.fixture(scope="session")
def grpc_metadata():
    """Create gRPC metadata with optional Bearer token."""
    token = get_access_token()
    return [("authorization", f"Bearer {token}")] if token else []


@pytest.fixture(scope="session")
def consumer_grpc_channel():
    """Create and manage gRPC channel for Event Consumer Service."""
    endpoint = _get_grpc_consumer_endpoint()
    if not endpoint:
        pytest.skip("SMOKE_TESTS_GRPC_CONSUMER_ENDPOINT not set")
    channel = create_grpc_channel_from_url(endpoint)
    yield channel
    channel.close()


@pytest.fixture(scope="session")
def consumer_grpc_stub(consumer_grpc_channel):
    """Create EventConsumerService gRPC stub."""
    from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2_grpc

    return event_consumer_pb2_grpc.EventConsumerServiceStub(consumer_grpc_channel)
