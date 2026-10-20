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

from typing import (
    Any,
    Dict,
    Mapping,
)

from conformance_tests.storage_testdata_generator import AbstractTestDataGenerator
from pytest_bdd import (
    given,
    parsers,
    then,
    when,
)

from ..common_fixtures import identity_from_fixture
from ..utils.structured_api_call import structured_api_call
from .call_write import _call_write
from .native_metadata_helpers import (
    alternate_native_metadata,
    assert_json_compatible_native_metadata,
    canonical_native_metadata,
    empty_native_metadata,
    extract_native_metadata,
    grpc_native_metadata_with_non_finite_number,
    malformed_rest_native_metadata_header,
    native_metadata_entry_matches,
    non_finite_rest_native_metadata_header,
    non_object_rest_native_metadata_header,
    shared_key_native_metadata,
)


@given("a version-like resource address for that address")
def version_like_resource_address_for_that_address(scenario_state):
    scenario_state.resource_address = f"{scenario_state.resource_address};1"


@then(
    parsers.parse(
        "calling '{protocol}' native metadata support on that address returns '{return_code}' and supports native metadata is '{supports}'"
    )
)
@when(
    parsers.parse(
        "calling '{protocol}' native metadata support on that address returns '{return_code}' and supports native metadata is '{supports}'"
    )
)
def native_metadata_support_returns_flag(protocol, return_code, supports, scenario_state, sapi_test_server_launch):
    response = _call_native_metadata_support(protocol, return_code, scenario_state)
    expected_support = _parse_bool(supports)
    if protocol == "GRPC":
        assert response.supports_native_metadata is expected_support
    elif protocol == "REST":
        response_json = response.json()
        assert response_json["supports_native_metadata"] is expected_support
    else:
        raise ValueError(f"unknown protocol: {protocol!r}")


@then(parsers.parse("calling '{protocol}' native metadata support on that address returns '{return_code}'"))
@when(parsers.parse("calling '{protocol}' native metadata support on that address returns '{return_code}'"))
def native_metadata_support_returns(protocol, return_code, scenario_state, sapi_test_server_launch):
    _call_native_metadata_support(protocol, return_code, scenario_state)


@then(parsers.parse("calling '{protocol}' write on that address with '{metadata_case}' native metadata returns '{return_code}'"))
@when(parsers.parse("calling '{protocol}' write on that address with '{metadata_case}' native metadata returns '{return_code}'"))
def write_with_native_metadata(protocol, metadata_case, return_code, scenario_state, sapi_test_server_launch, blob):
    kwargs = _native_metadata_write_kwargs(protocol, metadata_case)
    scenario_state.last_response = _call_write(
        protocol=protocol,
        return_value=return_code,
        resource_address=scenario_state.resource_address,
        scenario_state=scenario_state,
        blob=blob,
        continue_upload=True,
        upload_preference="body",
        **kwargs,
    )


@then(
    parsers.parse(
        "calling '{protocol}' write on that address with '{upload_preference}' upload preference and '{metadata_case}' native metadata the first roundtrip returns '{return_code}'"
    )
)
@when(
    parsers.parse(
        "calling '{protocol}' write on that address with '{upload_preference}' upload preference and '{metadata_case}' native metadata the first roundtrip returns '{return_code}'"
    )
)
def write_with_upload_preference_and_native_metadata_first_roundtrip(
    protocol,
    upload_preference,
    metadata_case,
    return_code,
    scenario_state,
    sapi_test_server_launch,
    blob,
):
    kwargs = _native_metadata_write_kwargs(protocol, metadata_case)
    scenario_state.last_response = _call_write(
        protocol=protocol,
        return_value=return_code,
        resource_address=scenario_state.resource_address,
        scenario_state=scenario_state,
        blob=blob,
        continue_upload=False,
        upload_preference=upload_preference,
        **kwargs,
    )


@given(
    parsers.parse(
        "a native metadata test object of size '{size:d}' with rand seed '{seed:d}' and '{metadata_case}' native metadata exists at that address using '{protocol}'"
    )
)
def native_metadata_test_object_with_seed(
    size,
    seed,
    metadata_case,
    protocol,
    scenario_state,
    sapi_test_server_launch,
):
    blob = AbstractTestDataGenerator.generate_random_bytes(size, seed)
    return_code = "OK" if protocol == "GRPC" else "201"
    kwargs = _native_metadata_write_kwargs(protocol, metadata_case)
    scenario_state.last_response = _call_write(
        protocol=protocol,
        return_value=return_code,
        resource_address=scenario_state.resource_address,
        scenario_state=scenario_state,
        blob=blob,
        continue_upload=True,
        upload_preference="body",
        **kwargs,
    )


