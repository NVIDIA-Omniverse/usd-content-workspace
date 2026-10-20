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

"""Read-only HTTP checks for the operator catalog, not target capabilities."""

from importlib.resources import as_file, files
from urllib.parse import urlsplit

import requests
from openapi_core import OpenAPI
from openapi_core.contrib.requests import RequestsOpenAPIRequest, RequestsOpenAPIResponse

from .transport import get


def _result(check_id: str, status: str, message: str) -> dict:
    return {"id": check_id, "status": status, "message": message}


def check_catalog(
    url: str,
    *,
    token: str | None = None,
    ca_bundle: str | bool = True,
    timeout: float = 10,
    expected_names: list[str] | None = None,
) -> list[dict]:
    """Check the exact supplied catalog operation URL without redirects.

    Diagnostics deliberately exclude exception text, URLs, and response data.
    An empty catalog passes the wire contract, not deployment acceptance.
    """
    try:
        parts = urlsplit(url)
        if parts.scheme not in ("http", "https") or not parts.netloc or parts.username:
            raise ValueError("invalid URL")
    except ValueError:
        return [_result("catalog.http", "FAIL", "Catalog operation URL is invalid.")]
    try:
        response = get(url, token=token, ca_bundle=ca_bundle, timeout=timeout)
    except (requests.RequestException, ValueError, OSError):
        return [_result("catalog.http", "INCONCLUSIVE", "Catalog request could not be completed; the HTTP contract could not be established.")]

    if response.status_code != 200:
        return [_result("catalog.http", "FAIL", "Catalog did not return HTTP 200.")]
    results = [_result("catalog.http", "PASS", "Catalog returned HTTP 200.")]
    try:
        resource = files("discovery_conformance.specs").joinpath("discovery_catalog.openapi.yaml")
        with as_file(resource) as spec_path:
            with spec_path.open(encoding="utf-8") as spec_file:
                api = OpenAPI.from_file(spec_file)
    except Exception:
        results.append(_result("catalog.schema", "INCONCLUSIVE", "Catalog validation profile could not be loaded."))
        return results

    try:
        # Validate against the canonical operation even behind a URL prefix.
        request = requests.Request("GET", "https://conformance.invalid/api/v2beta/services")
        api.validate_response(RequestsOpenAPIRequest(request), RequestsOpenAPIResponse(response))
        body = response.json()
        # Validators may coerce primitive types; require the actual JSON integer.
        if type(body["schema-version"]) is not int or body["schema-version"] != 2:
            raise ValueError("schema version")
    except Exception:
        results.append(_result("catalog.schema", "FAIL", "Response does not match the catalog HTTP profile."))
        return results
    results.append(_result("catalog.schema", "PASS", "Response matches the documentation-aligned catalog profile."))
    names = [service["name"] for service in body["services"]]
    results.append(_result(
        "catalog.order", "PASS" if names == sorted(names) else "FAIL",
        "Services are sorted by name." if names == sorted(names) else "Services are not sorted by name.",
    ))
    cache = response.headers.get("Cache-Control", "")
    directives = [
        (name.strip().lower(), value.strip().strip('"') if separator else None)
        for directive in requests.utils.parse_list_header(cache)
        for name, separator, value in [directive.partition("=")]
    ]
    max_ages = [value for name, value in directives if name == "max-age"]
    cache_valid = bool(max_ages) and all(value == "30" for value in max_ages)
    cache_valid = cache_valid and ("must-revalidate", None) in directives
    results.append(_result(
        "catalog.cache", "PASS" if cache_valid else "FAIL",
        "Catalog cache policy includes the documented directives." if cache_valid
        else "Catalog cache policy differs from the documented value.",
    ))
    if expected_names is None:
        results.append(_result("catalog.inventory", "SKIP", "No expected inventory supplied; a valid catalog does not prove discovery completeness."))
    else:
        missing = set(expected_names) - set(names)
        results.append(_result(
            "catalog.inventory", "FAIL" if missing else "PASS",
            "Expected service names are missing." if missing else "All expected service names are present.",
        ))
    return results
