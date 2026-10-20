# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Utility for measuring gRPC message sizes.

This module provides a simple way to get the size of gRPC messages.
"""

from typing import Any


def get_message_size(message: Any, default_size: int = 0) -> int:
    """
    Get the size of a gRPC message using the ByteSize() method if available.

    Args:
        message: The gRPC message to measure
        default_size: Default size to return if size calculation fails

    Returns:
        The size in bytes, or default_size if size couldn't be determined.
        Returns None only if message is None.
    """
    # Skip if message is None
    if message is None:
        return 0

    # Try to get size using ByteSize() method first (protobuf standard)
    try:
        if hasattr(message, "ByteSize"):
            return message.ByteSize()
    except Exception:
        pass

    # Try fallback method using __sizeof__ if available
    try:
        if hasattr(message, "__sizeof__"):
            return message.__sizeof__()
    except Exception:
        pass

    # Return default size if we couldn't determine size
    return default_size
