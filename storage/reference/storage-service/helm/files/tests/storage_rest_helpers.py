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

"""
Common test functions for Storage Service REST tests.
"""

import requests
from utils import (
    assert_read_response,
    assert_response,
    get_capabilities_routes_url,
    get_capabilities_services_url,
    get_capabilities_top_level_addresses_url,
    get_copy_url,
    get_filefolder_list_url,
    get_filefolder_liststat_url,
    get_fileobject_by_identity_url,
    get_fileobject_url,
    get_metadata_create_url,
    get_metadata_delete_url,
    get_metadata_get_url,
    get_versioning_url,
    timeout_rest_request,
    write_file_rest,
)


def run_list_services_test_rest(http_headers, service_endpoints):
    """Common test for ListServices (REST)."""

    response = timeout_rest_request(
        requests.get,
        get_capabilities_services_url(service_endpoints["http_storage"]),
        headers={**http_headers, "accept": "application/json"},
    )
    assert_response(response, 200, "ListServices(REST)")

    return response.json()["services"]


def run_list_top_level_addresses_test_rest(http_headers, service_endpoints):
    """Common test for ListTopLevelAddresses (REST)."""
    response = timeout_rest_request(
        requests.get,
        get_capabilities_top_level_addresses_url(service_endpoints["http_storage"]),
        headers={**http_headers, "accept": "application/json"},
    )
    assert_response(response, 200, "ListTopLevelAddresses(REST)")


def run_list_routes_test_rest(http_headers, service_endpoints):
    """Common test for ListRoutes (REST)."""
    response = timeout_rest_request(
        requests.get,
        get_capabilities_routes_url(service_endpoints["http_storage"]),
        headers={**http_headers, "accept": "application/json"},
    )
    assert_response(response, 200, "ListRoutes(REST)")


def run_stat_test_rest(resource_address, http_headers, service_endpoints):
    """Common test for Stat (REST)."""

    response = timeout_rest_request(
        requests.head,
        get_fileobject_url(service_endpoints["http_storage"], resource_address),
        headers=http_headers,
    )
    assert_response(response, 204, "Stat(REST)")

    return response


def run_read_from_address_test_rest(resource_address, http_headers, service_endpoints):
    """Common test for ReadFromAddress (REST)."""

    response = timeout_rest_request(
        requests.get,
        get_fileobject_url(service_endpoints["http_storage"], resource_address),
        headers=http_headers,
    )
    assert_read_response(response, "ReadFromAddress(REST)")

    return response


def run_enumerate_versions_test_rest(
    resource_address,
    http_headers,
    service_endpoints,
    create_multiple_versions_rest=None,
):
    """Common test for EnumerateVersions (REST)."""

    response = timeout_rest_request(
        requests.get,
        get_versioning_url(service_endpoints["http_storage"], resource_address),
        headers=http_headers,
    )
    assert_response(response, 200, "EnumerateVersions(REST)")
    return response


def run_filefolder_list_test_rest(
    resource_base_url,
    http_headers,
    service_endpoints,
):
    """Common test for List (REST)."""
    response = timeout_rest_request(
        requests.get,
        get_filefolder_list_url(
            service_endpoints["http_storage"], resource_base_url
        ),
        headers=http_headers,
    )
    assert_response(response, 200, "List(REST)")
    return response


def run_filefolder_liststat_test_rest(
    resource_base_url,
    http_headers,
    service_endpoints,
):
    """Common test for ListStat (REST)."""
    response = timeout_rest_request(
        requests.get,
        get_filefolder_liststat_url(
            service_endpoints["http_storage"], resource_base_url
        ),
        headers=http_headers,
    )
    assert_response(response, 200, "ListStat(REST)")

    return response


def run_write_test_rest(
    resource_address,
    http_headers,
    service_endpoints,
    content=None,
):
    """Common test for Write (REST)."""
    if content is None:
        content = b"test write operation data"

    response = write_file_rest(
        resource_address, content, http_headers, service_endpoints
    )
    assert_response(response, 201, "Write(REST)")

    return response


def run_delete_test_rest(resource_address, http_headers, service_endpoints):
    """Common test for Delete (REST)."""
    content = b"test delete operation data"
    write_file_rest(resource_address, content, http_headers, service_endpoints)

    response = timeout_rest_request(
        requests.delete,
        get_fileobject_url(service_endpoints["http_storage"], resource_address),
        headers=http_headers,
    )
    assert_response(response, 204, "Delete(REST)")


def run_read_by_identity_test_rest(resource_address, http_headers, service_endpoints):
    """Common test for Read by Identity (REST)."""
    identity = get_resource_identity_rest(
        resource_address, http_headers, service_endpoints
    )

    read_response = timeout_rest_request(
        requests.get,
        get_fileobject_by_identity_url(service_endpoints["http_storage"], identity),
        headers={**http_headers, "accept": "application/json"},
    )
    assert_read_response(read_response, "Read(REST)")
    return read_response


def run_copy_test_rest(resource_address, http_headers, service_endpoints):
    """Common test for Copy (REST)."""
    identity = get_resource_identity_rest(
        resource_address, http_headers, service_endpoints
    )

    copy_payload = {"destination_resource_address": f"{resource_address}_copy"}

    copy_response = timeout_rest_request(
        requests.post,
        get_copy_url(service_endpoints["http_storage"], identity),
        headers={**http_headers, "accept": "application/json"},
        json=copy_payload,
    )
    assert_response(copy_response, 201, "Copy(REST)")
    return copy_response


def run_update_metadata_test_rest(
    resource_address, http_headers, service_endpoints, metadata_key="test"
):
    """Test UpdateMetadata (REST)."""
    storage_endpoint = service_endpoints["http_storage"]
    json_headers = {
        **http_headers,
        "accept": "application/json",
        "Content-Type": "application/json",
    }
    url = get_metadata_create_url(storage_endpoint, resource_address, metadata_key)
    response = timeout_rest_request(requests.put, url, headers=json_headers, json=True)
    assert_response(response, [201, 204], "UpdateMetadata(REST)")


def run_get_metadata_test_rest(
    resource_address, http_headers, service_endpoints, metadata_key="test"
):
    """Test GetMetadata (REST)."""
    storage_endpoint = service_endpoints["http_storage"]
    json_headers = {
        **http_headers,
        "accept": "application/json",
        "Content-Type": "application/json",
    }
    url = get_metadata_get_url(storage_endpoint, resource_address)
    response = timeout_rest_request(
        requests.post, url, headers=json_headers, json=[metadata_key]
    )
    assert_response(response, 200, "GetMetadata(REST)")
    return response


def run_delete_metadata_test_rest(
    resource_address, http_headers, service_endpoints, metadata_key="test"
):
    """Test DeleteMetadata (REST)."""
    storage_endpoint = service_endpoints["http_storage"]
    url = get_metadata_delete_url(storage_endpoint, resource_address, metadata_key)
    response = timeout_rest_request(
        requests.delete, url, headers={**http_headers, "accept": "*/*"}
    )
    assert_response(response, 204, "DeleteMetadata(REST)")


def get_resource_identity_rest(
    resource_address, http_headers, service_endpoints
) -> str:
    """Get resource identity from stat response."""
    stat_url = get_fileobject_url(service_endpoints["http_storage"], resource_address)
    stat_response = timeout_rest_request(requests.head, stat_url, headers=http_headers)

    if stat_response.status_code != 204:
        raise RuntimeError(f"Stat failed with status {stat_response.status_code}")

    return stat_response.headers.get("x-nvidia-omniverse-storage-resource-identity")
