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

"""Metadata service implementation for gRPC.

This module implements the MetadataService from the Storage API, which allows
storing and retrieving user-defined metadata key-value pairs for resources.
This is separate from system metadata (size, modification time) and enables
applications to attach custom properties like tags, descriptions, or any
JSON-serializable data to files and folders.

The service supports optimistic concurrency control via ETags to prevent
concurrent modification conflicts.
"""

import json
from typing import (
    Any,
    Iterable,
)

import grpc
from google.protobuf.struct_pb2 import (
    ListValue,
    NullValue,
    Struct,
    Value,
)
from local_filesystem_service.backends.storage_backend_interface import ReadOnlyError
from local_filesystem_service.filesystem import get_backend
from local_filesystem_service.filesystem.file_system_provider import (
    EtagMismatchError,
    MetadataKeyNotFoundError,
)
from nvidia.omniverse.storage.metadata.v1alpha.metadata_pb2 import (
    BatchErrorEntry,
    DeleteMetadataBatchRequest,
    DeleteMetadataBatchRequestEntry,
    DeleteMetadataBatchResponse,
    DeleteMetadataBatchResponseEntry,
    DeleteMetadataBatchResponseEntrySuccess,
    DeleteMetadataRequest,
    DeleteMetadataResponse,
    GetMetadataBatchRequest,
    GetMetadataBatchRequestEntry,
    GetMetadataBatchResponse,
    GetMetadataBatchResponseEntry,
    GetMetadataRequest,
    GetMetadataResponse,
    QueryMetadataRequest,
    QueryMetadataResponse,
    QueryMetadataResponseEntry,
    ResourceLocation,
    UpdateMetadataBatchRequest,
    UpdateMetadataBatchRequestEntry,
    UpdateMetadataBatchResponse,
    UpdateMetadataBatchResponseEntry,
    UpdateMetadataBatchResponseEntrySuccess,
    UpdateMetadataRequest,
    UpdateMetadataResponse,
    UserMetadataValue,
)
from nvidia.omniverse.storage.metadata.v1alpha.metadata_pb2_grpc import (
    MetadataServiceServicer,
)

_DEFAULT_NAMESPACE = "default"


def _decode_value(value: Value):
    """Convert a protobuf Value to a Python native type.

    Recursively converts protobuf Value messages (which can represent any
    JSON type) into corresponding Python objects.

    Args:
        value: protobuf Value message to decode.

    Returns:
        Python native type (bool, float, str, list, dict, or None).
    """
    value_type = value.WhichOneof("kind")
    if value_type == "bool_value":
        return value.bool_value
    if value_type == "number_value":
        return value.number_value
    if value_type == "string_value":
        return value.string_value
    if value_type == "list_value":
        return [_decode_value(item) for item in value.list_value.values]
    if value_type == "struct_value":
        return {key: _decode_value(item) for key, item in value.struct_value.fields.items()}
    return None


def _encode_value(value):
    """Convert a Python native type to a protobuf Value.

    Recursively converts Python objects into protobuf Value messages that
    can represent any JSON type.

    Args:
        value: Python object to encode (bool, int, float, str, list, dict, or None).

    Returns:
        protobuf Value message.

    Note:
        Integers are converted to floats as per JSON/protobuf semantics.
    """
    if isinstance(value, bool):
        return Value(bool_value=value)
    if isinstance(value, int):
        return Value(number_value=float(value))
    if isinstance(value, float):
        return Value(number_value=value)
    if isinstance(value, str):
        return Value(string_value=value)
    if isinstance(value, list):
        return Value(list_value=ListValue(values=[_encode_value(item) for item in value]))
    if isinstance(value, dict):
        return Value(struct_value=Struct(fields={key: _encode_value(item) for key, item in value.items()}))
    return Value(null_value=NullValue.NULL_VALUE)


def _check_uri(uri: str) -> None:
    """Verify that a URI is either a valid resource address or identity.

    Raises ``ValueError`` (with a descriptive message) on failure so callers
    can decide whether to abort the RPC, surface a per-entry error, etc.
    """
    backend = get_backend()
    if backend.is_address_valid(uri):
        return
    try:
        backend.address_from_identity(uri)
    except ValueError as e:
        raise ValueError(f"Invalid resource URI (not a valid address or identity): {uri}") from e


