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

"""FileObjectService implementation for gRPC.

This module implements the FileObjectService from the Storage API, which
provides core file operations:
- Read: Download file content by resource identity
- ReadFromAddress: Download file content by resource address
- Write: Upload file content (body, redirect, or multipart)
- Delete: Remove a file
- Stat: Get file metadata
- Enumerate: Recursively list files in a directory tree
- Copy: Copy a file (v1alpha only)
- Move: Move/rename a file (v1alpha only)

The service supports multiple upload methods:
- Body: Small files streamed directly in the gRPC request
- Redirect: Medium files uploaded via HTTP redirect
- Multipart: Large files uploaded in multiple parts

Version management is automatic - each write creates a new immutable version.
"""

import contextlib
import os
import uuid
from urllib.parse import quote_plus

import grpc
from local_filesystem_service.backends.storage_backend_interface import (
    REDIRECT_NATIVE_METADATA_HEADER,
    EtagMismatchError,
    FolderMode,
)
from local_filesystem_service.filesystem import (
    NativeMetadataSupportAddressStatus,
    classify_native_metadata_support_address,
    effective_native_metadata_support,
    get_backend,
    is_read_only,
    is_versioning_enabled,
)
from local_filesystem_service.grpc_service.message_helpers import (
    metadata_message,
    native_metadata_from_proto,
    resource_info,
)
from local_filesystem_service.native_metadata import (
    NativeMetadataUnsupportedError,
    response_support_scope_for_identity,
    validate_and_gate_native_metadata_for_write,
)
from nvidia.omniverse.storage.fileobject.v1alpha import (
    fileobject_pb2 as fileobject_pb2_v1alpha,
)
from nvidia.omniverse.storage.fileobject.v1alpha import (
    fileobject_service_pb2 as fileobject_service_pb2_v1alpha,
)


def check_optimistic_locking_support(context, operation_name: str = "This operation"):
    """Check if the current backend supports optimistic locking (previous_version).

    Args:
        context: gRPC context to abort with error if not supported
        operation_name: Name of the operation for error message

    Raises:
        grpc.RpcError with UNIMPLEMENTED if backend doesn't support optimistic locking
    """
    backend = get_backend()
    support = backend.get_optimistic_locking_support()
    # Check if any optimistic locking is supported
    if not (support.write or support.delete or support.copy or support.move):
        context.abort(
            grpc.StatusCode.UNIMPLEMENTED,
            details=f"{operation_name} with previous_version parameter is not supported by {type(backend).__name__} backend. "
            f"Optimistic locking is only available with backends that support it.",
        )


def abort_if_read_only(context, operation_name: str = "This operation"):
    """Reject a mutating operation when the backend is served read-only.

    Args:
        context: gRPC context to abort with error if the backend is read-only.
        operation_name: Name of the operation for the error message.

    Raises:
        grpc.RpcError with FAILED_PRECONDITION if the backend is read-only.
    """
    if is_read_only():
        context.abort(
            grpc.StatusCode.FAILED_PRECONDITION,
            details=f"{operation_name} is not allowed: the storage is read-only.",
        )


def abort_on_previous_version_not_latest(resource_address, previous_version: str, context):
    """Validate that a specified version is still the latest version.

    This implements optimistic concurrency control for write operations. When
    a client specifies a previous_version (expected current state), this
    function verifies that version is still the latest. If not, it aborts
    the gRPC request with FAILED_PRECONDITION, preventing lost updates.

    Args:
        resource_address: Storage API resource address to check.
        previous_version: Encoded identity string of the expected latest version.
        context: gRPC ServicerContext for aborting the request.

    Raises:
        grpc.RpcError: Aborts with FAILED_PRECONDITION if:
            - The specified version is not the latest version
            - No version exists at the resource address

    Note:
        This is used in Write, Delete, Copy, and Move operations to ensure
        the client's view of the resource state is current.
    """
    backend = get_backend()
    # Check that the specified version is still the latest!
    try:
        # Get current latest version
        current_version_info = backend.stat(resource_address)
        current_identity = current_version_info.resource_identity

        if current_identity != previous_version:
            context.abort(
                grpc.StatusCode.FAILED_PRECONDITION,
                details="specified previous version is not latest version",
            )
    except FileNotFoundError:
        context.abort(
            grpc.StatusCode.FAILED_PRECONDITION,
            details="specified previous version, but no version was found at the address",
        )


def abort_on_native_metadata_not_usable(resource_address: str, native_metadata: dict, backend, context) -> None:
    try:
        validate_and_gate_native_metadata_for_write(resource_address, native_metadata, backend=backend)
    except ValueError as e:
        context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(e))
    except NativeMetadataUnsupportedError as e:
        context.abort(grpc.StatusCode.UNIMPLEMENTED, str(e))


def abort_on_native_metadata_support_query_not_authorized(resource_address: str, backend, context) -> None:
    status = classify_native_metadata_support_address(resource_address, backend=backend)
    if status == NativeMetadataSupportAddressStatus.OK:
        return
    if status in (
        NativeMetadataSupportAddressStatus.INVALID_ADDRESS,
        NativeMetadataSupportAddressStatus.VERSION_ADDRESS,
    ):
        context.abort(
            grpc.StatusCode.INVALID_ARGUMENT,
            f"Invalid native metadata support query address: {resource_address}",
        )
    if status == NativeMetadataSupportAddressStatus.PERMISSION_DENIED:
        context.abort(
            grpc.StatusCode.PERMISSION_DENIED,
            f"Permission denied for native metadata support query address: {resource_address}",
        )


