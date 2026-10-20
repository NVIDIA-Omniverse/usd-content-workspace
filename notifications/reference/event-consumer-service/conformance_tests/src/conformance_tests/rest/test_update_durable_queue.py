# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REST conformance tests for PATCH /api/v1beta/queues/durable (UpdateDurableQueue).

Contract under test:
  PATCH /api/v1beta/queues/durable
    Body: {
      "queue_id": "<id>",
      "current_filter_groups": [ FilterGroup... ],
      "new_filter_groups": [ FilterGroup... ]
    }

Expected behaviour:
  - Returns HTTP 200 with {"success": true} for an existing queue.
  - Returns HTTP 404 for a non-existent queue_id.
  - Returns HTTP 422 when queue_id, current_filter_groups, or new_filter_groups
    is missing from the body.
  - Returns HTTP 422 for invalid JSON body.
  - Returns HTTP 401 when auth is required and no token is sent (requires_auth).
"""

import httpx
import pytest

from conformance_tests.conftest import AUTH_REQUIRED

pytestmark = pytest.mark.rest

_NON_EXISTENT_QUEUE_ID = "00000000-0000-0000-0000-000000000000"
_CURRENT_FILTER = {
    "event_type": "conformance.original.event",
    "filters": [{"filter_type": "eq", "resource_id": "resource/"}],
}
_NEW_FILTER = {
    "event_type": "conformance.updated.event",
    "filters": [{"filter_type": "starts_with_greedy", "resource_id": "updated/"}],
}


class TestPatchUpdateDurableQueue:
    def _create_queue(self, http_client, filter_groups=None) -> str:
        """Helper: create a queue and return its queue_id."""
        body = {
            "filter_groups": (
                filter_groups if filter_groups is not None else [_CURRENT_FILTER]
            )
        }
        resp = http_client.post("/api/v1beta/queues/durable", json=body)
        assert resp.status_code == 200, f"Failed to create queue: {resp.text}"
        queue_id = resp.json().get("queue_id")
        assert queue_id, "CreateDurableQueue returned empty queue_id."
        return queue_id

    def _delete_queue(self, http_client, queue_id: str):
        """Helper: delete a queue (best-effort cleanup)."""
        http_client.delete(f"/api/v1beta/queues/durable?queue_id={queue_id}")

    def test_update_existing_queue_returns_200_with_success_true(self, http_client):
        """
        PATCH /api/v1beta/queues/durable with a valid body for an existing queue
        must return HTTP 200 and {"success": true}.
        """
        queue_id = self._create_queue(http_client)
        try:
            response = http_client.patch(
                "/api/v1beta/queues/durable",
                json={
                    "queue_id": queue_id,
                    "current_filter_groups": [_CURRENT_FILTER],
                    "new_filter_groups": [_NEW_FILTER],
                },
            )
            assert response.status_code == 200, (
                f"Expected HTTP 200 for updating existing queue '{queue_id}', "
                f"but got {response.status_code}. Body: {response.text}"
            )
            body = response.json()
            assert (
                body.get("success") is True
            ), f'Expected {{"success": true}} but got: {body}'
        finally:
            self._delete_queue(http_client, queue_id)

    def test_update_nonexistent_queue_returns_404(self, http_client):
        """
        PATCH /api/v1beta/queues/durable with a non-existent queue_id must
        return HTTP 404.
        """
        response = http_client.patch(
            "/api/v1beta/queues/durable",
            json={
                "queue_id": _NON_EXISTENT_QUEUE_ID,
                "current_filter_groups": [_CURRENT_FILTER],
                "new_filter_groups": [_NEW_FILTER],
            },
        )
        assert response.status_code == 404, (
            f"Expected HTTP 404 for non-existent queue '{_NON_EXISTENT_QUEUE_ID}', "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_update_with_missing_queue_id_returns_422(self, http_client):
        """
        PATCH /api/v1beta/queues/durable without 'queue_id' in the body must
        return HTTP 422.
        """
        response = http_client.patch(
            "/api/v1beta/queues/durable",
            json={
                "current_filter_groups": [_CURRENT_FILTER],
                "new_filter_groups": [_NEW_FILTER],
            },
        )
        assert response.status_code == 422, (
            f"Expected HTTP 422 for missing queue_id, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_update_with_missing_current_filter_groups_returns_422(self, http_client):
        """
        PATCH /api/v1beta/queues/durable without 'current_filter_groups' must
        return HTTP 422.
        """
        response = http_client.patch(
            "/api/v1beta/queues/durable",
            json={
                "queue_id": "some-queue-id",
                "new_filter_groups": [_NEW_FILTER],
            },
        )
        assert response.status_code == 422, (
            f"Expected HTTP 422 for missing current_filter_groups, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_update_with_missing_new_filter_groups_returns_422(self, http_client):
        """
        PATCH /api/v1beta/queues/durable without 'new_filter_groups' must
        return HTTP 422.
        """
        response = http_client.patch(
            "/api/v1beta/queues/durable",
            json={
                "queue_id": "some-queue-id",
                "current_filter_groups": [_CURRENT_FILTER],
            },
        )
        assert response.status_code == 422, (
            f"Expected HTTP 422 for missing new_filter_groups, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    def test_update_with_invalid_json_returns_422(self, http_client):
        """PATCH /api/v1beta/queues/durable with non-JSON body must return HTTP 422."""
        response = http_client.patch(
            "/api/v1beta/queues/durable",
            content="not json at all",
            headers={"Content-Type": "application/json"},
        )
        assert response.status_code == 422, (
            f"Expected HTTP 422 for invalid JSON, "
            f"but got {response.status_code}. Body: {response.text}"
        )

    @pytest.mark.skipif(not AUTH_REQUIRED, reason="AUTH_REQUIRED not set")
    def test_update_without_auth_returns_401(self, rest_base_url):
        """PATCH /api/v1beta/queues/durable without Authorization header must return 401."""
        response = httpx.patch(
            f"{rest_base_url}/api/v1beta/queues/durable",
            json={
                "queue_id": "some-id",
                "current_filter_groups": [_CURRENT_FILTER],
                "new_filter_groups": [_NEW_FILTER],
            },
        )
        assert response.status_code == 401, (
            f"Expected HTTP 401 when no Authorization header is sent, "
            f"but got {response.status_code}. Body: {response.text}"
        )
