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

import json
import math
from copy import deepcopy
from enum import Enum
from typing import (
    Any,
    Iterable,
    Mapping,
    Optional,
)

import grpc
import requests
from google.protobuf import json_format
from google.protobuf.struct_pb2 import Value

NATIVE_METADATA_REST_HEADER = "x-nvidia-omniverse-storage-native-metadata"
METADATA_REST_HEADER = "x-nvidia-omniverse-storage-metadata"


class NativeMetadataSupportProbeResult(str, Enum):
    SUPPORTED = "supported"
    UNSUPPORTED_PRESENT = "unsupported_present"
    ABSENT_ENDPOINT = "absent_endpoint"
    ERROR = "error"


def canonical_native_metadata() -> dict[str, Any]:
    return deepcopy(
        {
            "string_value": "native metadata value",
            "number_value": 42.5,
            "boolean_value": True,
            "nested_value": {
                "object": {"key": "value"},
                "list": ["item", 7, False, None],
            },
            "null_value": None,
        }
    )


def alternate_native_metadata() -> dict[str, Any]:
    return deepcopy(
        {
            "string_value": "alternate native metadata value",
            "number_value": 7,
            "boolean_value": False,
            "nested_value": {
                "object": {"key": "alternate"},
                "list": ["second", 11, True, None],
            },
            "null_value": None,
        }
    )


def shared_key_native_metadata() -> dict[str, Any]:
    return deepcopy(
        {
            "shared_key": "fileobject native value",
            "native_only": "fileobject value",
        }
    )


def empty_native_metadata() -> dict[str, Any]:
    return {}


def malformed_rest_native_metadata_header() -> str:
    return "{"


def non_object_rest_native_metadata_header() -> str:
    return "[]"


def non_finite_rest_native_metadata_header() -> str:
    return '{"not_finite":NaN}'


def grpc_native_metadata_with_non_finite_number() -> dict[str, Value]:
    return {"not_finite": Value(number_value=float("nan"))}


def serialize_native_metadata_for_rest_header(native_metadata: Mapping[str, Any]) -> str:
    if not isinstance(native_metadata, Mapping):
        raise TypeError("native metadata must be a JSON object")
    return json.dumps(native_metadata, separators=(",", ":"), allow_nan=False)


def native_metadata_rest_headers(native_metadata: Optional[Mapping[str, Any]] = None, raw_header: Optional[str] = None) -> dict[str, str]:
    if native_metadata is not None and raw_header is not None:
        raise ValueError("native_metadata and raw_header are mutually exclusive")
    if raw_header is not None:
        return {NATIVE_METADATA_REST_HEADER: raw_header}
    if native_metadata is None:
        return {}
    return {NATIVE_METADATA_REST_HEADER: serialize_native_metadata_for_rest_header(native_metadata)}


def protobuf_value_from_python(value: Any) -> Value:
    return json_format.ParseDict(value, Value())


def native_metadata_to_protobuf_values(native_metadata: Mapping[str, Any]) -> dict[str, Value]:
    if not isinstance(native_metadata, Mapping):
        raise TypeError("native metadata must be a JSON object")
    return {str(key): protobuf_value_from_python(value) for key, value in native_metadata.items()}


def apply_native_metadata_to_protobuf_map(
    protobuf_map: Any,
    native_metadata: Optional[Mapping[str, Any]] = None,
    raw_values: Optional[Mapping[str, Value]] = None,
) -> None:
    if native_metadata is not None and raw_values is not None:
        raise ValueError("native_metadata and raw_values are mutually exclusive")

    values = raw_values if raw_values is not None else native_metadata_to_protobuf_values(native_metadata or {})
    for key, value in values.items():
        protobuf_map[str(key)].CopyFrom(value)


def apply_native_metadata_to_grpc_write_parameters(
    params: Any,
    native_metadata: Optional[Mapping[str, Any]] = None,
    raw_values: Optional[Mapping[str, Value]] = None,
) -> None:
    if native_metadata is None and raw_values is None:
        return
    apply_native_metadata_to_protobuf_map(params.native_metadata, native_metadata=native_metadata, raw_values=raw_values)


def protobuf_native_metadata_to_dict(native_metadata: Mapping[str, Value]) -> dict[str, Any]:
    return {key: json_format.MessageToDict(value) for key, value in native_metadata.items()}


