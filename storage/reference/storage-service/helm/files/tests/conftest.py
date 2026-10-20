# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

import logging
import os
import random
import string
import time
from functools import cache
from typing import Dict
from urllib.parse import urlparse

import pytest
import requests
from nvidia.omniverse.storage.capabilities.v1alpha import capabilities_pb2_grpc
from nvidia.omniverse.storage.filefolder.v1alpha import filefolder_service_pb2_grpc
from nvidia.omniverse.storage.fileobject.v1alpha import fileobject_service_pb2_grpc
from nvidia.omniverse.storage.metadata.v1alpha import metadata_pb2_grpc
from nvidia.omniverse.storage.versioning.v1alpha import versioning_pb2_grpc
from utils import (
    create_grpc_channel_from_url,
    get_access_token,
    get_capabilities_top_level_addresses_url,
    get_top_level_addresses_grpc,
    timeout_rest_request,
    write_file_grpc,
    write_file_rest,
)

# Configure logging
logging.basicConfig(
    level=logging.INFO, format="%(filename)s - %(levelname)s: %(message)s"
)
logger = logging.getLogger(__name__)


class ConfigurationError(RuntimeError):
    """Raised when required configuration is missing or invalid."""

    pass


@cache
def _get_storage_endpoints() -> Dict[str, str]:
    """Get storage service endpoints from environment variables.

    Returns:
        Dict with endpoint URLs keyed by 'grpc_storage' and/or 'http_storage'

    Raises:
        ConfigurationError: If no endpoints are configured
    """
    endpoints = {}

    grpc_endpoint = os.getenv("SMOKE_TESTS_GRPC_ENDPOINT")
    http_endpoint = os.getenv("SMOKE_TESTS_REST_ENDPOINT")

    if grpc_endpoint:
        endpoints["grpc_storage"] = _add_port(grpc_endpoint.rstrip("/"))
        logger.info(f"Using gRPC storage endpoint: {grpc_endpoint}")

    if http_endpoint:
        endpoints["http_storage"] = _add_port(http_endpoint.rstrip("/"))
        logger.info(f"Using HTTP storage endpoint: {http_endpoint}")

    if not endpoints:
        raise ConfigurationError(
            "No storage endpoints configured. Set SMOKE_TESTS_GRPC_ENDPOINT and/or SMOKE_TESTS_REST_ENDPOINT."
        )

    logger.info(f"✅ Using {len(endpoints)} storage endpoint(s)")
    return endpoints


# Global state to track which endpoints passed preflight checks
_working_endpoints: Dict[str, bool] = {"http_storage": False, "grpc_storage": False}


def get_working_endpoints() -> Dict[str, bool]:
    """Get which endpoints passed preflight checks."""
    return _working_endpoints.copy()


