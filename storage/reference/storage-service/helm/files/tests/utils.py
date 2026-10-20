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

import base64
import json
import logging
import os
import time
import urllib.parse
from functools import wraps
from queue import Queue

import grpc
import pytest
import requests
from nvidia.omniverse.storage.capabilities.v1alpha.capabilities_pb2 import (
    ListTopLevelAddressesRequest,
)
from nvidia.omniverse.storage.capabilities.v1alpha.capabilities_pb2_grpc import (
    CapabilitiesServiceStub,
)
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_pb2 import Chunk, Header
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_service_pb2 import (
    CompleteRedirectUploadRequest,
    DeleteRequest,
    UploadPreference,
    WriteParameters,
    WriteRequest,
)

logger = logging.getLogger(__name__)


def get_versioning_url(http_endpoint, resource_address):
    """Get URL for versioning API endpoint."""
    address_encoded = urllib.parse.quote(resource_address, safe="")
    endpoint = f"/v1alpha/versioning/{address_encoded}/versions"
    return http_endpoint + endpoint


def get_filefolder_list_url(http_endpoint, folder_path):
    """Get URL for filefolder list endpoint."""
    address_encoded = urllib.parse.quote(folder_path, safe="")
    endpoint = f"/v1alpha/filefolder/list/{address_encoded}"
    return http_endpoint + endpoint


def get_filefolder_liststat_url(http_endpoint, folder_path):
    """Get URL for filefolder liststat endpoint."""
    address_encoded = urllib.parse.quote(folder_path, safe="")
    endpoint = f"/v1alpha/filefolder/liststat/{address_encoded}"
    return http_endpoint + endpoint


def get_fileobject_by_identity_url(http_endpoint, identity):
    """Get URL for fileobject by-identity API endpoint."""
    identity_encoded = urllib.parse.quote(identity, safe="")
    endpoint = f"/v1alpha/fileobject/by-identity/{identity_encoded}"
    return http_endpoint + endpoint


def get_metadata_create_url(http_endpoint, namespace, key):
    """Get URL for metadata create endpoint."""
    namespace_encoded = urllib.parse.quote(namespace, safe="")
    key_encoded = urllib.parse.quote(key, safe="")
    endpoint = f"/v1alpha/metadata/{namespace_encoded}/{key_encoded}"
    return http_endpoint + endpoint


def get_metadata_get_url(http_endpoint, namespace):
    """Get URL for metadata get endpoint."""
    namespace_encoded = urllib.parse.quote(namespace, safe="")
    endpoint = f"/v1alpha/metadata/{namespace_encoded}"
    return http_endpoint + endpoint


def get_metadata_delete_url(http_endpoint, namespace, key):
    """Get URL for metadata delete endpoint."""
    namespace_encoded = urllib.parse.quote(namespace, safe="")
    key_encoded = urllib.parse.quote(key, safe="")
    endpoint = f"/v1alpha/metadata/{namespace_encoded}/{key_encoded}"
    return http_endpoint + endpoint


def get_capabilities_services_url(http_endpoint):
    """Get URL for capabilities services endpoint."""
    endpoint = "/v1alpha/capabilities/services"
    return http_endpoint + endpoint


def get_capabilities_top_level_addresses_url(http_endpoint):
    """Get URL for capabilities top-level-addresses endpoint."""
    endpoint = "/v1alpha/capabilities/top-level-addresses"
    return http_endpoint + endpoint


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

    # Auto-detect TLS from URL scheme
    if not skip_tls:
        if grpc_url.startswith("https://"):
            skip_tls = False
        elif grpc_url.startswith("http://"):
            skip_tls = True
        else:
            raise ValueError(f"Invalid endpoint URL: {grpc_url}")

    clean_endpoint = grpc_url.removeprefix("https://").removeprefix("http://")
    return _create_grpc_channel(clean_endpoint, skip_tls)


def get_top_level_addresses_grpc(grpc_endpoint: str, grpc_metadata: list) -> list:
    """Get top-level-addresses via gRPC.

    Args:
        grpc_endpoint: The gRPC endpoint URL (e.g. "https://example.com:443")
        grpc_metadata: gRPC metadata as list of tuples, e.g. [("authorization", "Bearer token")]
    """
    channel = create_grpc_channel_from_url(grpc_endpoint)

    try:
        stub = CapabilitiesServiceStub(channel)
        request = ListTopLevelAddressesRequest()
        response = timeout_grpc_call(
            stub.ListTopLevelAddresses, request, metadata=grpc_metadata
        )
        return [item.top_level_address for item in response.items]
    finally:
        channel.close()


