# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

import sys
from pathlib import Path


def run_tests():
    import pytest

    tests_dir = Path(__file__).resolve().parent
    sys.exit(
        pytest.main(
            [
                str(tests_dir),
                "--junit-xml=reports/conformance-results.xml",
                "--strict-markers",
                "--override-ini",
                (
                    "markers=rest: REST API tests\n"
                    "optional: tests that are informational and may be skipped"
                ),
                "-p",
                "no:warnings",
                "-p",
                "no:cacheprovider",
                *sys.argv[1:],
            ]
        )
    )
