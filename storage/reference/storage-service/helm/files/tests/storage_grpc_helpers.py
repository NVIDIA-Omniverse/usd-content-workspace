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
Common test functions for Storage Service gRPC tests.
"""

from google.protobuf.struct_pb2 import Value
from utils import timeout_grpc_call, write_file_grpc

# gRPC imports (protobuf modules available via PYTHONPATH)
from nvidia.omniverse.storage.capabilities.v1alpha.capabilities_pb2 import (
    ListRoutesRequest,
    ListServicesRequest,
    ListTopLevelAddressesRequest,
)
from nvidia.omniverse.storage.filefolder.v1alpha.filefolder_service_pb2 import (
    FolderAddress,
    ListRequest,
    ListStatRequest,
)
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_service_pb2 import (
    CopyRequest,
    DeleteRequest,
    ReadFromAddressRequest,
    ReadRequest,
    StatRequest,
)
from nvidia.omniverse.storage.metadata.v1alpha.metadata_pb2 import (
    DeleteMetadataRequest,
    GetMetadataRequest,
    UpdateMetadataRequest,
)
from nvidia.omniverse.storage.versioning.v1alpha.versioning_pb2 import (
    EnumerateVersionsRequest,
)


def run_list_services_test_grpc(capabilities_grpc_stub, grpc_metadata):
    """Common test for ListServices (gRPC)."""
    request = ListServicesRequest()
    return timeout_grpc_call(
        capabilities_grpc_stub.ListServices, request, metadata=grpc_metadata
    )


def run_list_top_level_addresses_test_grpc(capabilities_grpc_stub, grpc_metadata):
    """Common test for ListTopLevelAddresses (gRPC)."""
    request = ListTopLevelAddressesRequest()
    timeout_grpc_call(
        capabilities_grpc_stub.ListTopLevelAddresses,
        request,
        metadata=grpc_metadata,
    )


def run_list_routes_test_grpc(capabilities_grpc_stub, grpc_metadata):
    """Common test for ListRoutes (gRPC)."""
    request = ListRoutesRequest()
    timeout_grpc_call(
        capabilities_grpc_stub.ListRoutes, request, metadata=grpc_metadata
    )


def run_write_test_grpc(
    resource_address,
    fileobject_grpc_stub,
    grpc_metadata,
    content=None,
):
    """Common test for Write (gRPC)."""
    if content is None:
        content = b"test write operation data"

    return write_file_grpc(
        resource_address, content, fileobject_grpc_stub, grpc_metadata
    )


def run_stat_test_grpc(resource_address, fileobject_grpc_stub, grpc_metadata):
    """Common test for Stat (gRPC)."""
    request = StatRequest(resource_address=resource_address)
    return timeout_grpc_call(fileobject_grpc_stub.Stat, request, metadata=grpc_metadata)


def run_readfromaddress_test_grpc(
    resource_address, fileobject_grpc_stub, grpc_metadata
):
    """Common test for ReadFromAddress (gRPC)."""
    request = ReadFromAddressRequest(resource_address=resource_address)
    response_stream = timeout_grpc_call(
        fileobject_grpc_stub.ReadFromAddress, request, metadata=grpc_metadata
    )

    responses = list(response_stream)
    assert len(responses) > 0, "ReadFromAddress operation returned no responses"
    return responses


def run_enumerate_versions_test_grpc(
    resource_address,
    versioning_grpc_stub,
    grpc_metadata,
):
    """Common test for EnumerateVersions (gRPC)."""
    request = EnumerateVersionsRequest(resource_address=resource_address)
    response_stream = timeout_grpc_call(
        versioning_grpc_stub.EnumerateVersions, request, metadata=grpc_metadata
    )

    return list(response_stream)


def run_list_test_grpc(
    resource_base_url,
    filefolder_grpc_stub,
    grpc_metadata,
):
    """Common test for List (gRPC)."""
    folder_address = FolderAddress(uri=resource_base_url)
    request = ListRequest(folder=folder_address)

    response_stream = timeout_grpc_call(
        filefolder_grpc_stub.List, request, metadata=grpc_metadata
    )

    return list(response_stream)


def run_liststat_test_grpc(
    resource_base_url,
    filefolder_grpc_stub,
    grpc_metadata,
):
    """Common test for ListStat (gRPC)."""
    folder_address = FolderAddress(uri=resource_base_url)
    request = ListStatRequest(folder=folder_address)

    response_stream = timeout_grpc_call(
        filefolder_grpc_stub.ListStat, request, metadata=grpc_metadata
    )

    return list(response_stream)


def run_delete_test_grpc(resource_address, fileobject_grpc_stub, grpc_metadata):
    """Common test for Delete (gRPC)."""

    content = b"delete test data"
    write_file_grpc(resource_address, content, fileobject_grpc_stub, grpc_metadata)

    request = DeleteRequest(resource_address=resource_address)
    timeout_grpc_call(fileobject_grpc_stub.Delete, request, metadata=grpc_metadata)


def run_read_by_identity_test_grpc(
    resource_address, fileobject_grpc_stub, grpc_metadata
):
    """Common test for Read by Identity (gRPC)."""

    stat_request = StatRequest(resource_address=resource_address)
    stat_response = timeout_grpc_call(
        fileobject_grpc_stub.Stat, stat_request, metadata=grpc_metadata
    )

    resource_identity = stat_response.resource_info.resource_identity
    read_request = ReadRequest(resource_identity=resource_identity)

    read_response_stream = timeout_grpc_call(
        fileobject_grpc_stub.Read, read_request, metadata=grpc_metadata
    )

    responses = list(read_response_stream)
    assert len(responses) > 0, "Read by Identity operation returned no responses"
    return responses


def run_copy_test_grpc(
    resource_address,
    fileobject_grpc_stub,
    grpc_metadata,
):
    """Common test for Copy (gRPC)."""

    stat_request = StatRequest(resource_address=resource_address)
    stat_response = timeout_grpc_call(
        fileobject_grpc_stub.Stat, stat_request, metadata=grpc_metadata
    )

    source_resource_identity = stat_response.resource_info.resource_identity

    copy_request = CopyRequest(
        source_resource_identity=source_resource_identity,
        destination_resource_address=f"{resource_address}_copy",
    )

    return timeout_grpc_call(
        fileobject_grpc_stub.Copy, copy_request, metadata=grpc_metadata
    )


def run_update_metadata_test_grpc(
    resource_address, metadata_grpc_stub, grpc_metadata, metadata_key="test"
):
    """Test UpdateMetadata (gRPC)."""
    metadata_value = Value(bool_value=True)
    request = UpdateMetadataRequest(
        uri=resource_address,
        user_metadata_key=metadata_key,
        user_metadata=metadata_value,
    )
    timeout_grpc_call(
        metadata_grpc_stub.UpdateMetadata, request, metadata=grpc_metadata
    )


def run_get_metadata_test_grpc(
    resource_address, metadata_grpc_stub, grpc_metadata, metadata_key="test"
):
    """Test GetMetadata (gRPC)."""
    request = GetMetadataRequest(
        uri=resource_address, user_metadata_keys=[metadata_key]
    )
    return timeout_grpc_call(
        metadata_grpc_stub.GetMetadata, request, metadata=grpc_metadata
    )


def run_delete_metadata_test_grpc(
    resource_address, metadata_grpc_stub, grpc_metadata, metadata_key="test"
):
    """Test DeleteMetadata (gRPC)."""
    request = DeleteMetadataRequest(
        uri=resource_address, user_metadata_key=metadata_key
    )
    timeout_grpc_call(
        metadata_grpc_stub.DeleteMetadata, request, metadata=grpc_metadata
    )
