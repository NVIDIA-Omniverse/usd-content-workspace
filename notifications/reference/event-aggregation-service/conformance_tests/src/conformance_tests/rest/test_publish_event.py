# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REST conformance tests for POST /api/v1beta/events (PublishEvent).

Contract under test:
  POST /api/v1beta/events
    Body: {
      "event": {
        "event_type": "<string>",
        "message": { ... },
        "occurred_at": "<ISO 8601>",
        "resource": { "resource_id": "<string>" }   (optional)
      }
    }

Expected behaviour:
  - Returns HTTP 200 with result.success == true for a valid event.
  - resource field is optional; omitting it is equivalent to resource_id=''.
  - occurred_at must match ISO 8601 with timezone; invalid formats → 422.
  - event_type must match ^[a-zA-Z0-9._-]+$ and be ≤256 chars; violations → 422.
  - message is required; missing → 422.
  - message must encode to at most 65536 JSON bytes; larger → 422.
  - resource.resource_id must be ≤2048 chars; longer → 422.
  - occurred_at must be ≤64 chars even when it matches the pattern; longer → 422.
  - occurred_at is required; missing → 422.
  - event_type is required; missing → 422.
  - Missing entire body → 422.
  - Non-JSON body → 422.
  - Returns HTTP 401 when auth is required and no token is sent (requires_auth).
