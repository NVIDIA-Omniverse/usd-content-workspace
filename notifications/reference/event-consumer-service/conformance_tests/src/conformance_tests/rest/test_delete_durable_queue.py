# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REST conformance tests for DELETE /api/v1beta/queues/durable (DeleteDurableQueue).

Contract under test:
  DELETE /api/v1beta/queues/durable?queue_id=<id>

Expected behaviour:
  - Returns HTTP 200 with {"success": true} for an existing queue.
  - Returns HTTP 404 for a non-existent queue_id.
  - Returns HTTP 422 when the queue_id query parameter is missing.
  - Returns HTTP 401 when auth is required and no token is sent (requires_auth).
"""

import httpx
import pytest

from conformance_tests.conftest import AUTH_REQUIRED

pytestmark = pytest.mark.rest

_NON_EXISTENT_QUEUE_ID = "00000000-0000-0000-0000-000000000000"


class TestDeleteDurableQueue:
    def _create_queue(self, http_client) -> str:
        """Helper: create a queue and return its queue_id."""
        resp = http_client.post(
            "/api/v1beta/queues/durable",
            json={
                "filter_groups": [
                    {"event_type": "conformance.delete.test", "filters": []}
                ]
            },
        )
        assert resp.status_code == 200, f"Failed to create queue: {resp.text}"
        queue_id = resp.json().get("queue_id")
        assert queue_id, "CreateDurableQueue returned empty queue_id."
        return queue_id

    def test_delete_existing_queue_returns_200_with_success_true(self, http_client):
        """
        DELETE /api/v1beta/queues/durable?queue_id=<existing> must return
        HTTP 200 and {"success": true}.
        """
        queue_id = self._create_queue(http_client)
        response = http_client.delete(f"/api/v1beta/queues/durable?queue_id={queue_id}")

        assert response.status_code == 200, (
            f"Expected HTTP 200 for deleting existing queue '{queue_id}', "
            f"but got {response.status_code}. Body: {response.text}"
        )
        body = response.json()
        assert (
            body.get("success") is True
        ), f'Expected {{"success": true}} but got: {body}'

    def test_delete_nonexistent_queue_returns_404(self, http_client):
        """
        DELETE /api/v1beta/queues/durable?queue_id=<unknown> must return HTTP 404.
        """
        response = http_client.delete(
            f"/api/v1beta/queues/durable?queue_id={_NON_EXISTENT_QUEUE_ID}"
        )
        assert response.status_code == 404, (
            f"Expected HTTP 404 when deleting non-existent queue "
            f"'{_NON_EXISTENT_QUEUE_ID}', but got {response.status_code}. "
            f"Body: {response.text}"
        )

    def test_delete_missing_queue_id_param_returns_422(self, http_client):
        """
        DELETE /api/v1beta/queues/durable without a queue_id query parameter
        must return HTTP 422 (Unprocessable Entity).
        """
        response = http_client.delete("/api/v1beta/queues/durable")
        assert response.status_code == 422, (
            f"Expected HTTP 422 when queue_id param is missing, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_delete_same_queue_twice_returns_404_on_second(self, http_client):
        """
        Deleting the same queue twice must return 404 on the second attempt.
        """
        queue_id = self._create_queue(http_client)

        first = http_client.delete(f"/api/v1beta/queues/durable?queue_id={queue_id}")
        assert first.status_code == 200, f"First delete failed: {first.text}"

        second = http_client.delete(f"/api/v1beta/queues/durable?queue_id={queue_id}")
        assert second.status_code == 404, (
            f"Expected HTTP 404 on second delete of queue '{queue_id}', "
            f"but got {second.status_code}. Body: {second.text}"
        )

    @pytest.mark.skipif(not AUTH_REQUIRED, reason="AUTH_REQUIRED not set")
    def test_delete_without_auth_returns_401(self, rest_base_url):
        """DELETE /api/v1beta/queues/durable without Authorization header must return 401."""
        response = httpx.delete(
            f"{rest_base_url}/api/v1beta/queues/durable?queue_id=some-id"
        )
        assert response.status_code == 401, (
            f"Expected HTTP 401 when no Authorization header is sent, "
            f"but got {response.status_code}. Body: {response.text}"
        )
