# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

import json
from pathlib import Path

import pytest
import requests

from discovery_conformance import rest_capabilities


@pytest.mark.parametrize("payload, expected", [
    ({"services": []}, "PASS"), ({}, "PASS"),
    ({"services": "wrong-type"}, "FAIL"),
    ({"services": [{"rest": [{"endpoints": [{"versions": [{}]}]}]}]}, "FAIL"),
])
def test_existing_openapi_contract(monkeypatch, payload, expected):
    response = requests.Response()
    response.status_code = 200
    response.headers["Content-Type"] = "application/json"
    response._content = json.dumps(payload).encode()
    monkeypatch.setattr(rest_capabilities, "get", lambda *args, **kwargs: response)
    checks = rest_capabilities.check_rest_capabilities("https://example.test/prefix/v1beta/services")
    assert checks[0]["status"] == expected


def test_backend_failure_does_not_leak_details(monkeypatch):
    def fail(*args, **kwargs):
        raise requests.ConnectionError("sensitive request details")
    monkeypatch.setattr(rest_capabilities, "get", fail)
    checks = rest_capabilities.check_rest_capabilities("https://example.test/v1beta/services")
    assert checks[0]["status"] == "INCONCLUSIVE"
    assert "sensitive" not in str(checks)


def test_bundled_spec_matches_repository():
    root = Path(__file__).resolve().parents[2]
    assert (root / "conformance/specs/capabilities_clean.openapi.yaml").read_bytes() == (root / "openapi/capabilities_clean.openapi.yaml").read_bytes()
