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

"""Filesystem module - provides storage backend access and configuration.

This module maintains a global storage backend instance (STORAGE_BACKEND) that
can be initialized with different backend implementations (filesystem, git, s3, etc.).

The backend is initialized by calling init_backend() with the appropriate configuration,
typically done from the CLI entry points in __main__.py files.
"""

import logging
import os
import tempfile
from dataclasses import dataclass
from enum import Enum
from typing import Optional

import typer
from local_filesystem_service.backends import (
    BackendConfig,
    StorageBackendInterface,
    create_backend,
    register_backend,
    register_backend_cli,
)
from local_filesystem_service.filesystem.file_system_provider import (
    FileSystemProvider,
)
from typing_extensions import Annotated

logger = logging.getLogger(__name__)

# Filesystem backend option names/defaults (shared by Typer + fallback startup path)
FILESERVICE_SERVER_BASE_URI_ENV = "FILESERVICE_SERVER_BASE_URI"
FILESERVICE_SERVER_BASE_URI_DEFAULT = "file-storage://fileservice"
FILESERVICE_STATIC_DIR_ENV = "FILESERVICE_STATIC_DIR"
FILESERVICE_TEST_FOLDER_MODE_ENV = "FILESERVICE_TEST_FOLDER_MODE"
FILESERVICE_TEST_FOLDER_MODE_DEFAULT = "native"
FILESERVICE_NATIVE_METADATA_MODE_ENV = "FILESERVICE_NATIVE_METADATA_MODE"
FILESERVICE_NATIVE_METADATA_MODE_DEFAULT = "enabled"
# Whether the backend keeps a version history ("versioned") or behaves as unversioned storage
# ("unversioned"): a single current version, hard deletes, and identity reads of deleted objects
# that fail with NOT_FOUND. Lets the example exercise IsVersioned == false.
FILESERVICE_VERSIONING_MODE_ENV = "FILESERVICE_VERSIONING_MODE"
FILESERVICE_VERSIONING_MODE_DEFAULT = "versioned"
# Whether the backend rejects mutations (write/delete/move), so IsReadOnly returns true.
FILESERVICE_READ_ONLY_ENV = "FILESERVICE_READ_ONLY"
FILESERVICE_READ_ONLY_DEFAULT = "false"
REDIRECT_HOST_ENV = "REDIRECT_HOST"
REDIRECT_HOST_DEFAULT = "http://localhost"
REDIRECT_PORT_ENV = "REDIRECT_PORT"
REDIRECT_PORT_DEFAULT = 8011


class NativeMetadataMode(str, Enum):
    ENABLED = "enabled"
    DISABLED = "disabled"


class VersioningMode(str, Enum):
    VERSIONED = "versioned"
    UNVERSIONED = "unversioned"


class NativeMetadataSupportAddressStatus(str, Enum):
    OK = "ok"
    INVALID_ADDRESS = "invalid_address"
    VERSION_ADDRESS = "version_address"
    PERMISSION_DENIED = "permission_denied"


@dataclass
class FilesystemConfig(BackendConfig):
    """Configuration for the local filesystem storage backend.

    Extends BackendConfig with filesystem-specific options.

    Attributes:
        backend_type: Type/name of backend (always "filesystem")
        base_uri: Base URI for resource addresses (e.g., "file-storage://fileservice")
        static_dir: Directory for file storage
        folder_mode: Folder simulation mode: "native", "no_empty", or "placeholder"
        redirect_host: Host for redirect URLs (e.g., "http://localhost")
        redirect_port: Port for redirect URLs (e.g., 8011)
        extra_config: Additional configuration options
    """

    static_dir: Optional[str] = None
    folder_mode: str = "native"
    redirect_host: str = "http://localhost"
    redirect_port: int = 8011
    native_metadata_mode: str = "enabled"
    versioning_mode: str = "versioned"
    read_only: bool = False


