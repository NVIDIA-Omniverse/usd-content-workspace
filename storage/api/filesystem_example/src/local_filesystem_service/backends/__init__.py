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

from .backend_factory import (
    BackendConfig,
    create_backend,
    get_backend_cli_commands,
    has_backend_cli,
    is_backend_registered,
    list_backends,
    register_backend,
    register_backend_cli,
)
from .storage_backend_interface import (
    EtagMismatchError,
    ListEntry,
    Metadata,
    MetadataKeyNotFoundError,
    RedirectUploadResult,
    StorageBackendInterface,
    VersionInfo,
    VersionsOrder,
)

__all__ = [
    # Interface
    "StorageBackendInterface",
    "Metadata",
    "VersionInfo",
    "ListEntry",
    "VersionsOrder",
    "EtagMismatchError",
    "MetadataKeyNotFoundError",
    "RedirectUploadResult",
    # Factory
    "BackendConfig",
    "create_backend",
    "register_backend",
    "register_backend_cli",
    "list_backends",
    "is_backend_registered",
    "get_backend_cli_commands",
    "has_backend_cli",
]
