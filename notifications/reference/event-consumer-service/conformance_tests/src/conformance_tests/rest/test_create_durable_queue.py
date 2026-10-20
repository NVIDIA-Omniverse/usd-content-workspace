# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REST conformance tests for POST /api/v1beta/queues/durable (CreateDurableQueue).

Contract under test:
  POST /api/v1beta/queues/durable
    Body: { "filter_groups": [ FilterGroup... ] }

Expected behaviour:
  - Returns HTTP 200 with {"queue_id": "<non-empty string>"}.
  - Accepts empty filter_groups list.
  - Successive calls return distinct queue_ids.
  - Returns HTTP 422 for missing body, invalid JSON, or malformed filter_groups.
  - Returns HTTP 401 when auth is required and no token is sent (requires_auth).
"""

import httpx
import pytest

from conformance_tests.conftest import AUTH_REQUIRED

pytestmark = pytest.mark.rest


class TestPostCreateDurableQueue:
    def test_create_with_valid_filter_returns_200_with_queue_id(
        self, http_client, valid_create_body
    ):
        """
        POST /api/v1beta/queues/durable with a valid FilterGroup must return
        HTTP 200 and a JSON body with a non-empty queue_id.
        """
        response = http_client.post(
            "/api/v1beta/queues/durable", json=valid_create_body
        )

        assert response.status_code == 200, (
            f"Expected HTTP 200 for valid CreateDurableQueue request, "
            f"but got {response.status_code}. Body: {response.text}"
        )
        body = response.json()
        assert body.get("queue_id"), (
            f"Expected response body to contain a non-empty 'queue_id', "
            f"but got: {body}"
        )

    def test_create_with_empty_filter_groups_returns_422(self, http_client):
        """
        POST /api/v1beta/queues/durable with an empty filter_groups list must
        return HTTP 422. At least one filter group is required to subscribe to events.
        """
        response = http_client.post(
            "/api/v1beta/queues/durable", json={"filter_groups": []}
        )
        assert response.status_code == 422, (
            f"Expected HTTP 422 for empty filter_groups (service requires at least "
            f"one filter group), but got {response.status_code}. Body: {response.text}"
        )

    def test_create_returns_unique_queue_ids(self, http_client, valid_create_body):
        """Each call to POST /api/v1beta/queues/durable must return a distinct queue_id."""
        resp1 = http_client.post("/api/v1beta/queues/durable", json=valid_create_body)
        resp2 = http_client.post("/api/v1beta/queues/durable", json=valid_create_body)

        assert resp1.status_code == 200
        assert resp2.status_code == 200

        id1 = resp1.json().get("queue_id")
        id2 = resp2.json().get("queue_id")

        assert id1 != id2, (
            f"Expected two separate CreateDurableQueue calls to return different "
            f"queue_ids, but both returned '{id1}'."
        )

    def test_create_with_missing_body_returns_422(self, http_client):
        """POST /api/v1beta/queues/durable without a body must return HTTP 422."""
        response = http_client.post("/api/v1beta/queues/durable")
        assert response.status_code == 422, (
            f"Expected HTTP 422 for missing body, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_create_with_invalid_json_returns_422(self, http_client):
        """POST /api/v1beta/queues/durable with non-JSON body must return HTTP 422."""
        response = http_client.post(
            "/api/v1beta/queues/durable",
            content="this is not json",
            headers={"Content-Type": "application/json"},
        )
        assert response.status_code == 422, (
            f"Expected HTTP 422 for invalid JSON body, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_create_with_multiple_filter_groups_returns_200(self, http_client):
        """POST /api/v1beta/queues/durable with multiple FilterGroups must succeed."""
        body = {
            "filter_groups": [
                {"event_type": "conformance.event.alpha", "filters": []},
                {"event_type": "conformance.event.beta", "filters": []},
            ]
        }
        response = http_client.post("/api/v1beta/queues/durable", json=body)

        assert response.status_code == 200, (
            f"Expected HTTP 200 for multiple filter groups, "
            f"but got {response.status_code}. Body: {response.text}"
        )
        assert response.json().get("queue_id"), "Expected non-empty queue_id."

    def test_create_with_starts_with_greedy_filter_returns_200(self, http_client):
        """POST /api/v1beta/queues/durable with starts_with_greedy filter type must succeed."""
        body = {
            "filter_groups": [
                {
                    "event_type": "conformance.greedy.event",
                    "filters": [
                        {"filter_type": "starts_with_greedy", "resource_id": "prefix/"}
                    ],
                }
            ]
        }
        response = http_client.post("/api/v1beta/queues/durable", json=body)

        assert response.status_code == 200, (
            f"Expected HTTP 200 for starts_with_greedy filter, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    @pytest.mark.skipif(not AUTH_REQUIRED, reason="AUTH_REQUIRED not set")
    def test_create_without_auth_returns_401(self, rest_base_url, valid_create_body):
        """POST /api/v1beta/queues/durable without Authorization header must return 401.

        Sends an otherwise-valid body so the 422 validation path doesn't run
        first and mask the missing-auth rejection.
        """
        response = httpx.post(
            f"{rest_base_url}/api/v1beta/queues/durable",
            json=valid_create_body,
        )
        assert response.status_code == 401, (
            f"Expected HTTP 401 when no Authorization header is sent, "
            f"but got {response.status_code}. Body: {response.text}"
        )
