# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import re
from functools import cache
from pathlib import Path

from notification_common.errors.errors import ServerError


@cache
def version_number():
    return _read_pyproject_field("version")


@cache
def project_name():
    return _read_pyproject_field("name")


def _read_pyproject_field(field_name):
    file = open(_pyproject_path(), "r")
    file_contents = file.read()
    file.close()

    match = re.search(rf'{field_name} = "([\s\S]*?)"', file_contents)
    if match is None:
        raise ServerError(f"{field_name} not found in pyproject.toml")
    return match.group(1)


def _pyproject_path():
    # First try to find pyproject.toml in the current working directory
    cwd = Path.cwd()
    pyproject_path = cwd / "pyproject.toml"

    if pyproject_path.exists():
        return str(pyproject_path)

    # If not found, fall back to the package's location
    return str(Path(__file__).parent.parent.parent / "pyproject.toml")
