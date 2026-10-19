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

"""Versioning service implementation for gRPC.

This module implements the VersioningService from the Storage API, which
provides operations for enumerating file versions. The Storage API maintains
immutable versions of files, and this service allows clients to discover and
access historical versions.
"""

import grpc
from local_filesystem_service.backends.storage_backend_interface import (
    VersionInfo,
)
from local_filesystem_service.backends.storage_backend_interface import (
    VersionsOrder as BackendVersionsOrder,
)
from local_filesystem_service.filesystem import (
    get_backend,
    is_versioning_enabled,
)
from local_filesystem_service.grpc_service.message_helpers import metadata_message


def make_versioning_service(fileobject_version, pb2_version, pb2_grpc_version, is_alpha: bool):
    """Create a dynamic VersioningService servicer for gRPC.

    This factory function creates a gRPC servicer class that implements the
    VersioningService interface. It uses dynamic type construction to adapt
    to different API versions (v1alpha, v1beta).

    Args:
        fileobject_version: The fileobject protobuf module containing common
                           message types (ResourceIdentity, Metadata, etc.).
        pb2_version: The versioning protobuf module containing service-specific
                    message types (VersionInfo, EnumerateVersionsResponse, etc.).
        pb2_grpc_version: The versioning gRPC module containing the servicer base class.
        is_alpha: Whether this is for the v1alpha API (includes resource_address
                 in VersionInfo) or v1beta (excludes resource_address).

    Returns:
        An instance of a dynamically created VersioningServiceServicer class
        with the EnumerateVersions method implemented.
    """

    def _version_info_for_version(version: VersionInfo, fallback_support_scope: str):
        """Convert a VersionInfo to a protobuf VersionInfo message.

        Args:
            version: VersionInfo containing version metadata and identifiers.

        Returns:
            A protobuf VersionInfo message with resource_info, sorting_key,
            and optionally resource_address (for v1alpha).
        """
        resource_identity = fileobject_version.ResourceIdentity(encoded_identity=version.resource_identity)
        support_scope = getattr(version.metadata, "native_metadata_support_scope", None) or fallback_support_scope
        metadata = metadata_message(fileobject_version, version.metadata, native_metadata_support_scope=support_scope)
        if is_alpha:
            return pb2_version.VersionInfo(
                resource_info=fileobject_version.ResourceInfo(resource_identity=resource_identity, metadata=metadata),
                sorting_key=version.sorting_key,
                resource_address=version.resource_address,
            )
        else:
            return pb2_version.VersionInfo(
                resource_info=fileobject_version.ResourceInfo(resource_identity=resource_identity, metadata=metadata),
                sorting_key=version.sorting_key,
            )

    def EnumerateVersions(self, request, context):
        """Enumerate all versions of a resource.

        Lists all available versions of a fileobject in the order determined by the
        storage service (newest first, oldest first, or by key). Each version
        represents an immutable snapshot of the fileobject at a point in time.

        Args:
            request: EnumerateVersionsRequest containing:
                    - resource_address: Address of the resource to enumerate
            context: gRPC ServicerContext for the request.

        Yields:
            EnumerateVersionsResponse containing:
            - items: List of VersionInfo messages with identity and metadata
            - versions_order: Ordering of the returned versions

        Raises:
            grpc.RpcError with:
            - INVALID_ARGUMENT: If resource_address is versioned or is a folder
            - NOT_FOUND: If resource_address doesn't exist

        Note:
            - Version addresses (containing version IDs) cannot be enumerated
            - Only files (not folders) can have versions
            - The versions_order field indicates how versions are sorted
        """
        if get_backend().is_version_address(request.resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                "EnumerateVersions does not work with versioned addresses",
            )
        if not get_backend().exists(request.resource_address):
            context.abort(grpc.StatusCode.NOT_FOUND, "Resource address not found")
        if get_backend().is_dir(request.resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                "Cannot enumerate versions of folder addresses",
            )
        if is_versioning_enabled():
            versions, versions_order = get_backend().enumerate_versions(request.resource_address)
        else:
            # Unversioned storage keeps no history: report only the current version.
            versions = [get_backend().stat(request.resource_address)]
            versions_order = BackendVersionsOrder.NEWEST_FIRST
        items = [_version_info_for_version(v, request.resource_address) for v in versions]

        # Map from interface VersionsOrder to protobuf enum
        versions_order_map = {
            BackendVersionsOrder.NEWEST_FIRST: pb2_version.VersionsOrder.VERSIONS_ORDER_NEWEST_FIRST,
            BackendVersionsOrder.OLDEST_FIRST: pb2_version.VersionsOrder.VERSIONS_ORDER_OLDEST_FIRST,
            BackendVersionsOrder.BY_KEY: pb2_version.VersionsOrder.VERSIONS_ORDER_BY_KEY,
        }
        proto_versions_order = versions_order_map[versions_order]

        yield pb2_version.EnumerateVersionsResponse(
            items=items,
            versions_order=proto_versions_order,
        )

    def IsVersioned(self, request, context):
        """Query whether the given resource address supports versioning."""
        resource_address = request.resource_address
        base = get_backend().base_uri.rstrip("/")
        address = resource_address.rstrip("/")
        if not (address == base or address.startswith(base + "/")):
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, f"Resource address is not served by this service: {resource_address}")
        return pb2_version.IsVersionedResponse(is_versioned=is_versioning_enabled())

    cls = type(
        "VersioningService",
        (pb2_grpc_version.VersioningServiceServicer,),
        {
            "EnumerateVersions": EnumerateVersions,
            "IsVersioned": IsVersioned,
        },
    )
    return cls()
