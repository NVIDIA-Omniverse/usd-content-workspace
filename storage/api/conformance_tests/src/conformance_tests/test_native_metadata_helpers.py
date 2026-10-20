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

from types import SimpleNamespace

import grpc
import pytest
from conformance_tests.steps.common_setup_steps import (
    service_does_not_support_native_metadata_for_fileobject,
    service_supports_native_metadata_for_fileobject,
)
from conformance_tests.steps.fileobject.native_metadata_helpers import (
    METADATA_REST_HEADER,
    NATIVE_METADATA_REST_HEADER,
    NativeMetadataSupportProbeResult,
    apply_native_metadata_to_grpc_write_parameters,
    canonical_native_metadata,
    empty_native_metadata,
    extract_native_metadata,
    grpc_native_metadata_with_non_finite_number,
    malformed_rest_native_metadata_header,
    native_metadata_entry_matches,
    native_metadata_rest_headers,
    non_object_rest_native_metadata_header,
    probe_fileobject_native_metadata_support,
)
from nvidia.omniverse.storage.fileobject.v1alpha import (
    fileobject_pb2,
    fileobject_service_pb2,
)


class FakeRpcError(grpc.RpcError):
    def __init__(self, code):
        super().__init__()
        self._code = code

    def code(self):
        return self._code


class FakeNativeMetadataSupportRequest:
    def __init__(self, resource_address):
        self.resource_address = resource_address


class FakeFileObjectPb2:
    GetNativeMetadataSupportRequest = FakeNativeMetadataSupportRequest


class FakeNativeMetadataSupportResponse:
    def __init__(self, supports_native_metadata):
        self.supports_native_metadata = supports_native_metadata


class FakeGrpcFileObjectClient:
    def __init__(self, responses):
        self.responses = responses
        self.requests = []

    def GetNativeMetadataSupport(self, request):
        self.requests.append(request.resource_address)
        response = self.responses[request.resource_address]
        if isinstance(response, Exception):
            raise response
        return FakeNativeMetadataSupportResponse(response)


class FakeRestResponse:
    def __init__(self, status_code=200, json_data=None, headers=None, content=b"{}"):
        self.status_code = status_code
        self._json_data = json_data if json_data is not None else {}
        self.headers = headers or {}
        self.content = content

    def json(self):
        return self._json_data


class FakeRestClient:
    def __init__(self, responses):
        self.responses = responses
        self.requests = []

    def get_native_metadata_support(self, address, fileobject_version):
        self.requests.append((fileobject_version, address))
        return self.responses[address]


class FakeSapiTestClient:
    def __init__(self, grpc_responses=None, rest_responses=None):
        self.grpc_client = FakeGrpcFileObjectClient(grpc_responses or {})
        self.rest = FakeRestClient(rest_responses or {})

    def fileobject_pb2(self, version):
        return FakeFileObjectPb2

    def fileobject_grpc_client(self, version):
        return self.grpc_client

    def rest_client(self):
        return self.rest


def _scenario_state(sapi_test_client, resource_address="storage://route-a/object.txt"):
    return SimpleNamespace(
        sapi_test_client=sapi_test_client,
        version_under_test={"fileobject": "v1alpha"},
        resource_address=resource_address,
    )


@pytest.mark.parametrize(
    ("supports", "expected"),
    [
        (True, NativeMetadataSupportProbeResult.SUPPORTED),
        (False, NativeMetadataSupportProbeResult.UNSUPPORTED_PRESENT),
    ],
)
def test_grpc_support_probe_classifies_supported_and_unsupported(supports, expected):
    address = "storage://route-a/prospective.txt"
    state = _scenario_state(FakeSapiTestClient(grpc_responses={address: supports}), address)

    result = probe_fileobject_native_metadata_support("GRPC", "v1alpha", address, state)

    assert result == expected
    assert state.sapi_test_client.grpc_client.requests == [address]


