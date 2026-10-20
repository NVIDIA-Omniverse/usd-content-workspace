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

"""Metadata service implementation for REST.

This module implements the MetadataService from the Storage API, which allows
storing and retrieving user-defined metadata key-value pairs for resources.
This is separate from system metadata (size, modification time) and enables
applications to attach custom properties like tags, descriptions, or any
JSON-serializable data to files and folders via HTTP endpoints.

The service supports optimistic concurrency control via ETags to prevent
concurrent modification conflicts.
"""

import asyncio
import json
from json import JSONDecodeError
from typing import (
    Any,
    Optional,
)

from fastapi import (
    Header,
    HTTPException,
    Request,
    Response,
)
from local_filesystem_service.backends.storage_backend_interface import ReadOnlyError
from local_filesystem_service.filesystem import get_backend
from local_filesystem_service.filesystem.file_system_provider import (
    EtagMismatchError,
    MetadataKeyNotFoundError,
)
from starlette import status

from .rest_messages import (
    METADATA_ETAG_TYPE,
    METADATA_KEY_TYPE,
    METADATA_URI_TYPE,
    BatchErrorEntry,
    BatchErrorEntryCode,
    BatchResponseOutcomeError,
    DeleteMetadataBatchRequest,
    DeleteMetadataBatchRequestEntry,
    DeleteMetadataBatchResponse,
    DeleteMetadataBatchResponseEntry,
    DeleteMetadataBatchResponseEntryOutcomeSuccess,
    DeleteMetadataBatchResponseEntrySuccess,
    FolderAddressResourceLocation,
    GetMetadataBatchRequest,
    GetMetadataBatchRequestEntry,
    GetMetadataBatchResponse,
    GetMetadataBatchResponseEntry,
    GetMetadataBatchResponseEntryOutcomeSuccess,
    HTTPValidationError,
    QueryMetadataRequest,
    QueryMetadataResponse,
    QueryMetadataResponseEntry,
    ResourceAddressResourceLocation,
    ResourceIdentityResourceLocation,
    ResourceLocation,
    UpdateMetadataBatchRequest,
    UpdateMetadataBatchRequestEntry,
    UpdateMetadataBatchResponse,
    UpdateMetadataBatchResponseEntry,
    UpdateMetadataBatchResponseEntryOutcomeSuccess,
    UpdateMetadataBatchResponseEntrySuccess,
    UserMetadataKeys,
    UserMetadataResponse,
    UserMetadataValue,
)
from .routes import metadata_service

_DEFAULT_QUERY_PAGE_SIZE = 100
_DEFAULT_NAMESPACE = "default"


@metadata_service.post(
    "/",
    responses={
        status.HTTP_200_OK: {
            "content": {"application/json": {"schema": UpdateMetadataBatchResponse.model_json_schema()}},
        },
        status.HTTP_422_UNPROCESSABLE_ENTITY: {"content": {"application/json": {"schema": HTTPValidationError.model_json_schema()}}},
    },
)
async def update_metadata_batch(request: UpdateMetadataBatchRequest) -> UpdateMetadataBatchResponse:
    _reject_empty_batch(request.requests)
    _reject_duplicate_entries(request.requests)

    backend = get_backend()

    async def update_one(idx: int, entry: UpdateMetadataBatchRequestEntry) -> UpdateMetadataBatchResponseEntry:
        try:
            etag = await asyncio.to_thread(
                backend.update_metadata,
                metadata_uri=_get_uri_from_resource_location(entry.resource_location),
                key=entry.user_metadata_key,
                value=json.dumps(entry.user_metadata),
                expected_etag=entry.expected_etag,
                namespace=entry.namespace or _DEFAULT_NAMESPACE,
            )
        except ValueError as ex:
            return UpdateMetadataBatchResponseEntry(
                outcome=_create_error_outcome(BatchErrorEntryCode.INVALID_ARGUMENT, str(ex)),
                request_index=idx,
            )
        except MetadataKeyNotFoundError as ex:
            return UpdateMetadataBatchResponseEntry(
                outcome=_create_error_outcome(BatchErrorEntryCode.NOT_FOUND, str(ex)),
                request_index=idx,
            )
        except (EtagMismatchError, ReadOnlyError) as ex:
            return UpdateMetadataBatchResponseEntry(
                outcome=_create_error_outcome(BatchErrorEntryCode.FAILED_PRECONDITION, str(ex)),
                request_index=idx,
            )

        return UpdateMetadataBatchResponseEntry(
            outcome=UpdateMetadataBatchResponseEntryOutcomeSuccess(
                success=UpdateMetadataBatchResponseEntrySuccess(etag=etag),
            ),
            request_index=idx,
        )

    return UpdateMetadataBatchResponse(results=[await update_one(i, e) for i, e in enumerate(request.requests)])


