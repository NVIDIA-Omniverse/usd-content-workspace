# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

from typing import Any

from google.protobuf.json_format import (
    MessageToDict,
    ParseDict,
)
from google.protobuf.struct_pb2 import (
    Struct,
)
from google.protobuf.timestamp_pb2 import Timestamp
from local_filesystem_service.filesystem import (
    get_backend,
)
from local_filesystem_service.native_metadata import native_metadata_for_response


def native_metadata_from_proto(native_metadata) -> dict[str, Any]:
    """Convert a protobuf native_metadata map into a Python dictionary."""
    native_struct = Struct()
    for key, value in native_metadata.items():
        native_struct.fields[key].CopyFrom(value)
    decoded = MessageToDict(native_struct)
    if isinstance(decoded, dict):
        return decoded
    return {}


def metadata_message(
    fileobject_version,
    backend_metadata,
    native_metadata_support_scope: str | None = None,
    backend=None,
    include_native_metadata: bool = True,
):
    """Create a FileObject Metadata protobuf message from backend metadata."""
    selected_backend = backend or get_backend()
    support_scope = native_metadata_support_scope
    if support_scope is None:
        support_scope = getattr(backend_metadata, "native_metadata_support_scope", None)
    timestamp = Timestamp()
    timestamp.FromDatetime(backend_metadata.last_modified_timestamp)
    metadata = fileobject_version.Metadata(
        data_object_size=backend_metadata.data_object_size,
        last_modified_timestamp=timestamp,
    )
    if include_native_metadata and "native_metadata" in metadata.DESCRIPTOR.fields_by_name:
        native_struct = Struct()
        ParseDict(
            native_metadata_for_response(backend_metadata.native_metadata, support_scope, backend=selected_backend),
            native_struct,
        )
        for key, value in native_struct.fields.items():
            metadata.native_metadata[key].CopyFrom(value)
    return metadata


def resource_info(
    fileobject_version,
    resource_address: str,
    native_metadata_support_scope: str | None = None,
    backend=None,
    include_native_metadata: bool = True,
):
    """Create a ResourceInfo protobuf message for a resource address.

    Constructs a complete ResourceInfo message containing both the resource
    identity (opaque identifier) and metadata (size, modification time) for
    the latest version at the given resource address.

    Args:
        fileobject_version: The fileobject protobuf module (v1alpha or v1beta)
                           containing ResourceInfo and related message types.
        resource_address: Storage API resource address (e.g., 'file-storage://server/path').

    Returns:
        A ResourceInfo protobuf message containing:
        - resource_identity: Opaque encoded identity for the latest version
        - metadata: File size and modification timestamp

    Note:
        This automatically resolves to the latest version of the resource.
    """
    backend = backend or get_backend()
    resource_identity = fileobject_version.ResourceIdentity(
        encoded_identity=backend.create_identity_from_resource_address(resource_address)
    )
    # Use backend's stat method to get metadata
    version_info = backend.stat(resource_address)

    support_scope = native_metadata_support_scope
    if support_scope is None:
        support_scope = resource_address

    metadata = metadata_message(
        fileobject_version,
        version_info.metadata,
        support_scope,
        backend=backend,
        include_native_metadata=include_native_metadata,
    )

    return fileobject_version.ResourceInfo(resource_identity=resource_identity, metadata=metadata)