@when(parsers.parse("calling '{protocol}' Read on the memorized response '{response_name}' returns '{return_code}'"))
def read_memorized_response(protocol, response_name, return_code, scenario_state, sapi_test_server_launch):
    from .call_read import _calling_read_returns_status_code

    resource_identity = _identity_from_response(scenario_state.memorized_responses[response_name])
    scenario_state.last_response = _calling_read_returns_status_code(
        protocol,
        resource_identity,
        scenario_state,
        return_code,
    )


@then(parsers.parse("the last response contains '{metadata_case}' native metadata"))
def last_response_contains_native_metadata(metadata_case, scenario_state):
    _assert_native_metadata_case(
        extract_native_metadata(scenario_state.last_response, resource_address=scenario_state.resource_address),
        metadata_case,
    )


@then("the last response contains empty or absent native metadata")
def last_response_contains_empty_or_absent_native_metadata(scenario_state):
    native_metadata = extract_native_metadata(scenario_state.last_response, resource_address=scenario_state.resource_address)
    assert native_metadata == {}


@then(parsers.parse("the last response does not contain '{metadata_case}' native metadata"))
def last_response_does_not_contain_native_metadata(metadata_case, scenario_state):
    _assert_native_metadata_case_absent(
        extract_native_metadata(scenario_state.last_response, resource_address=scenario_state.resource_address),
        metadata_case,
    )


@then(parsers.parse("the last enumerate result for that address contains '{metadata_case}' native metadata"))
def last_enumerate_result_contains_native_metadata(metadata_case, scenario_state):
    _assert_native_metadata_case(
        extract_native_metadata(
            scenario_state.last_enumerate_response,
            resource_address=scenario_state.resource_address,
        ),
        metadata_case,
    )


@then("the last enumerate result for that address contains empty or absent native metadata")
def last_enumerate_result_contains_empty_or_absent_native_metadata(scenario_state):
    native_metadata = extract_native_metadata(
        scenario_state.last_enumerate_response,
        resource_address=scenario_state.resource_address,
    )
    assert native_metadata == {}


@then(parsers.parse("the last enumerate result for that address does not contain '{metadata_case}' native metadata"))
def last_enumerate_result_does_not_contain_native_metadata(metadata_case, scenario_state):
    _assert_native_metadata_case_absent(
        extract_native_metadata(
            scenario_state.last_enumerate_response,
            resource_address=scenario_state.resource_address,
        ),
        metadata_case,
    )


@then(parsers.parse("the last ListStat result for that address contains '{metadata_case}' native metadata"))
def last_liststat_result_contains_native_metadata(metadata_case, scenario_state):
    _assert_native_metadata_case(
        extract_native_metadata(
            scenario_state.last_list_stat_response,
            resource_address=scenario_state.resource_address,
        ),
        metadata_case,
    )


@then("the last ListStat result for that address contains empty or absent native metadata")
def last_liststat_result_contains_empty_or_absent_native_metadata(scenario_state):
    native_metadata = extract_native_metadata(
        scenario_state.last_list_stat_response,
        resource_address=scenario_state.resource_address,
    )
    assert native_metadata == {}


@then(parsers.parse("the EnumerateVersions result for memorized response '{response_name}' contains '{metadata_case}' native metadata"))
def enumerate_versions_result_for_response_contains_native_metadata(response_name, metadata_case, scenario_state):
    expected_identity = _identity_from_response(scenario_state.memorized_responses[response_name])
    version_info = _enumerate_versions_item_for_identity(scenario_state.last_enumerate_versions_response.value, expected_identity)
    _assert_native_metadata_case(extract_native_metadata(version_info), metadata_case)


@then(parsers.parse("the resource identity returned is the same as the one in the memorized response '{response_name}'"))
def response_identity_same_as_memorized(response_name, scenario_state):
    memorized_identity = identity_from_fixture(scenario_state.memorized_responses[response_name])
    last_identity = identity_from_fixture(scenario_state.last_response)
    assert memorized_identity == last_identity


@then(parsers.parse("the last response contains native metadata key '{key}' with value '{value}'"))
def last_response_contains_native_metadata_key_value(key, value, scenario_state):
    native_metadata = extract_native_metadata(scenario_state.last_response, resource_address=scenario_state.resource_address)
    assert native_metadata.get(key) == value


