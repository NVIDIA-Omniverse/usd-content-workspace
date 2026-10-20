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

"""Call steps for the batch and query metadata endpoints.

Batch requests are assembled one entry at a time: each ``... request entry ...``
step appends a protocol-agnostic entry dict (carrying its own target URI) to the
matching list on ``scenario_state``. The corresponding
``calling ... batch ...`` step then translates those entries into the gRPC or
REST request, executes it once, stores the raw response (a list of streamed gRPC
pages or a REST ``requests.Response``) on ``scenario_state.last_metadata_response``
and clears the entry list so the next batch starts fresh.

The update-batch execution step is registered as both ``given`` and ``when`` so a
scenario can use it to establish state in its ``Given`` phase and still keep the
single operation under test in its lone ``When``.
"""

from typing import Optional

import pytest
from google.protobuf.struct_pb2 import Value
from pytest_bdd import (
    given,
    parsers,
    then,
    when,
)

from ..utils.structured_api_call import structured_api_call


def _that_address(scenario_state) -> str:
    return scenario_state.resource_address


def _alias_address(scenario_state, alias: str) -> str:
    named = getattr(scenario_state, "named_addresses", None) or {}
    if alias not in named:
        raise ValueError(f"unknown address alias {alias!r}; known: {sorted(named)}")
    return named[alias]


def _resolve_etag(scenario_state, entry: dict) -> Optional[str]:
    """Resolve an entry's expected ETag, dereferencing captured aliases."""
    if entry.get("captured_etag") is not None:
        return scenario_state.captured_batch_etags[entry["captured_etag"]]
    return entry.get("expected_etag")


def _apply_grpc_location(req_entry, entry: dict) -> None:
    """Set the entry's ResourceLocation arm: resource_identity when the entry
    carries ``arm == "identity"``, resource_address otherwise."""
    if entry.get("arm") == "identity":
        req_entry.resource_location.resource_identity.encoded_identity = entry["uri"]
    else:
        req_entry.resource_location.resource_address = entry["uri"]


def _rest_location(entry: dict) -> dict:
    if entry.get("arm") == "identity":
        return {"resource_identity": entry["uri"]}
    return {"resource_address": entry["uri"]}


@given(parsers.parse("metadata key '{key}' with value '{value}' exists at that address"))
def seed_user_metadata(key, value, scenario_state, request):
    try:
        setup = request.getfixturevalue("pre_read_only_storage_setup")
    except pytest.FixtureLookupError:
        pytest.skip("test harness cannot prepare metadata from before the service became read-only")
    setup.seed_user_metadata(scenario_state.resource_address, key, value)


# ---------------------------------------------------------------------------
# GetMetadataBatch request entries
# ---------------------------------------------------------------------------
# Registered as given/when/then: get-batch entries are assembled both as
# scenario setup (Given) and as post-delete verification (Then).
@given(parsers.parse("a get metadata batch request entry for key '{key}' on that address"))
@then(parsers.parse("a get metadata batch request entry for key '{key}' on that address"))
def add_get_batch_entry(key, scenario_state):
    scenario_state.get_metadata_batch_entries.append({"uri": _that_address(scenario_state), "key": key, "namespace": None})


@given(parsers.parse("a get metadata batch request entry for key '{key}' using the head resource identity"))
def add_get_batch_entry_for_head_identity(key, scenario_state):
    """Address the entry via the resource_identity arm, using the identity
    captured by "determining head resource identity with '<protocol>'"."""
    scenario_state.get_metadata_batch_entries.append(
        {"uri": scenario_state.previous_version, "key": key, "namespace": None, "arm": "identity"}
    )


@given(parsers.parse("a get metadata batch request entry for key '{key}' with that address in the resource identity arm"))
def add_get_batch_entry_with_address_as_identity(key, scenario_state):
    """Deliberately send a plain address through the resource_identity arm.

    Mirrors a client bug where a folder URI is routed through the batch call's
    identity arm; the service must reject the entry with INVALID_ARGUMENT
    rather than resolve it as an address."""
    scenario_state.get_metadata_batch_entries.append(
        {"uri": _that_address(scenario_state), "key": key, "namespace": None, "arm": "identity"}
    )