def test_support_probe_cache_is_keyed_by_protocol_version_and_resource_address():
    address_a = "storage://route-a/prospective.txt"
    address_b = "storage://route-b/prospective.txt"
    state = _scenario_state(FakeSapiTestClient(grpc_responses={address_a: True, address_b: False}), address_a)

    assert probe_fileobject_native_metadata_support("GRPC", "v1alpha", address_a, state) == NativeMetadataSupportProbeResult.SUPPORTED
    assert (
        probe_fileobject_native_metadata_support("GRPC", "v1alpha", address_b, state)
        == NativeMetadataSupportProbeResult.UNSUPPORTED_PRESENT
    )
    assert probe_fileobject_native_metadata_support("GRPC", "v1alpha", address_a, state) == NativeMetadataSupportProbeResult.SUPPORTED

    assert state.sapi_test_client.grpc_client.requests == [address_a, address_b]


def test_support_probe_follows_route_address_for_two_top_level_addresses():
    address_a = "omniverse://metadata-enabled/root/object.txt"
    address_b = "omniverse://metadata-disabled/root/object.txt"
    state = _scenario_state(FakeSapiTestClient(grpc_responses={address_a: True, address_b: False}), address_a)

    assert probe_fileobject_native_metadata_support("GRPC", "v1alpha", address_a, state) == NativeMetadataSupportProbeResult.SUPPORTED
    assert (
        probe_fileobject_native_metadata_support("GRPC", "v1alpha", address_b, state)
        == NativeMetadataSupportProbeResult.UNSUPPORTED_PRESENT
    )


def test_grpc_unimplemented_is_absent_endpoint():
    address = "storage://route-a/prospective.txt"
    state = _scenario_state(FakeSapiTestClient(grpc_responses={address: FakeRpcError(grpc.StatusCode.UNIMPLEMENTED)}), address)

    assert probe_fileobject_native_metadata_support("GRPC", "v1alpha", address, state) == NativeMetadataSupportProbeResult.ABSENT_ENDPOINT


@pytest.mark.parametrize("code", [grpc.StatusCode.INVALID_ARGUMENT, grpc.StatusCode.PERMISSION_DENIED])
def test_grpc_malformed_address_and_auth_failures_are_errors(code):
    address = "bad-address"
    state = _scenario_state(FakeSapiTestClient(grpc_responses={address: FakeRpcError(code)}), address)

    assert probe_fileobject_native_metadata_support("GRPC", "v1alpha", address, state) == NativeMetadataSupportProbeResult.ERROR


@pytest.mark.parametrize(
    ("supports", "expected"),
    [
        (True, NativeMetadataSupportProbeResult.SUPPORTED),
        (False, NativeMetadataSupportProbeResult.UNSUPPORTED_PRESENT),
    ],
)
def test_rest_support_probe_classifies_supported_and_unsupported(supports, expected):
    address = "storage://route-a/prospective.txt"
    response = FakeRestResponse(json_data={"supports_native_metadata": supports})
    state = _scenario_state(FakeSapiTestClient(rest_responses={address: response}), address)

    result = probe_fileobject_native_metadata_support("REST", "v1alpha", address, state)

    assert result == expected
    assert state.sapi_test_client.rest.requests == [("v1alpha", address)]


@pytest.mark.parametrize("status_code", [404, 405, 501])
def test_rest_absent_endpoint_statuses_are_absent_endpoint(status_code):
    address = "storage://route-a/prospective.txt"
    state = _scenario_state(FakeSapiTestClient(rest_responses={address: FakeRestResponse(status_code=status_code)}), address)

    assert probe_fileobject_native_metadata_support("REST", "v1alpha", address, state) == NativeMetadataSupportProbeResult.ABSENT_ENDPOINT


@pytest.mark.parametrize("status_code", [400, 401, 403])
def test_rest_malformed_address_and_auth_failures_are_errors(status_code):
    address = "bad-address"
    state = _scenario_state(FakeSapiTestClient(rest_responses={address: FakeRestResponse(status_code=status_code)}), address)

    assert probe_fileobject_native_metadata_support("REST", "v1alpha", address, state) == NativeMetadataSupportProbeResult.ERROR


