# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import os
import sys

import pytest


def run_tests() -> None:
    """Entry point for the run-conformance-tests CLI command."""
    import conformance_tests

    package_dir = os.path.dirname(os.path.abspath(conformance_tests.__file__))
    extra_args = sys.argv[1:]
    default_args = [
        package_dir,
        "--tb=short",
        "-v",
        "--junit-xml=reports/conformance-results.xml",
    ]
    os.makedirs("reports", exist_ok=True)
    sys.exit(pytest.main(default_args + extra_args))
