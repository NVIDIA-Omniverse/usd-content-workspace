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
REST conformance tests for the discovery endpoints.

Contract under test:
  GET /api/v1alpha/services
  GET /api/v1/services

Expected behaviour:
  - Returns HTTP 200 with a JSON body.
  - The body contains a top-level "schema-version" field equal to 1.
  - The body contains a non-empty "services" list.
  - Each service entry has the required fields: id, name, type.
  - Each service entry has at least one endpoint (grpc or rest).
"""

import pytest

pytestmark = pytest.mark.rest

_ENDPOINTS = ["/api/v1alpha/services", "/api/v1/services"]


class TestDiscoveryV1Alpha:
    def test_v1alpha_returns_200(self, http_client):
        response = http_client.get("/api/v1alpha/services")
        assert response.status_code == 200, (
            f"Expected HTTP 200 from GET /api/v1alpha/services, "
            f"but got {response.status_code}."
        )

    def test_v1alpha_returns_valid_json(self, http_client):
        response = http_client.get("/api/v1alpha/services")
        payload = response.json()
        assert isinstance(payload, dict), (
            f"Expected a JSON object at /api/v1alpha/services, got {type(payload)}."
        )

    def test_v1alpha_schema_version_is_1(self, http_client):
        payload = http_client.get("/api/v1alpha/services").json()
        assert payload.get("schema-version") == 1, (
            f"Expected 'schema-version' == 1, got {payload.get('schema-version')!r}."
        )

    def test_v1alpha_services_list_is_non_empty(self, http_client):
        payload = http_client.get("/api/v1alpha/services").json()
        services = payload.get("services", [])
        assert len(services) > 0, (
            "Expected a non-empty 'services' list in /api/v1alpha/services response."
        )

    def test_v1alpha_services_have_required_fields(self, http_client):
        payload = http_client.get("/api/v1alpha/services").json()
        for svc in payload.get("services", []):
            for field in ("id", "name", "type"):
                assert field in svc, (
                    f"Service entry is missing required field '{field}': {svc}"
                )

    def test_v1alpha_services_have_at_least_one_endpoint(self, http_client):
        payload = http_client.get("/api/v1alpha/services").json()
        for svc in payload.get("services", []):
            assert svc.get("grpc") or svc.get("rest"), (
                f"Service '{svc.get('id')}' has neither 'grpc' nor 'rest' endpoint."
            )


class TestDiscoveryV1:
    def test_v1_returns_200(self, http_client):
        response = http_client.get("/api/v1/services")
        assert response.status_code == 200, (
            f"Expected HTTP 200 from GET /api/v1/services, "
            f"but got {response.status_code}."
        )

    def test_v1_returns_valid_json(self, http_client):
        response = http_client.get("/api/v1/services")
        payload = response.json()
        assert isinstance(payload, dict), (
            f"Expected a JSON object at /api/v1/services, got {type(payload)}."
        )

    def test_v1_schema_version_is_1(self, http_client):
        payload = http_client.get("/api/v1/services").json()
        assert payload.get("schema-version") == 1, (
            f"Expected 'schema-version' == 1, got {payload.get('schema-version')!r}."
        )

    def test_v1_services_list_is_non_empty(self, http_client):
        payload = http_client.get("/api/v1/services").json()
        services = payload.get("services", [])
        assert len(services) > 0, (
            "Expected a non-empty 'services' list in /api/v1/services response."
        )

    def test_v1_services_have_required_fields(self, http_client):
        payload = http_client.get("/api/v1/services").json()
        for svc in payload.get("services", []):
            for field in ("id", "name", "type"):
                assert field in svc, (
                    f"Service entry is missing required field '{field}': {svc}"
                )

    def test_v1_services_have_at_least_one_endpoint(self, http_client):
        payload = http_client.get("/api/v1/services").json()
        for svc in payload.get("services", []):
            assert svc.get("grpc") or svc.get("rest"), (
                f"Service '{svc.get('id')}' has neither 'grpc' nor 'rest' endpoint."
            )