@metadata_service.delete(
    "/",
    responses={
        status.HTTP_200_OK: {
            "content": {"application/json": {"schema": DeleteMetadataBatchResponse.model_json_schema()}},
        },
        status.HTTP_422_UNPROCESSABLE_ENTITY: {"content": {"application/json": {"schema": HTTPValidationError.model_json_schema()}}},
    },
)
async def delete_metadata_batch(request: DeleteMetadataBatchRequest) -> DeleteMetadataBatchResponse:
    _reject_empty_batch(request.requests)
    _reject_duplicate_entries(request.requests)

    backend = get_backend()

    async def delete_one(idx: int, entry: DeleteMetadataBatchRequestEntry) -> DeleteMetadataBatchResponseEntry:
        try:
            await asyncio.to_thread(
                backend.delete_metadata,
                metadata_uri=_get_uri_from_resource_location(entry.resource_location),
                key=entry.user_metadata_key,
                expected_etag=entry.expected_etag,
                namespace=entry.namespace or _DEFAULT_NAMESPACE,
            )
        except ValueError as ex:
            return DeleteMetadataBatchResponseEntry(
                outcome=_create_error_outcome(BatchErrorEntryCode.INVALID_ARGUMENT, str(ex)),
                request_index=idx,
            )
        except (EtagMismatchError, ReadOnlyError) as ex:
            return DeleteMetadataBatchResponseEntry(
                outcome=_create_error_outcome(BatchErrorEntryCode.FAILED_PRECONDITION, str(ex)),
                request_index=idx,
            )

        return DeleteMetadataBatchResponseEntry(
            outcome=DeleteMetadataBatchResponseEntryOutcomeSuccess(
                success=DeleteMetadataBatchResponseEntrySuccess(),
            ),
            request_index=idx,
        )

    return DeleteMetadataBatchResponse(results=[await delete_one(i, e) for i, e in enumerate(request.requests)])


@metadata_service.post(
    "/query",
    responses={
        status.HTTP_200_OK: {
            "content": {"application/json": {"schema": QueryMetadataResponse.model_json_schema()}},
        },
        status.HTTP_422_UNPROCESSABLE_ENTITY: {"content": {"application/json": {"schema": HTTPValidationError.model_json_schema()}}},
    },
)
async def query_metadata_batch(request: QueryMetadataRequest) -> QueryMetadataResponse:
    backend = get_backend()

    if (max_page_size := request.max_page_size) is not None:
        if max_page_size <= 0:
            raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail="Page size must be a positive integer.")
    else:
        max_page_size = _DEFAULT_QUERY_PAGE_SIZE

    try:
        metadata = await asyncio.to_thread(
            backend.get_metadata,
            metadata_uri=_get_uri_from_resource_location(request.resource_location),
            keys=[],
            namespace=request.namespace or _DEFAULT_NAMESPACE,
        )
    except ValueError as ex:
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail=str(ex))

    # Use metadata keys for continuation handles.
    sorted_metadata = sorted(metadata.items(), key=lambda item: item[0])
    start_idx: int | None = None
    if last_key := request.continuation_handle:
        for idx, (key, _) in enumerate(sorted_metadata):
            if key > last_key:
                start_idx = idx
                break
    else:
        start_idx = 0

    # Nothing to return.
    if start_idx is None:
        return QueryMetadataResponse(results=[])

    # Return at most `max_page_size` items from the metadata set, starting from `start_idx`.
    # Whether we want to include a continuation handle.
    next_continuation_handle: str | None = None
    if start_idx + max_page_size < len(sorted_metadata):
        next_continuation_handle = sorted_metadata[start_idx + max_page_size - 1][0]

    return QueryMetadataResponse(
        results=[
            QueryMetadataResponseEntry(
                user_metadata_key=key,
                user_metadata=_decode_user_metadata_value(value),
            )
            for key, value in sorted_metadata[start_idx : start_idx + max_page_size]
        ],
        next_continuation_handle=next_continuation_handle,
    )


