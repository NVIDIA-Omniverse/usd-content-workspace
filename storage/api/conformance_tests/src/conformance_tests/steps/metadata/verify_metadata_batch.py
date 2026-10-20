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

"""Verification steps for the batch and query metadata endpoints.

Batch responses carry one entry per request entry, but the entries may arrive
in any order; every entry echoes the zero-based ``request_index`` of the request
that produced it. These steps normalise the gRPC streamed pages and the REST
JSON body into a single shape keyed by ``request_index`` so each response row can
be asserted individually, and they also assert the total number of rows.
"""

from typing import (
    Any,
    Dict,
    List,
)

import pytest
from pytest_bdd import (
    given,
    parsers,
    then,
    when,
)

# Per-entry error codes, mapped to the symbolic names used in feature tables.
# gRPC carries them as the BatchErrorEntry.Code enum (CODE_<NAME>); REST carries
# them as the lower-cased <name> string. Both normalise to <NAME>.
_GRPC_CODE_NAMES = {
    0: "UNSPECIFIED",
    1: "INVALID_ARGUMENT",
    2: "NOT_FOUND",
    3: "PERMISSION_DENIED",
    4: "FAILED_PRECONDITION",
    5: "INTERNAL",
    6: "RESOURCE_EXHAUSTED",
    7: "UNAVAILABLE",
}


def _decode_pb_value(value) -> Any:
    """Decode a protobuf Value into a Python native type."""
    kind = value.WhichOneof("kind")
    if kind == "string_value":
        return value.string_value
    if kind == "bool_value":
        return value.bool_value
    if kind == "number_value":
        return value.number_value
    if kind == "list_value":
        return [_decode_pb_value(v) for v in value.list_value.values]
    if kind == "struct_value":
        return {k: _decode_pb_value(v) for k, v in value.struct_value.fields.items()}
    return None


def _normalize_grpc_entry(entry) -> Dict[str, Any]:
    out: Dict[str, Any] = {"request_index": entry.request_index, "etag": None, "value": None, "error_code": None}
    which = entry.WhichOneof("outcome")
    if which == "success":
        out["kind"] = "success"
        success = entry.success
        fields = success.DESCRIPTOR.fields_by_name
        if "etag" in fields:
            out["etag"] = success.etag
        if "value" in fields:
            out["value"] = _decode_pb_value(success.value)
    elif which == "error":
        out["kind"] = "error"
        out["error_code"] = _GRPC_CODE_NAMES.get(entry.error.code, str(entry.error.code))
    else:
        pytest.fail(f"Batch entry has neither success nor error set: {entry!r}")
    return out


def _normalize_rest_entry(entry: dict) -> Dict[str, Any]:
    out: Dict[str, Any] = {"request_index": entry["request_index"], "etag": None, "value": None, "error_code": None}
    outcome = entry["outcome"]
    if "success" in outcome:
        out["kind"] = "success"
        success = outcome["success"] or {}
        out["etag"] = success.get("etag")
        out["value"] = success.get("value")
    elif "error" in outcome:
        out["kind"] = "error"
        out["error_code"] = outcome["error"]["code"].upper()
    else:
        pytest.fail(f"Batch entry has neither success nor error set: {entry!r}")
    return out


def _normalize_batch_entries(response) -> List[Dict[str, Any]]:
    """Normalise a batch response (gRPC page list or REST Response) into a list
    of per-entry dicts: ``{request_index, kind, etag, value, error_code}``."""
    if isinstance(response, list):
        entries: List[Dict[str, Any]] = []
        for page in response:
            entries.extend(_normalize_grpc_entry(e) for e in page.results)
        return entries
    # REST requests.Response
    body = response.json()
    return [_normalize_rest_entry(e) for e in body["results"]]


def _entry_at_index(scenario_state, index: int) -> Dict[str, Any]:
    entries = _normalize_batch_entries(scenario_state.last_metadata_response)
    matches = [e for e in entries if e["request_index"] == index]
    assert matches, f"No batch entry with request_index {index} in {entries!r}"
    assert len(matches) == 1, f"Multiple batch entries with request_index {index}: {matches!r}"
    return matches[0]


