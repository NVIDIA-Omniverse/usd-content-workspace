# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REST conformance tests for POST /api/v1beta/events/batch (BatchPublishEvents).

Contract under test:
  POST /api/v1beta/events/batch
    Body: {
      "events": [ Event, ... ]   (max 1000)
    }

Expected behaviour:
  - Returns HTTP 200 with a results array whose length equals the input count.
  - Each result in the array has success == true for valid events.
  - An empty events array returns HTTP 400 (not 422 — the service explicitly
    rejects it as a BadRequestError).
  - Missing events field → 422 (Pydantic validation).
  - An event in the list with an invalid field → 422.
  - events list exceeding 1000 items → 422.
  - An event whose message encodes to more than 65536 JSON bytes → 422 for
    the whole batch.
  - Returns HTTP 401 when auth is required and no token is sent (requires_auth).
"""

import pytest

from conformance_tests.conftest import (
    AUTH_REQUIRED,
    MAX_EVENT_MESSAGE_BYTES,
    make_rest_event,
    message_of_size,
)

pytestmark = pytest.mark.rest


class TestBatchPublishEvents:
    def test_batch_publish_single_event_returns_200(self, http_client):
        """
        POST /api/v1beta/events/batch with a single event must return HTTP 200.
        """
        response = http_client.post(
            "/api/v1beta/events/batch",
            json={"events": [make_rest_event()]},
        )
        assert response.status_code == 200, (
            f"Expected HTTP 200 for a batch of one event, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_batch_publish_multiple_events_returns_200(self, http_client):
        """
        POST /api/v1beta/events/batch with multiple events must return HTTP 200.
        """
        events = [make_rest_event() for _ in range(5)]
        response = http_client.post(
            "/api/v1beta/events/batch",
            json={"events": events},
        )
        assert response.status_code == 200, (
            f"Expected HTTP 200 for a batch of {len(events)} events, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_batch_response_results_count_matches_input(self, http_client):
        """
        The results array in the response must contain exactly as many entries
        as the input events list.
        """
        n = 3
        response = http_client.post(
            "/api/v1beta/events/batch",
            json={"events": [make_rest_event() for _ in range(n)]},
        )
        assert response.status_code == 200
        results = response.json().get("results", [])
        assert len(results) == n, (
            f"Expected {n} results for a batch of {n} events, "
            f"but got {len(results)}. Body: {response.text}"
        )

    def test_batch_all_results_have_success_true(self, http_client):
        """
        All PublishingResult entries in a successful batch must have
        success == true.
        """
        events = [make_rest_event() for _ in range(3)]
        response = http_client.post(
            "/api/v1beta/events/batch",
            json={"events": events},
        )
        assert response.status_code == 200
        for i, result in enumerate(response.json().get("results", [])):
            assert result.get("success") is True, (
                f"Expected results[{i}].success == true, " f"but got: {result}"
            )

    def test_batch_empty_events_list_returns_400(self, http_client):
        """
        POST /api/v1beta/events/batch with an empty events array must return
        HTTP 400.  An empty list is explicitly rejected — it is not a validation
        error (422) but a bad-request semantic error.
        """
        response = http_client.post(
            "/api/v1beta/events/batch",
            json={"events": []},
        )
        assert response.status_code == 400, (
            f"Expected HTTP 400 for empty events list, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_batch_missing_events_field_returns_422(self, http_client):
        """
        POST /api/v1beta/events/batch without the events field must return
        HTTP 422 (Pydantic required-field validation).
        """
        response = http_client.post("/api/v1beta/events/batch", json={})
        assert response.status_code == 422, (
            f"Expected HTTP 422 for missing events field, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_batch_invalid_event_in_list_returns_422(self, http_client):
        """
        POST /api/v1beta/events/batch where one event has an invalid field
        (missing occurred_at) must return HTTP 422.
        """
        bad_event = make_rest_event()
        del bad_event["occurred_at"]
        response = http_client.post(
            "/api/v1beta/events/batch",
            json={"events": [bad_event]},
        )
        assert response.status_code == 422, (
            f"Expected HTTP 422 for an event with missing occurred_at, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_batch_events_exceeding_max_returns_422(self, http_client):
        """
        POST /api/v1beta/events/batch with more than 1000 events must return
        HTTP 422.  The events list has a max_length of 1000.
        """
        events = [make_rest_event() for _ in range(1001)]
        response = http_client.post(
            "/api/v1beta/events/batch",
            json={"events": events},
        )
        assert response.status_code == 422, (
            f"Expected HTTP 422 for events list exceeding 1000 items, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_batch_with_oversize_message_returns_422(self, http_client):
        """
        A batch where one event's message encodes to more than 65536 JSON bytes
        must return HTTP 422 for the whole batch.
        """
        oversize = make_rest_event()
        oversize["message"] = message_of_size(MAX_EVENT_MESSAGE_BYTES + 1)
        response = http_client.post(
            "/api/v1beta/events/batch",
            json={"events": [make_rest_event(), oversize, make_rest_event()]},
        )
        assert response.status_code == 422, (
            f"Expected HTTP 422 for a batch with an oversize message, "
            f"but got {response.status_code}. Body: {response.text[:500]}"
        )

    def test_batch_invalid_json_returns_422(self, http_client):
        """
        POST /api/v1beta/events/batch with a non-JSON body must return HTTP 422.
        """
        response = http_client.post(
            "/api/v1beta/events/batch",
            content="not json",
            headers={"Content-Type": "application/json"},
        )
        assert response.status_code == 422, (
            f"Expected HTTP 422 for invalid JSON body, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    @pytest.mark.skipif(not AUTH_REQUIRED, reason="AUTH_REQUIRED not set")
    def test_batch_without_auth_returns_401(self, rest_base_url):
        """
        POST /api/v1beta/events/batch without an Authorization header must
        return HTTP 401 when the service has authentication enabled.
        """
        import httpx

        response = httpx.post(
            f"{rest_base_url}/api/v1beta/events/batch",
            json={"events": [make_rest_event()]},
        )
        assert response.status_code == 401, (
            f"Expected HTTP 401 when no Authorization header is sent, "
            f"but got {response.status_code}. Body: {response.text}"
        )
