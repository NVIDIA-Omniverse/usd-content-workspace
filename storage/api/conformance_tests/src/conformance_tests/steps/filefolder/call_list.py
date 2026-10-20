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

from typing import (
    Dict,
    Optional,
)

from pytest_bdd import (
    parsers,
    then,
    when,
)

from ..common_memory_steps import get_resource_address
from ..context_fixture import (
    ListResultFixture,
)
from ..utils.structured_api_call import structured_api_stream


@when(parsers.parse("calling '{protocol}' List exhaustively on that address returns '{status_code}'"))
@then(parsers.parse("calling '{protocol}' List exhaustively on that address returns '{status_code}'"))
@then(parsers.parse("calling '{protocol}' List on the memorized address '{memorized_name}' returns '{status_code}'"))
@then(parsers.parse("calling '{protocol}' List exhaustively with page size '{page_size:d}' on that address returns '{status_code}'"))
@when(parsers.parse("calling '{protocol}' List exhaustively with page size '{page_size:d}' on that address returns '{status_code}'"))
def step_list(protocol, status_code, scenario_state, memorized_name=None, page_size=None):
    scenario_state.last_list_response = _calling_list_returns_status_code(
        protocol=protocol,
        status_code=status_code,
        scenario_state=scenario_state,
        resource_address=get_resource_address(scenario_state, memorized_name),
        page_size=page_size,
    )


@when(parsers.parse("calling '{protocol}' ListStat exhaustively on that address returns '{status_code}'"))
@then(parsers.parse("calling '{protocol}' ListStat exhaustively on that address returns '{status_code}'"))
@then(parsers.parse("calling '{protocol}' ListStat on the memorized address '{memorized_name}' returns '{status_code}'"))
@when(parsers.parse("calling '{protocol}' ListStat on the memorized address '{memorized_name}' returns '{status_code}'"))
@then(parsers.parse("calling '{protocol}' ListStat exhaustively with page size '{page_size:d}' on that address returns '{status_code}'"))
@when(parsers.parse("calling '{protocol}' ListStat exhaustively with page size '{page_size:d}' on that address returns '{status_code}'"))
def step_liststat(protocol, status_code, scenario_state, memorized_name=None, page_size=None):
    scenario_state.last_list_stat_response = _calling_list_stat_returns_status_code(
        protocol=protocol,
        status_code=status_code,
        scenario_state=scenario_state,
        resource_address=get_resource_address(scenario_state, memorized_name),
        page_size=page_size,
    )


@then(
    parsers.parse("calling '{protocol}' ListStat with native metadata on the memorized address '{memorized_name}' returns '{status_code}'")
)
@when(
    parsers.parse("calling '{protocol}' ListStat with native metadata on the memorized address '{memorized_name}' returns '{status_code}'")
)
def step_liststat_with_native_metadata(protocol, status_code, scenario_state, memorized_name=None):
    scenario_state.last_list_stat_response = _calling_list_stat_returns_status_code(
        protocol=protocol,
        status_code=status_code,
        scenario_state=scenario_state,
        resource_address=get_resource_address(scenario_state, memorized_name),
        include_native_metadata=True,
    )


@when(parsers.parse("memorizing single file address from listed entries as '{response_name}'"))
def memorizing_resource_address_from_folder(response_name, scenario_state):
    response = scenario_state.last_list_response.value
    if isinstance(response, list):
        scenario_state.memorized_responses[response_name] = response[-1]
    elif isinstance(response, dict):
        scenario_state.memorized_responses[response_name] = response["sub_resource_addresses"][-1]
    else:
        raise TypeError(f"Unexpected response type: {type(response)}")


