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

"""
Shared fixtures and helpers for Discovery Service conformance tests.

Configuration is read from environment variables:

  CONFORMANCE_REST_ENDPOINT   - base URL of the discovery service
                                (default: http://localhost:8080)
  CONFORMANCE_HEALTH_ENDPOINT - health-check URL used by wait_for_service.sh
                                (default: http://localhost:8080/health)
  CONFORMANCE_REQUEST_TIMEOUT - per-request timeout in seconds (default: 30.0)
"""

import os

import httpx
import pytest

REST_ENDPOINT = os.environ.get("CONFORMANCE_REST_ENDPOINT", "http://localhost:8080")
REQUEST_TIMEOUT = float(os.environ.get("CONFORMANCE_REQUEST_TIMEOUT", "30.0"))


@pytest.fixture(scope="session")
def http_client():
    with httpx.Client(
        base_url=REST_ENDPOINT,
        timeout=httpx.Timeout(REQUEST_TIMEOUT, connect=10.0),
    ) as client:
        yield client