def get_capabilities_routes_url(http_endpoint):
    """Get URL for capabilities routes endpoint."""
    endpoint = "/v1alpha/capabilities/routes"
    return http_endpoint + endpoint


def get_copy_url(http_endpoint, identity):
    """Get URL for copy by-identity endpoint."""
    identity_encoded = urllib.parse.quote(identity, safe="")
    endpoint = f"/v1alpha/fileobject/by-identity/{identity_encoded}/copy"
    return http_endpoint + endpoint


def with_grpc_file_setup(additional_cleanup_suffixes=None):
    """Decorator that handles gRPC file write-operation-delete lifecycle.

    Args:
        additional_cleanup_suffixes: List of suffixes to append to resource_address for additional cleanup.
                                     E.g., ["_copy"] will also delete {resource_address}_copy
    """

    def decorator(test_func):
        @wraps(test_func)
        def wrapper(*args, **kwargs):
            resource_address = kwargs.get("resource_address")
            fileobject_grpc_stub = kwargs.get("fileobject_grpc_stub")
            grpc_metadata = kwargs.get("grpc_metadata")

            try:
                content = b"test data content"
                write_file_grpc(
                    resource_address, content, fileobject_grpc_stub, grpc_metadata
                )
                return test_func(*args, **kwargs)

            finally:
                delete_file_grpc(resource_address, fileobject_grpc_stub, grpc_metadata)
                if additional_cleanup_suffixes:
                    for suffix in additional_cleanup_suffixes:
                        try:
                            delete_file_grpc(
                                f"{resource_address}{suffix}",
                                fileobject_grpc_stub,
                                grpc_metadata,
                            )
                        except Exception as ex:
                            logger.warning(
                                "Failed to cleanup %s%s", resource_address, suffix, exc_info=ex
                            )

        return wrapper

    return decorator


def with_rest_file_setup(additional_cleanup_suffixes=None):
    """Decorator that handles REST file write-operation-delete lifecycle.

    Args:
        additional_cleanup_suffixes: List of suffixes to append to resource_address for additional cleanup.
                                     E.g., ["_copy"] will also delete {resource_address}_copy
    """

    def decorator(test_func):
        @wraps(test_func)
        def wrapper(*args, **kwargs):
            resource_address = kwargs.get("resource_address")
            http_headers = kwargs.get("http_headers")
            service_endpoints = kwargs.get("service_endpoints")

            content = b"standard test data content"

            try:
                write_file_rest(
                    resource_address, content, http_headers, service_endpoints
                )
                return test_func(*args, **kwargs)

            finally:
                delete_file_rest(resource_address, http_headers, service_endpoints)
                if additional_cleanup_suffixes:
                    for suffix in additional_cleanup_suffixes:
                        try:
                            delete_file_rest(
                                f"{resource_address}{suffix}",
                                http_headers,
                                service_endpoints,
                            )
                        except Exception as ex:
                            logger.warning(
                                "Failed to cleanup %s%s", resource_address, suffix, exc_info=ex
                            )

        return wrapper

    return decorator


def timeout_rest_request(request_func, *args, **kwargs):
    """Execute a request with timeout."""
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


def assert_read_response(response, operation_name):
    """Assert a read response, accepting 200 OK or 3xx CDN redirect."""
    status = response.status_code
    assert status == 200 or 300 <= status < 400, (
        f"{operation_name}: expected 200 or 3xx redirect, got {status}"
    )


def get_fileobject_url(http_endpoint, resource_address):
    """Get URL for fileobject API endpoint."""
    address_encoded = urllib.parse.quote(resource_address, safe="")
    endpoint = f"/v1alpha/fileobject/by-address/{address_encoded}"
    return http_endpoint + endpoint


_TOKEN_FILE_WAIT_SECONDS = 30.0
_TOKEN_FILE_POLL_INTERVAL = 1.0


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
            logger.info(
                "Waiting for token file %s to become readable…",
                path,
                exc_info=e,
            )
            time.sleep(_TOKEN_FILE_POLL_INTERVAL)