def _calling_list_returns_status_code(
    protocol,
    status_code,
    scenario_state,
    resource_address: Optional[str] = None,
    page_size: Optional[int] = None,
) -> ListResultFixture | None:
    def create_grpc_stream():
        return scenario_state.sapi_test_client.filefolder_grpc_client(scenario_state.version_under_test["filefolder"]).List(
            scenario_state.sapi_test_client.filefolder_pb2(scenario_state.version_under_test["filefolder"]).ListRequest(
                folder=scenario_state.sapi_test_client.filefolder_pb2(scenario_state.version_under_test["filefolder"]).FolderAddress(
                    uri=resource_address
                )
            )
        )

    def create_rest_call_with_params(params: Optional[Dict] = None):
        call_params = params or {}
        if page_size is not None:
            call_params["max_page_size"] = page_size

        return scenario_state.sapi_test_client.rest_client().list_data_objects(
            address=resource_address,
            filefolder_version=scenario_state.version_under_test["filefolder"],
            params=call_params if call_params else None,
        )

    def grpc_list_aggregator(response, result_fixture: ListResultFixture):
        if result_fixture.value is None:
            result_fixture.value = {"subfolder_addresses": [], "sub_resource_addresses": []}
        result_fixture.value["subfolder_addresses"].extend([folder.uri for folder in response.subfolder_addresses])
        result_fixture.value["sub_resource_addresses"].extend(response.sub_resource_addresses)
        result_fixture.counter = len(result_fixture.value["subfolder_addresses"]) + len(result_fixture.value["sub_resource_addresses"])
        return True  # Continue processing

    def rest_list_aggregator(response_data, result_fixture: ListResultFixture):
        """Aggregate REST list responses - handle complex structure"""
        # For REST, we want to maintain the structure, so we'll store it differently
        if not result_fixture.value:  # First call - initialize structure
            result_fixture.value = {"subfolder_addresses": [], "sub_resource_addresses": [], "next_continuation_handle": None}

        result_fixture.value["subfolder_addresses"].extend(response_data.get("subfolder_addresses", []))
        result_fixture.value["sub_resource_addresses"].extend(response_data.get("sub_resource_addresses", []))
        result_fixture.value["next_continuation_handle"] = response_data.get("next_continuation_handle")
        result_fixture.counter = len(result_fixture.value["subfolder_addresses"]) + len(result_fixture.value["sub_resource_addresses"])
        return True  # Continue processing

    return structured_api_stream(
        protocol=protocol,
        status_code=status_code,
        grpc_stream_call=create_grpc_stream,
        grpc_result_aggregator=grpc_list_aggregator,
        rest_call_factory=create_rest_call_with_params,
        rest_result_aggregator=rest_list_aggregator,
    )


def _calling_list_stat_returns_status_code(
    protocol,
    status_code,
    scenario_state,
    resource_address: str,
    page_size: Optional[int] = None,
    include_native_metadata: bool = False,
) -> ListResultFixture | None:
    def create_grpc_stream():
        pb2 = scenario_state.sapi_test_client.filefolder_pb2(scenario_state.version_under_test["filefolder"])
        request = pb2.ListStatRequest(folder=pb2.FolderAddress(uri=resource_address))
        if include_native_metadata and "include_native_metadata" in request.DESCRIPTOR.fields_by_name:
            request.include_native_metadata = True
        return scenario_state.sapi_test_client.filefolder_grpc_client(scenario_state.version_under_test["filefolder"]).ListStat(request)

    def create_rest_call_with_params(params: Optional[Dict] = None):
        call_params = params or {}
        if page_size is not None:
            call_params["max_page_size"] = page_size
        if include_native_metadata:
            call_params["include_native_metadata"] = "true"

        return scenario_state.sapi_test_client.rest_client().list_stat_data_objects(
            address=resource_address,
            filefolder_version=scenario_state.version_under_test["filefolder"],
            params=call_params if call_params else None,
        )

    def grpc_list_aggregator(response, result_fixture: ListResultFixture):
        if result_fixture.value is None:
            result_fixture.value = []
        result_fixture.value.extend([folder.uri for folder in response.subfolder_addresses])
        result_fixture.value.extend(response.entries)
        result_fixture.counter = len(result_fixture.value)
        return True  # Continue processing

    def rest_list_aggregator(response_data, result_fixture: ListResultFixture):
        """Aggregate REST list responses - handle complex structure"""
        # For REST, we want to maintain the structure, so we'll store it differently
        if not result_fixture.value:  # First call - initialize structure
            result_fixture.value = {"subfolder_addresses": [], "entries": [], "next_continuation_handle": None}

        result_fixture.value["subfolder_addresses"].extend(response_data.get("subfolder_addresses", []))
        result_fixture.value["entries"].extend(response_data.get("entries", []))
        result_fixture.value["next_continuation_handle"] = response_data.get("next_continuation_handle")
        result_fixture.counter = len(result_fixture.value["subfolder_addresses"]) + len(result_fixture.value["entries"])
        return True  # Continue processing

    return structured_api_stream(
        protocol=protocol,
        status_code=status_code,
        grpc_stream_call=create_grpc_stream,
        grpc_result_aggregator=grpc_list_aggregator,
        rest_call_factory=create_rest_call_with_params,
        rest_result_aggregator=rest_list_aggregator,
    )
