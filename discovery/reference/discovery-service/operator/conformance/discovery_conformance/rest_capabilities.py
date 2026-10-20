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

"""Validate the separate v1beta REST Capabilities contract."""

from importlib.resources import files

import requests
from openapi_core import OpenAPI
from openapi_core.contrib.requests import RequestsOpenAPIRequest, RequestsOpenAPIResponse

from .transport import get


def check_rest_capabilities(url, *, token=None, ca_bundle=True, timeout=10):
    try:
        response = get(url, token=token, ca_bundle=ca_bundle, timeout=timeout)
    except (requests.RequestException, ValueError, OSError):
        return [{"id": "capabilities-rest.reachable", "status": "INCONCLUSIVE",
                 "message": "REST Capabilities request failed; check connectivity, TLS and timeout."}]
    if response.status_code != 200:
        return [{"id": "capabilities-rest.status", "status": "FAIL",
                 "message": f"Expected HTTP 200, received {response.status_code}; redirects are not followed."}]
    try:
        spec = files("discovery_conformance.specs").joinpath("capabilities_clean.openapi.yaml")
        api = OpenAPI.from_file_path(str(spec))
    except Exception:
        return [{"id": "capabilities-rest.openapi", "status": "INCONCLUSIVE",
                 "message": "REST Capabilities validation profile could not be loaded."}]
    # Validate the operation independently of an ingress prefix on the supplied URL.
    request = requests.Request("GET", "https://contract.invalid/v1beta/services").prepare()
    try:
        api.validate_response(RequestsOpenAPIRequest(request), RequestsOpenAPIResponse(response))
    except Exception as error:
        return [{"id": "capabilities-rest.openapi", "status": "FAIL",
                 "message": f"Response violates the v1beta OpenAPI contract ({type(error).__name__})."}]
    return [{"id": "capabilities-rest.openapi", "status": "PASS",
             "message": "Response matches the v1beta REST Capabilities OpenAPI contract."}]
