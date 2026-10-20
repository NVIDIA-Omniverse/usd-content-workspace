# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Integration tests for the 'publish event' endpoint."""

import functools

import pytest

from app.lib.limits import MAX_EVENT_MESSAGE_BYTES
from app.lib.rabbitmq import publisher

# Chunk sizes for the bodies posted from a generator below. What the chunking
# buys these rows is the request *shape*, not the ASGI message count: httpx
# sends an iterator body as `transfer-encoding: chunked` with no
# `content-length`, which is what routes the request through the middleware's
# counting/replay branch instead of the declared-length fast path. The message
# count is out of these rows' hands -- `TestClient`'s transport calls
# `request.read()` on the outbound request, flattening a generator body into a
# single `http.request` message however it was chunked here (the transport's
# per-chunk branch is `# pragma: no cover` and unreachable). Ordering across
# several replayed messages is pinned by
# `test_replays_the_drained_messages_in_order` in
# `tests/app/rest/middleware/test_body_size_limit.py`.

# The chunk size for the streamed oversize body.
STREAM_CHUNK_BYTES = 1024 * 1024

# The chunk size for the accepted streamed body.
REPLAY_CHUNK_BYTES = 16
UNDER_CAP_BYTES = 512


@pytest.mark.asyncio
class TestPostPublishEvent:
    """Tests for POST /events endpoint."""

    @pytest.fixture
    def make_request(self, client):
        """Fixture to make POST requests to the 'publish endpoint'."""

        def _make_request(event_data, headers=None):
            return client.post(
                "/api/v1beta/events", json={"event": event_data}, headers=headers
            )

        return _make_request

    @pytest.fixture
    def post_body(self, post_raw_body):
        """Fixture to POST a raw body to the 'publish endpoint'."""
        return functools.partial(post_raw_body, "/api/v1beta/events")

    @pytest.fixture
    def mock_publish_success(self, mocker, successful_result):
        """Fixture that mocks publisher.publish with a successful result."""
        return mocker.patch.object(publisher, "publish", return_value=successful_result)

    async def test_publishes_event_successfully(
        self, sample_event, auth_headers, mock_publish_success, make_request
    ):
        """Test successful event publishing."""
        response = make_request(sample_event, auth_headers)

        assert response.status_code == 200
        result = response.json()
        assert result["result"]["success"] is True
        assert result["result"]["failure_reason"] is None

    async def test_publishes_event_without_resource(
        self,
        sample_event_without_resource,
        auth_headers,
        mock_publish_success,
        make_request,
    ):
        """Test publishing event without resource."""
        response = make_request(sample_event_without_resource, auth_headers)

        assert response.status_code == 200
        result = response.json()
        assert result["result"]["success"] is True

    async def test_returns_correct_response_structure(
        self, sample_event, auth_headers, mock_publish_success, make_request
    ):
        """Test response has the correct structure."""
        response = make_request(sample_event, auth_headers)

        assert response.status_code == 200
        result = response.json()
        assert "result" in result
        assert "event" in result["result"]
        assert "success" in result["result"]
        assert "failure_reason" in result["result"]

    async def test_calls_publisher_with_omni_event(
        self, sample_event, auth_headers, mock_publish_success, make_request
    ):
        """Test that publisher is called correctly."""
        make_request(sample_event, auth_headers)

        mock_publish_success.assert_called_once()
        call_args = mock_publish_success.call_args[0][0]
        assert call_args.event_type == "test.event"

    async def test_returns_401_for_missing_auth(self, sample_event, make_request):
        """Test missing authorization returns 401."""
        response = make_request(sample_event)
        assert response.status_code == 401

    @pytest.mark.parametrize(
        "event_data,expected_error",
        [
            (
                {"message": {"key": "value"}, "occurred_at": "2024-01-01T00:00:00Z"},
                "event_type",
            ),
            (
                {"event_type": "test.event", "occurred_at": "2024-01-01T00:00:00Z"},
                "message",
            ),
            ({"event_type": "test.event", "message": {"key": "value"}}, "occurred_at"),
            (
                {
                    "event_type": "test.event",
                    "message": {"key": "value"},
                    "occurred_at": "not-a-valid-datetime",
                },
                "occurred_at",
            ),
        ],
        ids=[
            "missing_event_type",
            "missing_message",
            "missing_occurred_at",
            "invalid_occurred_at_format",
        ],
    )
    async def test_returns_422_for_invalid_event(
        self, auth_headers, make_request, event_data, expected_error
    ):
        """Test that invalid events return 422."""
        response = make_request(event_data, auth_headers)
        assert response.status_code == 422

    async def test_handles_failed_publishing(
        self, sample_event, auth_headers, failed_result, mocker, make_request
    ):
        """Test handling of failed publishing."""
        mocker.patch.object(publisher, "publish", return_value=failed_result)

        response = make_request(sample_event, auth_headers)

        assert response.status_code == 200
        result = response.json()
        assert result["result"]["success"] is False
        assert result["result"]["failure_reason"] == "Test failure"

    async def test_uses_undetermined_when_auth_disabled(
        self, sample_event, auth_headers, mock_publish_success, mocker, make_request
    ):
        """Test that 'undetermined' is used as principal when auth is disabled."""
        from notification_common.auth import principal_id

        # Override the autouse fixture to return None (auth disabled)
        mocker.patch.object(principal_id, "principal_id", return_value=None)

        make_request(sample_event, auth_headers)

        mock_publish_success.assert_called_once()
        call_args = mock_publish_success.call_args[0][0]
        assert call_args.principal_identity == "undetermined"

    class TestWhenTheMessageExceedsTheLimit:
        """A message over ``MAX_EVENT_MESSAGE_BYTES`` is refused by validation."""

        @pytest.fixture
        def response(self, sample_event, message_of_size, auth_headers, make_request):
            event = {
                **sample_event,
                "message": message_of_size(MAX_EVENT_MESSAGE_BYTES + 1),
            }
            return make_request(event, auth_headers)

        async def test_returns_422(self, response):
            assert response.status_code == 422

        async def test_points_at_the_message(self, response):
            assert response.json()["detail"][0]["loc"] == ["body", "event", "message"]

        async def test_does_not_publish(self, response):
            publisher.publish.assert_not_called()

    class TestWhenTheBodyExceedsTheCap:
        """The body cap is enforced on the real app, ahead of everything else.

        The middleware unit tests pin the refusal itself; these rows pin that
        it is installed on this endpoint, that it runs before authentication
        and before the publisher is reached, and that it applies to a chunked
        body as well as a declared one.
        """

        @pytest.fixture
        def oversize_body(self, sized_json_body, max_request_bytes):
            """Return a valid publish body one byte over the cap."""
            return sized_json_body("event", max_request_bytes + 1)

        async def test_returns_413(self, oversize_body, auth_headers, post_body):
            response = post_body(oversize_body, auth_headers)

            assert response.status_code == 413

        async def test_does_not_reach_the_publisher(
            self, oversize_body, auth_headers, mock_publish_success, post_body
        ):
            post_body(oversize_body, auth_headers)

            mock_publish_success.assert_not_called()

        async def test_is_refused_before_authentication(self, oversize_body, post_body):
            # The cap is a resource bound rather than an authorization
            # control, so an unauthenticated oversize request is refused with
            # 413 -- not the 401 the same request under the cap would get.
            response = post_body(oversize_body)

            assert response.status_code == 413

        async def test_returns_413_for_a_chunked_body(
            self, oversize_body, auth_headers, post_body
        ):
            # A chunked body carries no content-length, so there is no
            # declaration for the cap to trust -- it has to count the body it
            # actually receives.
            def chunks():
                for start in range(0, len(oversize_body), STREAM_CHUNK_BYTES):
                    end = start + STREAM_CHUNK_BYTES
                    yield oversize_body[start:end]

            response = post_body(chunks(), auth_headers)

            assert response.status_code == 413

    class TestWhenTheBodyIsExactlyTheCap:
        """The boundary is inclusive: a body of exactly the cap is published."""

        @pytest.fixture
        def at_cap_body(self, sized_json_body, max_request_bytes):
            """Return a valid publish body of exactly the cap."""
            return sized_json_body("event", max_request_bytes)

        async def test_publishes_successfully(
            self, at_cap_body, auth_headers, mock_publish_success, post_body
        ):
            response = post_body(at_cap_body, auth_headers)

            assert response.status_code == 200

    class TestWhenTheBodyIsChunkedAndUnderTheCap:
        """An accepted chunked body still reaches the application intact.

        Every other row here declares a content-length, which the cap trusts
        and passes straight through; only a body sent without a trusted
        content-length takes the branch that drains the body and replays it to
        the application. This row pins that branch end to end: the body
        survives the drain and replay and is published (200), rather than
        arriving mangled and surfacing as a 422. It does not pin replay
        *ordering* -- `TestClient` delivers this body as a single
        `http.request` message, so there is no order to get wrong here; the
        middleware unit tests pin that.
        """

        async def test_publishes_successfully(
            self, sized_json_body, auth_headers, mock_publish_success, post_body
        ):
            body = sized_json_body("event", UNDER_CAP_BYTES)

            def chunks():
                for start in range(0, len(body), REPLAY_CHUNK_BYTES):
                    end = start + REPLAY_CHUNK_BYTES
                    yield body[start:end]

            response = post_body(chunks(), auth_headers)

            assert response.status_code == 200