def pytest_configure(config):
    """Validate environment variables and check endpoint connectivity before running tests."""
    global _working_endpoints

    logger.info(f"\n{'=' * 70}")
    logger.info("🚀 PREFLIGHT CHECKS")
    logger.info(f"{'=' * 70}\n")

    try:
        # Get authentication headers for preflight checks
        preflight_headers = _get_preflight_headers()
        if preflight_headers:
            logger.info("🔑 Using authentication for preflight checks")
        else:
            logger.info("⚠️  No authentication configured for preflight checks")

        # Check 1: Storage Endpoints
        endpoints = _get_storage_endpoints()
        storage_http = endpoints.get("http_storage", "")
        storage_grpc = endpoints.get("grpc_storage", "")

        # Check 2: Resource Bases - check each endpoint independently
        resource_bases = get_resource_bases()
        if not resource_bases:
            raise ConfigurationError("Resource Base: Not configured")

        errors = []

        # Check REST endpoint
        if storage_http:
            try:
                _check_endpoint_resource_bases(
                    storage_http, None, preflight_headers, resource_bases, "REST"
                )
                _working_endpoints["http_storage"] = True
            except Exception as e:
                errors.append(f"REST: {e}")
                logger.warning(f"⚠️  REST endpoint check failed: {e}")

        # Check gRPC endpoint
        if storage_grpc:
            try:
                _check_endpoint_resource_bases(
                    None, storage_grpc, preflight_headers, resource_bases, "gRPC"
                )
                _working_endpoints["grpc_storage"] = True
            except Exception as e:
                errors.append(f"gRPC: {e}")
                logger.warning(f"⚠️  gRPC endpoint check failed: {e}")

        # Fail only if NO endpoints are working
        if not any(_working_endpoints.values()):
            raise ConfigurationError(
                f"All endpoint checks failed: {'; '.join(errors)}"
            )

        logger.info(f"\n{'=' * 70}")
        if all(_working_endpoints.values()):
            logger.info("✅ All preflight checks passed!")
        else:
            working = [k for k, v in _working_endpoints.items() if v]
            logger.info(f"✅ Preflight checks passed for: {', '.join(working)}")
        logger.info(f"{'=' * 70}\n")

    except Exception as e:
        error_type = type(e).__name__
        logger.error(f"❌ [{error_type}] {e}")
        pytest.exit("Preflight check failed", returncode=1)


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


def get_resource_bases():
    """Get list of resource bases from environment variable."""
    env_value = os.environ.get("SMOKE_TESTS_RESOURCE_BASE", "")
    if not env_value:
        return []
    return [base.strip() for base in env_value.split(",") if base.strip()]


def pytest_generate_tests(metafunc):
    """Parametrize tests that use resource_base_url fixture."""
    if "resource_base_url" in metafunc.fixturenames:
        bases = get_resource_bases()
        if bases:
            metafunc.parametrize("resource_base_url", bases, indirect=True)


@pytest.fixture(scope="session")
def resource_base_url(request) -> str:
    # Get base from parametrize or fall back to first base
    if hasattr(request, "param"):
        base_url = request.param
    else:
        bases = get_resource_bases()
        if not bases:
            raise ConfigurationError(
                "SMOKE_TESTS_RESOURCE_BASE environment variable is not set"
            )
        base_url = bases[0]

    # Ensure trailing slash
    if not base_url.endswith("/"):
        base_url += "/"

    random_suffix = "".join(random.choices(string.ascii_lowercase + string.digits, k=8))
    return f"{base_url}test-{random_suffix}/"


@pytest.fixture
def resource_address(resource_base_url: str) -> str:
    random_suffix = "".join(random.choices(string.ascii_lowercase + string.digits, k=8))
    return f"{resource_base_url}file-{random_suffix}"


@pytest.fixture(scope="session")
def service_endpoints(http_headers):
    """Session-scoped fixture for service endpoints.

    Returns storage endpoints configured via environment variables.
    """
    return _get_storage_endpoints()


@pytest.fixture(scope="session")
def require_storage_rest_endpoint(service_endpoints):
    """Fixture that skips test if REST not configured, or fails if preflight failed."""
    if not service_endpoints.get("http_storage"):
        pytest.skip("SMOKE_TESTS_REST_ENDPOINT not configured")
    if not get_working_endpoints().get("http_storage"):
        pytest.fail("REST endpoint failed preflight check")


@pytest.fixture(scope="session")
def require_storage_grpc_endpoint(service_endpoints):
    """Fixture that skips test if gRPC not configured, or fails if preflight failed."""
    if not service_endpoints.get("grpc_storage"):
        pytest.skip("SMOKE_TESTS_GRPC_ENDPOINT not configured")
    if not get_working_endpoints().get("grpc_storage"):
        pytest.fail("gRPC endpoint failed preflight check")


# ============================================================================
# gRPC Fixtures
# ============================================================================


@pytest.fixture(scope="session")
def storage_grpc_channel(service_endpoints: Dict[str, str]):
    """Create and manage gRPC channel for storage service."""
    yield from _create_service_channel(service_endpoints, "grpc_storage")


