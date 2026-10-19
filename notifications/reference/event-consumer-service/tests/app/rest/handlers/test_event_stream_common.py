# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for common event streaming logic."""

import json
from unittest.mock import AsyncMock, MagicMock

import pytest

from app.lib.models.event_message import EventMessage
from app.lib.rabbitmq import DurableQueueConsumer, NonDurableQueueConsumer
from app.rest.models.sse_events import ConsumeEvent
from app.rest.handlers.event_stream_common import event_stream_generator
from tests.app.rest.handlers.conftest import AsyncIterator
from tests.fixtures.own_operation_dead_channel import (
    DURABLE_QUEUE_ID as OWN_OPERATION_QUEUE_ID,
    RECONNECT_GUIDANCE,
    OwnOperationDeadChannelRig,
)


@pytest.fixture
def queue_id():
    return "test-queue-id"


@pytest.fixture
def single_event_source(event_data):
    """Create an event source with a single event."""
    return AsyncIterator([EventMessage(event_data)])


@pytest.fixture
def error_event_source():
    """Create an event source that raises an error."""
    return lambda error: AsyncIterator(error=error)


@pytest.fixture
def multiple_event_source(event_data):
    """Factory for creating event sources with multiple events."""

    def _create_source(count=2):
        events = []
        for i in range(count):
            data = event_data.copy()
            if i > 0:
                data["event_type"] = f"test_event_{i+1}"
            events.append(EventMessage(data))
        return AsyncIterator(events)

    return _create_source


@pytest.fixture
def consume_events():
    """Helper to consume all events from the generator."""

    async def _consume(
        mock_request, event_source, queue_id, connection_metadata, cleanup_fn=None
    ):
        generator = event_stream_generator(
            request=mock_request,
            event_source=event_source,
            queue_id=queue_id,
            connection_metadata=connection_metadata,
            stream_type="test",
            cleanup_fn=cleanup_fn,
        )
        async for _ in generator:
            pass

    return _consume


@pytest.fixture
def collect_events():
    """Helper to collect all events from the generator."""

    async def _collect(
        mock_request, event_source, queue_id, connection_metadata, cleanup_fn=None
    ):
        events = []
        # noinspection PyTypeChecker
        async for event in event_stream_generator(
            request=mock_request,
            event_source=event_source,
            queue_id=queue_id,
            connection_metadata=connection_metadata,
            stream_type="test",
            cleanup_fn=cleanup_fn,
        ):
            events.append(event)
        return events

    return _collect


@pytest.mark.asyncio
class TestEventStreamGenerator:
    async def test_yields_connected_event_first(
        self,
        mock_request,
        event_data,
        queue_id,
        connection_metadata,
        single_event_source,
        collect_events,
    ):
        events_received = await collect_events(
            mock_request, single_event_source, queue_id, connection_metadata
        )

        assert len(events_received) >= 1
        assert events_received[0]["event"] == "connected"
        assert events_received[0]["id"] == queue_id
        # Data is serialized as JSON string
        data = json.loads(events_received[0]["data"])
        assert data["reconnect_token"] == queue_id

    async def test_yields_consume_events_from_source(
        self,
        mock_request,
        event_data,
        queue_id,
        connection_metadata,
        single_event_source,
        collect_events,
    ):
        events_received = await collect_events(
            mock_request, single_event_source, queue_id, connection_metadata
        )

        assert len(events_received) == 2
        assert events_received[1]["event"] == "event"
        # Data is serialized as JSON string containing the full EventData
        data = json.loads(events_received[1]["data"])
        assert data["message"] == event_data["message"]
        assert data["event_type"] == event_data["event_type"]

    async def test_calls_cleanup_function_in_finally(
        self,
        mock_request,
        queue_id,
        connection_metadata,
        single_event_source,
        consume_events,
    ):
        cleanup_fn = AsyncMock()

        await consume_events(
            mock_request, single_event_source, queue_id, connection_metadata, cleanup_fn
        )

        cleanup_fn.assert_called_once()

    async def test_cleanup_function_called_even_on_error(
        self,
        mock_request,
        queue_id,
        connection_metadata,
        error_event_source,
        consume_events,
    ):
        cleanup_fn = AsyncMock()

        await consume_events(
            mock_request,
            error_event_source(ValueError("Test error")),
            queue_id,
            connection_metadata,
            cleanup_fn,
        )

        cleanup_fn.assert_called_once()

    async def test_no_error_when_cleanup_function_is_none(
        self,
        mock_request,
        queue_id,
        connection_metadata,
        single_event_source,
        consume_events,
    ):
        # This should not raise an error
        await consume_events(
            mock_request, single_event_source, queue_id, connection_metadata, None
        )

    async def test_yields_error_event_on_exception(
        self,
        mock_request,
        queue_id,
        connection_metadata,
        error_event_source,
        collect_events,
    ):
        test_exception_message = "Something went wrong!"

        events_received = await collect_events(
            mock_request,
            error_event_source(Exception(test_exception_message)),
            queue_id,
            connection_metadata,
        )

        assert len(events_received) == 2
        assert events_received[0]["event"] == "connected"
        assert events_received[1]["event"] == "error"

    async def test_stops_when_request_disconnects(
        self,
        queue_id,
        connection_metadata,
        multiple_event_source,
        collect_events,
    ):
        # Create a request that reports disconnection after first yield
        mock_request = MagicMock()
        call_count = 0

        async def is_disconnected():
            nonlocal call_count
            call_count += 1
            return call_count > 1

        mock_request.is_disconnected = is_disconnected

        events_received = await collect_events(
            mock_request, multiple_event_source(2), queue_id, connection_metadata
        )

        # Should have connected event and first consume event only
        assert len(events_received) == 2
        assert events_received[0]["event"] == "connected"
        assert events_received[1]["event"] == "event"


