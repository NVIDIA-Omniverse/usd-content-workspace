# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Integration tests for the stream_durable_events endpoint."""

import pytest
from fastapi.testclient import TestClient
from notification_common.errors.errors import (
    QueueNotFoundError,
    AuthenticationError,
)

from app.lib.models.event_message import EventMessage
from app.lib.rabbitmq import DurableQueueConsumer
from app.rest.rest_server import rest_server
from tests.integration_tests.rest.conftest import (
    stream_endpoint_test,
    rabbitmq_event_stream_test,
    parse_sse_response,
)


@pytest.fixture
def client():
    """Return a FastAPI test client."""
    return TestClient(rest_server)


@pytest.fixture(autouse=True)
def reset_sse_starlette_event():
    try:
        from sse_starlette.sse import AppStatus

        AppStatus.should_exit_event = None
    except ImportError:
        pass


def _mock_consume(mocker, events):
    """Helper to mock consume with the given event dicts."""

    async def _gen():
        for event in events:
            yield EventMessage(event)

    mocker.patch.object(
        DurableQueueConsumer,
        "consume",
        side_effect=lambda *_args, **_kwargs: _gen(),
    )


@pytest.mark.asyncio
class TestGetStreamDurableEvents:
    @pytest.fixture
    def mock_event(self):
        return {
            "event_type": "test_event",
            "message": {"test": "data"},
            "occurred_at": "2023-01-01T00:00:00Z",
            "published_at": "2023-01-01T00:01:00Z",
        }

    @pytest.fixture
    def mock_queue_id(self):
        return "test-queue-id"

    @pytest.fixture
    def stream_endpoint_url(self, mock_queue_id):
        return f"/api/v1/events/stream-durable?queue_id={mock_queue_id}"

    @pytest.fixture
    def stream_endpoint_url_v1beta(self, mock_queue_id):
        return f"/api/v1beta/events/stream-durable?queue_id={mock_queue_id}"

    @pytest.fixture
    def setup_mock_rabbitmq(self, mocker, mock_event):
        def _setup(events=None):
            if events is None:
                events = [mock_event]

            mocker.patch.object(
                DurableQueueConsumer,
                "get_queue",
            )

            _mock_consume(mocker, events)

        return _setup

    @pytest.fixture
    def stream_response(
        self, client, mock_event, setup_mock_rabbitmq, stream_endpoint_url
    ):
        setup_mock_rabbitmq([mock_event])
        return client.get(
            stream_endpoint_url, headers={"Authorization": "Bearer test-token"}
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
            DurableQueueConsumer,
            "get_queue",
            side_effect=AuthenticationError("Token expired"),
        )

        response = client.get(
            f"/api/v1/events/stream-durable?queue_id={mock_queue_id}",
            headers=metadata,
        )

        assert response.status_code == 401
        assert "Token expired" in response.json().get("detail", "")

    async def test_returns_404_for_invalid_queue_id(
        self,
        client,
        mocker,
        setup_mock_rabbitmq,
        metadata,
        mock_queue_id,
    ):
        setup_mock_rabbitmq()
        mocker.patch.object(
            DurableQueueConsumer,
            "get_queue",
            side_effect=QueueNotFoundError("test_queue"),
        )

        response = client.get(
            f"/api/v1/events/stream-durable?queue_id={mock_queue_id}",
            headers=metadata,
        )

        assert response.status_code == 404

    async def test_returns_422_when_queue_id_is_missing(self, client, metadata):
        response = client.get(
            "/api/v1/events/stream-durable",
            headers=metadata,
        )

        assert response.status_code == 422

    async def test_v1beta_endpoint_streams_events(
        self, client, mock_event, setup_mock_rabbitmq, stream_endpoint_url_v1beta
    ):
        await stream_endpoint_test(
            client, mock_event, setup_mock_rabbitmq, stream_endpoint_url_v1beta
        )
