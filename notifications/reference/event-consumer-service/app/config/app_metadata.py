# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""App metadata sourced from pyproject.toml `[tool.app]`."""

import tomllib
from functools import cache
from pathlib import Path


@cache
def api_version() -> str:
    """The REST API version from pyproject `[tool.app] api_version`.

    Single source of truth for both the mounted REST router prefix
    (app/rest/rest_server.py) and the advertised Discovery capability
    path (app/grpc/grpc_server.py).
    """
    pyproject = tomllib.loads(Path("pyproject.toml").read_text())
    return pyproject["tool"]["app"]["api_version"]


@cache
def api_path_prefix() -> str:
    """The `/api/{version}` REST path prefix, derived once from api_version().

    Both the mounted REST router prefix and the advertised capability path
    derive from this, so they cannot drift.
    """
    return f"/api/{api_version()}"
