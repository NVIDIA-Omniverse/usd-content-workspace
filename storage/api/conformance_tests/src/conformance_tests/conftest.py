# SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

import os
import pathlib

import pytest


def pytest_configure(config):
    config.addinivalue_line("markers", "optional: mark test to allow for selection of running non-optional tests or only optional tests")
    config.addinivalue_line("markers", "unversioned: requires the service running with FILESERVICE_VERSIONING_MODE=unversioned")
    config.addinivalue_line("markers", "read_only: requires the service running with FILESERVICE_READ_ONLY=true")


def pytest_collection_modifyitems(config, items):
    """Skip the storage-mode scenarios unless the service under test runs in that mode.

    One server process serves one mode, so the ``@unversioned`` / ``@read_only`` scenarios only
    pass when the service was started with the matching environment. Keying the skip off the same
    environment (and the test module name) keeps them out of every default run — including the
    docker blackbox runner, which drives pytest directly without a marker filter — and lets the
    dedicated mode runs (``make conformance_test_unversioned`` / ``conformance_test_readonly``)
    execute them.
    """
    unversioned = os.environ.get("FILESERVICE_VERSIONING_MODE", "versioned").strip().lower() == "unversioned"
    read_only = os.environ.get("FILESERVICE_READ_ONLY", "false").strip().lower() in ("true", "1", "yes")
    skip_unversioned = pytest.mark.skip(reason="requires FILESERVICE_VERSIONING_MODE=unversioned")
    skip_read_only = pytest.mark.skip(reason="requires FILESERVICE_READ_ONLY=true")
    for item in items:
        if "test_unversioned_storage" in item.nodeid and not unversioned:
            item.add_marker(skip_unversioned)
        if "test_read_only_storage" in item.nodeid and not read_only:
            item.add_marker(skip_read_only)


def discover_pytest_plugins(root_dir: str) -> list[str]:
    """
    Recursively find all Python files under `root_dir` that start with 'call_', 'verify_', or 'common_',
    and return a list of dotted module paths for use in `pytest_plugins`.
    """
    base_path = pathlib.Path(__file__).parent / root_dir
    if not base_path.exists():
        return []

    plugins = []

    # Determine if we're running from installed package or source
    conftest_path = pathlib.Path(__file__).resolve()

    # Try to get the package name from the module itself
    try:
        package_name = __name__.split(".")[0]  # e.g., 'conformance_tests'
    except:
        # Fallback: use parent directory name
        package_name = conftest_path.parent.name

    package_root = conftest_path.parent

    for path in base_path.rglob("*.py"):
        if path.name.startswith(("call_", "verify_", "common_")):
            rel_path = path.relative_to(package_root)
            module = f"{package_name}." + ".".join(rel_path.with_suffix("").parts)
            plugins.append(module)

    return plugins


# Automatically populate pytest_plugins from the steps tree, all call_x, verify_x, and common_x files are added automatically
pytest_plugins = discover_pytest_plugins("steps")