@pytest.mark.asyncio
class TestEventStreamGeneratorMetrics:
    async def test_records_connected_event(
        self,
        mock_request,
        queue_id,
        connection_metadata,
        single_event_source,
        consume_events,
        mock_sse_record_event_sent,
    ):
        await consume_events(
            mock_request, single_event_source, queue_id, connection_metadata
        )

        mock_sse_record_event_sent.assert_any_call(connection_metadata, "connected")

    async def test_records_consume_event_type(
        self,
        mock_request,
        queue_id,
        connection_metadata,
        single_event_source,
        consume_events,
        mock_sse_record_event_sent,
    ):
        await consume_events(
            mock_request, single_event_source, queue_id, connection_metadata
        )

        mock_sse_record_event_sent.assert_any_call(connection_metadata, "test_event")

    async def test_records_error_event_on_exception(
        self,
        mock_request,
        queue_id,
        connection_metadata,
        error_event_source,
        consume_events,
        mock_sse_record_event_sent,
    ):
        await consume_events(
            mock_request,
            error_event_source(ValueError("Test error")),
            queue_id,
            connection_metadata,
        )

        mock_sse_record_event_sent.assert_any_call(connection_metadata, "error")

    async def test_records_connection_end_on_completion(
        self,
        mock_request,
        queue_id,
        connection_metadata,
        single_event_source,
        consume_events,
        mock_sse_end_connection,
    ):
        await consume_events(
            mock_request, single_event_source, queue_id, connection_metadata
        )

        mock_sse_end_connection.assert_called_once_with(
            metadata=connection_metadata,
            is_error_disconnect=False,
        )

    async def test_records_connection_end_on_error(
        self,
        mock_request,
        queue_id,
        connection_metadata,
        error_event_source,
        consume_events,
        mock_sse_end_connection,
    ):
        await consume_events(
            mock_request,
            error_event_source(ValueError("Test error")),
            queue_id,
            connection_metadata,
        )

        mock_sse_end_connection.assert_called_once_with(
            metadata=connection_metadata,
            is_error_disconnect=True,
        )


@pytest.mark.asyncio
class TestEventStreamGeneratorConsumerMetrics:
    async def test_records_success_when_event_processed(
        self,
        mock_request,
        queue_id,
        connection_metadata,
        single_event_source,
        consume_events,
        mock_record_success,
        mock_record_failure,
    ):
        await consume_events(
            mock_request, single_event_source, queue_id, connection_metadata
        )

        mock_record_success.assert_called_once()
        mock_record_failure.assert_not_called()

    async def test_records_failure_when_processing_fails(
        self,
        mock_request,
        queue_id,
        connection_metadata,
        single_event_source,
        consume_events,
        mock_record_success,
        mock_record_failure,
        mocker,
    ):
        mocker.patch.object(
            ConsumeEvent, "__init__", side_effect=ValueError("Processing failed")
        )

        await consume_events(
            mock_request, single_event_source, queue_id, connection_metadata
        )

        mock_record_failure.assert_called_once()
        # Success should not be called when processing fails
        mock_record_success.assert_not_called()

    async def test_continues_after_processing_failure(
        self,
        mock_request,
        queue_id,
        connection_metadata,
        multiple_event_source,
        collect_events,
        mock_record_success,
        mock_record_failure,
        mocker,
    ):
        # Make ConsumeEvent.__init__ fail on first call only
        original_init = ConsumeEvent.__init__
        calls = []

        def track_init(_self, *args, **kwargs):
            calls.append(1)
            if len(calls) == 1:
                raise ValueError("First processing failed")
            # noinspection PyArgumentList
            return original_init(_self, *args, **kwargs)

        mocker.patch.object(
            ConsumeEvent, "__init__", autospec=True, side_effect=track_init
        )

        events_received = await collect_events(
            mock_request, multiple_event_source(2), queue_id, connection_metadata
        )

        # First event processing should fail, second should succeed,
        # so we should have: connected event and second consume event
        assert len(events_received) == 2
        assert events_received[0]["event"] == "connected"
        assert events_received[1]["event"] == "event"

        # Both failure and success should be recorded
        assert mock_record_failure.call_count == 1
        assert mock_record_success.call_count == 1


