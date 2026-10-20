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

from typing import Optional

from conformance_tests.steps.common_fixtures import identity_from_fixture
from conformance_tests.steps.utils.structured_api_call import structured_api_call
from pytest_bdd import (
    parsers,
    when,
)


@when(
    parsers.parse(
        "calling '{protocol}' copy from memorized '{source_identity}' to memorized '{destination_address}' returns '{return_value}'"
    )
)
@when(
    parsers.parse(
        "calling '{protocol}' copy with '{previous_version_name}' from memorized '{source_identity}' to memorized '{destination_address}' returns '{return_value}'"
    )
)
def copy_object_without_previous_version(
    protocol,
    source_identity,
    destination_address,
    return_value,
    scenario_state,
    previous_version_name=None,
):
    previous_version_param = (
        identity_from_fixture(scenario_state.memorized_responses[previous_version_name]) if previous_version_name else None
    )
    _do_copy(
        protocol,
        scenario_state,
        source_identity,
        destination_address,
        return_value,
        previous_version_param=previous_version_param,
        fail_if_exists=None,
    )


@when(
    parsers.parse(
        "calling '{protocol}' copy with fail_if_exists from memorized '{source_identity}' to memorized '{destination_address}' returns '{return_value}'"
    )
)
def copy_object_with_fail_if_exists(
    protocol,
    source_identity,
    destination_address,
    return_value,
    scenario_state,
):
    _do_copy(
        protocol,
        scenario_state,
        source_identity,
        destination_address,
        return_value,
        previous_version_param=None,
        fail_if_exists=True,
    )


@when(
    parsers.parse(
        "calling '{protocol}' copy with explicit fail_if_exists '{flag}' from memorized '{source_identity}' to memorized '{destination_address}' returns '{return_value}'"
    )
)
def copy_object_with_explicit_fail_if_exists(
    protocol,
    flag,
    source_identity,
    destination_address,
    return_value,
    scenario_state,
):
    if flag == "true":
        fail_if_exists: Optional[bool] = True
    elif flag == "false":
        fail_if_exists = False
    else:
        raise ValueError(f"expected 'true' or 'false', got {flag!r}")
    _do_copy(
        protocol,
        scenario_state,
        source_identity,
        destination_address,
        return_value,
        previous_version_param=None,
        fail_if_exists=fail_if_exists,
    )


@when(
    parsers.parse(
        "calling '{protocol}' copy with fail_if_exists and '{previous_version_name}' from memorized '{source_identity}' to memorized '{destination_address}' returns '{return_value}'"
    )
)
def copy_object_with_fail_if_exists_and_previous_version(
    protocol,
    source_identity,
    destination_address,
    return_value,
    scenario_state,
    previous_version_name,
):
    previous_version_param = identity_from_fixture(scenario_state.memorized_responses[previous_version_name])
    _do_copy(
        protocol,
        scenario_state,
        source_identity,
        destination_address,
        return_value,
        previous_version_param=previous_version_param,
        fail_if_exists=True,
    )


def _do_copy(
    protocol,
    scenario_state,
    source_identity,
    destination_address,
    return_value,
    previous_version_param,
    fail_if_exists: Optional[bool],
):
    source_identity = scenario_state.memorized_responses[source_identity]
    destination_address = scenario_state.memorized_responses[destination_address]

    def grpc_call():
        common_pb2 = scenario_state.sapi_test_client.fileobject_common_pb2(scenario_state.version_under_test["fileobject"])
        source_identity_grpc = common_pb2.ResourceIdentity(encoded_identity=source_identity)
        request_kwargs = {
            "source_resource_identity": source_identity_grpc,
            "destination_resource_address": destination_address,
        }
        if previous_version_param is not None:
            request_kwargs["previous_version"] = common_pb2.ResourceIdentity(encoded_identity=previous_version_param)
        if fail_if_exists is not None:
            # None means "do not include the field" (proto3 explicit-presence
            # unset); True/False are passed faithfully.
            request_kwargs["fail_if_exists"] = fail_if_exists
        copy_request = scenario_state.sapi_test_client.fileobject_pb2(scenario_state.version_under_test["fileobject"]).CopyRequest(
            **request_kwargs
        )
        return scenario_state.sapi_test_client.fileobject_grpc_client(scenario_state.version_under_test["fileobject"]).Copy(copy_request)

    def rest_call():
        return scenario_state.sapi_test_client.rest_client().copy_object(
            source_identity,
            destination_address,
            previous_version_param,
            scenario_state.version_under_test["fileobject"],
            fail_if_exists=fail_if_exists,
        )

    scenario_state.last_response = structured_api_call(protocol, return_value, grpc_call=grpc_call, rest_call=rest_call)