# ---------------------------------------------------------------------------
# Batch response assertions
# ---------------------------------------------------------------------------
@then(parsers.parse("the metadata batch response contains '{count:d}' entries"))
def metadata_batch_response_contains_count_entries(count, scenario_state):
    entries = _normalize_batch_entries(scenario_state.last_metadata_response)
    assert len(entries) == count, f"Expected {count} batch entries, got {len(entries)}: {entries!r}"


@then(parsers.parse("the metadata batch entry at request index '{index:d}' succeeded"))
def metadata_batch_entry_succeeded(index, scenario_state):
    entry = _entry_at_index(scenario_state, index)
    assert entry["kind"] == "success", f"Entry {index} should be a success but was {entry!r}"


@then(parsers.parse("the metadata batch entry at request index '{index:d}' succeeded with value '{value}'"))
def metadata_batch_entry_succeeded_with_value(index, value, scenario_state):
    entry = _entry_at_index(scenario_state, index)
    assert entry["kind"] == "success", f"Entry {index} should be a success but was {entry!r}"
    assert entry["value"] == value, f"Entry {index} expected value '{value}', got '{entry['value']}'"


@then(parsers.parse("the metadata batch entry at request index '{index:d}' succeeded with an ETag"))
def metadata_batch_entry_succeeded_with_etag(index, scenario_state):
    entry = _entry_at_index(scenario_state, index)
    assert entry["kind"] == "success", f"Entry {index} should be a success but was {entry!r}"
    assert entry["etag"], f"Entry {index} should carry a non-empty ETag, got {entry!r}"


@then(parsers.parse("the metadata batch entry at request index '{index:d}' failed with code '{code}'"))
def metadata_batch_entry_failed_with_code(index, code, scenario_state):
    entry = _entry_at_index(scenario_state, index)
    assert entry["kind"] == "error", f"Entry {index} should be a failure but was {entry!r}"
    assert entry["error_code"] == code, f"Entry {index} expected error code '{code}', got '{entry['error_code']}'"


@given(parsers.parse("we capture the metadata batch entry at request index '{index:d}' ETag as '{alias}'"))
@when(parsers.parse("we capture the metadata batch entry at request index '{index:d}' ETag as '{alias}'"))
@then(parsers.parse("we capture the metadata batch entry at request index '{index:d}' ETag as '{alias}'"))
def capture_batch_entry_etag(index, alias, scenario_state):
    entry = _entry_at_index(scenario_state, index)
    assert entry["etag"], f"Entry {index} has no ETag to capture: {entry!r}"
    scenario_state.captured_batch_etags[alias] = entry["etag"]


# ---------------------------------------------------------------------------
# QueryMetadata response assertions
# ---------------------------------------------------------------------------
def _normalize_query_entries(response) -> Dict[str, Any]:
    """Normalise a query response into a ``{key: value}`` mapping."""
    result: Dict[str, Any] = {}
    if isinstance(response, list):
        for page in response:
            for entry in page.results:
                result[entry.user_metadata_key] = _decode_pb_value(entry.user_metadata.value)
        return result
    body = response.json()
    for entry in body["results"]:
        result[entry["user_metadata_key"]] = entry["user_metadata"]["value"]
    return result


@then(parsers.parse("the query metadata response contains '{count:d}' entries"))
def query_metadata_response_contains_count_entries(count, scenario_state):
    entries = _normalize_query_entries(scenario_state.last_metadata_response)
    assert len(entries) == count, f"Expected {count} query entries, got {len(entries)}: {entries!r}"


@then(parsers.parse("the query metadata response contains key '{key}' with value '{value}'"))
def query_metadata_response_contains_key_value(key, value, scenario_state):
    entries = _normalize_query_entries(scenario_state.last_metadata_response)
    assert key in entries, f"Key '{key}' not found in query response: {entries!r}"
    assert entries[key] == value, f"Key '{key}' expected value '{value}', got '{entries[key]}'"