# Note: FILE_SYSTEM_PROVIDER is dynamically set and may not be a FileSystemProvider instance
# It will be whatever backend type is initialized via init_backend()
__all__ = [
    "init_backend",
    "get_backend",
    "FILE_SYSTEM_PROVIDER",
    "FileSystemProvider",
    "FilesystemConfig",
    "STATIC_DIR",
    "REDIRECT_HOST",
    "REDIRECT_PORT",
    "SERVER_BASE_URI",
    "NativeMetadataMode",
    "NativeMetadataSupportAddressStatus",
    "NATIVE_METADATA_MODE",
    "parse_native_metadata_mode",
    "classify_native_metadata_support_address",
    "effective_native_metadata_support",
    "VersioningMode",
    "VERSIONING_MODE",
    "READ_ONLY",
    "parse_versioning_mode",
    "parse_read_only",
    "is_versioning_enabled",
    "is_read_only",
]

# =============================================================================
# Global Configuration (from environment variables, for backward compatibility)
# =============================================================================

# Default directory for storing files (used if FILESERVICE_STATIC_DIR not set)
default_tmpdir = os.path.join(tempfile.gettempdir(), "storage_api_test")

# Root directory where all files and metadata will be stored
# Structure:
#   STATIC_DIR/
#     content/     - Current versions of files by resource address
#     metadata/    - User-defined metadata key-value pairs
#     versions/    - Immutable version history by resource identity
#     uploads/     - Temporary storage for multipart uploads
STATIC_DIR = os.environ.get(FILESERVICE_STATIC_DIR_ENV, default_tmpdir)
os.makedirs(STATIC_DIR, exist_ok=True)

# Host and port for redirect URLs (when clients should upload/download via alternate endpoint)
REDIRECT_HOST = os.getenv(REDIRECT_HOST_ENV, REDIRECT_HOST_DEFAULT)
REDIRECT_PORT = int(os.getenv(REDIRECT_PORT_ENV, str(REDIRECT_PORT_DEFAULT)))

# Base URI that will be prepended to all resource addresses
# Example: "file-storage://fileservice" + "/path/to/file" = "file-storage://fileservice/path/to/file"
# Note that these are plus quoted when used in the REST endpoints
SERVER_BASE_URI = os.environ.get(FILESERVICE_SERVER_BASE_URI_ENV, FILESERVICE_SERVER_BASE_URI_DEFAULT)


def parse_native_metadata_mode(value: str) -> NativeMetadataMode:
    normalized = value.strip().lower()
    try:
        return NativeMetadataMode(normalized)
    except ValueError as e:
        raise ValueError(f"Got invalid value for native metadata mode: {value}, use one of 'enabled', 'disabled'") from e


NATIVE_METADATA_MODE = parse_native_metadata_mode(
    os.environ.get(FILESERVICE_NATIVE_METADATA_MODE_ENV, FILESERVICE_NATIVE_METADATA_MODE_DEFAULT)
)


def parse_versioning_mode(value: str) -> VersioningMode:
    normalized = value.strip().lower()
    try:
        return VersioningMode(normalized)
    except ValueError as e:
        raise ValueError(f"Got invalid value for versioning mode: {value}, use one of 'versioned', 'unversioned'") from e


def parse_read_only(value: bool | str) -> bool:
    if isinstance(value, bool):
        return value
    normalized = value.strip().lower()
    if normalized in ("true", "1", "yes"):
        return True
    if normalized in ("false", "0", "no", ""):
        return False
    raise ValueError(f"Got invalid value for read-only: {value}, use 'true' or 'false'")


VERSIONING_MODE = parse_versioning_mode(os.environ.get(FILESERVICE_VERSIONING_MODE_ENV, FILESERVICE_VERSIONING_MODE_DEFAULT))
READ_ONLY = parse_read_only(os.environ.get(FILESERVICE_READ_ONLY_ENV, FILESERVICE_READ_ONLY_DEFAULT))


def is_versioning_enabled() -> bool:
    """True when the backend keeps a version history; false for unversioned storage."""
    return VERSIONING_MODE == VersioningMode.VERSIONED


def is_read_only() -> bool:
    """True when the backend rejects mutations (write/delete/move)."""
    return READ_ONLY


# =============================================================================
# Global Storage Backend Instance
# =============================================================================

# Global storage backend instance (initialized by init_backend())
# This is set by the CLI entry points and used by the service layer
_storage_backend: Optional[StorageBackendInterface] = None

# Legacy FILE_SYSTEM_PROVIDER reference that will always point to the current backend
FILE_SYSTEM_PROVIDER: Optional[StorageBackendInterface] = None


