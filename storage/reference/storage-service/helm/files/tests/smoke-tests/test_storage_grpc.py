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
from storage_grpc_helpers import (
    run_copy_test_grpc,
    run_delete_metadata_test_grpc,
    run_delete_test_grpc,
    run_enumerate_versions_test_grpc,
    run_get_metadata_test_grpc,
    run_list_routes_test_grpc,
    run_list_services_test_grpc,
    run_list_test_grpc,
    run_list_top_level_addresses_test_grpc,
    run_liststat_test_grpc,
    run_read_by_identity_test_grpc,
    run_readfromaddress_test_grpc,
    run_stat_test_grpc,
    run_update_metadata_test_grpc,
    run_write_test_grpc,
)
from utils import delete_file_grpc, timeout_grpc_call, with_grpc_file_setup

logger = logging.getLogger(__name__)

@pytest.mark.grpc
def test_list_services_grpc(capabilities_grpc_stub, grpc_metadata):
    """Test gRPC ListServices operation."""
    run_list_services_test_grpc(capabilities_grpc_stub, grpc_metadata)


@pytest.mark.grpc
def test_list_top_level_addresses_grpc(capabilities_grpc_stub, grpc_metadata):
    """Test gRPC ListTopLevelAddresses operation."""
    run_list_top_level_addresses_test_grpc(capabilities_grpc_stub, grpc_metadata)


@pytest.mark.grpc
def test_list_routes_grpc(capabilities_grpc_stub, grpc_metadata):
    """Test gRPC ListRoutes operation."""
    run_list_routes_test_grpc(capabilities_grpc_stub, grpc_metadata)


@pytest.mark.grpc
def test_write_grpc(resource_address, fileobject_grpc_stub, grpc_metadata):
    """Test gRPC Write operation."""
    try:
        run_write_test_grpc(resource_address, fileobject_grpc_stub, grpc_metadata)
    finally:
        delete_file_grpc(resource_address, fileobject_grpc_stub, grpc_metadata)


@pytest.mark.grpc
@with_grpc_file_setup()
def test_stat_grpc(resource_address, fileobject_grpc_stub, grpc_metadata):
    """Test gRPC Stat operation."""
    run_stat_test_grpc(resource_address, fileobject_grpc_stub, grpc_metadata)


@pytest.mark.grpc
@with_grpc_file_setup()
def test_readfromaddress_grpc(resource_address, fileobject_grpc_stub, grpc_metadata):
    """Test gRPC ReadFromAddress operation."""
    run_readfromaddress_test_grpc(resource_address, fileobject_grpc_stub, grpc_metadata)


@pytest.mark.grpc
@with_grpc_file_setup()
def test_enumerate_versions_grpc(
    resource_address,
    versioning_grpc_stub,
    fileobject_grpc_stub,
    grpc_metadata,
):
    """Test gRPC EnumerateVersions operation."""
    run_enumerate_versions_test_grpc(
        resource_address, versioning_grpc_stub, grpc_metadata
    )


@pytest.mark.grpc
@with_grpc_file_setup()
def test_list_grpc(
    resource_address,
    resource_base_url,
    filefolder_grpc_stub,
    fileobject_grpc_stub,
    grpc_metadata,
):
    """Test gRPC List operation."""
    run_list_test_grpc(
        resource_base_url,
        filefolder_grpc_stub,
        grpc_metadata,
    )


@pytest.mark.grpc
@with_grpc_file_setup()
def test_liststat_grpc(
    resource_address,
    resource_base_url,
    filefolder_grpc_stub,
    fileobject_grpc_stub,
    grpc_metadata,
):
    """Test gRPC ListStat operation."""
    run_liststat_test_grpc(
        resource_base_url,
        filefolder_grpc_stub,
        grpc_metadata,
    )


@pytest.mark.grpc
def test_delete_grpc(resource_address, fileobject_grpc_stub, grpc_metadata):
    """Test gRPC Delete operation."""
    run_delete_test_grpc(resource_address, fileobject_grpc_stub, grpc_metadata)


@pytest.mark.grpc
@with_grpc_file_setup()
def test_read_by_identity_grpc(resource_address, fileobject_grpc_stub, grpc_metadata):
    """Test gRPC Read by Identity operation using stat output."""
    run_read_by_identity_test_grpc(
        resource_address, fileobject_grpc_stub, grpc_metadata
    )


@pytest.mark.grpc
@with_grpc_file_setup(additional_cleanup_suffixes=["_copy"])
def test_copy_grpc(
    resource_address,
    fileobject_grpc_stub,
    grpc_metadata,
):
    """Test gRPC Copy operation using stat output."""
    run_copy_test_grpc(resource_address, fileobject_grpc_stub, grpc_metadata)


@pytest.mark.grpc
@with_grpc_file_setup()
def test_update_metadata_grpc(
    require_metadata_grpc,
    resource_address,
    fileobject_grpc_stub,
    metadata_grpc_stub,
    grpc_metadata,
):
    """Test gRPC UpdateMetadata operation."""
    run_update_metadata_test_grpc(resource_address, metadata_grpc_stub, grpc_metadata)


@pytest.mark.grpc
@with_grpc_file_setup()
def test_get_metadata_grpc(
    require_metadata_grpc,
    resource_address,
    fileobject_grpc_stub,
    metadata_grpc_stub,
    grpc_metadata,
):
    """Test gRPC GetMetadata operation."""
    # First set metadata so we can get it
    run_update_metadata_test_grpc(resource_address, metadata_grpc_stub, grpc_metadata)
    run_get_metadata_test_grpc(resource_address, metadata_grpc_stub, grpc_metadata)


@pytest.mark.grpc
@with_grpc_file_setup()
def test_delete_metadata_grpc(
    require_metadata_grpc,
    resource_address,
    fileobject_grpc_stub,
    metadata_grpc_stub,
    grpc_metadata,
):
    """Test gRPC DeleteMetadata operation."""
    # First set metadata so we can delete it
    run_update_metadata_test_grpc(resource_address, metadata_grpc_stub, grpc_metadata)
    run_delete_metadata_test_grpc(resource_address, metadata_grpc_stub, grpc_metadata)
