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
    Any,
    Dict,
    Generator,
    Optional,
)

import pytest
from pytest_bdd import (
    parsers,
    then,
)

from ..common_fixtures import identity_from_fixture
from ..context_fixture import ContextFixture
from ..utils.structured_api_call import structured_api_call


@pytest.fixture
def last_move_response() -> Generator[ContextFixture, None, None]:
    """Fixture to store the last move response."""
    yield ContextFixture("last_move_response")


@then(
    parsers.parse(
        "calling '{protocol}' move from address '{source_address_name}' to address '{destination_address_name}' returns '{return_code}'"
    )
)
@then(
    parsers.parse(
        "calling '{protocol}' move with destination identity from address '{source_address_name}' "
        "to address '{destination_address_name}' "
        "with identity '{destination_identity_name}' "
        "returns '{return_code}'"
    )
)
@then(
    parsers.parse(
        "calling '{protocol}' move with source identity from address "
        "'{source_address_name}' and identity "
        "'{source_identity_name}' to address '{destination_address_name}' "
        "returns '{return_code}'"
    )
)
def calling_move_memorized_addresses(
    protocol,
    return_code,
    scenario_state,
    sapi_test_server_launch,
    last_move_response,
    source_address_name,
    source_identity_name=None,
    destination_address_name=None,
    destination_identity_name=None,
):
    source_address = scenario_state.memorized_responses.get(source_address_name)
    assert source_address, f"No previous response memorized as '{source_address_name}'"
    destination_address = scenario_state.memorized_responses[destination_address_name] if destination_address_name else None
    source_resource_identity = (
        identity_from_fixture(scenario_state.memorized_responses[source_identity_name]) if source_identity_name else None
    )
    destination_previous_version = (
        identity_from_fixture(scenario_state.memorized_responses[destination_identity_name]) if destination_identity_name else None
    )

    last_move_response.value = _perform_move_operation(
        protocol=protocol,
        return_code=return_code,
        source_address=source_address,
        destination_address=destination_address,
        source_identity=source_resource_identity,
        destination_previous_version=destination_previous_version,
        scenario_state=scenario_state,
    )


@then(
    parsers.parse(
        "calling '{protocol}' move with fail_if_exists from address '{source_address_name}' "
        "to address '{destination_address_name}' returns '{return_code}'"
    )
)
def calling_move_with_fail_if_exists(
    protocol,
    return_code,
    scenario_state,
    sapi_test_server_launch,
    last_move_response,
    source_address_name,
    destination_address_name,
):
    source_address = scenario_state.memorized_responses[source_address_name]
    destination_address = scenario_state.memorized_responses[destination_address_name]
    last_move_response.value = _perform_move_operation(
        protocol=protocol,
        return_code=return_code,
        source_address=source_address,
        destination_address=destination_address,
        source_identity=None,
        destination_previous_version=None,
        fail_if_exists=True,
        scenario_state=scenario_state,
    )


@then(
    parsers.parse(
        "calling '{protocol}' move with explicit fail_if_exists '{flag}' from address "
        "'{source_address_name}' to address '{destination_address_name}' returns '{return_code}'"
    )
)
def calling_move_with_explicit_fail_if_exists(
    protocol,
    flag,
    return_code,
    scenario_state,
    sapi_test_server_launch,
    last_move_response,
    source_address_name,
    destination_address_name,
):
    source_address = scenario_state.memorized_responses[source_address_name]
    destination_address = scenario_state.memorized_responses[destination_address_name]
    if flag == "true":
        fail_if_exists: Optional[bool] = True
    elif flag == "false":
        fail_if_exists = False
    else:
        raise ValueError(f"expected 'true' or 'false', got {flag!r}")
    last_move_response.value = _perform_move_operation(
        protocol=protocol,
        return_code=return_code,
        source_address=source_address,
        destination_address=destination_address,
        source_identity=None,
        destination_previous_version=None,
        fail_if_exists=fail_if_exists,
        scenario_state=scenario_state,
    )


@then(
    parsers.parse(
        "calling '{protocol}' move with fail_if_exists and destination identity from address "
        "'{source_address_name}' to address '{destination_address_name}' "
        "with identity '{destination_identity_name}' returns '{return_code}'"
    )
)
def calling_move_with_fail_if_exists_and_destination_identity(
    protocol,
    return_code,
    scenario_state,
    sapi_test_server_launch,
    last_move_response,
    source_address_name,
    destination_address_name,
    destination_identity_name,
):
    source_address = scenario_state.memorized_responses[source_address_name]
    destination_address = scenario_state.memorized_responses[destination_address_name]
    destination_previous_version = identity_from_fixture(scenario_state.memorized_responses[destination_identity_name])
    last_move_response.value = _perform_move_operation(
        protocol=protocol,
        return_code=return_code,
        source_address=source_address,
        destination_address=destination_address,
        source_identity=None,
        destination_previous_version=destination_previous_version,
        fail_if_exists=True,
        scenario_state=scenario_state,
    )


def _perform_move_operation(
    protocol: str,
    return_code: str,
    destination_address,
    scenario_state,
    source_address: str,
    source_identity: Optional[str] = None,
    destination_previous_version: Optional[str] = None,
    fail_if_exists: Optional[bool] = None,
):
    """Unified helper function to perform move operations."""

    if not isinstance(destination_address, str):
        raise TypeError(f"destination_address must be string, got {type(destination_address)}: {destination_address}")

    if not isinstance(source_address, str):
        raise TypeError(f"source_address must be string, got {type(source_address)}: {source_address}")

    def grpc_call():
        request_params = {
            "source_resource_address": source_address,
            "destination_resource_address": destination_address,
        }

        if source_identity is not None:
            request_params["source_previous_version"] = scenario_state.sapi_test_client.fileobject_common_pb2(
                scenario_state.version_under_test["fileobject"]
            ).ResourceIdentity(encoded_identity=source_identity)

        if destination_previous_version is not None:
            request_params["destination_previous_version"] = scenario_state.sapi_test_client.fileobject_common_pb2(
                scenario_state.version_under_test["fileobject"]
            ).ResourceIdentity(encoded_identity=destination_previous_version)

        if fail_if_exists is not None:
            # None means "do not include the field" (proto3 explicit-presence
            # unset); True/False are passed faithfully.
            request_params["fail_if_exists"] = fail_if_exists

        move_client = scenario_state.sapi_test_client.fileobject_grpc_client(scenario_state.version_under_test["fileobject"])
        move_request = scenario_state.sapi_test_client.fileobject_pb2(scenario_state.version_under_test["fileobject"]).MoveRequest(
            **request_params
        )
        return move_client.Move(move_request)

    def rest_call():
        json_payload: Dict[str, Any] = {"destination_resource_address": destination_address}

        if source_identity is not None:
            json_payload["source_previous_version"] = source_identity
        if destination_previous_version is not None:
            json_payload["destination_previous_version"] = destination_previous_version
        if fail_if_exists is not None:
            json_payload["fail_if_exists"] = fail_if_exists

        return scenario_state.sapi_test_client.rest_client().move_data_object(
            source_address,
            scenario_state.version_under_test["fileobject"],
            json=json_payload,
        )

    return structured_api_call(protocol, return_code, grpc_call=grpc_call, rest_call=rest_call)
