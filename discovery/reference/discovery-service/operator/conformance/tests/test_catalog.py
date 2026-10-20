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
from unittest.mock import patch

import pytest
import requests

from discovery_conformance.catalog import check_catalog


def response(body, status=200, content_type="application/json"):
    result = requests.Response()
    result.status_code = status
    result.headers["Content-Type"] = content_type
    result.headers["Cache-Control"] = "max-age=30, must-revalidate"
    result._content = json.dumps(body).encode()
    return result


def run(body, **kwargs):
    with patch("discovery_conformance.catalog.get", return_value=response(body)):
        return {item["id"]: item for item in check_catalog("https://example.test", **kwargs)}


def service(name="storage"):
    return {
        "name": name, "capability_source": "grpc",
        "rest": {"url": "https://rest.test", "endpoints": [
            {"name": "files", "versions": {"v1": {}}}
        ]},
        "grpc": {"url": "https://grpc.test", "endpoints": [{"service": "example.Files"}]},
    }


def test_empty_is_valid_not_inventory_proof():
    results = run({"schema-version": 2, "services": []})
    assert results["catalog.schema"]["status"] == "PASS"
    assert results["catalog.inventory"]["status"] == "SKIP"
    assert run({"schema-version": 2, "services": []}, expected_names=["storage"])["catalog.inventory"]["status"] == "FAIL"


def test_transports_and_optional_paths():
    body = {"schema-version": 2, "services": [service()]}
    assert run(body, expected_names=["storage"])["catalog.inventory"]["status"] == "PASS"
    body["services"][0]["rest"]["endpoints"][0]["versions"]["v1"]["path"] = "/files"
    assert run(body)["catalog.schema"]["status"] == "PASS"
    assert run({"schema-version": 2, "services": [{"name": "storage", "capability_source": "grpc"}]})["catalog.schema"]["status"] == "PASS"


@pytest.mark.parametrize("body", [
    {}, {"schema-version": 1, "services": []},
    {"schema-version": "2", "services": []},
    {"schema-version": 2.0, "services": []},
    {"schema-version": 2, "services": None},
    {"schema-version": 2, "services": [{}]},
    {"schema-version": 2, "services": [{"name": "x", "capability_source": "grpc", "rest": None}]},
    {"schema-version": 2, "services": [{"name": "x", "capability_source": "grpc", "grpc": None}]},
])
def test_invalid_schema(body):
    assert run(body)["catalog.schema"]["status"] == "FAIL"


@pytest.mark.parametrize("mutation", [
    lambda s: s["rest"].pop("url"),
    lambda s: s["grpc"]["endpoints"][0].pop("service"),
    lambda s: s["rest"]["endpoints"][0].update(versions=[]),
    lambda s: s["rest"]["endpoints"][0]["versions"]["v1"].update(path=None),
])
def test_invalid_transport(mutation):
    entry = service()
    mutation(entry)
    assert run({"schema-version": 2, "services": [entry]})["catalog.schema"]["status"] == "FAIL"


def test_sorting_and_expected_names():
    results = run({"schema-version": 2, "services": [service("z"), service("a")]}, expected_names=["a"])
    assert results["catalog.order"]["status"] == "FAIL"
    assert results["catalog.inventory"]["status"] == "PASS"


@pytest.mark.parametrize("status", [201, 302, 401, 404, 500])
def test_exact_status(status):
    with patch("discovery_conformance.catalog.get", return_value=response({}, status)):
        assert check_catalog("https://example.test")[0]["status"] == "FAIL"


def test_options_and_redaction():
    with patch("discovery_conformance.catalog.get", return_value=response({"schema-version": 2, "services": []})) as get:
        results = check_catalog("https://example.test/api/v2beta/services", token="secret", ca_bundle="ca.pem", timeout=3)
        get.assert_called_once_with("https://example.test/api/v2beta/services", token="secret", ca_bundle="ca.pem", timeout=3)
        assert "secret" not in json.dumps(results)
    with patch("discovery_conformance.catalog.get", side_effect=requests.Timeout("secret response body")):
        assert "secret" not in json.dumps(check_catalog("https://example.test"))


def test_content_type_and_invalid_json():
    for result in [response({}, content_type="text/html"), response({})]:
        result._content = b"secret not JSON"
        with patch("discovery_conformance.catalog.get", return_value=result):
            checks = check_catalog("https://example.test")
            assert checks[-1]["status"] == "FAIL"
            assert "secret" not in json.dumps(checks)


def test_cache_policy():
    result = response({"schema-version": 2, "services": []})
    result.headers.pop("Cache-Control")
    with patch("discovery_conformance.catalog.get", return_value=result):
        checks = {item["id"]: item for item in check_catalog("https://example.test")}
        assert checks["catalog.cache"]["status"] == "FAIL"


def test_wrong_content_type_with_valid_json():
    result = response({"schema-version": 2, "services": []}, content_type="text/html")
    with patch("discovery_conformance.catalog.get", return_value=result):
        assert check_catalog("https://example.test")[-1]["status"] == "FAIL"


def test_profile_failure_is_inconclusive():
    with patch("discovery_conformance.catalog.get", return_value=response({})), patch(
        "discovery_conformance.catalog.OpenAPI.from_file", side_effect=ValueError("secret")
    ):
        results = check_catalog("https://example.test")
        assert results[-1]["status"] == "INCONCLUSIVE"
        assert "secret" not in json.dumps(results)


@pytest.mark.parametrize("url", ["file:///etc/passwd", "https://user:secret@example.test", "not-a-url"])
def test_invalid_url_never_sends(url):
    with patch("discovery_conformance.catalog.get") as get:
        assert check_catalog(url)[0]["status"] == "FAIL"
        get.assert_not_called()


@pytest.mark.parametrize("error", [requests.Timeout, requests.ConnectionError, requests.exceptions.SSLError])
def test_unreachable_is_inconclusive(error):
    with patch("discovery_conformance.catalog.get", side_effect=error("secret")):
        checks = check_catalog("https://example.test/api/v2beta/services")
        assert checks[0]["status"] == "INCONCLUSIVE"
        assert "secret" not in json.dumps(checks)


@pytest.mark.parametrize("url", [
    "https://example.test/custom/services?view=external",
    "https://example.test/prefix/api/v2/services/",
    "https://example.test",
])
def test_exact_url_is_preserved(url):
    with patch("discovery_conformance.catalog.get", return_value=response({"schema-version": 2, "services": []})) as get:
        check_catalog(url)
        get.assert_called_once_with(url, token=None, ca_bundle=True, timeout=10)


@pytest.mark.parametrize("header, expected", [
    ("must-revalidate, max-age=30", "PASS"),
    (' PUBLIC , MAX-AGE = "30" , Must-Revalidate ', "PASS"),
    ('private="field,other", max-age=30, must-revalidate', "PASS"),
    ("max-age=30", "FAIL"),
    ("must-revalidate", "FAIL"),
    ("max-age=31, must-revalidate", "FAIL"),
    ("max-age=30, max-age=60, must-revalidate", "FAIL"),
    ("max-age=30, must-revalidate=false", "FAIL"),
])
def test_cache_directive_semantics(header, expected):
    result = response({"schema-version": 2, "services": []})
    result.headers["Cache-Control"] = header
    with patch("discovery_conformance.catalog.get", return_value=result):
        checks = {item["id"]: item for item in check_catalog("https://example.test/api/v2beta/services")}
        assert checks["catalog.cache"]["status"] == expected