@given(parsers.parse("a get metadata batch request entry for key '{key}' with an invalid resource identity"))
def add_get_batch_entry_with_invalid_identity(key, scenario_state):
    """Send a malformed value (neither an encoded identity nor an address)
    through the resource_identity arm; the service must reject the entry with
    INVALID_ARGUMENT rather than treat it as a routable resource."""
    identity = scenario_state.sapi_test_client.generator().make_invalid_resource_identity()
    scenario_state.get_metadata_batch_entries.append({"uri": identity, "key": key, "namespace": None, "arm": "identity"})


@given(parsers.parse("a get metadata batch request entry for key '{key}' on the '{alias}' address"))
@then(parsers.parse("a get metadata batch request entry for key '{key}' on the '{alias}' address"))
def add_get_batch_entry_on_alias(key, alias, scenario_state):
    scenario_state.get_metadata_batch_entries.append({"uri": _alias_address(scenario_state, alias), "key": key, "namespace": None})


@given(parsers.parse("a get metadata batch request entry for key '{key}' on that address in namespace '{namespace}'"))
@then(parsers.parse("a get metadata batch request entry for key '{key}' on that address in namespace '{namespace}'"))
def add_get_batch_entry_in_namespace(key, namespace, scenario_state):
    scenario_state.get_metadata_batch_entries.append({"uri": _that_address(scenario_state), "key": key, "namespace": namespace})


# ---------------------------------------------------------------------------
# UpdateMetadataBatch request entries
# ---------------------------------------------------------------------------
@given(parsers.parse("an update metadata batch request entry for key '{key}' with value '{value}' on that address"))
def add_update_batch_entry(key, value, scenario_state):
    scenario_state.update_metadata_batch_entries.append(
        {"uri": _that_address(scenario_state), "key": key, "value": value, "namespace": None}
    )


@given(parsers.parse("an update metadata batch request entry for key '{key}' with value '{value}' using the head resource identity"))
def add_update_batch_entry_for_head_identity(key, value, scenario_state):
    scenario_state.update_metadata_batch_entries.append(
        {"uri": scenario_state.previous_version, "key": key, "value": value, "namespace": None, "arm": "identity"}
    )


@given(parsers.parse("an update metadata batch request entry for key '{key}' with value '{value}' on the '{alias}' address"))
def add_update_batch_entry_on_alias(key, value, alias, scenario_state):
    scenario_state.update_metadata_batch_entries.append(
        {"uri": _alias_address(scenario_state, alias), "key": key, "value": value, "namespace": None}
    )


@given(
    parsers.parse("an update metadata batch request entry for key '{key}' with value '{value}' on that address in namespace '{namespace}'")
)
def add_update_batch_entry_in_namespace(key, value, namespace, scenario_state):
    scenario_state.update_metadata_batch_entries.append(
        {"uri": _that_address(scenario_state), "key": key, "value": value, "namespace": namespace}
    )


@given(parsers.parse("an update metadata batch request entry for key '{key}' with value '{value}' using ETag '{etag}' on that address"))
def add_update_batch_entry_with_etag(key, value, etag, scenario_state):
    scenario_state.update_metadata_batch_entries.append(
        {"uri": _that_address(scenario_state), "key": key, "value": value, "namespace": None, "expected_etag": etag}
    )


@given(
    parsers.parse(
        "an update metadata batch request entry for key '{key}' with value '{value}' using captured ETag '{alias}' on that address"
    )
)
def add_update_batch_entry_with_captured_etag(key, value, alias, scenario_state):
    scenario_state.update_metadata_batch_entries.append(
        {"uri": _that_address(scenario_state), "key": key, "value": value, "namespace": None, "captured_etag": alias}
    )


# ---------------------------------------------------------------------------
# DeleteMetadataBatch request entries
# ---------------------------------------------------------------------------
@given(parsers.parse("a delete metadata batch request entry for key '{key}' on that address"))
def add_delete_batch_entry(key, scenario_state):
    scenario_state.delete_metadata_batch_entries.append({"uri": _that_address(scenario_state), "key": key, "namespace": None})


@given(parsers.parse("a delete metadata batch request entry for key '{key}' on that address in namespace '{namespace}'"))
def add_delete_batch_entry_in_namespace(key, namespace, scenario_state):
    scenario_state.delete_metadata_batch_entries.append({"uri": _that_address(scenario_state), "key": key, "namespace": namespace})