@pytest.fixture(scope="session")
def fileobject_grpc_stub(storage_grpc_channel):
    """Create a gRPC FileObjectService stub."""
    return fileobject_service_pb2_grpc.FileObjectServiceStub(storage_grpc_channel)


@pytest.fixture(scope="session")
def versioning_grpc_stub(storage_grpc_channel):
    """Create a gRPC stub for VersioningService."""
    return versioning_pb2_grpc.VersioningServiceStub(storage_grpc_channel)


@pytest.fixture(scope="session")
def filefolder_grpc_stub(storage_grpc_channel):
    """Create a gRPC stub for FileFolderService."""
    return filefolder_service_pb2_grpc.FileFolderServiceStub(storage_grpc_channel)


@pytest.fixture(scope="session")
def metadata_grpc_stub(storage_grpc_channel):
    """Create a gRPC stub for MetadataService."""
    return metadata_pb2_grpc.MetadataServiceStub(storage_grpc_channel)


@pytest.fixture(scope="session")
def capabilities_grpc_stub(storage_grpc_channel):
    """Create a gRPC stub for CapabilitiesService."""
    return capabilities_pb2_grpc.CapabilitiesServiceStub(storage_grpc_channel)


@pytest.fixture(scope="session")
def grpc_metadata():
    """Create gRPC metadata with authorization token."""
    token = get_access_token()
    return [("authorization", f"Bearer {token}")] if token else []


@pytest.fixture(scope="session")
def available_services_grpc(capabilities_grpc_stub, grpc_metadata):
    """Get list of available service names from gRPC ListServices."""
    from storage_grpc_helpers import run_list_services_test_grpc

    response = run_list_services_test_grpc(capabilities_grpc_stub, grpc_metadata)
    return [s.service_name for s in response.services]


@pytest.fixture(scope="session")
def available_services_rest(http_headers, service_endpoints):
    """Get list of available service names from REST ListServices."""
    from storage_rest_helpers import run_list_services_test_rest

    if not service_endpoints.get("http_storage"):
        return []
    services = run_list_services_test_rest(http_headers, service_endpoints)
    return [s.get("service_name", "") for s in services]


@pytest.fixture
def require_metadata_grpc(available_services_grpc):
    """Skip test if MetadataService is not available via gRPC."""
    if "metadata" not in available_services_grpc:
        pytest.skip()


@pytest.fixture
def require_metadata_rest(available_services_rest):
    """Skip test if MetadataService is not available via REST."""
    if "metadata" not in available_services_rest:
        pytest.skip()


def _create_service_channel(service_endpoints: Dict[str, str], endpoint_key: str):
    grpc_endpoint = service_endpoints.get(endpoint_key)
    if not grpc_endpoint:
        pytest.skip(f"'{endpoint_key}' endpoint not configured")

    if not get_working_endpoints().get(endpoint_key):
        pytest.fail(f"'{endpoint_key}' endpoint failed preflight check")

    channel = create_grpc_channel_from_url(grpc_endpoint)

    yield channel
    channel.close()


# ============================================================================
# Version Testing Fixtures
# ============================================================================


@pytest.fixture
def create_multiple_versions_rest(http_headers, service_endpoints):
    """Fixture that creates multiple versions of a file via REST."""

    def _create_versions(resource_address, num_versions=3):
        """Create multiple versions of a file.

        Args:
            resource_address: The file address to create versions for
            num_versions: Number of additional versions to create (default: 3)

        Returns:
            int: Total number of versions created
        """
        for i in range(num_versions):
            content = f"version {i + 1} content".encode()
            write_file_rest(resource_address, content, http_headers, service_endpoints)
            logger.info(f"Created version {i + 1}/{num_versions}")
        return num_versions

    return _create_versions