def make_fileobject_service_servicer(
    STATIC_DIR: str,
    REDIRECT_HOST: str,
    fileobject_version,
    pb2_version,
    pb2_grpc_version,
    redirect_port: int,
    version_tag: str,
):
    """Create a dynamic FileObjectService servicer for gRPC.

    This factory function creates a gRPC servicer class that implements the
    FileObjectService interface with all core file operations. It adapts to
    different API versions (v1alpha includes Copy/Move, v1beta does not).

    Args:
        STATIC_DIR: Base directory for file storage and temporary uploads.
        REDIRECT_HOST: Hostname for constructing redirect URLs (e.g., 'http://localhost').
        fileobject_version: The fileobject protobuf module containing message types.
        pb2_version: The fileobject service protobuf module containing request/response types.
        pb2_grpc_version: The fileobject gRPC module containing the servicer base class.
        redirect_port: Port number for HTTP redirect endpoints.
        version_tag: API version string ('v1alpha' or 'v1beta').
                    - v1alpha includes Copy and Move methods
                    - v1beta includes only stable operations

    Returns:
        An instance of a dynamically created FileObjectServiceServicer class
        with all file operation methods implemented for the specified version.
    """

    def Enumerate(self, request, context):
        """Recursively enumerate all files under a directory tree.

        Lists all files (not folders) recursively within the specified directory.
        Results are streamed in batches (yielded responses).

        Args:
            request: EnumerateRequest containing:
                    - resource_address: Directory address to enumerate
            context: gRPC ServicerContext for the request.

        Yields:
            EnumerateResponse messages, each containing a batch of AddressInfo
            items with resource addresses and metadata.

        Raises:
            grpc.RpcError with:
            - INVALID_ARGUMENT: If resource_address is versioned
            - NOT_FOUND: If resource_address doesn't exist or is a file

        Note:
            This is recursive (unlike FileFolderService.List which is non-recursive).
            Large directory trees are streamed in batches for efficiency.
        """
        if get_backend().is_version_address(request.resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                "Versioned addresses can not be enumerated.",
            )
        if not get_backend().exists(request.resource_address):
            context.abort(grpc.StatusCode.NOT_FOUND, "Resource address not found")
        if get_backend().is_file(request.resource_address):
            context.abort(grpc.StatusCode.NOT_FOUND, "Cannot enumerate file addresses")

        for entries in get_backend().enumerate(request.resource_address):
            # Each entry is already a ListEntry with resource_address and metadata
            address_infos = []
            for entry in entries:
                # Convert to the gRPC response format
                ai = fileobject_version.AddressInfo(
                    resource_address=entry.resource_address,
                    metadata=(
                        metadata_message(
                            fileobject_version,
                            entry.metadata,
                            native_metadata_support_scope=entry.resource_address,
                        )
                        if entry.metadata
                        else None
                    ),
                )
                address_infos.append(ai)
            yield pb2_version.EnumerateResponse(items=address_infos)

    def Stat(self, request, context):
        """Get metadata for a file resource.

        Retrieves the resource identity (opaque version identifier) and metadata
        (size, modification time) for the latest version of a file.

        Args:
            request: StatRequest containing:
                    - resource_address: File address to query
            context: gRPC ServicerContext for the request.

        Returns:
            StatResponse containing:
            - resource_info: ResourceInfo with identity and metadata

        Raises:
            grpc.RpcError with:
            - INVALID_ARGUMENT: If resource_address is invalid
            - NOT_FOUND: If resource is a directory or doesn't exist
            - PERMISSION_DENIED: If read permission is denied

        Note:
            This returns information about the latest version of the file.
        """
        if not get_backend().is_address_valid(request.resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                f"Invalid resource address: {request.resource_address}",
            )
        try:
            # Call stat to check for file found and permission
            get_backend().stat(request.resource_address)
            # Calculate response
            return pb2_version.StatResponse(
                resource_info=resource_info(
                    fileobject_version,
                    request.resource_address,
                    native_metadata_support_scope=request.resource_address,
                )
            )
        except IsADirectoryError as e:
            context.abort(grpc.StatusCode.NOT_FOUND, f"{str(e)} is a directory")
        except PermissionError as e:
            context.abort(grpc.StatusCode.PERMISSION_DENIED, str(e))
        except FileNotFoundError as e:
            context.abort(grpc.StatusCode.NOT_FOUND, str(e))

    def Read(self, request, context):
        """Download file content by resource identity.

        Retrieves a specific file version by its opaque resource identity.
        Supports both direct body streaming and HTTP redirect for downloads.

        Args:
            request: ReadRequest containing:
                    - resource_identity: Opaque identifier for specific version
                    - download_preference: BODY (stream in response) or
                                          REDIRECT (provide HTTP download URL)
            context: gRPC ServicerContext for the request.

        Yields:
            ReadResponse messages containing:
            - First response: metadata (size, modification time)
            - Subsequent responses (if BODY): chunks of file content
            - Or single redirect response (if REDIRECT): download URL

        Raises:
            grpc.RpcError with:
            - INVALID_ARGUMENT: If identity is invalid or download_preference unknown
            - NOT_FOUND: If resource identity doesn't exist
            - PERMISSION_DENIED: If read permission is denied

        Note:
            This reads a specific version by identity (immutable).
            Use ReadFromAddress to always get the latest version.
        """
        backend = get_backend()
        resource_identity_str = request.resource_identity.encoded_identity

        # Use backend's stat_identity to validate and get metadata
        try:
            version_info = backend.stat_identity(resource_identity_str)
        except ValueError as e:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(e))
            return
        except FileNotFoundError as e:
            context.abort(grpc.StatusCode.NOT_FOUND, str(e))
            return
        except (PermissionError, IsADirectoryError) as e:
            context.abort(grpc.StatusCode.PERMISSION_DENIED, str(e))
            return

        support_scope = response_support_scope_for_identity(resource_identity_str, version_info.metadata, backend=backend)
        metadata = metadata_message(
            fileobject_version,
            version_info.metadata,
            native_metadata_support_scope=support_scope,
            backend=backend,
        )

        # Yield metadata first
        yield pb2_version.ReadResponse(metadata=metadata)

        # Handle download preference
        if request.download_preference in [
            pb2_version.DownloadPreference.DOWNLOAD_PREFERENCE_BODY,
            pb2_version.DownloadPreference.DOWNLOAD_PREFERENCE_UNSPECIFIED,
        ] or (
            request.download_preference == pb2_version.DownloadPreference.DOWNLOAD_PREFERENCE_REDIRECT
            and not backend.supports_redirect_download()
        ):
            # Stream content using backend's read_from_identity
            try:
                for chunk in backend.read_from_identity(resource_identity_str):
                    yield pb2_version.ReadResponse(chunk=fileobject_version.Chunk(chunk=chunk))
            except (FileNotFoundError, PermissionError) as e:
                context.abort(grpc.StatusCode.PERMISSION_DENIED, str(e))
        elif request.download_preference == pb2_version.DownloadPreference.DOWNLOAD_PREFERENCE_REDIRECT:
            try:
                url = backend.construct_redirect_url_for_identity(resource_identity_str, REDIRECT_HOST, redirect_port)
                yield pb2_version.ReadResponse(redirect=fileobject_version.Redirect(redirect_target_url=url, additional_headers=[]))
            except (ValueError, OSError, NotImplementedError) as e:
                context.abort(grpc.StatusCode.INTERNAL, f"Failed to construct redirect URL: {e}")
        else:
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                f"download preference {request.download_preference} is not a valid input",
            )

    def ReadFromAddress(self, request, context):
        """Download file content by resource address (latest version).

        Retrieves the latest version of a file by its resource address.
        Supports both direct body streaming and HTTP redirect for downloads.

        Args:
            request: ReadFromAddressRequest containing:
                    - resource_address: File address to download
                    - download_preference: BODY (stream in response) or
                                          REDIRECT (provide HTTP download URL)
            context: gRPC ServicerContext for the request.

        Yields:
            ReadFromAddressResponse messages containing:
            - First response: resource_info with identity and metadata
            - Subsequent responses (if BODY): chunks of file content
            - Or single redirect response (if REDIRECT): download URL

        Raises:
            grpc.RpcError with:
            - INVALID_ARGUMENT: If address is invalid or download_preference unknown
            - NOT_FOUND: If resource doesn't exist or is a folder
            - PERMISSION_DENIED: If read permission is denied

        Note:
            This always returns the latest version. Use Read with a specific
            identity to retrieve historical versions.
        """
        if not get_backend().is_address_valid(request.resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                f"Invalid resource address: {request.resource_address}",
            )

        if get_backend().is_dir(request.resource_address):
            error_code_to_use = (
                grpc.StatusCode.NOT_FOUND if get_backend().folder_mode() == FolderMode.HYBRID else grpc.StatusCode.INVALID_ARGUMENT
            )
            context.abort(
                error_code_to_use,
                f"Resource address is folder, can't be read: {request.resource_address}. No regular object found at that address.",
            )

        backend = get_backend()
        if backend.exists(request.resource_address):
            try:
                backend.check_read_permission_on_address(request.resource_address)
                yield pb2_version.ReadFromAddressResponse(
                    resource_info=resource_info(
                        fileobject_version,
                        request.resource_address,
                        native_metadata_support_scope=request.resource_address,
                        backend=backend,
                    )
                )
                if request.download_preference in [
                    pb2_version.DownloadPreference.DOWNLOAD_PREFERENCE_BODY,
                    pb2_version.DownloadPreference.DOWNLOAD_PREFERENCE_UNSPECIFIED,
                ] or (
                    request.download_preference == pb2_version.DownloadPreference.DOWNLOAD_PREFERENCE_REDIRECT
                    and not backend.supports_redirect_download()
                ):
                    for chunk in backend.read_from_address(request.resource_address):
                        yield pb2_version.ReadFromAddressResponse(chunk=fileobject_version.Chunk(chunk=chunk))
                elif request.download_preference == pb2_version.DownloadPreference.DOWNLOAD_PREFERENCE_REDIRECT:
                    url = backend.construct_redirect_url(request.resource_address, REDIRECT_HOST, redirect_port)
                    yield pb2_version.ReadFromAddressResponse(
                        redirect=fileobject_version.Redirect(redirect_target_url=url, additional_headers=[])
                    )
                else:
                    context.abort(
                        grpc.StatusCode.INVALID_ARGUMENT,
                        f"download preference {request.download_preference} is not a valid input",
                    )
            except (PermissionError, IsADirectoryError) as e:
                context.abort(code=grpc.StatusCode.PERMISSION_DENIED, details=str(e))
        else:
            context.abort(code=grpc.StatusCode.NOT_FOUND, details="Resource not found")

    def _create_multipart_upload(
        resource_address: str,
        expected_head_version: str | None,
        fail_if_exists: bool = False,
        native_metadata: dict | None = None,
    ) -> pb2_version.CreateMultipartUploadResponse:
        """Create a new multipart upload session.

        Initializes a multipart upload by creating a unique upload ID and
        preparing storage for the parts.

        Args:
            resource_address: Destination address for the file being uploaded.
            expected_head_version: Optional expected current version (for optimistic locking).
            fail_if_exists: If True, the final CompleteMultipartUpload will fail
                with ALREADY_EXISTS when a resource exists at the destination.
            native_metadata: Optional native metadata to persist with the
                completed object version.

        Returns:
            CreateMultipartUploadResponse containing:
            - upload_id: Unique identifier for this upload session
            - first_part_write_redirect: Redirect URL for uploading part 0
            - minimum_size_per_part: Minimum bytes per part (0)
            - maximum_size_per_part: Maximum bytes per part (4 MiB)
        """
        backend = get_backend()
        new_upload_id = str(uuid.uuid4())
        backend.create_upload_session(new_upload_id, native_metadata=native_metadata)
        return pb2_version.CreateMultipartUploadResponse(
            upload_id=backend.encode_upload_id(new_upload_id, expected_head_version, fail_if_exists),
            first_part_write_redirect=_redirect_for_part(new_upload_id, resource_address, 0),
            minimum_size_per_part=0,
            maximum_size_per_part=4 * 1024 * 1024,
        )

    def FetchWriteTypeInfo(self, request, context):
        """Query recommended upload methods for different file sizes.

        Returns size-based recommendations for which upload method (body,
        redirect, or multipart) should be used for files of various sizes.

        For Nucleus backend: Only BODY upload is supported.
        For Filesystem backend: BODY, REDIRECT, and MULTIPART are supported.

        Args:
            request: FetchWriteTypeInfoRequest containing:
                    - destination_resource_address: Target address for upload
            context: gRPC ServicerContext for the request.

        Returns:
            FetchWriteTypeInfoResponse with write_type_intervals based on backend:

            Filesystem backend:
            - 0-1 KiB: BODY (stream in gRPC request)
            - 1 KiB-1 MiB: REDIRECT (upload via HTTP)
            - 1 MiB+: MULTIPART (upload in multiple parts)

            Nucleus backend:
            - All sizes: BODY (stream in gRPC request)

        Raises:
            grpc.RpcError with:
            - INVALID_ARGUMENT: If destination is a versioned address

        Note:
            These are recommendations. Clients can choose different methods
            but should respect the multipart size limits.
        """
        if get_backend().is_version_address(request.destination_resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                "Versioned addresses can not be written to.",
            )

        # Check if backend supports redirects and multipart uploads
        backend = get_backend()
        supports_redirects = backend.supports_redirect_upload()
        supports_multipart = backend.supports_multipart_upload()

        if supports_redirects:
            # Filesystem backend - supports body, redirect, and multipart
            intervals = [
                pb2_version.WriteTypeForSizeInterval(
                    minimum_data_object_size=0,
                    maximum_data_object_size=1024,
                    preferred_upload_method=pb2_version.UploadPreference.UPLOAD_PREFERENCE_BODY,
                ),
                pb2_version.WriteTypeForSizeInterval(
                    minimum_data_object_size=1024,
                    maximum_data_object_size=1024 * 1024,
                    preferred_upload_method=pb2_version.UploadPreference.UPLOAD_PREFERENCE_REDIRECT,
                ),
                pb2_version.WriteTypeForSizeInterval(
                    minimum_data_object_size=1024 * 1024,
                    maximum_data_object_size=pow(2, 53),
                    preferred_upload_method=pb2_version.UploadPreference.UPLOAD_PREFERENCE_MULTIPART,
                ),
            ]
        else:
            # Nucleus or other backends - only support body upload
            intervals = [
                pb2_version.WriteTypeForSizeInterval(
                    minimum_data_object_size=0,
                    maximum_data_object_size=pow(2, 53),
                    preferred_upload_method=pb2_version.UploadPreference.UPLOAD_PREFERENCE_BODY,
                ),
            ]

        return pb2_version.FetchWriteTypeInfoResponse(write_type_intervals=intervals)

    def GetNativeMetadataSupport(self, request, context):
        """Query native metadata support for a resource address."""
        backend = get_backend()
        abort_on_native_metadata_support_query_not_authorized(request.resource_address, backend, context)
        return pb2_version.GetNativeMetadataSupportResponse(
            supports_native_metadata=effective_native_metadata_support(
                request.resource_address,
                backend=backend,
            )
        )

    def IsReadOnly(self, request, context):
        """Query whether the given resource address is read-only."""
        resource_address = request.resource_address
        base = get_backend().base_uri.rstrip("/")
        address = resource_address.rstrip("/")
        if not (address == base or address.startswith(base + "/")):
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, f"Resource address is not served by this service: {resource_address}")
        return pb2_version.IsReadOnlyResponse(is_read_only=is_read_only())

    def Write(self, request_iterator, context):
        """Upload file content to create a new version.

        Handles file uploads using multiple methods:
        - BODY: Small files streamed directly in the request
        - REDIRECT: Medium files uploaded via HTTP redirect
        - MULTIPART: Large files uploaded in multiple parts

        The method is determined by the upload_preference in the first request,
        or auto-selected based on data_object_size.

        Args:
            request_iterator: Iterator of WriteRequest messages:
                - First request: WriteParameters with destination, size, preferences
                - Subsequent requests: Chunk messages with file data (for BODY method)
            context: gRPC ServicerContext for the request.

        Yields:
            WriteResponse messages:
            - For BODY: WriteChunksAccepted, then final resource_info
            - For REDIRECT: WriteRedirectProperties with upload URL
            - For MULTIPART: CreateMultipartUploadResponse

        Raises:
            grpc.RpcError with FAILED_PRECONDITION if the backend is read-only.

        Raises:
            grpc.RpcError with:
            - INVALID_ARGUMENT: If params invalid, address versioned, or chunks out of order
            - FAILED_PRECONDITION: If previous_version doesn't match current version
            - PERMISSION_DENIED: If write permission denied

        Note:
            Supports optimistic locking via previous_version parameter.
            Each write creates a new immutable version.
        """
        abort_if_read_only(context, "Write")
        destination_address = None
        upload_preference = pb2_version.UploadPreference.UPLOAD_PREFERENCE_UNSPECIFIED
        path = None
        file = None
        write_fail_if_exists = False
        previous_version = None
        native_metadata = {}
        try:
            with contextlib.ExitStack() as exit_stack:
                for request in request_iterator:
                    if request.HasField("params"):
                        if path is not None:
                            context.abort(
                                grpc.StatusCode.INVALID_ARGUMENT,
                                "Supply WriteRequest params only once per upload!",
                            )
                        data_size = request.params.data_object_size
                        if request.params.HasField("upload_preference"):
                            upload_preference = request.params.upload_preference
                        if "native_metadata" in request.params.DESCRIPTOR.fields_by_name:
                            try:
                                native_metadata = native_metadata_from_proto(request.params.native_metadata)
                            except ValueError as e:
                                context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(e))

                        if upload_preference == pb2_version.UploadPreference.UPLOAD_PREFERENCE_UNSPECIFIED:
                            if 0 <= data_size < 1024:
                                # Small file, use chunk method
                                upload_preference = pb2_version.UploadPreference.UPLOAD_PREFERENCE_BODY
                            elif 1024 <= data_size < 1024 * 1024:
                                # Medium file, use redirect method
                                upload_preference = pb2_version.UploadPreference.UPLOAD_PREFERENCE_REDIRECT
                            elif data_size >= 1024 * 1024:
                                # Large file, upload this in parts
                                upload_preference = pb2_version.UploadPreference.UPLOAD_PREFERENCE_MULTIPART
                            else:
                                context.abort(
                                    grpc.StatusCode.INVALID_ARGUMENT,
                                    details="data_object_size invalid",
                                )

                        # Compute fail_if_exists before validating the previous_version
                        # so the mutual-exclusivity check happens first and we don't
                        # leak version info on a malformed request.
                        write_fail_if_exists = bool(request.params.HasField("fail_if_exists") and request.params.fail_if_exists)

                        if request.params.HasField("previous_version") and write_fail_if_exists:
                            context.abort(
                                grpc.StatusCode.INVALID_ARGUMENT,
                                details="fail_if_exists and previous_version are mutually exclusive",
                            )
                            return

                        if request.params.HasField("previous_version"):
                            previous_version = request.params.previous_version.encoded_identity
                            # Check if backend supports optimistic locking
                            check_optimistic_locking_support(context, "Write operation")

                            # Check that the specified version is still the latest!
                            abort_on_previous_version_not_latest(
                                request.params.destination_resource_address,
                                previous_version,
                                context,
                            )

                        if write_fail_if_exists:
                            backend = get_backend()
                            if not backend.get_optimistic_locking_support().write_fail_if_exists:
                                context.abort(
                                    grpc.StatusCode.UNIMPLEMENTED,
                                    details=f"Write operation with fail_if_exists is not supported by {type(backend).__name__} backend.",
                                )

                        if not get_backend().is_address_valid(request.params.destination_resource_address):
                            context.abort(
                                grpc.StatusCode.INVALID_ARGUMENT,
                                "destination address is not valid!",
                            )

                        if get_backend().is_version_address(request.params.destination_resource_address):
                            context.abort(
                                grpc.StatusCode.INVALID_ARGUMENT,
                                f"Cannot write to individual version address: {request.params.destination_resource_address}",
                            )

                        # Check if backend supports redirect/multipart uploads
                        backend = get_backend()
                        try:
                            backend.check_write_permission_on_address(request.params.destination_resource_address)
                        except ValueError as e:
                            context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(e))
                        except (PermissionError, IsADirectoryError, OSError) as e:
                            context.abort(grpc.StatusCode.PERMISSION_DENIED, f"Can't write to that resource address: {e}")
                        abort_on_native_metadata_not_usable(
                            request.params.destination_resource_address,
                            native_metadata,
                            backend,
                            context,
                        )
                        supports_redirects = backend.supports_redirect_upload()
                        supports_multipart = backend.supports_multipart_upload()

                        # If client requested redirect/multipart but backend doesn't support it, use body instead
                        if not supports_redirects and upload_preference == pb2_version.UploadPreference.UPLOAD_PREFERENCE_REDIRECT:
                            upload_preference = pb2_version.UploadPreference.UPLOAD_PREFERENCE_BODY
                        if not supports_multipart and upload_preference == pb2_version.UploadPreference.UPLOAD_PREFERENCE_MULTIPART:
                            upload_preference = pb2_version.UploadPreference.UPLOAD_PREFERENCE_BODY

                        destination_address = request.params.destination_resource_address

                        if write_fail_if_exists and upload_preference in (
                            pb2_version.UploadPreference.UPLOAD_PREFERENCE_REDIRECT,
                            pb2_version.UploadPreference.UPLOAD_PREFERENCE_MULTIPART,
                        ):
                            # The redirect path uploads via a separate HTTP
                            # endpoint that doesn't see the WriteRequest, so
                            # the fail_if_exists check has to happen here
                            # before the redirect is issued. Multipart's
                            # CompleteMultipartUpload re-checks via
                            # decode_upload_id for defense in depth against
                            # races between this check and finalization.
                            if get_backend().exists(destination_address):
                                context.abort(
                                    grpc.StatusCode.ALREADY_EXISTS,
                                    details=f"Resource already exists at {destination_address}",
                                )

                        if upload_preference == pb2_version.UploadPreference.UPLOAD_PREFERENCE_BODY:
                            # For body upload, collect chunks and write to backend
                            if supports_redirects:
                                # Backend supports redirects: use temporary file
                                relative_path = backend.force_relative_path(destination_address)
                                # Use a unique temp file per request to avoid race conditions
                                # when multiple uploads target the same destination concurrently
                                unique_suffix = uuid.uuid4().hex
                                path = str(os.path.join(backend._uploads_dir, "upload_stream", f"{relative_path}.{unique_suffix}"))
                                backend.safe_makedirs(backend.safe_dirname(path), exist_ok=True)
                                file = exit_stack.enter_context(backend.safe_open(path, "wb"))
                            else:
                                # Other backends (e.g., Nucleus): collect in memory
                                file = None
                                file_content = bytearray()
                            yield pb2_version.WriteResponse(write_chunks_accepted=pb2_version.WriteChunksAccepted())
                        elif upload_preference == pb2_version.UploadPreference.UPLOAD_PREFERENCE_REDIRECT:
                            additional_headers = []
                            redirect_upload_token = backend.create_redirect_upload(
                                native_metadata,
                                resource_address=destination_address,
                                previous_version=previous_version,
                                fail_if_exists=write_fail_if_exists,
                            )
                            upload_route = f"/upload-token/{redirect_upload_token}/" if redirect_upload_token is not None else "/upload/"
                            url = f"{REDIRECT_HOST}:{redirect_port}{upload_route}" + quote_plus(destination_address.replace("\\", "/"))
                            if redirect_upload_token is not None:
                                additional_headers.append(
                                    fileobject_version.Header(
                                        name=REDIRECT_NATIVE_METADATA_HEADER,
                                        value=redirect_upload_token,
                                    )
                                )
                            yield pb2_version.WriteResponse(
                                write_redirect=pb2_version.WriteRedirectProperties(
                                    redirect_target_url=url,
                                    method=pb2_version.UploadMethod.UPLOAD_METHOD_POST,
                                    additional_headers=additional_headers,
                                    completion_header_names=["x-nvidia-storage-upload-location"],
                                )
                            )
                            return
                        elif upload_preference == pb2_version.UploadPreference.UPLOAD_PREFERENCE_MULTIPART:
                            yield pb2_version.WriteResponse(
                                multipart_upload=_create_multipart_upload(
                                    destination_address,
                                    previous_version,
                                    fail_if_exists=write_fail_if_exists,
                                    native_metadata=native_metadata,
                                )
                            )
                            return

                    elif request.HasField("chunk"):
                        if destination_address is None:
                            context.abort(
                                code=grpc.StatusCode.INVALID_ARGUMENT,
                                details="No WriteParameters message received before Chunk",
                            )
                        if upload_preference == pb2_version.UploadPreference.UPLOAD_PREFERENCE_BODY:
                            if supports_redirects:
                                # FileSystemProvider: write to file
                                if file is None:
                                    context.abort(
                                        code=grpc.StatusCode.INVALID_ARGUMENT,
                                        details="Failed to specify upload metadata first or file could not be written to!",
                                    )
                                    return
                                file.write(request.chunk.chunk)
                            else:
                                # Other backends: accumulate in memory
                                file_content.extend(request.chunk.chunk)
                        else:
                            context.abort(
                                code=grpc.StatusCode.INVALID_ARGUMENT,
                                details="Don't upload chunked data for the given upload method",
                            )
                if upload_preference == pb2_version.UploadPreference.UPLOAD_PREFERENCE_BODY:
                    # Upload complete, determine metadata and resource info and return it
                    if destination_address is None:
                        context.abort(
                            code=grpc.StatusCode.INVALID_ARGUMENT,
                            details="No WriteParameters message received before Chunk",
                        )
                        return

                    if supports_redirects:
                        # Backend supports redirects: read from temp file and write to backend
                        if file is None or path is None:
                            context.abort(
                                code=grpc.StatusCode.INVALID_ARGUMENT,
                                details="Failed to specify upload metadata first or file could not be written to!",
                            )
                            return
                        # Close the file explicitly before reading it back to ensure all data is on disk
                        file.close()
                        try:
                            # Now copy the temporary file into the content and versions directories
                            with backend.safe_open(path, "rb") as f:
                                data = f.read()
                                backend.write_version(
                                    destination_address,
                                    data,
                                    previous_version=previous_version,
                                    fail_if_exists=write_fail_if_exists,
                                    native_metadata=native_metadata,
                                )
                        finally:
                            # Clean up the unique temp file
                            if os.path.exists(path):
                                os.unlink(path)
                    else:
                        # Other backends (e.g., Nucleus): write accumulated content directly
                        data = bytes(file_content)
                        get_backend().write_version(
                            destination_address,
                            data,
                            previous_version=previous_version,
                            fail_if_exists=write_fail_if_exists,
                            native_metadata=native_metadata,
                        )

                    yield pb2_version.WriteResponse(
                        resource_info=resource_info(
                            fileobject_version,
                            destination_address,
                            native_metadata_support_scope=destination_address,
                        )
                    )
        except FileExistsError as e:
            context.abort(
                grpc.StatusCode.ALREADY_EXISTS,
                details=str(e),
            )
        except EtagMismatchError as e:
            context.abort(grpc.StatusCode.FAILED_PRECONDITION, details=str(e))
        except (PermissionError, IsADirectoryError, OSError) as e:
            context.abort(
                grpc.StatusCode.PERMISSION_DENIED,
                details=f"Can't write to that resource address: {e}",
            )
        except grpc.RpcError:
            # Re-raise intentional aborts from context.abort() - don't convert to INTERNAL
            raise

    def CompleteRedirectUpload(self, request, context):
        """Complete a redirect-based upload and finalize the new version.

        After a client uploads via HTTP redirect, this method verifies the
        upload completion headers and returns the resource info for the new version.

        Args:
            request: CompleteRedirectUploadRequest containing:
                    - destination_resource_address: Target address
                    - additional_headers: Completion headers from HTTP upload
                                        (must include 'x-nvidia-storage-upload-location')
            context: gRPC ServicerContext for the request.

        Returns:
            CompleteRedirectUploadResponse containing:
            - resource_info: Identity and metadata for the new version

        Raises:
            grpc.RpcError with:
            - UNIMPLEMENTED: If backend doesn't support redirect uploads
            - INVALID_ARGUMENT: If address invalid or required headers missing/invalid
            - NOT_FOUND: If uploaded file cannot be found

        Note:
            The 'x-nvidia-storage-upload-location' header contains the
            version path created by the HTTP upload endpoint.
        """
        # Check if backend supports redirect uploads
        backend = get_backend()
        if not backend.supports_redirect_upload():
            context.abort(
                grpc.StatusCode.UNIMPLEMENTED,
                details=f"Redirect uploads not supported by {type(backend).__name__} backend",
            )

        if not backend.exists(request.destination_resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details=f"Invalid argument: {request.destination_resource_address}",
            )

        # Check that the client provides the correct upload headers for this redirect to be valid
        if len(request.additional_headers) == 0:
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details="No additional headers received",
            )

        # Convert headers to dict
        headers_dict = {h.name: h.value for h in request.additional_headers}

        try:
            result = backend.complete_redirect_upload(request.destination_resource_address, headers_dict)

            metadata = metadata_message(
                fileobject_version,
                result.metadata,
                native_metadata_support_scope=request.destination_resource_address,
                backend=backend,
            )
            resource_identity = fileobject_version.ResourceIdentity(encoded_identity=result.resource_identity)

            return pb2_version.CompleteRedirectUploadResponse(
                resource_info=fileobject_version.ResourceInfo(resource_identity=resource_identity, metadata=metadata)
            )
        except ValueError as e:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(e))
        except FileNotFoundError as e:
            context.abort(grpc.StatusCode.NOT_FOUND, str(e))

    def _redirect_for_part(upload_id, destination_resource_address, part_number):
        """Generate redirect properties for uploading a multipart part.

        Args:
            upload_id: Unique upload session identifier.
            destination_resource_address: Target address for the complete file.
            part_number: Zero-based part number.

        Returns:
            WriteRedirectProperties with:
            - redirect_target_url: HTTP URL for uploading this part
            - method: POST
            - completion_header_names: ['local-file'] to be returned after upload
            - additional_headers: Demo headers
        """
        backend = get_backend()
        redirect_props = backend.construct_upload_part_redirect(
            upload_id, part_number, destination_resource_address, REDIRECT_HOST, redirect_port
        )
        return pb2_version.WriteRedirectProperties(
            method=pb2_version.UploadMethod.UPLOAD_METHOD_POST,
            redirect_target_url=redirect_props["redirect_target_url"],
            completion_header_names=redirect_props["completion_header_names"],
            additional_headers=[fileobject_version.Header(name=h[0], value=h[1]) for h in redirect_props["additional_headers"]],
        )

    def UploadPart(self, request, context):
        """Get redirect URLs for uploading one or more multipart parts.

        Returns HTTP redirect URLs for uploading the specified part(s) of
        a multipart upload.

        Args:
            request: UploadPartRequest containing:
                    - upload_id: Upload session identifier
                    - destination_resource_address: Target address
                    - part_number: Starting part number
                    - part_count: Optional number of consecutive parts (default 1)
            context: gRPC ServicerContext for the request.

        Returns:
            UploadPartResponse containing:
            - part_write_redirects: List of WriteRedirectProperties,
                                   one for each part

        Note:
            part_count allows batch-requesting redirect URLs for efficiency.
        """
        # Check if backend supports multipart uploads
        backend = get_backend()
        if not backend.supports_multipart_upload():
            context.abort(
                grpc.StatusCode.UNIMPLEMENTED,
                details=f"Multipart uploads not supported by {type(backend).__name__} backend",
            )

        num_parts = 1
        upload_id, _, _ = backend.decode_upload_id(request.upload_id)
        if request.HasField("part_count"):
            num_parts = request.part_count
        redirects = []
        for p in range(num_parts):
            part_number = request.part_number + p
            redirects.append(_redirect_for_part(upload_id, request.destination_resource_address, part_number))

        return pb2_version.UploadPartResponse(part_write_redirects=redirects)

    def CompleteMultipartUpload(self, request: pb2_version.CompleteMultipartUploadRequest, context):
        """Finalize a multipart upload by assembling all parts.

        Assembles the uploaded parts into the final file, creates a new version,
        and cleans up the upload session.

        Args:
            request: CompleteMultipartUploadRequest containing:
                    - upload_id: Upload session identifier
                    - destination_resource_address: Target address
                    - parts: Ordered list of PartInfo with headers from uploads
            context: gRPC ServicerContext for the request.

        Returns:
            CompleteMultipartUploadResponse containing:
            - resource_info: Identity and metadata for the new version

        Raises:
            grpc.RpcError with:
            - UNIMPLEMENTED: If backend doesn't support multipart uploads
            - INVALID_ARGUMENT: If address invalid, parts out of order, or headers missing
            - FAILED_PRECONDITION: If upload_id doesn't exist or previous_version doesn't match

        Note:
            Parts must be provided in order (part_number 0, 1, 2, ...).
            The upload session directory is cleaned up after completion.
        """
        # Check if backend supports multipart uploads
        backend = get_backend()
        if not backend.supports_multipart_upload():
            context.abort(
                grpc.StatusCode.UNIMPLEMENTED,
                details=f"Multipart uploads not supported by {type(backend).__name__} backend",
            )

        if not backend.is_address_valid(request.destination_resource_address):
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, f"Invalid resource address: {request.destination_resource_address}")
        if backend.is_version_address(request.destination_resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                f"Cannot write to individual version address: {request.destination_resource_address}",
            )
        try:
            backend.check_write_permission_on_address(request.destination_resource_address)
        except ValueError as e:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(e))
        except (PermissionError, IsADirectoryError, OSError) as e:
            context.abort(grpc.StatusCode.PERMISSION_DENIED, f"Can't write to that resource address: {e}")
        assembled_data = bytearray()

        upload_id, previous_version, fail_if_exists = backend.decode_upload_id(request.upload_id)
        if not backend.upload_session_exists(upload_id):
            context.abort(
                grpc.StatusCode.FAILED_PRECONDITION,
                f"no multipart upload with id {upload_id} exists",
            )
        native_metadata = backend.get_upload_session_native_metadata(upload_id)
        try:
            abort_on_native_metadata_not_usable(
                request.destination_resource_address,
                native_metadata,
                backend,
                context,
            )
        except grpc.RpcError:
            backend.cleanup_upload_session(upload_id)
            raise
        if previous_version:
            # Check if backend supports optimistic locking
            check_optimistic_locking_support(context, "Multipart upload")

            # Check that the specified version is still the latest!
            abort_on_previous_version_not_latest(request.destination_resource_address, previous_version, context)

        # Assemble the parts into the final file
        i = 0
        for part in request.parts:
            # Make sure the parts are sorted (not a global requirement for all storage services)
            if part.part_number != i:
                context.abort(
                    grpc.StatusCode.INVALID_ARGUMENT,
                    details=f"invalid part_number {part.part_number}, expected {i}",
                )
            i += 1
            header_name = "local-file"
            found = False
            for header in part.headers:
                if header.name == header_name:
                    found = True
                    source_file = header.value
                    with backend.safe_open(source_file, "rb") as input_file:
                        data = input_file.read()
                        assembled_data.extend(data)
                    break
            if not found:
                context.abort(
                    grpc.StatusCode.INVALID_ARGUMENT,
                    f"part {part.part_number} is missing the required header {header_name}",
                )

        try:
            backend.write_version(
                request.destination_resource_address,
                assembled_data,
                previous_version=previous_version,
                fail_if_exists=fail_if_exists,
                native_metadata=native_metadata,
            )
        except FileExistsError as e:
            context.abort(grpc.StatusCode.ALREADY_EXISTS, details=str(e))
        except EtagMismatchError as e:
            backend.cleanup_upload_session(upload_id)
            context.abort(grpc.StatusCode.FAILED_PRECONDITION, details=str(e))
            return
        backend.cleanup_upload_session(upload_id)
        return pb2_version.CompleteMultipartUploadResponse(
            resource_info=resource_info(
                fileobject_version,
                request.destination_resource_address,
                native_metadata_support_scope=request.destination_resource_address,
                backend=backend,
            )
        )

    def AbortMultipartUpload(self, request, context):
        """Cancel a multipart upload and clean up resources.

        Aborts an in-progress multipart upload and removes any uploaded parts.
        This operation is idempotent - aborting a non-existent upload succeeds.

        Args:
            request: AbortMultipartUploadRequest containing:
                    - upload_id: Upload session identifier
            context: gRPC ServicerContext for the request.

        Returns:
            AbortMultipartUploadResponse (empty message).

        Note:
            Always returns success, even if the upload_id doesn't exist.
            This matches the behavior of cloud storage services where you
            cannot inspect abort status.
        """
        # Check if backend supports multipart uploads
        backend = get_backend()
        if not backend.supports_multipart_upload():
            context.abort(
                grpc.StatusCode.UNIMPLEMENTED,
                details=f"Multipart uploads not supported by {type(backend).__name__} backend",
            )

        upload_id, _, _ = backend.decode_upload_id(request.upload_id)
        backend.cleanup_upload_session(upload_id)
        # Abort always returns success, even if there was nothing to abort. The reason is the hyperscalers do not allow
        # to inspect the status of an abort, so the client can just trust it worked ot it has to inspect the status afterward
        return pb2_version.AbortMultipartUploadResponse()

    def Delete(self, request: pb2_version.DeleteRequest, context):
        """Delete a file and all its versions.

        Removes a file from the storage service, deleting all versions.
        This operation is idempotent - deleting a non-existent file succeeds.

        Args:
            request: DeleteRequest containing:
                    - resource_address: File address to delete
                    - previous_version: Optional expected current version (for optimistic locking)
            context: gRPC ServicerContext for the request.

        Returns:
            DeleteResponse (empty message).

        Raises:
            grpc.RpcError with:
            - INVALID_ARGUMENT: If address is invalid, versioned, or not a file
            - FAILED_PRECONDITION: If previous_version doesn't match current version

        Note:
            - Cannot delete individual versions, only the entire file
            - Cannot delete versioned addresses or folders
            - Supports optimistic locking via previous_version
        """
        abort_if_read_only(context, "Delete")

        if not get_backend().is_address_valid(request.resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details=f"invalid resource_address: {request.resource_address}",
            )

        if get_backend().is_version_address(request.resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details=f"Cannot delete individual version address: {request.resource_address}",
            )

        # Idempotent: deleting a non-existent object still succeeds.
        if not get_backend().exists(request.resource_address):
            return pb2_version.DeleteResponse()

        if not get_backend().is_file(request.resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details=f"Failed to remove {request.resource_address}, not a file",
            )

        if request.HasField("previous_version"):
            # Check if backend supports optimistic locking
            check_optimistic_locking_support(context, "Delete operation")

            # Check that the specified version is still the latest!
            abort_on_previous_version_not_latest(
                request.resource_address,
                request.previous_version.encoded_identity,
                context,
            )

        if is_versioning_enabled():
            # Versioned storage keeps prior versions readable by identity after delete.
            get_backend().remove_by_address(request.resource_address)
        else:
            # Unversioned storage has no history: hard-delete so identity reads of the removed
            # object fail with NOT_FOUND.
            get_backend().obliterate(request.resource_address)
        return pb2_version.DeleteResponse()

    def Copy(self, request: fileobject_service_pb2_v1alpha.CopyRequest, context):
        """Copy a file version to a new destination (v1alpha only).

        Creates a copy of a specific file version at a new address, creating
        a new version at the destination.

        Args:
            request: CopyRequest containing:
                    - source_resource_identity: Identity of version to copy
                    - destination_resource_address: Target address
                    - previous_version: Optional expected version at destination (for optimistic locking)
            context: gRPC ServicerContext for the request.

        Returns:
            CopyResponse containing:
            - resource_identity: Identity of the newly created version at destination

        Raises:
            grpc.RpcError with:
            - INVALID_ARGUMENT: If identities/addresses invalid, or destination is versioned
            - FAILED_PRECONDITION: If previous_version doesn't match destination current version
            - PERMISSION_DENIED: If read permission denied on source

        Note:
            This is a v1alpha-only method, not available in v1beta.
            The copy creates a new version at the destination.
        """
        abort_if_read_only(context, "Copy")
        backend = get_backend()

        # Convert source identity to resource address
        # Use url_from_identity to preserve version info for versioned copies
        try:
            source_address = backend.url_from_identity(request.source_resource_identity.encoded_identity)
        except (ValueError, KeyError) as e:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, details=f"Invalid source identity: {e}")

        # Validate source exists
        if not backend.exists(source_address):
            context.abort(
                grpc.StatusCode.NOT_FOUND,
                details="Source resource not found",
            )

        # Validate source is a file, not a folder
        if backend.is_dir(source_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details="Cannot copy folders, only files",
            )

        # Validate destination address
        if not backend.is_address_valid(request.destination_resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details=f"{request.destination_resource_address} is not a valid destination address",
            )
        if backend.is_version_address(request.destination_resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details=f"Cannot copy to individual version address: {request.destination_resource_address}",
            )

        copy_fail_if_exists = bool(request.HasField("fail_if_exists") and request.fail_if_exists)

        if request.HasField("previous_version") and copy_fail_if_exists:
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details="fail_if_exists and previous_version are mutually exclusive",
            )
            return

        # Check previous version if specified
        if request.HasField("previous_version"):
            # Check if backend supports optimistic locking
            check_optimistic_locking_support(context, "Copy operation")

            abort_on_previous_version_not_latest(
                request.destination_resource_address,
                request.previous_version.encoded_identity,
                context,
            )

        if copy_fail_if_exists and not backend.get_optimistic_locking_support().copy_fail_if_exists:
            context.abort(
                grpc.StatusCode.UNIMPLEMENTED,
                details=f"Copy operation with fail_if_exists is not supported by {type(backend).__name__} backend.",
            )

        try:
            # Perform the copy using backend's copy method
            new_identity = backend.copy(
                source_address,
                request.destination_resource_address,
                fail_if_exists=copy_fail_if_exists,
            )
            return fileobject_service_pb2_v1alpha.CopyResponse(
                resource_identity=fileobject_pb2_v1alpha.ResourceIdentity(encoded_identity=new_identity)
            )
        except FileExistsError as e:
            context.abort(grpc.StatusCode.ALREADY_EXISTS, details=str(e))
        except PermissionError as e:
            context.abort(grpc.StatusCode.PERMISSION_DENIED, details=str(e))
        except (FileNotFoundError, ValueError) as e:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, details=str(e))
        except grpc.RpcError:
            raise
        except Exception as e:
            context.abort(grpc.StatusCode.INTERNAL, details=f"Copy failed: {e}")

    def Move(self, request: fileobject_service_pb2_v1alpha.MoveRequest, context):
        """Move/rename a file to a new address (v1alpha only).

        Moves a file from one address to another, preserving all versions.
        This is effectively a rename operation.

        Args:
            request: MoveRequest containing:
                    - source_resource_address: Current file address
                    - destination_resource_address: New file address
                    - source_previous_version: Optional expected version at source (for optimistic locking)
                    - destination_previous_version: Optional expected version at destination (for optimistic locking)
            context: gRPC ServicerContext for the request.

        Returns:
            MoveResponse containing:
            - resource_identity: Identity of the version at the new address

        Raises:
            grpc.RpcError with:
            - INVALID_ARGUMENT: If addresses invalid, versioned, or source is directory
            - NOT_FOUND: If source doesn't exist
            - FAILED_PRECONDITION: If source_previous_version or destination_previous_version don't match
            - PERMISSION_DENIED: If permission denied

        Note:
            - This is a v1alpha-only method, not available in v1beta
            - Cannot move directories, only files
            - Cannot move from/to versioned addresses
        """
        abort_if_read_only(context, "Move")
        backend = get_backend()

        if not request.source_resource_address:
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details="source_resource_address is required",
            )

        if backend.is_version_address(request.source_resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details=f"Cannot move from individual version address: {request.source_resource_address}",
            )
        if backend.is_version_address(request.destination_resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details=f"Cannot move to individual version address: {request.destination_resource_address}",
            )

        if not backend.is_address_valid(request.source_resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details="Invalid source resource address",
            )

        # Check source exists
        if not backend.exists(request.source_resource_address):
            context.abort(grpc.StatusCode.NOT_FOUND, details="Source file not found")

        # Check source is a file, not a folder
        if backend.is_dir(request.source_resource_address):
            error_code_to_send = (
                grpc.StatusCode.NOT_FOUND if backend.folder_mode() == FolderMode.HYBRID else grpc.StatusCode.INVALID_ARGUMENT
            )
            context.abort(error_code_to_send, details="Cannot move directories")

        # Check source_previous_version if specified
        if request.HasField("source_previous_version"):
            # Check if backend supports optimistic locking
            check_optimistic_locking_support(context, "Move operation")

            # Force a validity check of the given resource identity
            try:
                backend.address_from_identity(request.source_previous_version.encoded_identity)
            except ValueError:
                context.abort(grpc.StatusCode.INVALID_ARGUMENT, f"Invalid source_previous_version given: {request.source_previous_version}")

            try:
                current_info = backend.stat(request.source_resource_address)
            except FileNotFoundError:
                context.abort(
                    grpc.StatusCode.FAILED_PRECONDITION,
                    details=f"Source resource not found at {request.source_resource_address}",
                )
            except Exception as e:
                context.abort(
                    grpc.StatusCode.INVALID_ARGUMENT,
                    details=f"Invalid source_previous_version: {e}",
                )

            if current_info.resource_identity != request.source_previous_version.encoded_identity:
                context.abort(
                    grpc.StatusCode.FAILED_PRECONDITION,
                    details="source_previous_version no longer matches current version at source_resource_address",
                )

        # Validate destination address
        if not backend.is_address_valid(request.destination_resource_address):
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details=f"{request.destination_resource_address} is not a valid destination address",
            )

        move_fail_if_exists = bool(request.HasField("fail_if_exists") and request.fail_if_exists)

        if request.HasField("destination_previous_version") and move_fail_if_exists:
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                details="fail_if_exists and destination_previous_version are mutually exclusive",
            )
            return

        # Check destination_previous_version if specified
        if request.HasField("destination_previous_version"):
            # Check if backend supports optimistic locking
            check_optimistic_locking_support(context, "Move operation")

            abort_on_previous_version_not_latest(
                request.destination_resource_address,
                request.destination_previous_version.encoded_identity,
                context,
            )

        if move_fail_if_exists and not backend.get_optimistic_locking_support().move_fail_if_exists:
            context.abort(
                grpc.StatusCode.UNIMPLEMENTED,
                details=f"Move operation with fail_if_exists is not supported by {type(backend).__name__} backend.",
            )

        try:
            # Perform the move using backend's move method
            result_identity = backend.move(
                source_resource_address=request.source_resource_address,
                destination_resource_address=request.destination_resource_address,
                fail_if_exists=move_fail_if_exists,
            )

            return fileobject_service_pb2_v1alpha.MoveResponse(
                resource_identity=fileobject_pb2_v1alpha.ResourceIdentity(encoded_identity=result_identity)
            )
        except FileExistsError as e:
            context.abort(grpc.StatusCode.ALREADY_EXISTS, details=str(e))
        except FileNotFoundError as e:
            context.abort(grpc.StatusCode.NOT_FOUND, details=str(e))
        except PermissionError as e:
            context.abort(grpc.StatusCode.PERMISSION_DENIED, details=str(e))
        except OSError as e:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, details=str(e))

    def GetOptimisticLockingSupport(self, request, context):
        """Query the server's support for optimistic locking.

        Returns information about which operations support conditional execution
        with previous_version parameter for the given resource address.

        Args:
            request: GetOptimisticLockingSupportRequest containing:
                    - resource_address: The resource address to check support for
            context: gRPC ServicerContext for the request.

        Returns:
            GetOptimisticLockingSupportResponse containing boolean flags for:
            - write: True if Write supports previous_version
            - delete: True if Delete supports previous_version
            - copy: True if Copy supports previous_version
            - move: True if Move supports source_previous_version/destination_previous_version
        """
        # Note: Currently, the backend returns the same support for all addresses.
        # In the future, this could vary based on the resource_address.
        backend = get_backend()
        support = backend.get_optimistic_locking_support()

        return fileobject_service_pb2_v1alpha.GetOptimisticLockingSupportResponse(
            supports_write=support.write,
            supports_delete=support.delete,
            supports_copy=support.copy,
            supports_move=support.move,
            supports_write_fail_if_exists=support.write_fail_if_exists,
            supports_copy_fail_if_exists=support.copy_fail_if_exists,
            supports_move_fail_if_exists=support.move_fail_if_exists,
        )

    methods = {
        "Enumerate": Enumerate,
        "Stat": Stat,
        "Read": Read,
        "ReadFromAddress": ReadFromAddress,
        "FetchWriteTypeInfo": FetchWriteTypeInfo,
        "Write": Write,
        "CompleteRedirectUpload": CompleteRedirectUpload,
        "UploadPart": UploadPart,
        "CompleteMultipartUpload": CompleteMultipartUpload,
        "AbortMultipartUpload": AbortMultipartUpload,
        "Delete": Delete,
    }
    if version_tag == "v1alpha":
        methods["Copy"] = Copy
        methods["Move"] = Move
        methods["GetOptimisticLockingSupport"] = GetOptimisticLockingSupport
        methods["GetNativeMetadataSupport"] = GetNativeMetadataSupport
        methods["IsReadOnly"] = IsReadOnly

    cls = type(
        "FileSystemServiceServicer",
        (pb2_grpc_version.FileObjectServiceServicer,),
        methods,
    )

    return cls()