def init_backend(config: BackendConfig) -> StorageBackendInterface:
    """Initialize the global storage backend instance.

    This should be called once at application startup with the desired backend
    configuration. Subsequent calls will replace the existing backend.

    Args:
        config: Configuration for the storage backend (e.g., FilesystemConfig)

    Returns:
        The created storage backend instance

    Example:
        config = FilesystemConfig(
            backend_type="filesystem",
            base_uri="file-storage://fileservice",
            static_dir="/tmp/storage"
        )
        backend = init_backend(config)
    """
    global _storage_backend, FILE_SYSTEM_PROVIDER, NATIVE_METADATA_MODE, VERSIONING_MODE, READ_ONLY
    if hasattr(config, "native_metadata_mode"):
        NATIVE_METADATA_MODE = parse_native_metadata_mode(config.native_metadata_mode)
    if hasattr(config, "versioning_mode"):
        VERSIONING_MODE = parse_versioning_mode(config.versioning_mode)
    if hasattr(config, "read_only"):
        READ_ONLY = parse_read_only(config.read_only)
    _storage_backend = create_backend(config)
    # Update FILE_SYSTEM_PROVIDER to point to the new backend for backward compatibility
    FILE_SYSTEM_PROVIDER = _storage_backend
    logger.info(f"Initialized storage backend: {config.backend_type}")
    return _storage_backend


def get_backend() -> StorageBackendInterface:
    """Get the global storage backend instance.

    Returns:
        The current storage backend

    Raises:
        RuntimeError: If backend has not been initialized
    """
    if _storage_backend is None:
        raise RuntimeError(
            "Storage backend not initialized. Call init_backend() first "
            "or use the legacy FILE_SYSTEM_PROVIDER for backward compatibility."
        )
    return _storage_backend


def classify_native_metadata_support_address(
    resource_address: str,
    backend: Optional[StorageBackendInterface] = None,
) -> NativeMetadataSupportAddressStatus:
    """Classify address/auth readiness for a native metadata support decision."""
    selected_backend = backend or get_backend()
    if not selected_backend.is_address_valid(resource_address):
        return NativeMetadataSupportAddressStatus.INVALID_ADDRESS
    if selected_backend.is_version_address(resource_address):
        return NativeMetadataSupportAddressStatus.VERSION_ADDRESS
    try:
        if not selected_backend.check_write_permission_on_address(resource_address):
            return NativeMetadataSupportAddressStatus.PERMISSION_DENIED
    except (IsADirectoryError, ValueError):
        return NativeMetadataSupportAddressStatus.INVALID_ADDRESS
    except (PermissionError, NotImplementedError):
        return NativeMetadataSupportAddressStatus.PERMISSION_DENIED
    except OSError:
        return NativeMetadataSupportAddressStatus.PERMISSION_DENIED
    return NativeMetadataSupportAddressStatus.OK


def effective_native_metadata_support(
    resource_address: str,
    backend: Optional[StorageBackendInterface] = None,
    mode: Optional[NativeMetadataMode | str] = None,
) -> bool:
    """Return the pure effective native metadata support value for an address."""
    selected_backend = backend or get_backend()
    selected_mode = parse_native_metadata_mode(mode) if isinstance(mode, str) else mode or NATIVE_METADATA_MODE
    if selected_mode == NativeMetadataMode.DISABLED:
        return False
    return selected_backend.supports_native_metadata(resource_address)


# =============================================================================
# Legacy Support (Backward Compatibility)
# =============================================================================

# Initialize default backend using environment variables for backward compatibility
_default_backend = FileSystemProvider(
    STATIC_DIR,
    SERVER_BASE_URI,
    os.environ.get(FILESERVICE_TEST_FOLDER_MODE_ENV, FILESERVICE_TEST_FOLDER_MODE_DEFAULT),
    read_only=READ_ONLY,
    versioned=is_versioning_enabled(),
)
_storage_backend = _default_backend
FILE_SYSTEM_PROVIDER = _default_backend
logger.info("Initialized default filesystem backend from environment variables")


