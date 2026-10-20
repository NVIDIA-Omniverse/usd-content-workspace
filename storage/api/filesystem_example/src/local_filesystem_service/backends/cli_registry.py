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

"""Registry for backend CLI commands."""

import logging
from typing import (
    TYPE_CHECKING,
    Callable,
    Dict,
)

if TYPE_CHECKING:
    from .backend_factory import BackendConfig

logger = logging.getLogger(__name__)

# Type for backend CLI functions
# These functions take CLI arguments and return a BackendConfig
BackendCliFunc = Callable[..., "BackendConfig"]

# Global registry of backend CLI commands
_backend_cli_registry: Dict[str, BackendCliFunc] = {}


def register_backend_cli(name: str, help_text: str = "") -> Callable[[BackendCliFunc], BackendCliFunc]:
    """Decorator to register a backend CLI command.

    Args:
        name: Name of the backend command (e.g., "filesystem")
        help_text: Help text for the command

    Returns:
        Decorator function
    """

    def decorator(cli_func: BackendCliFunc) -> BackendCliFunc:
        if name in _backend_cli_registry:
            logger.warning(f"Backend CLI '{name}' is already registered, overwriting")

        # Attach docstring if provided (and not already present)
        if help_text and not cli_func.__doc__:
            cli_func.__doc__ = help_text

        _backend_cli_registry[name] = cli_func
        logger.info(f"Registered storage backend CLI: {name}")
        return cli_func

    return decorator


def get_backend_cli_commands() -> Dict[str, BackendCliFunc]:
    """Get all registered backend CLI commands.

    Returns:
        Dictionary mapping backend name to CLI function
    """
    return _backend_cli_registry.copy()


def has_backend_cli(name: str) -> bool:
    """Check if a backend has a registered CLI command.

    Args:
        name: Backend name to check

    Returns:
        True if registered, False otherwise
    """
    return name in _backend_cli_registry