def extract_native_metadata(response: Any, resource_address: Optional[str] = None) -> dict[str, Any]:
    if response is None:
        return {}

    if isinstance(response, requests.Response):
        return extract_native_metadata_from_rest_response(response, resource_address=resource_address)

    if hasattr(response, "headers") and hasattr(response, "content") and hasattr(response, "json"):
        return extract_native_metadata_from_rest_response(response, resource_address=resource_address)

    if isinstance(response, list):
        if all(isinstance(item, Mapping) for item in response):
            return extract_native_metadata_from_rest_items(response, resource_address=resource_address)
        return extract_native_metadata_from_grpc_items(response, resource_address=resource_address)

    if isinstance(response, Mapping):
        return extract_native_metadata_from_json(response, resource_address=resource_address)

    if hasattr(response, "value"):
        return extract_native_metadata(response.value, resource_address=resource_address)

    if _has_present_field(response, "resource_info"):
        return extract_native_metadata_from_metadata(response.resource_info.metadata)

    if _has_present_field(response, "metadata"):
        return extract_native_metadata_from_metadata(response.metadata)

    if hasattr(response, "items"):
        return extract_native_metadata_from_grpc_items(response.items, resource_address=resource_address)

    if hasattr(response, "resource_info"):
        return extract_native_metadata_from_metadata(response.resource_info.metadata)

    if hasattr(response, "metadata"):
        return extract_native_metadata_from_metadata(response.metadata)

    raise ValueError(f"cannot extract native metadata from {type(response)!r}")


def extract_native_metadata_from_rest_response(response: requests.Response, resource_address: Optional[str] = None) -> dict[str, Any]:
    if METADATA_REST_HEADER in response.headers:
        return extract_native_metadata_from_rest_metadata_header(response)

    if not response.content:
        return {}

    try:
        return extract_native_metadata_from_json(response.json(), resource_address=resource_address)
    except ValueError:
        return {}


def extract_native_metadata_from_rest_metadata_header(response: requests.Response) -> dict[str, Any]:
    return extract_native_metadata_from_metadata(json.loads(response.headers[METADATA_REST_HEADER]))


def extract_native_metadata_from_json(data: Any, resource_address: Optional[str] = None) -> dict[str, Any]:
    if isinstance(data, list):
        return extract_native_metadata_from_rest_items(data, resource_address=resource_address)

    if not isinstance(data, Mapping):
        raise ValueError(f"expected JSON object or item list, got {type(data)!r}")

    if "items" in data:
        return extract_native_metadata_from_rest_items(data["items"], resource_address=resource_address)

    if "entries" in data:
        return extract_native_metadata_from_rest_items(data["entries"], resource_address=resource_address)

    if "resource_info" in data:
        return extract_native_metadata_from_json(data["resource_info"], resource_address=resource_address)

    if "metadata" in data:
        return extract_native_metadata_from_metadata(data["metadata"])

    if "native_metadata" in data:
        return extract_native_metadata_from_metadata(data)

    return {}


def extract_native_metadata_from_rest_items(items: Iterable[Any], resource_address: Optional[str]) -> dict[str, Any]:
    for item in items:
        if not isinstance(item, Mapping):
            continue
        if resource_address is not None and not _resource_addresses_match(item.get("resource_address"), resource_address):
            continue
        return extract_native_metadata_from_json(item, resource_address=resource_address)
    raise LookupError(f"no REST enumerate item found for resource address {resource_address!r}")


def extract_native_metadata_from_grpc_items(items: Iterable[Any], resource_address: Optional[str]) -> dict[str, Any]:
    for item in items:
        item_address = getattr(item, "resource_address", None)
        if resource_address is not None and not _resource_addresses_match(item_address, resource_address):
            continue
        if _has_present_field(item, "resource_info") or hasattr(item, "resource_info"):
            return extract_native_metadata_from_metadata(item.resource_info.metadata)
        if _has_present_field(item, "metadata") or hasattr(item, "metadata"):
            return extract_native_metadata_from_metadata(item.metadata)
    raise LookupError(f"no gRPC enumerate item found for resource address {resource_address!r}")


def _resource_addresses_match(actual: Any, expected: str) -> bool:
    if not isinstance(actual, str):
        return False
    return actual.replace("\\", "/") == expected.replace("\\", "/")


def extract_native_metadata_from_metadata(metadata: Any) -> dict[str, Any]:
    if metadata is None:
        return {}

    if isinstance(metadata, Mapping):
        native_metadata = metadata.get("native_metadata", metadata.get("nativeMetadata", {}))
        if native_metadata is None:
            return {}
        if not isinstance(native_metadata, Mapping):
            raise ValueError("native_metadata must be a JSON object")
        return deepcopy(dict(native_metadata))

    if hasattr(metadata, "native_metadata"):
        return protobuf_native_metadata_to_dict(metadata.native_metadata)

    raise ValueError(f"cannot extract native metadata from metadata value {type(metadata)!r}")


