# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

import pytest
from pytest_bdd import (
    given,
    parsers,
    then,
)

from ..utils.structured_api_call import structured_api_call


def _call_is_versioned(protocol, status_code, scenario_state):
    """Call IsVersioned over the selected transport and validate its status."""
    resource_address = scenario_state.resource_address
    versioning_version = scenario_state.version_under_test["versioning"]

    def grpc_call():
        pb2 = scenario_state.sapi_test_client.versioning_pb2(versioning_version)
        return scenario_state.sapi_test_client.versioning_grpc_client(versioning_version).IsVersioned(
            pb2.IsVersionedRequest(resource_address=resource_address)
        )

    def rest_call():
        return scenario_state.sapi_test_client.rest_client().is_versioned(resource_address, versioning_version)

    return structured_api_call(protocol, status_code, grpc_call=grpc_call, rest_call=rest_call)


@given(parsers.parse("the '{protocol}' storage retains object versions"))
def storage_retains_object_versions(protocol, scenario_state):
    """Skip version-retention scenarios when the selected storage is unversioned."""
    status_code = "OK" if protocol == "GRPC" else "200"
    response = _call_is_versioned(protocol, status_code, scenario_state)
    is_versioned = response.is_versioned if protocol == "GRPC" else response.json()["is_versioned"]
    if not is_versioned:
        pytest.skip("storage does not retain object versions")


@then(parsers.parse("calling '{protocol}' IsVersioned on that address returns '{status_code}' with a boolean is_versioned field"))
def calling_is_versioned_on_address_returns_status_code(protocol, status_code, scenario_state):
    """Verify IsVersioned returns a boolean value for the resource address."""
    response = _call_is_versioned(protocol, status_code, scenario_state)
    if protocol == "GRPC":
        assert isinstance(response.is_versioned, bool)
    else:
        assert isinstance(response.json()["is_versioned"], bool)


@then(parsers.parse("calling '{protocol}' IsVersioned on that address returns '{status_code}' with is_versioned '{expected}'"))
def calling_is_versioned_on_address_returns_value(protocol, status_code, expected, scenario_state):
    """Verify IsVersioned returns the expected value for the resource address."""
    response = _call_is_versioned(protocol, status_code, scenario_state)
    expected_value = expected.strip().lower() == "true"
    actual = response.is_versioned if protocol == "GRPC" else response.json()["is_versioned"]
    assert actual == expected_value, f"expected is_versioned={expected_value}, got {actual}"