def get_access_token():
    token_file = os.environ.get("SMOKE_TESTS_ACCESS_TOKEN_FILE_PATH")
    microsoft_url = os.environ.get("SMOKE_TESTS_AUTH_MICROSOFT_URL")
    login_password = os.environ.get("SMOKE_TESTS_AUTH_CREDENTIALS", "").strip()
    scope = os.environ.get("SMOKE_TESTS_AUTH_SCOPE")
    has_client_credentials = bool(microsoft_url and login_password and scope)

    if token_file and has_client_credentials:
        raise RuntimeError(
            "Conflicting auth: both SMOKE_TESTS_ACCESS_TOKEN_FILE_PATH and "
            "SMOKE_TESTS_AUTH_* are set. Configure exactly one auth mode."
        )

    if token_file:
        return _read_access_token_from_file(token_file)

    if not has_client_credentials:
        logger.warning("Missing required OAuth environment variables")
        return None

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


def write_file_grpc(resource_address, content, fileobject_grpc_stub, grpc_metadata):
    """Helper function to write a file via gRPC.

    Handles both inline uploads and redirect (pre-signed URL) uploads.
    """

    request_queue: Queue[WriteRequest | None] = Queue(5)

    def write_request_generator():
        while True:
            request = request_queue.get()
            if request is None:
                break
            yield request

    try:
        # Send initial params
        params = WriteParameters(
            destination_resource_address=resource_address,
            data_object_size=len(content),
            upload_preference=UploadPreference.UPLOAD_PREFERENCE_BODY,
        )
        request_queue.put(WriteRequest(params=params))

        response_stream = timeout_grpc_call(
            fileobject_grpc_stub.Write,
            write_request_generator(),
            metadata=grpc_metadata,
        )

        # Get first response to determine upload method
        flow_control = next(response_stream)

        # Check for immediate completion (0-byte write)
        if flow_control.HasField("resource_info"):
            return [flow_control]

        # Inline upload accepted
        if flow_control.HasField("write_chunks_accepted"):
            request_queue.put(WriteRequest(chunk=Chunk(chunk=content)))
            request_queue.put(None)
            responses = [flow_control] + list(response_stream)
            return responses

        # Redirect to pre-signed URL
        if flow_control.HasField("write_redirect"):
            redirect = flow_control.write_redirect

            # Upload via HTTP to the pre-signed URL
            http_response = requests.request(
                url=redirect.redirect_target_url,
                method="PUT" if redirect.method == 1 else "POST",
                headers={h.name: h.value for h in redirect.additional_headers},
                data=content,
            )
            http_response.raise_for_status()

            # Complete the redirect upload
            requested_headers = set(redirect.completion_header_names)
            response_headers_lower = {k.lower(): k for k in http_response.headers}

            fileobject_grpc_stub.CompleteRedirectUpload(
                CompleteRedirectUploadRequest(
                    destination_resource_address=resource_address,
                    additional_headers=[
                        Header(
                            name=h,
                            value=http_response.headers[
                                response_headers_lower[h.lower()]
                            ],
                        )
                        for h in requested_headers
                        if h.lower() in response_headers_lower
                    ],
                ),
                metadata=grpc_metadata,
            )
            return [flow_control]

        raise ValueError(f"Unexpected write response type: {flow_control}")

    finally:
        # Ensure the generator is properly closed
        request_queue.put(None)


def delete_file_grpc(resource_address, fileobject_grpc_stub, grpc_metadata):
    """Helper function to delete a file via gRPC."""
    request = DeleteRequest(resource_address=resource_address)
    return timeout_grpc_call(
        fileobject_grpc_stub.Delete, request, metadata=grpc_metadata
    )


def write_file_rest(resource_address, content, http_headers, service_endpoints):
    """Helper function to write a file via REST."""
    response = timeout_rest_request(
        requests.put,
        get_fileobject_url(service_endpoints["http_storage"], resource_address),
        headers=http_headers,
        data=content,
        params={"data_object_size": len(content)},
    )
    return response


def delete_file_rest(resource_address, http_headers, service_endpoints):
    """Helper function to delete a file via REST."""
    return timeout_rest_request(
        requests.delete,
        get_fileobject_url(service_endpoints["http_storage"], resource_address),
        headers=http_headers,
    )