@pytest.mark.parametrize(
    ("supports", "guard", "raises_skip"),
    [
        (True, service_supports_native_metadata_for_fileobject, False),
        (False, service_does_not_support_native_metadata_for_fileobject, False),
        (True, service_does_not_support_native_metadata_for_fileobject, True),
        (False, service_supports_native_metadata_for_fileobject, True),
    ],
)
def test_native_metadata_guard_passes_only_matching_explicit_capability(supports, guard, raises_skip):
    address = "storage://route-a/prospective.txt"
    state = _scenario_state(FakeSapiTestClient(grpc_responses={address: supports}), address)

    if raises_skip:
        with pytest.raises(pytest.skip.Exception):
            guard("GRPC", state, None)
    else:
        assert guard("GRPC", state, None) is True


@pytest.mark.parametrize(
    "guard",
    [service_supports_native_metadata_for_fileobject, service_does_not_support_native_metadata_for_fileobject],
)
def test_native_metadata_guard_skips_by_default_when_endpoint_is_absent(guard):
    address = "storage://route-a/prospective.txt"
    state = _scenario_state(FakeSapiTestClient(grpc_responses={address: FakeRpcError(grpc.StatusCode.UNIMPLEMENTED)}), address)

    with pytest.raises(pytest.skip.Exception):
        guard("GRPC", state, None)


def test_native_metadata_guard_fails_probe_errors():
    address = "bad-address"
    state = _scenario_state(FakeSapiTestClient(grpc_responses={address: FakeRpcError(grpc.StatusCode.INVALID_ARGUMENT)}), address)

    with pytest.raises(pytest.fail.Exception):
        service_supports_native_metadata_for_fileobject("GRPC", state, None)


def test_native_metadata_rest_headers_omit_absent_and_preserve_explicit_empty():
    assert native_metadata_rest_headers() == {}
    assert native_metadata_rest_headers(empty_native_metadata()) == {NATIVE_METADATA_REST_HEADER: "{}"}
    assert native_metadata_rest_headers(raw_header=malformed_rest_native_metadata_header()) == {NATIVE_METADATA_REST_HEADER: "{"}
    assert native_metadata_rest_headers(raw_header=non_object_rest_native_metadata_header()) == {NATIVE_METADATA_REST_HEADER: "[]"}


def test_apply_native_metadata_to_grpc_write_parameters_preserves_absent_and_explicit_empty():
    params_absent = fileobject_service_pb2.WriteParameters(destination_resource_address="storage://route-a/object.txt")
    apply_native_metadata_to_grpc_write_parameters(params_absent)
    assert dict(params_absent.native_metadata) == {}

    params_empty = fileobject_service_pb2.WriteParameters(destination_resource_address="storage://route-a/object.txt")
    apply_native_metadata_to_grpc_write_parameters(params_empty, native_metadata=empty_native_metadata())
    assert dict(params_empty.native_metadata) == {}

    params = fileobject_service_pb2.WriteParameters(destination_resource_address="storage://route-a/object.txt")
    apply_native_metadata_to_grpc_write_parameters(params, native_metadata=canonical_native_metadata())
    assert "string_value" in params.native_metadata
    assert params.native_metadata["boolean_value"].bool_value is True

    params_invalid = fileobject_service_pb2.WriteParameters(destination_resource_address="storage://route-a/object.txt")
    apply_native_metadata_to_grpc_write_parameters(params_invalid, raw_values=grpc_native_metadata_with_non_finite_number())
    assert "not_finite" in params_invalid.native_metadata


def test_invalid_input_helper_values_cover_future_scenarios():
    assert malformed_rest_native_metadata_header() == "{"
    assert non_object_rest_native_metadata_header() == "[]"


def test_native_metadata_entry_matches_distinguishes_missing_from_null():
    assert native_metadata_entry_matches({"null_value": None}, "null_value", None)
    assert not native_metadata_entry_matches({}, "null_value", None)
    assert not native_metadata_entry_matches({"null_value": "not-null"}, "null_value", None)


