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

import json
from typing import Optional

from pytest_bdd import (
    parsers,
    when,
)

from ..utils.structured_api_call import structured_api_call


# Helper function to normalize keys parameter
def normalize_keys(keys_input):
    # Convert single string to list
    return [keys_input] if isinstance(keys_input, str) else keys_input


@when(parsers.parse("calling '{protocol}' get metadata with keys '{keys}' on that address returns '{return_value}'"))
def calling_get_metadata_returns_status_code(
    protocol,
    keys,
    return_value,
    scenario_state,
    sapi_test_server_launch,
):
    scenario_state.last_metadata_response = _call_get_metadata(protocol, return_value, scenario_state, keys, namespace=None)


def _call_get_metadata(protocol, return_value, scenario_state, keys, namespace: Optional[str]):
    keys_list = json.loads(keys)

    def grpc_call():
        request = scenario_state.sapi_test_client.metadata_pb2(scenario_state.version_under_test["metadata"]).GetMetadataRequest(
            uri=scenario_state.resource_address,
            user_metadata_keys=normalize_keys(keys_list),
        )
        if namespace is not None:
            request.namespace = namespace
        return scenario_state.sapi_test_client.metadata_grpc_client(scenario_state.version_under_test["metadata"]).GetMetadata(request)

    def rest_call():
        kwargs = {}
        if namespace is not None:
            kwargs["headers"] = {"Namespace": namespace}
        return scenario_state.sapi_test_client.rest_client().get_metadata(
            scenario_state.resource_address,
            metadata_version=scenario_state.version_under_test["metadata"],
            metadata_keys=normalize_keys(keys_list),
            **kwargs,
        )

    return structured_api_call(protocol, return_value, grpc_call=grpc_call, rest_call=rest_call)


@when(parsers.parse("we memorize the last metadata response as '{response_name}'"))
def memorized_metadata_response(response_name, scenario_state):
    scenario_state.memorized_responses[response_name] = scenario_state.last_metadata_response


@when(parsers.parse("calling '{protocol}' get metadata with keys '{keys}' on the '{alias}' address returns '{return_value}'"))
def calling_get_metadata_on_alias_returns_status_code(
    protocol,
    keys,
    alias,
    return_value,
    scenario_state,
    sapi_test_server_launch,
):
    """Sibling of the on-that-address step for multi-URI scenarios. Resolves
    ``alias`` against ``scenario_state.named_addresses`` (populated by the
    ``a resource address for 'X' as 'ALIAS'`` setup step).
    """
    keys_list = json.loads(keys)
    named = getattr(scenario_state, "named_addresses", None) or {}
    if alias not in named:
        raise ValueError(f"unknown address alias {alias!r}; known: {sorted(named)}")
    uri = named[alias]

    def grpc_call():
        request = scenario_state.sapi_test_client.metadata_pb2(scenario_state.version_under_test["metadata"]).GetMetadataRequest(
            uri=uri,
            user_metadata_keys=normalize_keys(keys_list),
        )
        return scenario_state.sapi_test_client.metadata_grpc_client(scenario_state.version_under_test["metadata"]).GetMetadata(request)

    def rest_call():
        return scenario_state.sapi_test_client.rest_client().get_metadata(
            uri,
            metadata_version=scenario_state.version_under_test["metadata"],
            metadata_keys=normalize_keys(keys_list),
        )

    scenario_state.last_metadata_response = structured_api_call(protocol, return_value, grpc_call=grpc_call, rest_call=rest_call)
