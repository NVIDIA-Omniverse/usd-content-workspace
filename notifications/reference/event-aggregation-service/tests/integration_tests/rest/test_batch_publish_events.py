# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Integration tests for the batch publish events endpoint."""

import functools

import pytest

from app.lib.limits import MAX_EVENT_MESSAGE_BYTES
from app.lib.rabbitmq import publisher


@pytest.mark.asyncio
class TestPostBatchPublishEvents:
    """Tests for POST /events/batch endpoint."""

    @pytest.fixture
    def make_request(self, client):
        """Fixture to make POST requests to batch publish endpoint."""

        def _make_request(events, headers=None):
            return client.post(
                "/api/v1beta/events/batch", json={"events": events}, headers=headers
            )

        return _make_request

    @pytest.fixture
    def post_body(self, post_raw_body):
        """Fixture to POST a raw body to the batch publish endpoint."""
        return functools.partial(post_raw_body, "/api/v1beta/events/batch")

    @pytest.fixture
    def mock_batch_publish_success(self, mocker, successful_result):
        """Fixture that mocks batch_publish with successful results."""

        def _mock(count=1):
            return mocker.patch.object(
                publisher,
                "batch_publish",
                return_value=[successful_result] * count,
            )

        return _mock

    async def test_publishes_batch_successfully(
        self, sample_event, auth_headers, mock_batch_publish_success, make_request
    ):
        """Test successful batch publishing."""
        mock_batch_publish_success(2)

        response = make_request([sample_event, sample_event], auth_headers)

        assert response.status_code == 200
        result = response.json()
        assert len(result["results"]) == 2
        assert all(r["success"] is True for r in result["results"])

    async def test_publishes_single_event_in_batch(
        self, sample_event, auth_headers, mock_batch_publish_success, make_request
    ):
        """Test batch with single event."""
        mock_batch_publish_success(1)

        response = make_request([sample_event], auth_headers)

        assert response.status_code == 200
        result = response.json()
        assert len(result["results"]) == 1

    async def test_returns_correct_response_structure(
        self, sample_event, auth_headers, mock_batch_publish_success, make_request
    ):
        """Test response has the correct structure."""
        mock_batch_publish_success(1)

        response = make_request([sample_event], auth_headers)

        assert response.status_code == 200
        result = response.json()
        assert "results" in result
        assert isinstance(result["results"], list)
        for r in result["results"]:
            assert "event" in r
            assert "success" in r
            assert "failure_reason" in r

    async def test_calls_batch_publisher_correctly(
        self, sample_event, auth_headers, mock_batch_publish_success, make_request
    ):
        """Test that batch_publish is called correctly."""
        mock = mock_batch_publish_success(2)

        make_request([sample_event, sample_event], auth_headers)

        mock.assert_called_once()
        call_args = mock.call_args[0][0]
        assert len(call_args) == 2

    async def test_handles_partial_failures(
        self,
        sample_event,
        auth_headers,
        successful_result,
        failed_result,
        mocker,
        make_request,
    ):
        """Test handling partial failures in batch."""
        mocker.patch.object(
            publisher,
            "batch_publish",
            return_value=[successful_result, failed_result],
        )

        response = make_request([sample_event, sample_event], auth_headers)

        assert response.status_code == 200
        result = response.json()
        assert len(result["results"]) == 2
        assert result["results"][0]["success"] is True
        assert result["results"][1]["success"] is False
        assert result["results"][1]["failure_reason"] == "Test failure"

    async def test_returns_401_for_missing_auth(self, sample_event, make_request):
        """Test missing authorization returns 401."""
        response = make_request([sample_event])
        assert response.status_code == 401

    async def test_returns_400_for_empty_events_list(self, auth_headers, make_request):
        """Test empty events list returns 400."""
        response = make_request([], auth_headers)

        assert response.status_code == 400
        assert "at least one event" in response.json()["detail"]

    async def test_returns_422_for_invalid_event_in_batch(
        self, auth_headers, make_request
    ):
        """Test invalid event in batch returns 422."""
        invalid_event = {"event_type": "test.event"}

        response = make_request([invalid_event], auth_headers)
        assert response.status_code == 422

    async def test_handles_multiple_events_with_different_types(
        self, auth_headers, mock_batch_publish_success, make_request
    ):
        """Test batch with different event types."""
        event1 = {
            "event_type": "test.event.one",
            "message": {"key": "value1"},
            "occurred_at": "2024-01-01T00:00:00Z",
        }
        event2 = {
            "event_type": "test.event.two",
            "message": {"key": "value2"},
            "occurred_at": "2024-01-01T00:00:00Z",
        }

        mock_batch_publish_success(2)

        response = make_request([event1, event2], auth_headers)

        assert response.status_code == 200
        result = response.json()
        assert len(result["results"]) == 2

    async def test_uses_undetermined_when_auth_disabled(
        self,
        sample_event,
        auth_headers,
        mock_batch_publish_success,
        mocker,
        make_request,
    ):
        """Test that 'undetermined' is used as principal when auth is disabled."""
        from notification_common.auth import principal_id

        # Override the autouse fixture to return None (auth disabled)
        mocker.patch.object(principal_id, "principal_id", return_value=None)
        mock = mock_batch_publish_success(1)

        make_request([sample_event], auth_headers)

        mock.assert_called_once()
        call_args = mock.call_args[0][0]
        assert call_args[0].principal_identity == "undetermined"

    class TestWhenAnEventMessageExceedsTheLimit:
        """One oversize message refuses the whole batch; nothing is published."""

        @pytest.fixture
        def response(self, sample_event, message_of_size, auth_headers, make_request):
            oversize = {
                **sample_event,
                "message": message_of_size(MAX_EVENT_MESSAGE_BYTES + 1),
            }
            return make_request([sample_event, oversize, sample_event], auth_headers)

        async def test_returns_422(self, response):
            assert response.status_code == 422

        async def test_points_at_the_offending_message(self, response):
            assert response.json()["detail"][0]["loc"] == [
                "body",
                "events",
                1,
                "message",
            ]

        async def test_does_not_publish(self, response):
            publisher.batch_publish.assert_not_called()

    class TestWhenTheBodyExceedsTheCap:
        """The batch endpoint is capped as well, and by the same middleware.

        A batch is the shape most likely to grow past the cap, so it carries
        its own row for the refusal. The rest of the cap's behavior -- that it
        runs before authentication and before the publisher is reached -- is
        app-wide and is pinned once, on the single-event endpoint.
        """

        @pytest.fixture
        def oversize_body(self, sized_json_body, max_request_bytes):
            """Return a valid batch body one byte over the cap."""
            return sized_json_body("events", max_request_bytes + 1)

        async def test_returns_413(self, oversize_body, auth_headers, post_body):
            response = post_body(oversize_body, auth_headers)

            assert response.status_code == 413

    class TestWhenTheBodyIsExactlyTheCap:
        """The boundary is inclusive: a batch of exactly the cap is published."""

        async def test_publishes_successfully(
            self,
            sized_json_body,
            max_request_bytes,
            auth_headers,
            mock_batch_publish_success,
            post_body,
        ):
            mock_batch_publish_success(1)

            response = post_body(
                sized_json_body("events", max_request_bytes), auth_headers
            )

            assert response.status_code == 200
