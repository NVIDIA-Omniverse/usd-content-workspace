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

import contextlib
from typing import Dict

import pytest
from pytest_bdd import (
    parsers,
    then,
    when,
)


def extract_metadata_entries(response) -> Dict:
    """Extract metadata entries from either gRPC or REST response."""
    if hasattr(response, "user_metadata"):
        # gRPC response
        return response.user_metadata
    else:
        # REST response
        if not response:
            return {}
        try:
            return response.json()
        except (ValueError, AttributeError) as e:
            pytest.fail(f"Can't extract metadata from non-json response {response}: {e}")
            raise AssertionError("Unreachable")  # Help mypy understand pytest.fail doesn't return


@then(parsers.parse("the metadata response contains '{count:d}' entries"))
def metadata_response_contains_count_entries(count, scenario_state):
    response = scenario_state.last_metadata_response
    entries = extract_metadata_entries(response)

    assert len(entries) == count, f"Expected {count} entries, got {len(entries)}"


@then(parsers.parse("the metadata response contains key '{key}' with value '{value}'"))
def metadata_response_contains_key_value(key, value, scenario_state):
    response = scenario_state.last_metadata_response
    entries = extract_metadata_entries(response)
    assert key in entries, f"Key '{key}' not found in metadata response"
    if hasattr(response, "user_metadata"):
        # gRPC response
        assert entries[key].value.string_value == value, f"Expected value '{value}', got '{entries[key].value.string_value}'"
    else:
        # REST response
        assert entries[key]["value"] == value, f"Expected value '{value}', got '{entries[key]['value']}'"


@when(parsers.parse("we capture the just-returned metadata ETag as '{var_name}'"))
def capture_metadata_etag(var_name, scenario_state):
    """Stash the etag from ``scenario_state.last_metadata_response`` into
    ``scenario_state.captured_etags[var_name]``. Used by race-condition
    scenarios that need to feed the same ETag back into a later
    optimistic-locked batch call.
    """
    response = scenario_state.last_metadata_response
    etag = None
    if hasattr(response, "etag"):
        etag = response.etag
    elif hasattr(response, "headers"):
        etag = response.headers.get("ETag") or response.headers.get("etag")
        if not etag:
            try:
                body = response.json()
                if isinstance(body, dict):
                    etag = body.get("etag")
            except (ValueError, AttributeError):
                pass
    assert etag, f"No ETag found on last_metadata_response: {response!r}"
    if not hasattr(scenario_state, "captured_etags") or scenario_state.captured_etags is None:
        scenario_state.captured_etags = {}
    scenario_state.captured_etags[var_name] = etag


@then(parsers.parse("the metadata response contains key '{key}' with value in '{values_json}'"))
def metadata_response_contains_key_value_in_set(key, values_json, scenario_state):
    """Used by the conflicting-writes batch scenario where the resulting value is
    one of several allowed possibilities -- the server may apply either entry of
    a same-(uri, key) conflict, but never something the client did not send.
    """
    import json as _json

    candidates = _json.loads(values_json)
    if not isinstance(candidates, list):
        raise ValueError(f"`values_json` must decode to a JSON array, got: {values_json!r}")

    response = scenario_state.last_metadata_response
    entries = extract_metadata_entries(response)
    assert key in entries, f"Key '{key}' not found in metadata response"
    if hasattr(response, "user_metadata"):
        actual = entries[key].value.string_value
    else:
        actual = entries[key]["value"]
    assert actual in candidates, f"Expected value in {candidates!r}, got {actual!r}"


@then(parsers.parse("the metadata response contains key '{key}' with numeric value '{value}'"))
def metadata_response_contains_key_numeric_value(key, value, scenario_state):
    response = scenario_state.last_metadata_response
    expected_value = float(value)
    entries = extract_metadata_entries(response)
    assert key in entries, f"Key '{key}' not found in metadata response"

    if hasattr(response, "user_metadata"):
        # gRPC response
        assert (
            entries[key].value.number_value == expected_value
        ), f"Expected value '{expected_value}', got '{entries[key].value.number_value}'"
    else:
        # REST response
        assert entries[key]["value"] == expected_value, f"Expected value '{expected_value}', got '{entries[key]['value']}'"


@then(parsers.parse("the metadata response contains key '{key}' with boolean value '{value}'"))
def metadata_response_contains_key_boolean_value(key, value, scenario_state):
    response = scenario_state.last_metadata_response
    expected_value = value.lower() == "true"
    entries = extract_metadata_entries(response)
    assert key in entries, f"Key '{key}' not found in metadata response"

    if hasattr(response, "user_metadata"):
        # gRPC response
        assert entries[key].value.bool_value == expected_value, f"Expected value '{expected_value}', got '{entries[key].value.bool_value}'"
    else:
        # REST response
        assert entries[key]["value"] == expected_value, f"Expected value '{expected_value}', got '{entries[key]['value']}'"


@then(parsers.parse("the metadata response contains an ETag"))
def metadata_response_contains_etag(scenario_state):
    response = scenario_state.last_metadata_response
    if hasattr(response, "etag"):
        # gRPC response
        assert response.etag, "ETag should be present in metadata response"
    else:
        # REST response
        etag_in_headers = "etag" in response.headers
        with contextlib.suppress(ValueError, AttributeError):
            etag_in_body = "etag" in response.json()
        assert etag_in_headers or etag_in_body, "ETag should be present in metadata response"