"""

import pytest

from conformance_tests.conftest import (
    AUTH_REQUIRED,
    MAX_EVENT_MESSAGE_BYTES,
    MAX_RESOURCE_ID_LENGTH,
    make_rest_event,
    message_of_size,
)

pytestmark = pytest.mark.rest


class TestPublishEvent:
    def test_publish_valid_event_returns_200_with_success_true(self, http_client):
        """
        POST /api/v1beta/events with a fully-valid event must return HTTP 200
        and result.success == true.
        """
        response = http_client.post(
            "/api/v1beta/events",
            json={"event": make_rest_event(with_resource=True)},
        )
        assert response.status_code == 200, (
            f"Expected HTTP 200 for a valid publish request, "
            f"but got {response.status_code}. Body: {response.text}"
        )
        body = response.json()
        assert (
            body.get("result", {}).get("success") is True
        ), f"Expected result.success == true, but got: {body}"

    def test_publish_event_without_resource_returns_200(self, http_client):
        """
        POST /api/v1beta/events without the optional resource field must
        still return HTTP 200 and result.success == true.
        """
        response = http_client.post(
            "/api/v1beta/events",
            json={"event": make_rest_event(with_resource=False)},
        )
        assert response.status_code == 200, (
            f"Expected HTTP 200 when resource is omitted, "
            f"but got {response.status_code}. Body: {response.text}"
        )
        assert response.json().get("result", {}).get("success") is True

    def test_publish_event_response_echoes_event_type(self, http_client):
        """
        The result.event in the response must echo back the submitted event_type.
        """
        event_type = "conformance.echo.check"
        event = make_rest_event()
        event["event_type"] = event_type
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 200
        returned_type = (
            response.json().get("result", {}).get("event", {}).get("event_type")
        )
        assert returned_type == event_type, (
            f"Expected result.event.event_type == '{event_type}', "
            f"but got '{returned_type}'."
        )

    def test_publish_with_timestamp_with_offset_returns_200(self, http_client):
        """
        occurred_at with a non-UTC timezone offset (e.g. +05:30) must be
        accepted and return HTTP 200.
        """
        event = make_rest_event()
        event["occurred_at"] = "2024-06-15T14:30:00+05:30"
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 200, (
            f"Expected HTTP 200 for occurred_at with timezone offset, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_publish_with_microseconds_in_timestamp_returns_200(self, http_client):
        """
        occurred_at with microseconds (e.g. 2024-01-15T10:30:00.123456Z) must
        be accepted and return HTTP 200.
        """
        event = make_rest_event()
        event["occurred_at"] = "2024-01-15T10:30:00.123456Z"
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 200, (
            f"Expected HTTP 200 for occurred_at with microseconds, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_publish_with_dotted_event_type_returns_200(self, http_client):
        """
        event_type values containing dots (common convention: 'service.noun.verb')
        must be accepted.
        """
        event = make_rest_event()
        event["event_type"] = "omni.storage.file.created"
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 200, (
            f"Expected HTTP 200 for dotted event_type, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_publish_missing_event_type_returns_422(self, http_client):
        """
        POST /api/v1beta/events without event_type must return HTTP 422.
        """
        event = make_rest_event()
        del event["event_type"]
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 422, (
            f"Expected HTTP 422 for missing event_type, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_publish_missing_message_returns_422(self, http_client):
        """
        POST /api/v1beta/events without message must return HTTP 422.
        """
        event = make_rest_event()
        del event["message"]
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 422, (
            f"Expected HTTP 422 for missing message, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_publish_missing_occurred_at_returns_422(self, http_client):
        """
        POST /api/v1beta/events without occurred_at must return HTTP 422.
        """
        event = make_rest_event()
        del event["occurred_at"]
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 422, (
            f"Expected HTTP 422 for missing occurred_at, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_publish_invalid_occurred_at_format_returns_422(self, http_client):
        """
        occurred_at that does not match the ISO 8601 pattern (e.g. plain date
        string without time or timezone) must return HTTP 422.
        """
        event = make_rest_event()
        event["occurred_at"] = "2024-01-15"  # missing time + timezone
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 422, (
            f"Expected HTTP 422 for invalid occurred_at format '2024-01-15', "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_publish_event_type_with_spaces_returns_422(self, http_client):
        """
        event_type containing spaces violates the pattern ^[a-zA-Z0-9._-]+$
        and must return HTTP 422.
        """
        event = make_rest_event()
        event["event_type"] = "invalid event type"
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 422, (
            f"Expected HTTP 422 for event_type with spaces, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_publish_event_type_exceeds_max_length_returns_422(self, http_client):
        """
        event_type longer than 256 characters must return HTTP 422.
        """
        event = make_rest_event()
        event["event_type"] = "a" * 257
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 422, (
            f"Expected HTTP 422 for event_type exceeding 256 chars, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_publish_message_exceeding_max_size_returns_422(self, http_client):
        """
        A message encoding to more than 65536 JSON bytes must return HTTP 422.
        """
        event = make_rest_event()
        event["message"] = message_of_size(MAX_EVENT_MESSAGE_BYTES + 1)
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 422, (
            f"Expected HTTP 422 for a message over 65536 JSON bytes, "
            f"but got {response.status_code}. Body: {response.text[:500]}"
        )

    def test_publish_message_at_max_size_returns_200(self, http_client):
        """
        A message of exactly 65536 JSON bytes must be published: the limit is
        inclusive.
        """
        event = make_rest_event()
        event["message"] = message_of_size(MAX_EVENT_MESSAGE_BYTES)
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 200, (
            f"Expected HTTP 200 for a message of exactly 65536 JSON bytes, "
            f"but got {response.status_code}. Body: {response.text[:500]}"
        )
        assert response.json()["result"]["success"] is True

    def test_publish_occurred_at_exceeding_max_length_returns_422(self, http_client):
        """
        occurred_at longer than 64 characters must return HTTP 422, even when it
        matches the timestamp pattern (which allows any number of fractional
        digits, so the length is its only bound).
        """
        event = make_rest_event()
        event["occurred_at"] = "2024-01-15T10:30:00." + "0" * 60 + "Z"
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 422, (
            f"Expected HTTP 422 for occurred_at exceeding 64 chars, "
            f"but got {response.status_code}. Body: {response.text[:500]}"
        )

    def test_publish_resource_id_exceeding_max_length_returns_422(self, http_client):
        """
        resource.resource_id longer than 2048 characters must return HTTP 422.
        """
        event = make_rest_event()
        event["resource"]["resource_id"] = "a" * (MAX_RESOURCE_ID_LENGTH + 1)
        response = http_client.post("/api/v1beta/events", json={"event": event})
        assert response.status_code == 422, (
            f"Expected HTTP 422 for resource_id exceeding 2048 chars, "
            f"but got {response.status_code}. Body: {response.text[:500]}"
        )

    def test_publish_missing_body_returns_422(self, http_client):
        """
        POST /api/v1beta/events with no body must return HTTP 422.
        """
        response = http_client.post(
            "/api/v1beta/events",
            content="",
            headers={"Content-Type": "application/json"},
        )
        assert response.status_code == 422, (
            f"Expected HTTP 422 for empty body, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_publish_invalid_json_returns_422(self, http_client):
        """
        POST /api/v1beta/events with a non-JSON body must return HTTP 422.
        """
        response = http_client.post(
            "/api/v1beta/events",
            content="not json at all",
            headers={"Content-Type": "application/json"},
        )
        assert response.status_code == 422, (
            f"Expected HTTP 422 for invalid JSON body, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    @pytest.mark.skipif(not AUTH_REQUIRED, reason="AUTH_REQUIRED not set")
    def test_publish_without_auth_returns_401(self, rest_base_url):
        """
        POST /api/v1beta/events without an Authorization header must return
        HTTP 401 when the service has authentication enabled.
        """
        import httpx

        response = httpx.post(
            f"{rest_base_url}/api/v1beta/events",
            json={"event": make_rest_event()},
        )
        assert response.status_code == 401, (
            f"Expected HTTP 401 when no Authorization header is sent, "
            f"but got {response.status_code}. Body: {response.text}"
        )