def _validate_uri(uri: str, context) -> None:
    """Validate ``uri`` and abort with INVALID_ARGUMENT on failure.

    Thin gRPC wrapper around :func:`_check_uri` for the single-entry handlers
    that want the request to fail at the transport level.
    """
    try:
        _check_uri(uri)
    except ValueError as e:
        context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(e))


class FilesystemMetadataService(MetadataServiceServicer):
    """gRPC servicer for user-defined metadata operations.

    Implements the MetadataService interface from the Storage API v1alpha,
    providing operations to get, update, and delete custom metadata associated
    with resources (files or folders).

    Metadata values are stored as JSON and support ETags for optimistic
    concurrency control.
    """

    def GetMetadata(self, request: GetMetadataRequest, context):
        """Retrieve user-defined metadata key-value pairs for a resource.

        Gets the specified metadata keys (or all keys if none specified) for
        a resource. Each value includes an ETag for use in conditional updates.

        Args:
            request: GetMetadataRequest containing:
                    - uri: Resource address or identity
                    - user_metadata_keys: List of keys to retrieve (empty = all keys)
            context: gRPC ServicerContext for the request.

        Returns:
            GetMetadataResponse mapping key names to UserMetadataValue objects,
            each containing the value (as protobuf Value) and an etag.

        Raises:
            grpc.RpcError with:
            - INVALID_ARGUMENT: If URI is invalid
            - INTERNAL: On unexpected errors

        Note:
            Values are stored as JSON. Non-JSON strings are returned as-is.
        """
        _validate_uri(request.uri, context)

        try:
            metadata = get_backend().get_metadata(
                request.uri,
                list(request.user_metadata_keys),
            )

            # Convert to protobuf format
            proto_metadata = {}
            for key, value_info in metadata.items():
                # Parse the stored JSON value back to Python type
                try:
                    parsed_value = json.loads(value_info["value"])
                except (json.JSONDecodeError, ValueError):
                    # If not valid JSON, use as string
                    parsed_value = value_info["value"]

                proto_metadata[key] = UserMetadataValue(value=_encode_value(parsed_value), etag=value_info["etag"])

            return GetMetadataResponse(user_metadata=proto_metadata)

        except ValueError as e:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(e))

    def UpdateMetadata(self, request: UpdateMetadataRequest, context):
        """Create or update a user-defined metadata key-value pair.

        Sets or updates a metadata key for a resource. Supports conditional
        updates via ETags to prevent lost updates in concurrent scenarios.

        Args:
            request: UpdateMetadataRequest containing:
                    - uri: Resource address or identity
                    - user_metadata_key: Key name to update
                    - user_metadata: protobuf Value containing the new value
                    - expected_etag: Optional ETag for conditional update
            context: gRPC ServicerContext for the request.

        Returns:
            UpdateMetadataResponse containing the new etag for the key.

        Raises:
            grpc.RpcError with:
            - INVALID_ARGUMENT: If URI/key invalid or value can't be serialized
            - FAILED_PRECONDITION: If ETag doesn't match (precondition failed)
            - INTERNAL: On unexpected errors

        Note:
            Values are serialized as JSON. To update conditionally, first call
            GetMetadata to get current ETag, then include it in expected_etag.
        """
        _validate_uri(request.uri, context)

        try:
            python_value = _decode_value(request.user_metadata)

            value_str = json.dumps(python_value, sort_keys=True, separators=(",", ":"))

            expected_etag = request.expected_etag or None

            new_etag = get_backend().update_metadata(
                request.uri,
                request.user_metadata_key,
                value_str,
                expected_etag,
            )

            return UpdateMetadataResponse(etag=new_etag)

        except EtagMismatchError as e:
            context.abort(grpc.StatusCode.FAILED_PRECONDITION, str(e))
        except (ValueError, MetadataKeyNotFoundError) as e:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(e))

    def DeleteMetadata(self, request: DeleteMetadataRequest, context):
        """Delete a user-defined metadata key-value pair.

        Removes a metadata key from a resource. Supports conditional deletion
        via ETags to prevent accidental deletion in concurrent scenarios.

        Args:
            request: DeleteMetadataRequest containing:
                    - uri: Resource address or identity
                    - user_metadata_key: Key name to delete
                    - expected_etag: Optional ETag for conditional deletion
            context: gRPC ServicerContext for the request.

        Returns:
            DeleteMetadataResponse (empty message).

        Raises:
            grpc.RpcError with:
            - INVALID_ARGUMENT: If URI/key invalid or key doesn't exist
            - FAILED_PRECONDITION: If ETag doesn't match (precondition failed)
            - INTERNAL: On unexpected errors

        Note:
            For conditional deletion, include the current ETag in expected_etag.
        """
        _validate_uri(request.uri, context)

        try:
            expected_etag = request.expected_etag or None

            get_backend().delete_metadata(
                request.uri,
                request.user_metadata_key,
                expected_etag,
            )

            return DeleteMetadataResponse()

        except EtagMismatchError as e:
            context.abort(grpc.StatusCode.FAILED_PRECONDITION, str(e))
        except (ValueError, MetadataKeyNotFoundError) as e:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(e))

    def GetMetadataBatch(self, request: GetMetadataBatchRequest, context):
        if not request.requests:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "Batch request must contain at least one entry.")

        backend = get_backend()

        def get_one(idx: int, entry: GetMetadataBatchRequestEntry) -> GetMetadataBatchResponseEntry:
            try:
                metadata = backend.get_metadata(
                    metadata_uri=_get_resource_uri_from_resource_location(entry.resource_location),
                    keys=[entry.user_metadata_key],
                    namespace=entry.namespace or _DEFAULT_NAMESPACE,
                )
            except ValueError as ex:
                return GetMetadataBatchResponseEntry(
                    error=BatchErrorEntry(
                        code=BatchErrorEntry.CODE_INVALID_ARGUMENT,
                        message=str(ex),
                    ),
                    request_index=idx,
                )

            if (value := next(iter(metadata.values()), None)) is None:
                return GetMetadataBatchResponseEntry(
                    error=BatchErrorEntry(
                        code=BatchErrorEntry.CODE_NOT_FOUND,
                        message="Metadata value not found.",
                    ),
                    request_index=idx,
                )

            return GetMetadataBatchResponseEntry(
                success=_create_user_metadata_value_from_value_info(value),
                request_index=idx,
            )

        yield GetMetadataBatchResponse(results=[get_one(i, e) for i, e in enumerate(request.requests)])

    def QueryMetadata(self, request: QueryMetadataRequest, context):
        backend = get_backend()

        try:
            metadata = backend.get_metadata(
                metadata_uri=_get_resource_uri_from_resource_location(request.resource_location),
                keys=[],
                namespace=request.namespace or _DEFAULT_NAMESPACE,
            )
        except ValueError as ex:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, str(ex))
            raise

        yield QueryMetadataResponse(
            results=[
                QueryMetadataResponseEntry(
                    user_metadata_key=key,
                    user_metadata=_create_user_metadata_value_from_value_info(value),
                )
                for key, value in metadata.items()
            ]
        )

    def UpdateMetadataBatch(self, request: UpdateMetadataBatchRequest, context):
        if not request.requests:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "Batch request must contain at least one entry.")

        _abort_on_duplicate_entries(request.requests, context)

        backend = get_backend()

        def update_one(idx: int, entry: UpdateMetadataBatchRequestEntry) -> UpdateMetadataBatchResponseEntry:
            try:
                etag = backend.update_metadata(
                    metadata_uri=_get_resource_uri_from_resource_location(entry.resource_location),
                    key=entry.user_metadata_key,
                    value=json.dumps(_decode_value(entry.user_metadata)),
                    expected_etag=entry.expected_etag or None,
                    namespace=entry.namespace or _DEFAULT_NAMESPACE,
                )
            except ValueError as ex:
                return UpdateMetadataBatchResponseEntry(
                    error=BatchErrorEntry(
                        code=BatchErrorEntry.CODE_INVALID_ARGUMENT,
                        message=str(ex),
                    ),
                    request_index=idx,
                )
            except MetadataKeyNotFoundError as ex:
                return UpdateMetadataBatchResponseEntry(
                    error=BatchErrorEntry(
                        code=BatchErrorEntry.CODE_NOT_FOUND,
                        message=str(ex),
                    ),
                    request_index=idx,
                )
            except (EtagMismatchError, ReadOnlyError) as ex:
                return UpdateMetadataBatchResponseEntry(
                    error=BatchErrorEntry(
                        code=BatchErrorEntry.CODE_FAILED_PRECONDITION,
                        message=str(ex),
                    ),
                    request_index=idx,
                )

            return UpdateMetadataBatchResponseEntry(
                success=UpdateMetadataBatchResponseEntrySuccess(
                    etag=etag,
                ),
                request_index=idx,
            )

        yield UpdateMetadataBatchResponse(results=[update_one(i, e) for i, e in enumerate(request.requests)])

    def DeleteMetadataBatch(self, request: DeleteMetadataBatchRequest, context):
        if not request.requests:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "Batch request must contain at least one entry.")

        _abort_on_duplicate_entries(request.requests, context)

        backend = get_backend()

        def delete_one(idx: int, entry: DeleteMetadataBatchRequestEntry) -> DeleteMetadataBatchResponseEntry:
            try:
                backend.delete_metadata(
                    metadata_uri=_get_resource_uri_from_resource_location(entry.resource_location),
                    key=entry.user_metadata_key,
                    expected_etag=entry.expected_etag or None,
                    namespace=entry.namespace or _DEFAULT_NAMESPACE,
                )
            except ValueError as ex:
                return DeleteMetadataBatchResponseEntry(
                    error=BatchErrorEntry(
                        code=BatchErrorEntry.CODE_INVALID_ARGUMENT,
                        message=str(ex),
                    ),
                    request_index=idx,
                )

            except (EtagMismatchError, ReadOnlyError) as ex:
                return DeleteMetadataBatchResponseEntry(
                    error=BatchErrorEntry(
                        code=BatchErrorEntry.CODE_FAILED_PRECONDITION,
                        message=str(ex),
                    ),
                    request_index=idx,
                )

            return DeleteMetadataBatchResponseEntry(
                success=DeleteMetadataBatchResponseEntrySuccess(),
                request_index=idx,
            )

        yield DeleteMetadataBatchResponse(results=[delete_one(i, e) for i, e in enumerate(request.requests)])


