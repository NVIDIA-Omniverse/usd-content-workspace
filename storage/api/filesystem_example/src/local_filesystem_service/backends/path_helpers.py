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

"""Helper functions for gRPC service request handlers.

This module provides common utility functions used across multiple gRPC
service handlers for constructing response messages and validating versioning constraints.
"""

import os
import urllib.parse


def get_relative_path_from_address(base_url: str, resource_address: str) -> str:
    """Extract the relative path from a resource address.

    This is a backend-agnostic helper that extracts the path portion
    from a resource address by removing the backend's base URI.

    Args:
        base_url: Base resource address (e.g., "file-storage://server/")
        resource_address: Full resource address (e.g., "file-storage://server/path/to/file")

    Returns:
        Relative path portion (e.g., "/path/to/file")
    """
    parsed_address = urllib.parse.urlparse(resource_address)
    base_address = urllib.parse.urlparse(base_url)
    if parsed_address.scheme != base_address.scheme or parsed_address.netloc != base_address.netloc:
        raise ValueError(f"{resource_address} is not within base address {base_url}, program error!")
    path = parsed_address.path
    # Check if the path is absolute
    if os.path.isabs(path):
        # Remove drive letter (if present)
        path_without_drive = os.path.splitdrive(path)[1]
        # Remove leading slashes or backslashes
        path = path_without_drive.lstrip("/\\")

    # Sanitize path to prevent directory traversal attacks
    return sanitize_path(path)


def sanitize_path(path: str) -> str:
    """Sanitize a path to prevent directory traversal attacks while supporting . and .. operations."""
    # Convert to use forward slashes for consistency
    normalized = path.replace("\\", "/")

    # Split into components and process them with a stack-based approach
    components: list[str] = []
    for component in normalized.split("/"):
        if component == "" or component == ".":
            # Skip empty components and current directory references
            continue
        elif component == "..":
            # Handle parent directory - only pop if we have components to pop
            if components:
                components.pop()
            else:
                # Attempting to go above the root - this is a path traversal attack
                raise ValueError(f"Path traversal attempt detected: {path}")
        else:
            # Regular component - add it to the stack
            components.append(component)

    # Rejoin the safe components
    safe_path = "/".join(components)

    # Ensure the resulting path doesn't start with / (should be relative)
    return safe_path.lstrip("/")
