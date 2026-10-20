# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REST conformance tests for GET /api/v1beta/events/stream (non-durable event stream).

Contract under test:
  GET /api/v1beta/events/stream
    Query params:
      filter_groups (optional) - URL-encoded JSON FilterGroup array
      reconnect_token (optional) - token to reconnect to an existing queue
      previous_filter_groups (optional) - previous filters when changing filters

Expected behaviour:
  - Returns HTTP 200 with Content-Type: text/event-stream.
  - The first SSE event sent is named "connected" and contains a
    "message" field that includes "Connection established".
  - The first SSE event also includes a "reconnect_token" field.
  - Returns HTTP 404 when an invalid reconnect_token is provided.
  - Returns HTTP 422 when filter_groups contains an invalid resource_id.
"""

import json
import urllib.parse

import pytest

from conformance_tests.conftest import read_sse_first_event

pytestmark = pytest.mark.rest

_NON_EXISTENT_TOKEN = "00000000-0000-0000-0000-000000000000"


class TestStreamEvents:
    def test_stream_returns_200_with_sse_content_type(self, valid_filter_groups_json):
        """
        GET /api/v1beta/events/stream must return HTTP 200 with
        Content-Type starting with 'text/event-stream'.
        """
        path = f"/api/v1beta/events/stream?filter_groups={valid_filter_groups_json}"
        status, content_type, _ = read_sse_first_event(path)

        assert status == 200, (
            f"Expected HTTP 200 from GET /api/v1beta/events/stream, "
            f"but got {status}."
        )
        assert content_type.startswith(
            "text/event-stream"
        ), f"Expected Content-Type 'text/event-stream' but got '{content_type}'."

    def test_stream_sends_connected_event_first(self, valid_filter_groups_json):
        """
        The first SSE event on a new non-durable stream must be named 'connected'
        and include a message containing 'Connection established'.
        """
        path = f"/api/v1beta/events/stream?filter_groups={valid_filter_groups_json}"
        status, _, first_event = read_sse_first_event(path)

        assert status == 200
        assert first_event is not None, (
            "Expected to receive at least one SSE event after connecting, "
            "but received none within the timeout."
        )
        assert first_event.get("event") == "connected", (
            f"Expected the first SSE event to be named 'connected', "
            f"but got event='{first_event.get('event')}'. "
            f"Full event: {first_event}"
        )
        message = str(first_event.get("data", {}).get("message", ""))
        assert "Connection established" in message, (
            f"Expected the 'connected' event data to contain 'Connection established' "
            f"in the message field, but got: '{message}'. Full event: {first_event}"
        )

    def test_stream_connected_event_includes_reconnect_token(
        self, valid_filter_groups_json
    ):
        """
        The 'connected' SSE event must include a non-empty 'reconnect_token'
        field so clients can reconnect after a disconnect.
        """
        path = f"/api/v1beta/events/stream?filter_groups={valid_filter_groups_json}"
        status, _, first_event = read_sse_first_event(path)

        assert status == 200
        assert first_event is not None, (
            "Expected to receive the 'connected' SSE event so its "
            "'reconnect_token' could be verified, but received none within "
            "the timeout. A compliant service must send the acknowledgement "
            "immediately after connection."
        )

        data = first_event.get("data", {})
        assert data.get("reconnect_token"), (
            f"Expected the 'connected' SSE event to include a non-empty "
            f"'reconnect_token', but got: {data}"
        )

    def test_stream_without_filters_or_token_returns_error(self):
        """
        GET /api/v1beta/events/stream with neither filter_groups nor reconnect_token
        must return an error (400 or 422). Subscribing to all events without any
        filter criteria is not supported.
        """
        status, _, _ = read_sse_first_event("/api/v1beta/events/stream")
        assert status in (400, 422), (
            f"Expected HTTP 400 or 422 when no filter_groups or reconnect_token "
            f"are provided, but got {status}."
        )

    def test_stream_with_invalid_reconnect_token_returns_404(self):
        """
        GET /api/v1beta/events/stream with a reconnect_token that does not
        correspond to any known queue must return HTTP 404.
        """
        status, _, body = read_sse_first_event(
            f"/api/v1beta/events/stream?reconnect_token={_NON_EXISTENT_TOKEN}"
        )
        assert status == 404, (
            f"Expected HTTP 404 for unknown reconnect_token '{_NON_EXISTENT_TOKEN}', "
            f"but got {status}. Body: {body}"
        )

    def test_stream_with_invalid_storage_resource_id_returns_422(self):
        """
        filter_groups with a resource_id that does not end with '/' (required for
        storage event subscriptions) must return HTTP 422.
        """
        invalid_filters = [
            {
                "event_type": "omni.storage.dir_created",
                "filters": [
                    {
                        "filter_type": "starts_with_greedy",
                        "resource_id": "https://blob.core.windows.net/some-path/no-trailing-slash",
                    }
                ],
            }
        ]

        encoded = urllib.parse.quote(json.dumps(invalid_filters))
        status, _, body = read_sse_first_event(
            f"/api/v1beta/events/stream?filter_groups={encoded}"
        )
        assert status == 422, (
            f"Expected HTTP 422 for invalid storage resource_id, "
            f"but got {status}. Body: {body}"
        )
        # The exact error message is implementation-defined; just verify 422 is returned.