# The non-durable queue the own-operation rig consumes from. Its broker name
# embeds it, as production's does.
_OWN_OPERATION_NON_DURABLE_QUEUE_ID = "non-durable-whose-channel-dies"


def _durable_event_source(_rig):
    return DurableQueueConsumer.consume(OWN_OPERATION_QUEUE_ID), None


def _non_durable_event_source(rig):
    # A REAL consumer over the rig's queue, with its real cleanup_channel()
    # as the stream's cleanup_fn, as the non-durable handler wires it.
    consumer = NonDurableQueueConsumer(
        rig.queue, _OWN_OPERATION_NON_DURABLE_QUEUE_ID, MagicMock()
    )
    return consumer.consume(), consumer.cleanup_channel


# Each SSE stream's event source, with the broker name of the queue it reads.
_OWN_OPERATION_SSE_STREAMS = {
    "durable": (_durable_event_source, f"event-queue:{OWN_OPERATION_QUEUE_ID}"),
    "non_durable": (
        _non_durable_event_source,
        f"event-queue:{_OWN_OPERATION_NON_DURABLE_QUEUE_ID}:nd",
    ),
}


@pytest.fixture(params=list(_OWN_OPERATION_SSE_STREAMS))
def run_own_operation_rig(request, mock_request, connection_metadata, mocker):
    async def _run():
        make_event_source, queue_name = _OWN_OPERATION_SSE_STREAMS[request.param]
        rig = OwnOperationDeadChannelRig(mocker, queue_name)
        event_source, cleanup_fn = make_event_source(rig)

        async def _client():
            return [
                event
                async for event in event_stream_generator(
                    request=mock_request,
                    event_source=event_source,
                    queue_id="the-stream-id",
                    connection_metadata=connection_metadata,
                    stream_type="test",
                    cleanup_fn=cleanup_fn,
                )
            ]

        return await rig.run(_client())

    return _run


@pytest.mark.asyncio
class TestEventStreamGeneratorOverAConsumeLoopOwnOperationOnADeadChannel:
    """
    A consume loop whose own skip-ack finds the channel dead ends both SSE
    streams with the reconnect guidance and a narrowed cleanup
    (issue b.7yv).

    The rig (``tests/fixtures/own_operation_dead_channel.py``) runs this
    generator over the real ``DurableQueueConsumer.consume`` or
    ``NonDurableQueueConsumer.consume`` and the real delayed-ack loop: two
    deliveries reach the client, then the loop acks a delivery the
    authorization check denies, against a channel the broker has just
    killed, with a fourth buffered behind it. Both the client-text and the
    no-cancel assertions tell the fix apart here: nothing between the loop
    and this generator classifies a raw channel error, so before it the
    stream ended with a bare "Internal Server Error".
    """

    async def test_the_error_carries_the_reconnect_guidance(
        self, run_own_operation_rig
    ):
        result = await run_own_operation_rig()

        assert json.loads(result.seen[-1]["data"])["message"] == RECONNECT_GUIDANCE

    async def test_the_error_carries_status_500(self, run_own_operation_rig):
        # Not 503: that is the capacity-shed status the proxy's retry policy
        # and the capacity metrics key on, and this is not a capacity event.
        result = await run_own_operation_rig()

        assert json.loads(result.seen[-1]["data"])["status_code"] == 500

    async def test_the_error_is_the_last_event_of_a_live_stream(
        self, run_own_operation_rig
    ):
        # The connected event and the two deliveries the loop yielded before
        # the failing skip-ack come first, so the error ends a stream that
        # was live, and nothing follows it.
        result = await run_own_operation_rig()

        assert [event["event"] for event in result.seen] == [
            "connected",
            "event",
            "event",
            "error",
        ]

    async def test_no_consumer_cancel_is_sent(self, run_own_operation_rig):
        result = await run_own_operation_rig()

        result.cancel.assert_not_awaited()

    async def test_no_outstanding_delivery_is_nacked(self, run_own_operation_rig):
        result = await run_own_operation_rig()

        assert result.outstanding_nacks == [0, 0]

    async def test_no_buffered_delivery_is_nacked(self, run_own_operation_rig):
        result = await run_own_operation_rig()

        assert result.buffered_nacks == 0

    async def test_the_pool_channel_is_released_exactly_once(
        self, run_own_operation_rig
    ):
        result = await run_own_operation_rig()

        result.return_channel.assert_awaited_once()
