# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for durable events stream handler."""

from unittest.mock import MagicMock

import pytest
from notification_common.errors.errors import QueueNotFoundError
from sse_starlette import sse

from app.lib.rabbitmq import DurableQueueConsumer
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase
from app.lib.models.event_message import EventMessage
from app.lib.observability.sse_metrics import sse_metrics
from app.rest.models.sse_events import ConsumeEvent
from tests.app.rest.handlers.conftest import AsyncIterator

# noinspection PyProtectedMember
from app.rest.handlers.durable_events_stream_handler import (
    _durable_event_generator,
    durable_events_stream,
)


@pytest.fixture
def mock_sse_start_connection(mocker):
    return mocker.patch.object(sse_metrics, "start_connection")


@pytest.fixture
def mock_sse_response():
    return MagicMock()


@pytest.fixture
def mock_sse_response_cls(mocker, mock_sse_response):
    mock_class = mocker.patch.object(
        sse, "EventSourceResponse", return_value=mock_sse_response
    )
    return mock_class


@pytest.fixture
def mock_validation_get_channel(mocker, mock_chan):
    """
    Mock the pooled-channel checkout the handler takes to validate the queue.

    autospec pins the mock to the connector's real get_channel signature, so a
    handler that passes a misspelled pool kwarg (e.g. durable=True) raises
    TypeError instead of silently satisfying a durable_pool=True assertion.
    """
    channel_context = mocker.AsyncMock()
    channel_context.__aenter__.return_value = mock_chan

    mock_get_channel = mocker.patch.object(
        RabbitMQBase.connector, "get_channel", autospec=True
    )
    mock_get_channel.return_value = channel_context
    return mock_get_channel


async def _consume_durable_events(mock_request, queue_id, connection_metadata):
    async def mock_gen():
        yield EventMessage(
            {
                "event_type": "test_event",
                "message": {"test": "data"},
                "occurred_at": "2023-01-01T00:00:00Z",
                "published_at": "2023-01-01T00:01:00Z",
            },
        )

    generator = _durable_event_generator(
        mock_request,
        mock_gen(),
        queue_id,
        connection_metadata,
    )
    async for _ in generator:
        pass


@pytest.mark.asyncio
class TestDurableEventsStream:
    async def test_validates_queue_exists_before_streaming(
        self, mock_request, mock_queue_id, mock_get_queue, mock_durable_consume
    ):
        await durable_events_stream(mock_request, mock_queue_id)

        # Verify get_queue was called to validate the queue exists
        mock_get_queue.assert_called_once()

    async def test_validation_checkout_comes_from_the_durable_pool(
        self,
        mock_request,
        mock_queue_id,
        mock_validation_get_channel,
        mock_get_queue,
        mock_durable_consume,
    ):
        # The queue-existence check is a durable-path acquisition: it must be
        # taken from the durable pool, or every REST durable stream's checkout
        # lands in the non-durable pool's stats.
        await durable_events_stream(mock_request, mock_queue_id)

        mock_validation_get_channel.assert_called_once_with(durable_pool=True)

    async def test_validation_runs_on_the_durable_pool_channel(
        self,
        mock_request,
        mock_queue_id,
        mock_chan,
        mock_validation_get_channel,
        mock_get_queue,
        mock_durable_consume,
    ):
        await durable_events_stream(mock_request, mock_queue_id)

        assert mock_get_queue.call_args[0][0] is mock_chan

    async def test_raises_exception_if_queue_not_found(
        self, mock_request, mock_queue_id, mocker
    ):
        mocker.patch.object(
            DurableQueueConsumer,
            "get_queue",
            side_effect=QueueNotFoundError("test_queue"),
        )

        with pytest.raises(QueueNotFoundError):
            await durable_events_stream(mock_request, mock_queue_id)

    async def test_generates_sse_response(
        self,
        mock_request,
        mock_queue_id,
        mock_sse_response_cls,
        mock_get_queue,
        mock_durable_consume,
    ):
        await durable_events_stream(mock_request, mock_queue_id)
        assert mock_sse_response_cls.call_count == 1

    async def test_streams_back_sse_responses(
        self,
        mock_request,
        mock_queue_id,
        mock_sse_response,
        mock_sse_response_cls,
        mock_get_queue,
        mock_durable_consume,
    ):
        await durable_events_stream(mock_request, mock_queue_id)

        mock_sse_response_cls.assert_called_once()
        assert mock_sse_response_cls.return_value is mock_sse_response

    async def test_sse_response_ping_interval(
        self,
        mock_request,
        mock_queue_id,
        mock_sse_response_cls,
        mock_get_queue,
        mock_durable_consume,
    ):
        await durable_events_stream(mock_request, mock_queue_id)
        assert mock_sse_response_cls.call_args[1]["ping"] == 10

    async def test_exception_in_consume_sends_error_event(
        self, mock_request, mock_queue_id, connection_metadata, mocker
    ):
        test_exception_message = "Something went wrong during consumption!"

        faulty_iterator = AsyncIterator(error=Exception(test_exception_message))

        generator = _durable_event_generator(
            mock_request,
            faulty_iterator,
            mock_queue_id,
            connection_metadata,
        )

        events_received = []
        async for event in generator:
            events_received.append(event)

        connected_event_dict = events_received[0]
        assert connected_event_dict["event"] == "connected"

        error_event_dict = events_received[1]
        assert error_event_dict["event"] == "error"