@pytest.fixture
def create_multiple_versions_grpc(fileobject_grpc_stub, grpc_metadata):
    """Fixture that creates multiple versions of a file via gRPC."""

    def _create_versions(resource_address, num_versions=3):
        """Create multiple versions of a file.

        Args:
            resource_address: The file address to create versions for
            num_versions: Number of additional versions to create (default: 3)

        Returns:
            int: Total number of versions created
        """
        for i in range(num_versions):
            content = f"version {i + 1} content".encode()
            write_file_grpc(
                resource_address, content, fileobject_grpc_stub, grpc_metadata
            )
            logger.info(f"Created version {i + 1}/{num_versions}")
        return num_versions

    return _create_versions


# =============================================================================
# Preflight Check Helpers
# =============================================================================
# These functions are used by pytest_configure() to validate environment
# and connectivity before running tests:
#   - _add_port(): Normalize endpoint URLs with correct ports
#   - _get_preflight_headers(): Get auth headers for preflight requests
#   - _check_resource_bases_in_top_level_addresses(): Verify resource bases exist
# =============================================================================


def _add_port(endpoint: str) -> str:
    """Ensure endpoint has correct port based on scheme."""
    parsed = urlparse(endpoint)
    if endpoint.startswith("http://"):
        return endpoint if parsed.port else f"{endpoint}:80"
    elif endpoint.startswith("grpc://"):
        base = endpoint.removeprefix("grpc://")
        return f"https://{base}" if parsed.port else f"https://{base}:443"
    elif endpoint.startswith("https://"):
        return endpoint if parsed.port else f"{endpoint}:443"
    else:
        return f"https://{endpoint}" if parsed.port else f"https://{endpoint}:443"


def _get_preflight_headers() -> dict:
    """Get authentication headers for preflight checks."""
    token = get_access_token()
    if token:
        return {"Authorization": f"Bearer {token}"}
    return {}


def _get_top_level_addresses_rest(storage_http: str, headers: dict) -> list[str]:
    """Get top-level addresses via REST API."""
    url = get_capabilities_top_level_addresses_url(storage_http)
    req_headers = {"accept": "application/json", **headers}
    response = timeout_rest_request(requests.get, url, headers=req_headers)
    response.raise_for_status()
    data = response.json()
    items = data.get("items", [])
    return [item.get("top_level_address", "") for item in items]


def _get_top_level_addresses(
    storage_http: str, storage_grpc: str, headers: dict
) -> list[str]:
    """Get top-level addresses from storage service.

    Tries REST first, falls back to gRPC if REST is not available.
    """
    if storage_http:
        return _get_top_level_addresses_rest(storage_http, headers)

    if storage_grpc:
        grpc_metadata = (
            [(k.lower(), v) for k, v in headers.items()] if headers else []
        )
        return get_top_level_addresses_grpc(storage_grpc, grpc_metadata)

    raise ConfigurationError("No storage endpoints configured")


def _check_endpoint_resource_bases(
    storage_http: str | None,
    storage_grpc: str | None,
    headers: dict,
    resource_bases: list[str],
    endpoint_name: str,
) -> None:
    """Check if resource bases exist via a specific endpoint.

    Raises:
        ConfigurationError: If resource bases cannot be verified via this endpoint
    """
    top_level_addresses = _get_top_level_addresses(
        storage_http or "", storage_grpc or "", headers
    )

    # Check each resource base
    missing_bases = []
    for resource_base in resource_bases:
        resource_base_normalized = resource_base.rstrip("/")
        found = False
        for addr in top_level_addresses:
            addr_normalized = addr.rstrip("/")
            if (
                resource_base_normalized == addr_normalized
                or resource_base_normalized.startswith(addr_normalized + "/")
            ):
                logger.info(
                    f"✅ [{endpoint_name}] Resource Base: '{resource_base}' found ({addr})"
                )
                found = True
                break
        if not found:
            missing_bases.append(resource_base)

    if missing_bases:
        available = ", ".join(top_level_addresses)
        raise ConfigurationError(
            f"Resource bases not found: {missing_bases}. Available: {available}"
        )
