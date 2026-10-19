# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Shared fixtures and helpers for REST conformance tests."""

import json
import urllib.parse

import pytest

# ---------------------------------------------------------------------------
# Shared filter-group helpers
# ---------------------------------------------------------------------------


def make_filter_groups_json(filter_groups: list[dict]) -> str:
    """URL-encode a list of filter group dicts for use as the filter_groups query parameter."""
    return urllib.parse.quote(json.dumps(filter_groups))


@pytest.fixture(scope="session")
def valid_filter_group_dict() -> dict:
    """A single valid FilterGroup dict for use in REST requests."""
    return {
        "event_type": "conformance.test.event",
        "filters": [
            {
                "filter_type": "eq",
                "resource_id": "test-resource/",
            }
        ],
    }


@pytest.fixture(scope="session")
def valid_filter_groups_json(valid_filter_group_dict) -> str:
    """URL-encoded JSON string of a single valid FilterGroup for query params."""
    return make_filter_groups_json([valid_filter_group_dict])


@pytest.fixture(scope="session")
def valid_create_body(valid_filter_group_dict) -> dict:
    """JSON body for a valid CreateDurableQueue POST request."""
    return {"filter_groups": [valid_filter_group_dict]}
