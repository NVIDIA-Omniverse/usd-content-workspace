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
    when,
)

from ..utils.structured_api_call import structured_api_call


@when(parsers.parse("calling '{protocol}' ListTopLevelAddresses returns '{status_code}'"))
def calling_top_level_addresses_returns_status_code(protocol, status_code, scenario_state):
    def grpc_call():
        return scenario_state.sapi_test_client.capabilities_grpc_client(
            scenario_state.version_under_test["capabilities"]
        ).ListTopLevelAddresses(
            scenario_state.sapi_test_client.capabilities_pb2(
                scenario_state.version_under_test["capabilities"]
            ).ListTopLevelAddressesRequest()
        )

    def rest_call():
        return scenario_state.sapi_test_client.rest_client().list_top_level_addresses(
            capabilities_version=scenario_state.version_under_test["capabilities"]
        )

    scenario_state.last_response = structured_api_call(protocol, status_code, grpc_call=grpc_call, rest_call=rest_call)


def _first_top_level_address(protocol, response):
    if protocol == "GRPC":
        assert response.items, "ListTopLevelAddresses did not return any items"
        return response.items[0].top_level_address

    items = response.json()["items"]
    assert items, "ListTopLevelAddresses did not return any items"
    return items[0]["top_level_address"]
