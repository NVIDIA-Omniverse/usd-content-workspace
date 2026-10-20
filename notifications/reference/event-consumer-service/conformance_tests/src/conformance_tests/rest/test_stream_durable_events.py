# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REST conformance tests for GET /api/v1beta/events/stream-durable
(durable event stream).

Contract under test:
  GET /api/v1beta/events/stream-durable?queue_id=<id>

Expected behaviour:
  - Returns HTTP 200 with Content-Type: text/event-stream when queue exists.
  - The first SSE event is named "connected" with a message containing
    "Connection established".
  - Returns HTTP 404 for a non-existent queue_id.
  - Returns HTTP 422 when the queue_id query parameter is missing.
"""

import httpx
import pytest

from conformance_tests.conftest import (
    AUTH_REQUIRED,
    read_sse_first_event,
)

pytestmark = pytest.mark.rest

_NON_EXISTENT_QUEUE_ID = "00000000-0000-0000-0000-000000000000"


class TestStreamDurableEvents:
    def _create_queue(self, http_client) -> str:
        """Helper: create a durable queue and return its queue_id."""
        resp = http_client.post(
            "/api/v1beta/queues/durable",
            json={
                "filter_groups": [
                    {"event_type": "conformance.durable.stream.test", "filters": []}
                ]
            },
        )
        assert resp.status_code == 200, f"Failed to create queue: {resp.text}"
        queue_id = resp.json().get("queue_id")
        assert queue_id, "CreateDurableQueue returned empty queue_id."
        return queue_id

    def _delete_queue(self, http_client, queue_id: str):
        """Helper: delete a queue (best-effort cleanup)."""
        http_client.delete(f"/api/v1beta/queues/durable?queue_id={queue_id}")

    def test_stream_durable_returns_200_with_sse_content_type(self, http_client):
        """
        GET /api/v1beta/events/stream-durable?queue_id=<existing> must return
        HTTP 200 with Content-Type starting with 'text/event-stream'.
        """
        queue_id = self._create_queue(http_client)
        try:
            status, content_type, _ = read_sse_first_event(
                f"/api/v1beta/events/stream-durable?queue_id={queue_id}"
            )
            assert status == 200, (
                f"Expected HTTP 200 for durable stream on queue '{queue_id}', "
                f"but got {status}."
            )
            assert content_type.startswith(
                "text/event-stream"
            ), f"Expected Content-Type 'text/event-stream' but got '{content_type}'."
        finally:
            self._delete_queue(http_client, queue_id)

    def test_stream_durable_sends_connected_event_first(self, http_client):
        """
        The first SSE event on a durable stream must be named 'connected'
        and contain a message with 'Connection established'.
        """
        queue_id = self._create_queue(http_client)
        try:
            status, _, first_event = read_sse_first_event(
                f"/api/v1beta/events/stream-durable?queue_id={queue_id}"
            )
            assert status == 200

            assert first_event is not None, (
                f"Expected to receive at least one SSE event on durable stream "
                f"for queue '{queue_id}', but received none within the timeout."
            )
            assert first_event.get("event") == "connected", (
                f"Expected the first SSE event to be named 'connected', "
                f"but got event='{first_event.get('event')}'. "
                f"Full event: {first_event}"
            )
            message = str(first_event.get("data", {}).get("message", ""))
            assert "Connection established" in message, (
                f"Expected the 'connected' event data to contain "
                f"'Connection established' in the message field, "
                f"but got: '{message}'. Full event: {first_event}"
            )
        finally:
            self._delete_queue(http_client, queue_id)

    def test_stream_durable_nonexistent_queue_returns_404(self, http_client):
        """
        GET /api/v1beta/events/stream-durable?queue_id=<unknown> must return
        HTTP 404 immediately (not a streaming SSE error).
        """
        status, _, body = read_sse_first_event(
            f"/api/v1beta/events/stream-durable?queue_id={_NON_EXISTENT_QUEUE_ID}"
        )
        assert status == 404, (
            f"Expected HTTP 404 for non-existent queue '{_NON_EXISTENT_QUEUE_ID}', "
            f"but got {status}. Body: {body}"
        )

    def test_stream_durable_missing_queue_id_returns_422(self, http_client):
        """
        GET /api/v1beta/events/stream-durable without the queue_id query param
        must return HTTP 422.
        """
        # This is a regular request (not SSE) since the service returns 422 immediately
        response = http_client.get("/api/v1beta/events/stream-durable")
        assert response.status_code == 422, (
            f"Expected HTTP 422 when queue_id param is missing, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    @pytest.mark.skipif(not AUTH_REQUIRED, reason="AUTH_REQUIRED not set")
    def test_stream_durable_without_auth_returns_401(self, http_client, rest_base_url):
        """
        GET /api/v1beta/events/stream-durable without Authorization header
        must return HTTP 401.
        """
        response = httpx.get(
            f"{rest_base_url}/api/v1beta/events/stream-durable?queue_id=some-id"
        )
        assert response.status_code == 401, (
            f"Expected HTTP 401 when no Authorization header is sent, "
            f"but got {response.status_code}. Body: {response.text}"
        )
