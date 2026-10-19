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

from pytest_bdd import (
    parsers,
    then,
)

from ..utils.structured_api_call import structured_api_call


@then(parsers.parse("calling '{protocol}' IsReadOnly on that address returns '{status_code}' with a boolean is_read_only field"))
def calling_is_read_only_on_address_returns_status_code(protocol, status_code, scenario_state):
    resource_address = scenario_state.resource_address
    fileobject_version = scenario_state.version_under_test["fileobject"]

    def grpc_call():
        pb2 = scenario_state.sapi_test_client.fileobject_pb2(fileobject_version)
        return scenario_state.sapi_test_client.fileobject_grpc_client(fileobject_version).IsReadOnly(
            pb2.IsReadOnlyRequest(resource_address=resource_address)
        )

    def rest_call():
        return scenario_state.sapi_test_client.rest_client().is_read_only(resource_address, fileobject_version)

    response = structured_api_call(protocol, status_code, grpc_call=grpc_call, rest_call=rest_call)
    if protocol == "GRPC":
        assert isinstance(response.is_read_only, bool)
    else:
        assert isinstance(response.json()["is_read_only"], bool)


@then(parsers.parse("calling '{protocol}' IsReadOnly on that address returns '{status_code}' with is_read_only '{expected}'"))
def calling_is_read_only_on_address_returns_value(protocol, status_code, expected, scenario_state):
    resource_address = scenario_state.resource_address
    fileobject_version = scenario_state.version_under_test["fileobject"]

    def grpc_call():
        pb2 = scenario_state.sapi_test_client.fileobject_pb2(fileobject_version)
        return scenario_state.sapi_test_client.fileobject_grpc_client(fileobject_version).IsReadOnly(
            pb2.IsReadOnlyRequest(resource_address=resource_address)
        )

    def rest_call():
        return scenario_state.sapi_test_client.rest_client().is_read_only(resource_address, fileobject_version)

    response = structured_api_call(protocol, status_code, grpc_call=grpc_call, rest_call=rest_call)
    expected_value = expected.strip().lower() == "true"
    actual = response.is_read_only if protocol == "GRPC" else response.json()["is_read_only"]
    assert actual == expected_value, f"expected is_read_only={expected_value}, got {actual}"
