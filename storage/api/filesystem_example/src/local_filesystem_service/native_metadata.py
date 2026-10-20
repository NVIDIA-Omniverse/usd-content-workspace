# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

"""Shared validation for FileObject-native metadata."""

import json
from typing import (
    Any,
    Dict,
    Optional,
)


class NativeMetadataUnsupportedError(Exception):
    """Raised when valid non-empty native metadata is sent to unsupported storage."""


def parse_rest_native_metadata_header(header_value: Optional[str]) -> Dict[str, Any]:
    """Parse and validate the REST native metadata header."""
    if header_value is None:
        return {}
    try:
        native_metadata = json.loads(header_value)
    except json.JSONDecodeError as e:
        raise ValueError("Native metadata header must contain a JSON object") from e
    if not isinstance(native_metadata, dict):
        raise ValueError("Native metadata header must contain a JSON object")
    validate_native_metadata(native_metadata)
    return native_metadata


def validate_and_gate_native_metadata_for_write(
    resource_address: str,
    native_metadata: Dict[str, Any],
    backend=None,
) -> Dict[str, Any]:
    """Validate metadata and reject non-empty input when native metadata is unsupported.

    Callers must perform address validation and write authorization before this
    helper so address/auth errors keep their higher precedence.
    """
    validate_native_metadata(native_metadata)
    if native_metadata:
        from local_filesystem_service.filesystem import (
            effective_native_metadata_support,
        )

        if not effective_native_metadata_support(resource_address, backend=backend):
            raise NativeMetadataUnsupportedError("Native metadata is not supported for this resource address")
    return native_metadata


def should_return_native_metadata(native_metadata_support_scope: Optional[str], backend=None) -> bool:
    """Return whether response metadata may expose FileObject-native metadata."""
    import local_filesystem_service.filesystem as filesystem_state

    if native_metadata_support_scope:
        try:
            return filesystem_state.effective_native_metadata_support(native_metadata_support_scope, backend=backend)
        except (OSError, ValueError):
            return False
    return filesystem_state.NATIVE_METADATA_MODE == filesystem_state.NativeMetadataMode.ENABLED


def native_metadata_for_response(
    native_metadata: Optional[Dict[str, Any]],
    native_metadata_support_scope: Optional[str],
    backend=None,
) -> Dict[str, Any]:
    """Return native metadata only when the relevant support scope supports it."""
    if not native_metadata:
        return {}
    if not should_return_native_metadata(native_metadata_support_scope, backend=backend):
        return {}
    return native_metadata


def response_support_scope_for_identity(resource_identity: str, metadata=None, backend=None) -> Optional[str]:
    """Return the best support scope for identity-based response shaping."""
    stored_scope = getattr(metadata, "native_metadata_support_scope", None)
    if stored_scope:
        return stored_scope
    if backend is None or not hasattr(backend, "address_from_identity"):
        return None
    try:
        return backend.address_from_identity(resource_identity)
    except (OSError, ValueError, json.JSONDecodeError):
        return None


def validate_native_metadata(native_metadata: Dict[str, Any]) -> None:
    """Validate metadata has JSON-compatible values before it reaches a backend."""
    try:
        json.dumps(native_metadata, separators=(",", ":"), ensure_ascii=True, allow_nan=False)
    except (TypeError, ValueError) as e:
        raise ValueError("Native metadata values must be JSON serializable") from e
