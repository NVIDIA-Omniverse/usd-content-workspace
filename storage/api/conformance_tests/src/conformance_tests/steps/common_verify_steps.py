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

import grpc
from pytest_bdd import (
    parsers,
    then,
)

from .common_fixtures import (
    identity_from_fixture,
    size_from_fixture,
)
from .utils.grpc_helpers import map_grpc_codes


@then(parsers.parse("the resource identity returned is different from the one in the memorized response '{response_name}'"))
def response_identity_different_from_memorized(response_name, scenario_state):
    memorized_identity = identity_from_fixture(scenario_state.memorized_responses[response_name])
    last_identity = identity_from_fixture(scenario_state.last_response)
    assert memorized_identity != last_identity, "Wrong identity delivered"


@then(parsers.cfparse("the '{protocol}' result's resource info has size '{size:Number}'", extra_types={"Number": int}))
def the_result_resource_info_size(protocol, size, scenario_state):
    assert size_from_fixture(scenario_state.last_response) == size


@then(parsers.parse("the '{protocol}' call should return '{status_code}'"))
def then_call_should_return_status_code(protocol, status_code, scenario_state):
    if hasattr(scenario_state.last_response, "value"):
        response = scenario_state.last_response.value
    else:
        response = scenario_state.last_response

    if protocol == "GRPC":
        if status_code == "OK":
            assert not isinstance(response, grpc.RpcError), f"Expected OK but got gRPC error: {scenario_state.last_response}"
            assert response.HasField("resource_info"), "Expected resource_info in response"
        else:
            assert isinstance(response, grpc.RpcError), f"Expected gRPC error but got: {scenario_state.last_response}"
            assert response.code() == map_grpc_codes(status_code)
    elif protocol == "REST":
        assert response.status_code == int(status_code)
    else:
        raise ValueError(f"unknown protocol: {protocol}")