@given(parsers.parse("a delete metadata batch request entry for key '{key}' using ETag '{etag}' on that address"))
def add_delete_batch_entry_with_etag(key, etag, scenario_state):
    scenario_state.delete_metadata_batch_entries.append(
        {"uri": _that_address(scenario_state), "key": key, "namespace": None, "expected_etag": etag}
    )


@given(parsers.parse("a delete metadata batch request entry for key '{key}' using captured ETag '{alias}' on that address"))
def add_delete_batch_entry_with_captured_etag(key, alias, scenario_state):
    scenario_state.delete_metadata_batch_entries.append(
        {"uri": _that_address(scenario_state), "key": key, "namespace": None, "captured_etag": alias}
    )


# ---------------------------------------------------------------------------
# Execution steps
# ---------------------------------------------------------------------------
# Registered as when/then: get-batch is the operation under test in its own
# scenarios (When) and a verification read in the delete scenarios (Then).
@when(parsers.parse("calling '{protocol}' get metadata batch returns '{return_value}'"))
@then(parsers.parse("calling '{protocol}' get metadata batch returns '{return_value}'"))
def calling_get_metadata_batch(protocol, return_value, scenario_state, sapi_test_server_launch):
    entries = scenario_state.get_metadata_batch_entries
    pb2 = scenario_state.sapi_test_client.metadata_pb2(scenario_state.version_under_test["metadata"])

    def grpc_call():
        request = pb2.GetMetadataBatchRequest()
        for entry in entries:
            req_entry = request.requests.add()
            _apply_grpc_location(req_entry, entry)
            req_entry.user_metadata_key = entry["key"]
            if entry["namespace"] is not None:
                req_entry.namespace = entry["namespace"]
        client = scenario_state.sapi_test_client.metadata_grpc_client(scenario_state.version_under_test["metadata"])
        return list(client.GetMetadataBatch(request))

    def rest_call():
        return scenario_state.sapi_test_client.rest_client().get_metadata_batch(
            metadata_version=scenario_state.version_under_test["metadata"],
            json={"requests": [_rest_get_entry(e) for e in entries]},
        )

    scenario_state.last_metadata_response = structured_api_call(protocol, return_value, grpc_call=grpc_call, rest_call=rest_call)
    scenario_state.get_metadata_batch_entries = []


@given(parsers.parse("calling '{protocol}' update metadata batch returns '{return_value}'"))
@when(parsers.parse("calling '{protocol}' update metadata batch returns '{return_value}'"))
def calling_update_metadata_batch(protocol, return_value, scenario_state, sapi_test_server_launch):
    entries = scenario_state.update_metadata_batch_entries
    pb2 = scenario_state.sapi_test_client.metadata_pb2(scenario_state.version_under_test["metadata"])

    def grpc_call():
        request = pb2.UpdateMetadataBatchRequest()
        for entry in entries:
            req_entry = request.requests.add()
            _apply_grpc_location(req_entry, entry)
            req_entry.user_metadata_key = entry["key"]
            req_entry.user_metadata.CopyFrom(Value(string_value=entry["value"]))
            etag = _resolve_etag(scenario_state, entry)
            if etag is not None:
                req_entry.expected_etag = etag
            if entry["namespace"] is not None:
                req_entry.namespace = entry["namespace"]
        client = scenario_state.sapi_test_client.metadata_grpc_client(scenario_state.version_under_test["metadata"])
        return list(client.UpdateMetadataBatch(request))

    def rest_call():
        return scenario_state.sapi_test_client.rest_client().update_metadata_batch(
            metadata_version=scenario_state.version_under_test["metadata"],
            json={"requests": [_rest_update_entry(scenario_state, e) for e in entries]},
        )

    scenario_state.last_metadata_response = structured_api_call(protocol, return_value, grpc_call=grpc_call, rest_call=rest_call)
    scenario_state.update_metadata_batch_entries = []