@metadata_service.post(
    "/get",
    responses={
        status.HTTP_200_OK: {
            "content": {"application/json": {"schema": GetMetadataBatchResponse.model_json_schema()}},
        },
        status.HTTP_422_UNPROCESSABLE_ENTITY: {"content": {"application/json": {"schema": HTTPValidationError.model_json_schema()}}},
    },
)
async def get_metadata_batch(request: GetMetadataBatchRequest) -> GetMetadataBatchResponse:
    _reject_empty_batch(request.requests)

    backend = get_backend()

    async def get_one(idx: int, entry: GetMetadataBatchRequestEntry) -> GetMetadataBatchResponseEntry:
        try:
            metadata = await asyncio.to_thread(
                backend.get_metadata,
                metadata_uri=_get_uri_from_resource_location(entry.resource_location),
                keys=[entry.user_metadata_key],
                namespace=entry.namespace or _DEFAULT_NAMESPACE,
            )
        except ValueError as ex:
            return GetMetadataBatchResponseEntry(
                outcome=_create_error_outcome(BatchErrorEntryCode.INVALID_ARGUMENT, str(ex)),
                request_index=idx,
            )

        if (metadata_value := next(iter(metadata.values()), None)) is None:
            return GetMetadataBatchResponseEntry(
                outcome=_create_error_outcome(BatchErrorEntryCode.NOT_FOUND, "Metadata key not found."),
                request_index=idx,
            )

        return GetMetadataBatchResponseEntry(
            outcome=GetMetadataBatchResponseEntryOutcomeSuccess(
                success=_decode_user_metadata_value(metadata_value),
            ),
            request_index=idx,
        )

    return GetMetadataBatchResponse(results=[await get_one(i, e) for i, e in enumerate(request.requests)])


@metadata_service.post(
    "/{uri:path}",
    responses={
        status.HTTP_200_OK: {
            "description": "Successful Response, data in body.",
            "content": {"application/json": {"schema": UserMetadataResponse.model_json_schema()}},
        },
        status.HTTP_422_UNPROCESSABLE_ENTITY: {"content": {"application/json": {"schema": HTTPValidationError.model_json_schema()}}},
    },
)
async def get_metadata(
    uri: METADATA_URI_TYPE,
    keys: UserMetadataKeys,
) -> UserMetadataResponse:
    """Retrieve user-defined metadata key-value pairs for a resource.

    The Metadata API allows storing custom key-value data associated with
    resources (files or folders). This is separate from system metadata
    (size, modification time) and is useful for application-specific data
    like tags, descriptions, custom properties, etc.

    Args:
        uri: Resource address or identity to get metadata for
        keys: UserMetadataKeys object specifying which keys to retrieve
              If keys list is empty, returns all metadata

    Returns:
        UserMetadataResponse mapping key names to UserMetadataValue objects
        Each value includes the value itself and an etag for optimistic locking

    Raises:
        HTTPException(400): If URI is invalid
        HTTPException(500): On internal errors

    Note:
        Values are stored as JSON. Non-JSON strings are returned as-is.
        ETags can be used with update_metadata() to prevent concurrent modifications.

    Example:
        POST /v1alpha/metadata/myfile.txt
        Body: {\"keys\": [\"tag\", \"description\"]}
        Response: {\"tag\": {\"value\": \"important\", \"etag\": \"abc\"}, ...}
    """
    backend = get_backend()

    # Validate that the URI is a valid resource address or identity
    if not await asyncio.to_thread(backend.is_address_valid, uri):
        # Try to validate as identity
        try:
            await asyncio.to_thread(backend.stat_identity, uri)
        except (ValueError, FileNotFoundError):
            raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail=f"Invalid resource address or identity: {uri}")

    try:
        metadata = await asyncio.to_thread(backend.get_metadata, uri, keys)

        rest_metadata = {}
        for key, value_info in metadata.items():
            try:
                parsed_value = json.loads(value_info["value"])
            except (JSONDecodeError, ValueError):
                parsed_value = value_info["value"]

            rest_metadata[key] = UserMetadataValue(value=parsed_value, etag=value_info["etag"])

        return UserMetadataResponse(rest_metadata)

    except ValueError as err:
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail=str(err)) from err