# Import and register built-in backends
def _register_builtin_backends():
    """Register built-in storage backends and their CLI commands."""

    @register_backend("filesystem")
    def create_filesystem_backend(config: FilesystemConfig) -> StorageBackendInterface:
        """Create a local filesystem storage backend.

        This backend stores files in a local directory structure with support
        for versioning, metadata, and multipart uploads.

        Args:
            config: FilesystemConfig with filesystem-specific options
        """
        # Use provided static_dir or create default
        parse_native_metadata_mode(config.native_metadata_mode)
        static_dir = config.static_dir
        if not static_dir:
            static_dir = default_tmpdir

        os.makedirs(static_dir, exist_ok=True)

        return FileSystemProvider(
            static_dir=static_dir,
            base_uri=config.base_uri,
            test_folder_mode=config.folder_mode,
            read_only=parse_read_only(config.read_only),
            versioned=parse_versioning_mode(config.versioning_mode) == VersioningMode.VERSIONED,
        )

    @register_backend_cli("filesystem", "Start service with local filesystem storage")
    def filesystem_cli_command(
        base_uri: Annotated[
            str,
            typer.Option(
                "--base-uri",
                help="Base URI for resource addresses",
                envvar=FILESERVICE_SERVER_BASE_URI_ENV,
            ),
        ] = FILESERVICE_SERVER_BASE_URI_DEFAULT,
        static_dir: Annotated[
            Optional[str],
            typer.Option(
                "--static-dir",
                help="Directory for file storage",
                envvar=FILESERVICE_STATIC_DIR_ENV,
            ),
        ] = None,
        folder_mode: Annotated[
            str,
            typer.Option(
                "--folder-mode",
                help="Folder simulation mode: native, no_empty, or placeholder",
                envvar=FILESERVICE_TEST_FOLDER_MODE_ENV,
            ),
        ] = FILESERVICE_TEST_FOLDER_MODE_DEFAULT,
        redirect_host: Annotated[
            str,
            typer.Option(
                "--redirect-host",
                help="Host for redirect URLs",
                envvar=REDIRECT_HOST_ENV,
            ),
        ] = REDIRECT_HOST_DEFAULT,
        redirect_port: Annotated[
            int,
            typer.Option(
                "--redirect-port",
                help="Port for redirect URLs",
                envvar=REDIRECT_PORT_ENV,
            ),
        ] = REDIRECT_PORT_DEFAULT,
        native_metadata_mode: Annotated[
            str,
            typer.Option(
                "--native-metadata-mode",
                help="Native metadata mode: enabled or disabled",
                envvar=FILESERVICE_NATIVE_METADATA_MODE_ENV,
            ),
        ] = FILESERVICE_NATIVE_METADATA_MODE_DEFAULT,
        versioning_mode: Annotated[
            str,
            typer.Option(
                "--versioning-mode",
                help="Versioning mode: versioned or unversioned",
                envvar=FILESERVICE_VERSIONING_MODE_ENV,
            ),
        ] = FILESERVICE_VERSIONING_MODE_DEFAULT,
        read_only: Annotated[
            bool,
            typer.Option(
                "--read-only/--no-read-only",
                help="Serve the backend as read-only (reject write/delete/move)",
                envvar=FILESERVICE_READ_ONLY_ENV,
            ),
        ] = False,
    ) -> FilesystemConfig:
        """Configure local filesystem backend.

        The filesystem backend stores files in a local directory with support for:
        - Multiple folder simulation modes (native, no_empty, placeholder)
        - Versioning and metadata
        - Redirect-based uploads/downloads

        Examples:
            # Use default temporary directory
            ... filesystem

            # Specify custom storage directory
            ... filesystem --static-dir /data/storage

            # S3-like folder behavior (no empty folders)
            ... filesystem --folder-mode no_empty

            # Custom redirect endpoint
            ... filesystem --redirect-host http://cdn.example.com --redirect-port 9000
        """
        return FilesystemConfig(
            backend_type="filesystem",
            base_uri=base_uri,
            static_dir=static_dir,
            folder_mode=folder_mode,
            redirect_host=redirect_host,
            redirect_port=redirect_port,
            native_metadata_mode=native_metadata_mode,
            versioning_mode=versioning_mode,
            read_only=read_only,
        )


# Register built-in backends on module import
_register_builtin_backends()