@when(parsers.parse("calling '{protocol}' delete metadata batch returns '{return_value}'"))
def calling_delete_metadata_batch(protocol, return_value, scenario_state, sapi_test_server_launch):
    entries = scenario_state.delete_metadata_batch_entries
    pb2 = scenario_state.sapi_test_client.metadata_pb2(scenario_state.version_under_test["metadata"])

    def grpc_call():
        request = pb2.DeleteMetadataBatchRequest()
        for entry in entries:
            req_entry = request.requests.add()
            _apply_grpc_location(req_entry, entry)
            req_entry.user_metadata_key = entry["key"]
            etag = _resolve_etag(scenario_state, entry)
            if etag is not None:
                req_entry.expected_etag = etag
            if entry["namespace"] is not None:
                req_entry.namespace = entry["namespace"]
        client = scenario_state.sapi_test_client.metadata_grpc_client(scenario_state.version_under_test["metadata"])
        return list(client.DeleteMetadataBatch(request))

    def rest_call():
        return scenario_state.sapi_test_client.rest_client().delete_metadata_batch(
            metadata_version=scenario_state.version_under_test["metadata"],
            json={"requests": [_rest_delete_entry(scenario_state, e) for e in entries]},
        )

    scenario_state.last_metadata_response = structured_api_call(protocol, return_value, grpc_call=grpc_call, rest_call=rest_call)
    scenario_state.delete_metadata_batch_entries = []


@when(parsers.parse("calling '{protocol}' query metadata on that address returns '{return_value}'"))
def calling_query_metadata(protocol, return_value, scenario_state, sapi_test_server_launch):
    _query_metadata(protocol, return_value, scenario_state, namespace=None, max_page_size=None)


@when(parsers.parse("calling '{protocol}' query metadata on that address in namespace '{namespace}' returns '{return_value}'"))
def calling_query_metadata_in_namespace(protocol, namespace, return_value, scenario_state, sapi_test_server_launch):
    _query_metadata(protocol, return_value, scenario_state, namespace=namespace, max_page_size=None)


@when(parsers.parse("calling '{protocol}' query metadata on that address with a page size of '{page_size:d}' returns '{return_value}'"))
def calling_query_metadata_with_page_size(protocol, page_size, return_value, scenario_state, sapi_test_server_launch):
    _query_metadata(protocol, return_value, scenario_state, namespace=None, max_page_size=page_size)


def _query_metadata(protocol, return_value, scenario_state, namespace: Optional[str], max_page_size: Optional[int]):
    pb2 = scenario_state.sapi_test_client.metadata_pb2(scenario_state.version_under_test["metadata"])
    uri = scenario_state.resource_address

    def grpc_call():
        request = pb2.QueryMetadataRequest()
        request.resource_location.resource_address = uri
        if namespace is not None:
            request.namespace = namespace
        client = scenario_state.sapi_test_client.metadata_grpc_client(scenario_state.version_under_test["metadata"])
        return list(client.QueryMetadata(request))

    def rest_call():
        body: dict = {"resource_location": {"resource_address": uri}}
        if namespace is not None:
            body["namespace"] = namespace
        if max_page_size is not None:
            body["max_page_size"] = max_page_size
        return scenario_state.sapi_test_client.rest_client().query_metadata(
            metadata_version=scenario_state.version_under_test["metadata"],
            json=body,
        )

    scenario_state.last_metadata_response = structured_api_call(protocol, return_value, grpc_call=grpc_call, rest_call=rest_call)


# ---------------------------------------------------------------------------
# REST entry builders
# ---------------------------------------------------------------------------
def _rest_get_entry(entry: dict) -> dict:
    body = {"resource_location": _rest_location(entry), "user_metadata_key": entry["key"]}
    if entry["namespace"] is not None:
        body["namespace"] = entry["namespace"]
    return body


def _rest_update_entry(scenario_state, entry: dict) -> dict:
    body = {"resource_location": _rest_location(entry), "user_metadata_key": entry["key"], "user_metadata": entry["value"]}
    etag = _resolve_etag(scenario_state, entry)
    if etag is not None:
        body["expected_etag"] = etag
    if entry["namespace"] is not None:
        body["namespace"] = entry["namespace"]
    return body


def _rest_delete_entry(scenario_state, entry: dict) -> dict:
    body = {"resource_location": _rest_location(entry), "user_metadata_key": entry["key"]}
    etag = _resolve_etag(scenario_state, entry)
    if etag is not None:
        body["expected_etag"] = etag
    if entry["namespace"] is not None:
        body["namespace"] = entry["namespace"]
    return body
