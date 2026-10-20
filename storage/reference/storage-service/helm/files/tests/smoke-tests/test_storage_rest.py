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

import pytest
from storage_rest_helpers import (
    run_copy_test_rest,
    run_delete_metadata_test_rest,
    run_delete_test_rest,
    run_enumerate_versions_test_rest,
    run_filefolder_list_test_rest,
    run_filefolder_liststat_test_rest,
    run_get_metadata_test_rest,
    run_list_routes_test_rest,
    run_list_services_test_rest,
    run_list_top_level_addresses_test_rest,
    run_read_by_identity_test_rest,
    run_read_from_address_test_rest,
    run_stat_test_rest,
    run_update_metadata_test_rest,
    run_write_test_rest,
)
from utils import delete_file_rest, with_rest_file_setup

logger = logging.getLogger(__name__)


@pytest.mark.rest
def test_list_services_http(
    require_storage_rest_endpoint, http_headers, service_endpoints
):
    """Test HTTP ListServices operation."""
    run_list_services_test_rest(http_headers, service_endpoints)


@pytest.mark.rest
def test_list_top_level_addresses_http(
    require_storage_rest_endpoint, http_headers, service_endpoints
):
    """Test HTTP ListTopLevelAddresses operation."""
    run_list_top_level_addresses_test_rest(http_headers, service_endpoints)


@pytest.mark.rest
def test_list_routes_http(
    require_storage_rest_endpoint, http_headers, service_endpoints
):
    """Test HTTP ListRoutes operation."""
    run_list_routes_test_rest(http_headers, service_endpoints)


@pytest.mark.rest
@with_rest_file_setup()
def test_stat_http(
    require_storage_rest_endpoint,
    resource_address,
    http_headers,
    service_endpoints,
):
    """Test HTTP Stat operation."""
    run_stat_test_rest(resource_address, http_headers, service_endpoints)


@pytest.mark.rest
@with_rest_file_setup()
def test_read_from_address_http(
    require_storage_rest_endpoint,
    resource_address,
    http_headers,
    service_endpoints,
):
    """Test HTTP ReadFromAddress operation."""
    run_read_from_address_test_rest(resource_address, http_headers, service_endpoints)


@pytest.mark.rest
@with_rest_file_setup()
def test_enumerate_versions_http(
    require_storage_rest_endpoint,
    resource_address,
    http_headers,
    service_endpoints,
):
    """Test enumerating versions via versioning API."""
    run_enumerate_versions_test_rest(resource_address, http_headers, service_endpoints)


@pytest.mark.rest
@with_rest_file_setup()
def test_filefolder_list_http(
    require_storage_rest_endpoint,
    resource_address,
    resource_base_url,
    http_headers,
    service_endpoints,
):
    """Test HTTP List operation."""
    run_filefolder_list_test_rest(
        resource_base_url, http_headers, service_endpoints
    )


@pytest.mark.rest
@with_rest_file_setup()
def test_filefolder_liststat_http(
    require_storage_rest_endpoint,
    resource_address,
    resource_base_url,
    http_headers,
    service_endpoints,
):
    """Test HTTP ListStat operation."""
    run_filefolder_liststat_test_rest(
        resource_base_url, http_headers, service_endpoints
    )


@pytest.mark.rest
def test_write_http(
    require_storage_rest_endpoint,
    resource_address,
    http_headers,
    service_endpoints,
):
    """Test HTTP Write operation."""
    try:
        run_write_test_rest(resource_address, http_headers, service_endpoints)
    finally:
        delete_file_rest(resource_address, http_headers, service_endpoints)


@pytest.mark.rest
def test_delete_http(
    require_storage_rest_endpoint,
    resource_address,
    http_headers,
    service_endpoints,
):
    """Test HTTP Delete operation."""
    run_delete_test_rest(resource_address, http_headers, service_endpoints)


@pytest.mark.rest
@with_rest_file_setup()
def test_read_by_identity_http(
    require_storage_rest_endpoint,
    resource_address,
    http_headers,
    service_endpoints,
):
    """Test HTTP Read by Identity operation using stat output."""
    run_read_by_identity_test_rest(resource_address, http_headers, service_endpoints)


@pytest.mark.rest
@with_rest_file_setup(additional_cleanup_suffixes=["_copy"])
def test_copy_http(
    require_storage_rest_endpoint,
    resource_address,
    http_headers,
    service_endpoints,
):
    """Test HTTP Copy by Identity operation using stat output."""
    run_copy_test_rest(resource_address, http_headers, service_endpoints)


@pytest.mark.rest
@with_rest_file_setup()
def test_update_metadata_http(
    require_storage_rest_endpoint,
    require_metadata_rest,
    resource_address,
    http_headers,
    service_endpoints,
):
    """Test HTTP UpdateMetadata operation."""
    run_update_metadata_test_rest(resource_address, http_headers, service_endpoints)


@pytest.mark.rest
@with_rest_file_setup()
def test_get_metadata_http(
    require_storage_rest_endpoint,
    require_metadata_rest,
    resource_address,
    http_headers,
    service_endpoints,
):
    """Test HTTP GetMetadata operation."""
    # First set metadata so we can get it
    run_update_metadata_test_rest(resource_address, http_headers, service_endpoints)
    run_get_metadata_test_rest(resource_address, http_headers, service_endpoints)


@pytest.mark.rest
@with_rest_file_setup()
def test_delete_metadata_http(
    require_storage_rest_endpoint,
    require_metadata_rest,
    resource_address,
    http_headers,
    service_endpoints,
):
    """Test HTTP DeleteMetadata operation."""
    # First set metadata so we can delete it
    run_update_metadata_test_rest(resource_address, http_headers, service_endpoints)
    run_delete_metadata_test_rest(resource_address, http_headers, service_endpoints)
