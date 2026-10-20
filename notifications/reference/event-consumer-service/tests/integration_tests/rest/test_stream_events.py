# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Integration tests for the stream events endpoint."""

import urllib.parse

import pytest
from notification_common.errors.errors import (
    QueueNotFoundError,
    AuthenticationError,
)

from app.lib.models.event_message import EventMessage
from app.lib.rabbitmq import NonDurableQueueConsumer
from tests.conftest import assert_non_durable_get_instance_call
from tests.integration_tests.rest.conftest import (
    stream_endpoint_test,
    parse_sse_response,
    rabbitmq_event_stream_test,
)


async def async_generator(items):
    """Helper function to create an async generator from a list of items."""
    for item in items:
        yield EventMessage(item)


@pytest.mark.asyncio
class TestGetStreamEvents:
    @pytest.fixture
    def mock_event(self):
        """Return a test event for streaming."""
        return {
            "event_type": "test_event",
            "message": {"test": "data"},
            "occurred_at": "2023-01-01T00:00:00Z",
            "published_at": "2023-01-01T00:01:00Z",
        }

    @pytest.fixture
    def stream_endpoint_url(self, valid_filter_groups_json):
        """Return the properly formatted URL for the stream events endpoint."""
        encoded_filters = urllib.parse.quote(valid_filter_groups_json)
        return f"/api/v1/events/stream?filter_groups={encoded_filters}"

    @pytest.fixture
    def stream_endpoint_url_v1beta(self, valid_filter_groups_json):
        """Return the properly formatted URL for the v1beta stream events endpoint."""
        encoded_filters = urllib.parse.quote(valid_filter_groups_json)
        return f"/api/v1beta/events/stream?filter_groups={encoded_filters}"

    @pytest.fixture
    def setup_mock_rabbitmq(self, mocker, mock_event, mocked_non_durable_consumer):
        """Set up common RabbitMQ mocks for streaming tests."""
        mocked_non_durable_consumer.queue_id = "mock-queue-id"

        def _setup(events=None):
            if events is None:
                events = [mock_event]

            mocker.patch.object(
                mocked_non_durable_consumer,
                "consume",
                return_value=async_generator(events),
            )

            return mocked_non_durable_consumer.queue

        return _setup

    # @pytest.fixture
    # def mocked_initialize_queue(self, setup_mock_rabbitmq, mock_event, mocker):
    #     mock_reconnect_queue = setup_mock_rabbitmq(events=[mock_event])
    #     return mocker.patch.object(
    #         non_durable_queue_consumer,
    #         "initialize_queue",
    #         return_value=mock_reconnect_queue,
    #     )

    @pytest.fixture
    def stream_response(
        self, client, mock_event, setup_mock_rabbitmq, stream_endpoint_url
    ):
        setup_mock_rabbitmq([mock_event])
        return client.get(
            stream_endpoint_url, headers={"Authorization": "Bearer test-token"}
        )

    async def test_returns_422_for_invalid_filter_groups(self, client):
        invalid_json = (
            '[{"event_type":"omni.storage.dir_created","filters":['
            '{"filter_type":2,"resource_id":"https://exampleaccount'
            ".blob.core.windows.net/example-container/ov-content/Users/"
            'user@example.com"}]}]'
        )
        encoded_invalid_json = urllib.parse.quote(invalid_json)

        response = client.get(
            f"/api/v1/events/stream?filter_groups={encoded_invalid_json}",
            headers={"Authorization": "Bearer test-token"},
        )

        assert response.status_code == 422
        error_detail = response.json().get("detail", "")

        assert error_detail == (
            "When consuming storage events only folders values are "
            "supported, and the resource_id value must end with a '/'"
        )

    async def test_returns_200_on_success(self, stream_response):
        assert stream_response.status_code == 200

    async def test_content_type_is_event_stream(self, stream_response):
        assert stream_response.headers.get("content-type", "").startswith(
            "text/event-stream"
        )

    async def test_streams_connection_event(self, stream_response):
        content_str = stream_response.content.decode("utf-8")
        events = parse_sse_response(content_str)

        connected_event = events[0]
        assert connected_event.get("event") == "connected"
        assert "Connection established" in str(
            connected_event.get("data", {}).get("message", "")
        )

    async def test_streams_event_from_rabbitmq(self, stream_response, mock_event):
        await rabbitmq_event_stream_test(mock_event, stream_response)

    async def test_returns_401_for_expired_auth_token(
        self, client, metadata, mock_queue_id, mocker
    ):
        mocker.patch.object(
            NonDurableQueueConsumer,
            "get_instance",
            side_effect=AuthenticationError("Token expired"),
        )

        response = client.get(
            f"/api/v1/events/stream?reconnect_token={mock_queue_id}",
            headers=metadata,
        )

        assert response.status_code == 401
        assert "Token expired" in response.json().get("detail", "")

    async def test_returns_404_for_invalid_reconnect_token(
        self,
        client,
        mocker,
        setup_mock_rabbitmq,
        metadata,
        mock_queue_id,
    ):
        setup_mock_rabbitmq()
        mocker.patch.object(
            NonDurableQueueConsumer,
            "get_instance",
            side_effect=QueueNotFoundError("test_queue"),
        )

        response = client.get(
            f"/api/v1/events/stream?reconnect_token={mock_queue_id}",
            headers=metadata,
        )

        assert response.status_code == 404

    @pytest.mark.parametrize(
        "query_token_value, header_token_value, expected_token_to_be_called_with",
        [
            (
                "token_from_query",
                "token_from_header",
                "token_from_header",
            ),
            (
                None,
                "token_from_header",
                "token_from_header",
            ),
            (
                "token_from_query",
                None,
                "token_from_query",
            ),
        ],
    )
    async def test_reconnect_token_resolution(
        self,
        client,
        mock_event,
        setup_mock_rabbitmq,
        metadata,
        query_token_value,
        header_token_value,
        expected_token_to_be_called_with,
    ):
        request_url = "/api/v1/events/stream"
        request_headers = dict(metadata)

        if query_token_value:
            request_url += f"?reconnect_token={query_token_value}"

        if header_token_value:
            request_headers["Last-Event-ID"] = header_token_value

        client.get(request_url, headers=request_headers)

        assert_non_durable_get_instance_call(expected_token_to_be_called_with, None)

    async def test_no_filter_groups_when_reconnect_header_present_without_previous(
        self,
        client,
        mock_event,
        setup_mock_rabbitmq,
        metadata,
        stream_endpoint_url,
        valid_filter_groups,
    ):
        # The browser replay: an EventSource opened on a fresh-connect URL
        # replays that URL's `filter_groups` and no
        # `previous_filter_groups`, so the stream resumes as it was rather
        # than failing as an incomplete filter update. A request that DOES
        # carry `previous_filter_groups` is a deliberate update and keeps
        # both lists -- pinned in
        # `tests/app/rest/test_routes.py::TestLastEventIdFilterUpdate`.
        header_token_value = "token_from_header"

        request_headers = dict(metadata)
        request_headers["Last-Event-ID"] = header_token_value

        client.get(
            stream_endpoint_url,
            headers=request_headers,
        )

        assert_non_durable_get_instance_call(header_token_value, None)

    async def test_v1beta_endpoint_streams_events(
        self, client, mock_event, setup_mock_rabbitmq, stream_endpoint_url_v1beta
    ):
        await stream_endpoint_test(
            client, mock_event, setup_mock_rabbitmq, stream_endpoint_url_v1beta
        )