@metadata_service.put(
    "/{uri:path}/{key}",
    status_code=status.HTTP_201_CREATED,
    responses={
        status.HTTP_201_CREATED: {
            "description": "Successful Response",
            "headers": {
                "ETag": {
                    "description": "The ETag of the updated metadata key.",
                    "required": True,
                    "schema": {"type": "string", "format": "etag", "maxLength": 256},
                }
            },
        },
        status.HTTP_412_PRECONDITION_FAILED: {
            "description": "Precondition Failed",
            "content": {"application/json": {"schema": HTTPValidationError.model_json_schema()}},
        },
        status.HTTP_422_UNPROCESSABLE_ENTITY: {"content": {"application/json": {"schema": HTTPValidationError.model_json_schema()}}},
    },
)
async def update_metadata(
    uri: METADATA_URI_TYPE,
    key: METADATA_KEY_TYPE,
    request: Request,
    if_match: Optional[METADATA_ETAG_TYPE] = Header(None, alias="If-Match"),
) -> Response:
    """Create or update a user-defined metadata key-value pair.

    Sets or updates a metadata key for a resource. Supports conditional
    updates via ETags to prevent lost updates in concurrent scenarios.

    Args:
        uri: Resource address or identity
        key: Metadata key name
        request: Request with JSON body containing the value
                Body can be: {\"value\": <any JSON>} or just <any JSON>
        if_match: Optional ETag for conditional update (If-Match header)
                 If provided, update only succeeds if current ETag matches

    Returns:
        Response (201 Created) with ETag header containing new ETag

    Raises:
        HTTPException(400): If URI/key invalid or JSON malformed
        HTTPException(412): If ETag doesn't match (precondition failed)
        HTTPException(500): On internal errors

    Note:
        Values are serialized as JSON. To update conditionally, first call
        get_metadata() to get current ETag, then include it in If-Match header.

    Example (unconditional):
        PUT /v1alpha/metadata/myfile.txt/tag
        Body: {\"value\": \"important\"}
        Response (201): Headers: ETag: \"xyz123\"

    Example (conditional):
        PUT /v1alpha/metadata/myfile.txt/tag
        Headers: If-Match: \"xyz123\"
        Body: {\"value\": \"very-important\"}
        Response (201) if ETag matches, (412) if not
    """
    backend = get_backend()

    # Validate that the URI is a valid resource address or identity
    if not await asyncio.to_thread(backend.is_address_valid, uri):
        # Try to validate as identity
        try:
            await asyncio.to_thread(backend.stat_identity, uri)
        except (ValueError, FileNotFoundError):
            raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail=f"Invalid resource address or identity: {uri}")

    try:
        body = await request.body()
        request_data = json.loads(body.decode())

        if isinstance(request_data, dict) and "value" in request_data:
            value = request_data["value"]
        else:
            value = request_data

        value_str = json.dumps(value, sort_keys=True, separators=(",", ":"))

        new_etag = await asyncio.to_thread(backend.update_metadata, uri, key, value_str, if_match)

        return Response(status_code=status.HTTP_201_CREATED, headers={"ETag": new_etag})

    except JSONDecodeError as err:
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail="Invalid JSON") from err
    except EtagMismatchError as err:
        raise HTTPException(status_code=status.HTTP_412_PRECONDITION_FAILED, detail=str(err)) from err
    except (ValueError, MetadataKeyNotFoundError) as err:
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail=str(err)) from err