def _abort_on_duplicate_entries(entries: Iterable, context) -> None:
    seen = set()
    for entry in entries:
        try:
            uri = _get_resource_uri_from_resource_location(entry.resource_location)
        except ValueError:
            continue
        identity = (uri, entry.user_metadata_key, entry.namespace or _DEFAULT_NAMESPACE)
        if identity in seen:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, f"Duplicate batch entry for key '{entry.user_metadata_key}'.")
        seen.add(identity)


def _get_resource_uri_from_resource_location(value: ResourceLocation) -> str:
    if value.HasField("resource_address"):
        return value.resource_address
    elif value.HasField("resource_identity"):
        encoded_identity = value.resource_identity.encoded_identity
        # The resource_identity arm must carry a valid encoded resource identity.
        # Validate it via the backend's address_from_identity, the interface's
        # identity-validation entry point: a non-identity value such as a plain
        # folder address does not decode as an identity and is rejected (batch
        # handlers surface this as a per-entry INVALID_ARGUMENT). The identity is
        # returned unchanged rather than the resolved address so metadata stays
        # keyed by identity, consistent with the single-entry handlers; the
        # backend re-validates it when the operation runs.
        try:
            get_backend().address_from_identity(encoded_identity)
        except ValueError as e:
            raise ValueError(f"Invalid resource identity (not an encoded identity): {encoded_identity}") from e
        return encoded_identity
    elif value.HasField("folder_address"):
        return value.folder_address.uri

    raise ValueError("Invalid resource location.")


def _create_user_metadata_value_from_value_info(value_info: dict[str, Any]) -> UserMetadataValue:
    try:
        parsed_value = json.loads(value_info["value"])
    except ValueError:
        parsed_value = value_info["value"]

    return UserMetadataValue(value=_encode_value(parsed_value), etag=value_info["etag"])
