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
from typing import (
    Any,
    Dict,
)

from conformance_tests.storage_testclient import ConformanceTestClient
from pytest import fixture


class ScenarioStateFixture:
    def __init__(self, sapi_test_client: ConformanceTestClient):
        """Initialize the mutable state shared by the steps in one scenario."""
        self.sapi_test_client = sapi_test_client
        self.resource_address: str | None = None
        self.resource_identity: Any = None
        self.previous_version: Any = None
        self.namespaces: Dict[str, str] = {}
        self.version_under_test: Dict[str, str] = {}
        self.last_response: Any = None
        self.last_enumerate_response: Any = None
        self.last_list_response: Any = None
        self.last_list_stat_response: Any = None
        self.last_metadata_response: Any = None
        self.last_enumerate_versions_response: Any = None
        self.memorized_responses: Dict[str, Any] = {}
        self.seeded_blob_sizes: Dict[int, int] = {}
        # Per-operation batch request entries, built up one cucumber step at a
        # time and drained by the matching "calling ... batch" execution step.
        self.get_metadata_batch_entries: list = []
        self.update_metadata_batch_entries: list = []
        self.delete_metadata_batch_entries: list = []
        self.captured_batch_etags: Dict[str, str] = {}


@fixture
def scenario_state(sapi_test_client) -> ScenarioStateFixture:
    return ScenarioStateFixture(sapi_test_client)


def identity_from_fixture(fixture_value: Any):
    """Extract an encoded resource identity from supported response fixtures."""
    if hasattr(fixture_value, "encoded_identity"):
        return fixture_value.encoded_identity
    elif hasattr(fixture_value, "resource_info"):
        return fixture_value.resource_info.resource_identity.encoded_identity
    elif hasattr(fixture_value, "resource_identity"):
        resource_identity = fixture_value.resource_identity
        return resource_identity.encoded_identity if hasattr(resource_identity, "encoded_identity") else resource_identity
    elif hasattr(fixture_value, "headers") and "x-nvidia-omniverse-storage-resource-identity" in fixture_value.headers:
        return fixture_value.headers["x-nvidia-omniverse-storage-resource-identity"]
    elif hasattr(fixture_value, "json"):
        try:
            response_json = fixture_value.json()
        except ValueError:
            return fixture_value
        return identity_from_fixture(response_json)
    elif isinstance(fixture_value, dict):
        if "resource_info" in fixture_value:
            return fixture_value["resource_info"]["resource_identity"]
        if "resource_identity" in fixture_value:
            return fixture_value["resource_identity"]
    return fixture_value


def size_from_fixture(last_response):
    if hasattr(last_response, "value"):
        last_response = last_response.value
    if hasattr(last_response, "resource_info"):
        return last_response.resource_info.metadata.data_object_size
    elif hasattr(last_response, "metadata"):
        return last_response.metadata.data_object_size
    elif hasattr(last_response, "headers") and "x-nvidia-omniverse-storage-metadata" in last_response.headers:
        try:
            response = json.loads(last_response.headers["x-nvidia-omniverse-storage-metadata"])
        except (json.JSONDecodeError, KeyError) as e:
            raise ValueError(f"Failed to parse metadata from headers: {e}") from e
        return response["data_object_size"]
    elif hasattr(last_response, "content"):
        return len(last_response.content)
    else:
        raise ValueError(f"last_response has surprising type: {last_response}")
