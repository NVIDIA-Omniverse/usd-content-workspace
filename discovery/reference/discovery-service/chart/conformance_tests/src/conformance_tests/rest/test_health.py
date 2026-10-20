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
REST conformance tests for GET /health.

Contract under test:
  GET /health

Expected behaviour:
  - Returns HTTP 200.
  - Response body indicates healthy status.
"""

import pytest

pytestmark = pytest.mark.rest


class TestHealth:
    def test_health_returns_200(self, http_client):
        response = http_client.get("/health")
        assert response.status_code == 200, (
            f"Expected HTTP 200 from GET /health, but got {response.status_code}."
        )

    def test_health_response_indicates_healthy(self, http_client):
        response = http_client.get("/health")
        assert "healthy" in response.text.lower(), (
            f"Expected response body to contain 'healthy', but got: {response.text!r}"
        )
