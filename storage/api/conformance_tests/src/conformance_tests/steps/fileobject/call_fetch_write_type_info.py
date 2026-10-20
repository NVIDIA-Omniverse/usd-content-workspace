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


@when(parsers.parse("calling '{protocol}' FetchWriteTypeInfo on that address returns '{return_value}'"))
@then(parsers.parse("calling '{protocol}' FetchWriteTypeInfo on that address returns '{return_value}'"))
def calling_fetch_write_type_info_returns(
    protocol,
    return_value,
    scenario_state,
    sapi_test_server_launch,
):
    def grpc_call():
        return scenario_state.sapi_test_client.fileobject_grpc_client(scenario_state.version_under_test["fileobject"]).FetchWriteTypeInfo(
            scenario_state.sapi_test_client.fileobject_pb2(scenario_state.version_under_test["fileobject"]).FetchWriteTypeInfoRequest(
                destination_resource_address=scenario_state.resource_address
            )
        )

    def rest_call():
        return scenario_state.sapi_test_client.rest_client().get_upload_options(
            scenario_state.resource_address, scenario_state.version_under_test["fileobject"]
        )

    scenario_state.last_response = structured_api_call(protocol, return_value, grpc_call=grpc_call, rest_call=rest_call)