@metadata_service.delete(
    "/{uri:path}/{key}",
    status_code=status.HTTP_204_NO_CONTENT,
    responses={
        status.HTTP_204_NO_CONTENT: {"description": "Successful Response"},
        status.HTTP_412_PRECONDITION_FAILED: {
            "description": "Precondition Failed",
            "content": {"application/json": {"schema": HTTPValidationError.model_json_schema()}},
        },
        status.HTTP_422_UNPROCESSABLE_ENTITY: {"content": {"application/json": {"schema": HTTPValidationError.model_json_schema()}}},
    },
)
async def delete_metadata(
    uri: METADATA_URI_TYPE,
    key: METADATA_KEY_TYPE,
    if_match: Optional[METADATA_ETAG_TYPE] = Header(None, alias="If-Match"),
) -> Response:
    """Delete a user-defined metadata key-value pair.

    Removes a metadata key from a resource. Supports conditional deletion
    via ETags to prevent accidental deletion in concurrent scenarios.

    Args:
        uri: Resource address or identity
        key: Metadata key name to delete
        if_match: Optional ETag for conditional deletion (If-Match header)
                 If provided, delete only succeeds if current ETag matches

    Returns:
        Response (204 No Content) on successful deletion

    Raises:
        HTTPException(400): If URI/key invalid or key doesn't exist
        HTTPException(412): If ETag doesn't match (precondition failed)
        HTTPException(500): On internal errors

    Note:
        Idempotent operation - deleting a non-existent key returns success.
        For conditional deletion, include the current ETag in If-Match header.

    Example (unconditional):
        DELETE /v1alpha/metadata/myfile.txt/tag
        Response (204)

    Example (conditional):
        DELETE /v1alpha/metadata/myfile.txt/tag
        Headers: If-Match: \"xyz123\"
        Response (204) if ETag matches, (412) if not
    """
    backend = get_backend()

    # Validate that the URI is a valid resource address or identity
    if not await asyncio.to_thread(backend.is_address_valid, uri):
        # Try to validate as identity
        try:
            await asyncio.to_thread(backend.stat_identity, uri)
        except (ValueError, FileNotFoundError):
            raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail=f"Invalid resource address or identity: {uri}")

    try:
        await asyncio.to_thread(backend.delete_metadata, uri, key, if_match)
        return Response(status_code=status.HTTP_204_NO_CONTENT)

    except EtagMismatchError as err:
        raise HTTPException(status_code=status.HTTP_412_PRECONDITION_FAILED, detail=str(err)) from err
    except (ValueError, MetadataKeyNotFoundError) as err:
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail=str(err)) from err


def _get_uri_from_resource_location(location: ResourceLocation) -> str:
    if isinstance(location, ResourceAddressResourceLocation):
        return location.resource_address
    elif isinstance(location, ResourceIdentityResourceLocation):
        encoded_identity = location.resource_identity
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
    elif isinstance(location, FolderAddressResourceLocation):
        return location.folder_address


def _reject_empty_batch(entries: list):
    if not entries:
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail="Batch request must contain at least one entry.")


def _reject_duplicate_entries(entries) -> None:
    seen = set()
    for entry in entries:
        try:
            uri = _get_uri_from_resource_location(entry.resource_location)
        except ValueError:
            # An unresolvable location cannot collide; the per-entry handler
            # reports it as INVALID_ARGUMENT.
            continue
        identity = (
            uri,
            entry.user_metadata_key,
            entry.namespace or _DEFAULT_NAMESPACE,
        )
        if identity in seen:
            raise HTTPException(
                status_code=status.HTTP_400_BAD_REQUEST,
                detail=f"Duplicate batch entry for key '{entry.user_metadata_key}'.",
            )
        seen.add(identity)


def _create_error_outcome(code: BatchErrorEntryCode, message: str) -> BatchResponseOutcomeError:
    return BatchResponseOutcomeError(error=BatchErrorEntry(code=code, message=message))


def _decode_user_metadata_value(value_info: dict[str, Any]) -> UserMetadataValue:
    try:
        value = json.loads(value_info["value"])
    except ValueError:
        value = value_info["value"]

    return UserMetadataValue(value=value, etag=value_info["etag"])