def _call_native_metadata_support(protocol: str, return_code: str, scenario_state) -> Any:
    def grpc_call():
        version = scenario_state.version_under_test["fileobject"]
        return scenario_state.sapi_test_client.fileobject_grpc_client(version).GetNativeMetadataSupport(
            scenario_state.sapi_test_client.fileobject_pb2(version).GetNativeMetadataSupportRequest(
                resource_address=scenario_state.resource_address
            )
        )

    def rest_call():
        return scenario_state.sapi_test_client.rest_client().get_native_metadata_support(
            scenario_state.resource_address,
            scenario_state.version_under_test["fileobject"],
        )

    response = structured_api_call(protocol, return_code, grpc_call=grpc_call, rest_call=rest_call)
    scenario_state.last_response = response
    return response


def _identity_from_response(response: Any) -> Any:
    if hasattr(response, "json"):
        try:
            body = response.json()
        except ValueError:
            body = None
        if isinstance(body, Mapping):
            if "resource_identity" in body:
                return body["resource_identity"]
            if "resource_info" in body:
                return identity_from_fixture(body)
    return identity_from_fixture(response)


def _enumerate_versions_item_for_identity(items: Any, expected_identity: str) -> Any:
    if not isinstance(items, list):
        raise AssertionError("Expected EnumerateVersions result fixture to contain a list")
    for item in items:
        if identity_from_fixture(item) == expected_identity:
            return item
    raise AssertionError(f"EnumerateVersions did not return resource identity {expected_identity!r}")


def _parse_bool(value: str) -> bool:
    if value == "true":
        return True
    if value == "false":
        return False
    raise ValueError(f"expected 'true' or 'false', got {value!r}")


def _native_metadata_write_kwargs(protocol: str, metadata_case: str) -> Dict[str, Any]:
    metadata_case = metadata_case.lower().replace("_", "-")
    if metadata_case in {"canonical", "non-empty", "alternate", "shared-key", "empty"}:
        metadata = _native_metadata_case(metadata_case)
        return {"native_metadata": metadata}
    if metadata_case in {"omitted", "absent", "none"}:
        return {}

    if protocol == "REST":
        return _rest_native_metadata_write_kwargs(metadata_case)
    if protocol == "GRPC":
        return _grpc_native_metadata_write_kwargs(metadata_case)
    raise ValueError(f"unknown protocol: {protocol!r}")


def _rest_native_metadata_write_kwargs(metadata_case: str) -> Dict[str, Any]:
    rest_cases = {
        "malformed-json": malformed_rest_native_metadata_header,
        "non-object-json": non_object_rest_native_metadata_header,
        "non-finite": non_finite_rest_native_metadata_header,
    }
    if metadata_case not in rest_cases:
        raise ValueError(f"unknown REST native metadata case: {metadata_case!r}")
    return {"rest_native_metadata_header": rest_cases[metadata_case]()}


def _grpc_native_metadata_write_kwargs(metadata_case: str) -> Dict[str, Any]:
    grpc_cases = {
        "non-finite": grpc_native_metadata_with_non_finite_number,
    }
    if metadata_case not in grpc_cases:
        raise ValueError(f"unknown gRPC native metadata case: {metadata_case!r}")
    return {"grpc_native_metadata_values": grpc_cases[metadata_case]()}


def _assert_native_metadata_case(actual_native_metadata: Dict[str, Any], metadata_case: str) -> None:
    expected_native_metadata = _expected_native_metadata_case(metadata_case)
    assert_json_compatible_native_metadata(actual_native_metadata)
    for key, value in expected_native_metadata.items():
        assert native_metadata_entry_matches(actual_native_metadata, key, value)


def _assert_native_metadata_case_absent(actual_native_metadata: Dict[str, Any], metadata_case: str) -> None:
    expected_native_metadata = _expected_native_metadata_case(metadata_case)
    assert_json_compatible_native_metadata(actual_native_metadata)
    for key, value in expected_native_metadata.items():
        assert not native_metadata_entry_matches(actual_native_metadata, key, value)


def _expected_native_metadata_case(metadata_case: str) -> Dict[str, Any]:
    metadata = _native_metadata_case(metadata_case)
    if metadata is None:
        return {}
    return metadata


def _native_metadata_case(metadata_case: str) -> Dict[str, Any] | None:
    if metadata_case in {"canonical", "non-empty"}:
        return canonical_native_metadata()
    if metadata_case == "alternate":
        return alternate_native_metadata()
    if metadata_case in {"shared-key", "shared_key"}:
        return shared_key_native_metadata()
    if metadata_case == "empty":
        return empty_native_metadata()
    if metadata_case in {"omitted", "absent", "none"}:
        return None
    return None