@pytest.mark.asyncio
class TestDurableConsumerMetrics:
    async def test_records_success_when_event_processed_successfully(
        self,
        mock_request,
        mock_queue_id,
        mock_record_success,
        mock_record_failure,
        connection_metadata,
        mock_durable_consume,
    ):
        await _consume_durable_events(mock_request, mock_queue_id, connection_metadata)

        mock_record_success.assert_called_once()
        mock_record_failure.assert_not_called()

    async def test_records_failure_when_event_processing_fails(
        self,
        mock_request,
        mock_queue_id,
        mock_record_success,
        mock_record_failure,
        connection_metadata,
        mocker,
    ):
        async def mock_gen():
            yield EventMessage(
                {
                    "event_type": "test_event",
                    "message": {"test": "data"},
                    "occurred_at": "2023-01-01T00:00:00Z",
                    "published_at": "2023-01-01T00:01:00Z",
                },
            )

        mocker.patch.object(
            ConsumeEvent, "__init__", side_effect=ValueError("Processing failed")
        )

        generator = _durable_event_generator(
            mock_request,
            mock_gen(),
            mock_queue_id,
            connection_metadata,
        )
        async for _ in generator:
            pass

        mock_record_failure.assert_called_once()
        mock_record_success.assert_not_called()


@pytest.mark.asyncio
class TestDurableSseMetrics:
    async def test_durable_events_stream_records_connection_start(
        self,
        mock_request,
        mock_queue_id,
        mock_sse_start_connection,
        mock_get_queue,
        mock_durable_consume,
    ):
        await durable_events_stream(mock_request, mock_queue_id)

        mock_sse_start_connection.assert_called_once()

    async def test_durable_event_generator_records_connection_events(
        self,
        mock_request,
        mock_queue_id,
        mock_sse_record_event_sent,
        connection_metadata,
        mock_durable_consume,
    ):
        await _consume_durable_events(mock_request, mock_queue_id, connection_metadata)

        mock_sse_record_event_sent.assert_any_call(connection_metadata, "connected")
        mock_sse_record_event_sent.assert_any_call(connection_metadata, "test_event")

    async def test_durable_event_generator_records_connection_end_on_completion(
        self,
        mock_request,
        mock_queue_id,
        mock_sse_end_connection,
        connection_metadata,
        mock_durable_consume,
    ):
        await _consume_durable_events(mock_request, mock_queue_id, connection_metadata)

        mock_sse_end_connection.assert_called_once_with(
            metadata=connection_metadata,
            is_error_disconnect=False,
        )

    async def test_durable_event_generator_records_error_on_exception(
        self,
        mock_request,
        mock_queue_id,
        mock_sse_end_connection,
        mock_sse_record_event_sent,
        connection_metadata,
        mocker,
    ):
        faulty_iterator = AsyncIterator(error=ValueError("Test error"))

        generator = _durable_event_generator(
            mock_request,
            faulty_iterator,
            mock_queue_id,
            connection_metadata,
        )
        async for _ in generator:
            pass

        mock_sse_record_event_sent.assert_any_call(connection_metadata, "error")

        mock_sse_end_connection.assert_called_once_with(
            metadata=connection_metadata,
            is_error_disconnect=True,
        )