def _resource_info_with_native_metadata(native_metadata):
    metadata = fileobject_pb2.Metadata(data_object_size=12)
    apply_native_metadata_to_grpc_write_parameters(
        SimpleNamespace(native_metadata=metadata.native_metadata),
        native_metadata=native_metadata,
    )
    return fileobject_pb2.ResourceInfo(
        resource_identity=fileobject_pb2.ResourceIdentity(encoded_identity="identity"),
        metadata=metadata,
    )


@pytest.mark.parametrize(
    "response",
    [
        fileobject_service_pb2.WriteResponse(resource_info=_resource_info_with_native_metadata(canonical_native_metadata())),
        fileobject_service_pb2.CompleteRedirectUploadResponse(
            resource_info=_resource_info_with_native_metadata(canonical_native_metadata())
        ),
        fileobject_service_pb2.CompleteMultipartUploadResponse(
            resource_info=_resource_info_with_native_metadata(canonical_native_metadata())
        ),
        fileobject_service_pb2.StatResponse(resource_info=_resource_info_with_native_metadata(canonical_native_metadata())),
        fileobject_service_pb2.ReadResponse(metadata=_resource_info_with_native_metadata(canonical_native_metadata()).metadata),
        fileobject_service_pb2.ReadFromAddressResponse(resource_info=_resource_info_with_native_metadata(canonical_native_metadata())),
        fileobject_service_pb2.EnumerateResponse(
            items=[
                fileobject_pb2.AddressInfo(
                    resource_address="storage://route-a/object.txt",
                    metadata=_resource_info_with_native_metadata(canonical_native_metadata()).metadata,
                )
            ]
        ),
    ],
)
def test_extract_native_metadata_from_grpc_response_shapes(response):
    assert extract_native_metadata(response, resource_address="storage://route-a/object.txt") == canonical_native_metadata()


def test_extract_native_metadata_from_grpc_list_fixture_shape():
    response = SimpleNamespace(
        value=[
            fileobject_pb2.AddressInfo(
                resource_address="storage://route-a/object.txt",
                metadata=_resource_info_with_native_metadata(canonical_native_metadata()).metadata,
            )
        ]
    )

    assert extract_native_metadata(response, resource_address="storage://route-a/object.txt") == canonical_native_metadata()


def test_extract_native_metadata_matches_windows_style_enumerate_addresses():
    metadata = canonical_native_metadata()
    expected_address = "file-storage://fileservice/root/native_metadata_test/object.bin"

    rest_response = {
        "items": [
            {
                "resource_address": "file-storage://fileservice/root\\native_metadata_test\\object.bin",
                "metadata": {"native_metadata": metadata},
            }
        ]
    }
    grpc_response = SimpleNamespace(
        value=[
            fileobject_pb2.AddressInfo(
                resource_address="file-storage://fileservice/root\\native_metadata_test\\object.bin",
                metadata=_resource_info_with_native_metadata(metadata).metadata,
            )
        ]
    )

    assert extract_native_metadata(rest_response, resource_address=expected_address) == metadata
    assert extract_native_metadata(grpc_response, resource_address=expected_address) == metadata


def test_extract_native_metadata_from_rest_resource_info_and_enumerate_shapes():
    metadata = canonical_native_metadata()
    resource_info_json = {
        "resource_identity": "identity",
        "metadata": {
            "data_object_size": 12,
            "native_metadata": metadata,
        },
    }

    assert extract_native_metadata(resource_info_json) == metadata
    assert extract_native_metadata({"resource_info": resource_info_json}) == metadata
    assert (
        extract_native_metadata(
            {
                "items": [
                    {"resource_address": "storage://route-a/other.txt", "metadata": {"native_metadata": {}}},
                    {"resource_address": "storage://route-a/object.txt", "metadata": {"native_metadata": metadata}},
                ]
            },
            resource_address="storage://route-a/object.txt",
        )
        == metadata
    )


def test_extract_native_metadata_from_rest_metadata_header():
    response = FakeRestResponse(
        headers={
            METADATA_REST_HEADER: '{"data_object_size":12,"native_metadata":{"string_value":"native metadata value"}}',
        },
        content=b"",
    )

    assert extract_native_metadata(response) == {"string_value": "native metadata value"}