def probe_fileobject_native_metadata_support(
    protocol: str,
    version: Optional[str],
    resource_address: Optional[str],
    scenario_state: Any,
) -> NativeMetadataSupportProbeResult:
    protocol = protocol.upper()
    if not version or not resource_address:
        return NativeMetadataSupportProbeResult.ERROR

    cache_key = (protocol, version, resource_address)
    cache = getattr(scenario_state, "_native_metadata_support_cache", None)
    if cache is None:
        cache = {}
        setattr(scenario_state, "_native_metadata_support_cache", cache)
    if cache_key in cache:
        return cache[cache_key]

    if protocol == "GRPC":
        result = _probe_fileobject_native_metadata_support_grpc(version, resource_address, scenario_state)
    elif protocol == "REST":
        result = _probe_fileobject_native_metadata_support_rest(version, resource_address, scenario_state)
    else:
        result = NativeMetadataSupportProbeResult.ERROR

    cache[cache_key] = result
    return result


def _probe_fileobject_native_metadata_support_grpc(
    version: str,
    resource_address: str,
    scenario_state: Any,
) -> NativeMetadataSupportProbeResult:
    try:
        pb2 = scenario_state.sapi_test_client.fileobject_pb2(version)
        response = scenario_state.sapi_test_client.fileobject_grpc_client(version).GetNativeMetadataSupport(
            pb2.GetNativeMetadataSupportRequest(resource_address=resource_address)
        )
    except grpc.RpcError as exc:
        if exc.code() == grpc.StatusCode.UNIMPLEMENTED:
            return NativeMetadataSupportProbeResult.ABSENT_ENDPOINT
        return NativeMetadataSupportProbeResult.ERROR
    except NotImplementedError:
        return NativeMetadataSupportProbeResult.ABSENT_ENDPOINT
    except Exception:
        return NativeMetadataSupportProbeResult.ERROR

    if getattr(response, "supports_native_metadata", False):
        return NativeMetadataSupportProbeResult.SUPPORTED
    return NativeMetadataSupportProbeResult.UNSUPPORTED_PRESENT


def _probe_fileobject_native_metadata_support_rest(
    version: str,
    resource_address: str,
    scenario_state: Any,
) -> NativeMetadataSupportProbeResult:
    try:
        response = scenario_state.sapi_test_client.rest_client().get_native_metadata_support(resource_address, version)
    except NotImplementedError:
        return NativeMetadataSupportProbeResult.ABSENT_ENDPOINT
    except Exception:
        return NativeMetadataSupportProbeResult.ERROR

    if response.status_code in (404, 405, 501):
        return NativeMetadataSupportProbeResult.ABSENT_ENDPOINT
    if response.status_code != 200:
        return NativeMetadataSupportProbeResult.ERROR

    try:
        supports_native_metadata = response.json()["supports_native_metadata"]
    except (ValueError, KeyError, TypeError):
        return NativeMetadataSupportProbeResult.ERROR

    return NativeMetadataSupportProbeResult.SUPPORTED if supports_native_metadata else NativeMetadataSupportProbeResult.UNSUPPORTED_PRESENT


def _has_present_field(message: Any, field_name: str) -> bool:
    try:
        return message.HasField(field_name)
    except (AttributeError, ValueError):
        return False


def assert_json_compatible_native_metadata(native_metadata: Mapping[str, Any]) -> None:
    for key, value in native_metadata.items():
        if not isinstance(key, str):
            raise AssertionError("native metadata keys must be strings")
        _assert_json_compatible_value(value)


def native_metadata_entry_matches(native_metadata: Mapping[str, Any], key: str, value: Any) -> bool:
    return key in native_metadata and native_metadata[key] == value


def _assert_json_compatible_value(value: Any) -> None:
    if value is None or isinstance(value, (str, bool)):
        return
    if isinstance(value, (int, float)):
        if not math.isfinite(value):
            raise AssertionError("native metadata numbers must be finite")
        return
    if isinstance(value, Mapping):
        assert_json_compatible_native_metadata(value)
        return
    if isinstance(value, list):
        for item in value:
            _assert_json_compatible_value(item)
        return
    raise AssertionError(f"unsupported native metadata value type: {type(value)!r}")
