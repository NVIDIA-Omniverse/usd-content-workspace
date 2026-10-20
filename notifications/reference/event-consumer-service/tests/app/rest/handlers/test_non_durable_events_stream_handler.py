# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for non-durable events stream handler."""

import logging
from unittest.mock import AsyncMock, MagicMock

import pytest
from sse_starlette import sse

from app.lib.queue_names import redact_reconnect_token
from app.lib.rabbitmq import NonDurableQueueConsumer
from app.lib.models.event_message import EventMessage
from app.lib.observability.sse_metrics import sse_metrics
from app.rest.handlers import non_durable_events_stream_handler as handler_module
from app.rest.models.sse_events import ConsumeEvent
from tests.app.rest.handlers.conftest import AsyncIterator

# noinspection PyProtectedMember
from app.rest.handlers.non_durable_events_stream_handler import (
    _event_generator,
    non_durable_events_stream,
)

from tests.fixtures.reconnect_token import (
    RECONNECT_TOKEN as SECRET_RECONNECT_TOKEN,
)


@pytest.fixture(autouse=True)
def setup_instance(mocker, mocked_non_durable_consumer):
    mocker.patch.object(
        NonDurableQueueConsumer,
        "get_instance",
        return_value=mocked_non_durable_consumer,
    )


@pytest.fixture
def mock_sse_start_connection(mocker):
    return mocker.patch.object(sse_metrics, "start_connection")


@pytest.fixture
def test_filter_groups(valid_filter_group):
    return [valid_filter_group]


@pytest.fixture
def mock_queue():
    queue = AsyncMock()
    queue.name = "test_queue"
    return queue


@pytest.fixture
def mock_sse_response():
    return MagicMock()


@pytest.fixture
def mock_sse_response_cls(mocker, mock_sse_response):
    mock_class = mocker.patch.object(
        sse, "EventSourceResponse", return_value=mock_sse_response
    )
    return mock_class


@pytest.fixture(autouse=True)
def mock_consume(mocker, event_data):
    async def mock_consume(*_args, **_kwargs):
        yield EventMessage(event_data)

    mocker.patch.object(NonDurableQueueConsumer, "consume", return_value=mock_consume())
    return mock_consume


async def _consume_events(
    mock_request, mocked_non_durable_consumer, connection_metadata
):
    generator = _event_generator(
        mock_request, mocked_non_durable_consumer, connection_metadata
    )
    async for _ in generator:
        pass


@pytest.mark.asyncio
class TestNonDurableEventsStream:
    @pytest.mark.parametrize(
        "filters",
        ["test_filter_groups", None],
        ids=["with_filters", "without_filters"],
    )
    async def test_creates_non_durable_queue(
        self, mock_request, filters, test_filter_groups, mocked_non_durable_consumer
    ):
        if filters:
            filters = test_filter_groups

        await non_durable_events_stream(mock_request, filters, filters, "queue-id")

        # noinspection PyUnresolvedReferences
        NonDurableQueueConsumer.get_instance.assert_called_once_with(
            "queue-id", filters, filters
        )

    @pytest.mark.parametrize(
        "filters",
        ["test_filter_groups", None],
        ids=["with_filters", "without_filters"],
    )
    async def test_generates_sse_response(
        self, mock_request, mock_sse_response_cls, filters, test_filter_groups
    ):
        if filters:
            filters = test_filter_groups

        await non_durable_events_stream(mock_request, filters)
        assert mock_sse_response_cls.call_count == 1

    @pytest.mark.parametrize(
        "filters",
        ["test_filter_groups", None],
        ids=["with_filters", "without_filters"],
    )
    async def test_streams_back_sse_responses(
        self,
        mock_request,
        mock_sse_response,
        mock_sse_response_cls,
        test_filter_groups,
        filters,
    ):
        if filters:
            filters = test_filter_groups

        await non_durable_events_stream(mock_request, filters)

        mock_sse_response_cls.assert_called_once()
        assert mock_sse_response_cls.return_value is mock_sse_response

    async def test_sse_response_ping_interval(
        self, mock_request, mock_sse_response_cls
    ):
        await non_durable_events_stream(mock_request, None)
        assert mock_sse_response_cls.call_args[1]["ping"] == 10

    async def test_raises_exception_thrown_by_rabbitmq(self, mock_request, mocker):
        mocker.patch.object(
            NonDurableQueueConsumer,
            "get_instance",
            side_effect=Exception("Failed to create queue"),
        )

        with pytest.raises(Exception):
            await non_durable_events_stream(mock_request, None, None)

    async def test_calls_get_queue_with_reconnect_token(
        self, mocker, mock_request, mock_queue, mocked_non_durable_consumer
    ):
        mocked_init_queue = mocker.patch.object(
            NonDurableQueueConsumer,
            "get_instance",
            return_value=mocked_non_durable_consumer,
        )
        # A reconnect token is the queue id the previous stream was issued,
        # passed straight through to get_instance. It carries nothing
        # derived from the caller.
        reconnect_token = "test-reconnect-token"

        await non_durable_events_stream(mock_request, None, None, reconnect_token)

        mocked_init_queue.assert_called_once_with(reconnect_token, None, None)

    async def test_exception_in_consume_sends_error_event(
        self,
        mock_request,
        mock_queue,
        mocked_non_durable_consumer,
        connection_metadata,
        mocker,
    ):
        test_exception_message = "Something went wrong during consumption!"

        faulty_iterator = AsyncIterator(error=Exception(test_exception_message))
        mocker.patch.object(
            mocked_non_durable_consumer, "consume", return_value=faulty_iterator
        )

        generator = _event_generator(
            mock_request, mocked_non_durable_consumer, connection_metadata
        )

        events_received = []
        async for event in generator:
            events_received.append(event)

        connected_event_dict = events_received[0]
        assert connected_event_dict["event"] == "connected"

        error_event_dict = events_received[1]
        assert error_event_dict["event"] == "error"


@pytest.mark.asyncio
class TestNonDurableStreamSecretHygiene:
    """
    The SSE stream's reconnect token never becomes observability data.

    Token hygiene: a client presents this token to reattach to the queue it
    identifies, so a copy of it in a log line, a span attribute, or anything
    else exported spreads that identifier to every system downstream of
    them. Keeping it out of telemetry is defense in depth.
    The one place it legitimately appears is the SSE ``id:`` field, which is
    how the client is HANDED the token and what a browser echoes back as
    ``Last-Event-ID``.
    """

    @pytest.fixture
    def secret_consumer(self, mocker):
        queue = AsyncMock()
        queue.name = f"event-queue:{SECRET_RECONNECT_TOKEN}:nd"
        consumer = NonDurableQueueConsumer(queue, SECRET_RECONNECT_TOKEN, MagicMock())
        mocker.patch.object(
            NonDurableQueueConsumer, "get_instance", return_value=consumer
        )
        return consumer

    @pytest.fixture
    def span_attributes(self, mocker):
        # Patched on the handler module's own alias, so a rename of the
        # import fails here instead of leaving a test asserting nothing.
        return mocker.patch.object(
            handler_module.common_tracing, "add_current_span_attributes"
        )

    async def test_span_carries_the_redacted_queue_reference(
        self, mock_request, secret_consumer, span_attributes
    ):
        await non_durable_events_stream(
            mock_request, None, None, SECRET_RECONNECT_TOKEN
        )

        assert span_attributes.call_args.kwargs["queue_ref"] == redact_reconnect_token(
            SECRET_RECONNECT_TOKEN
        )

    async def test_span_does_not_carry_the_reconnect_token(
        self, mock_request, secret_consumer, span_attributes
    ):
        await non_durable_events_stream(
            mock_request, None, None, SECRET_RECONNECT_TOKEN
        )

        assert SECRET_RECONNECT_TOKEN not in str(span_attributes.call_args)

    async def test_span_carries_no_queue_id_attribute(
        self, mock_request, secret_consumer, span_attributes
    ):
        # The durable spans keep ``queue_id``; this one must not, or a
        # dashboard joining on that key would pull tokens into view.
        await non_durable_events_stream(
            mock_request, None, None, SECRET_RECONNECT_TOKEN
        )

        assert "queue_id" not in span_attributes.call_args.kwargs

    async def test_span_reports_the_stream_as_a_reconnect(
        self, mock_request, secret_consumer, span_attributes
    ):
        await non_durable_events_stream(
            mock_request, None, None, SECRET_RECONNECT_TOKEN
        )

        assert span_attributes.call_args.kwargs["is_reconnect"] is True

    @pytest.fixture
    def failing_stream_records(
        self, caplog, mocker, mock_request, secret_consumer, connection_metadata
    ):
        """
        Drive the stream's failure path and hand back what it logged.

        The failure path is chosen deliberately: it is the only branch this
        handler logs on, so a guard driven over the healthy path would
        capture nothing and pass no matter what the code did.
        """

        async def _run():
            mocker.patch.object(
                secret_consumer,
                "consume",
                return_value=AsyncIterator(error=ValueError("broker went away")),
            )
            with caplog.at_level(logging.DEBUG):
                await non_durable_events_stream(
                    mock_request, None, None, SECRET_RECONNECT_TOKEN
                )
                await _consume_events(
                    mock_request, secret_consumer, connection_metadata
                )
            return caplog.records

        return _run

    async def test_the_failure_path_logs_something_to_inspect(
        self, failing_stream_records
    ):
        # Guards the guard below: an empty capture would make it vacuous.
        assert await failing_stream_records() != []

    async def test_no_log_record_names_the_reconnect_token(
        self, failing_stream_records
    ):
        records = await failing_stream_records()

        # ``record.__dict__`` covers the rendered message, the interpolation
        # args, and anything passed through ``extra`` -- all three reach the
        # log aggregator.
        leaked = [r for r in records if SECRET_RECONNECT_TOKEN in str(r.__dict__)]
        assert leaked == []

    async def test_the_connected_event_still_hands_the_client_its_token(
        self, mock_request, secret_consumer, connection_metadata
    ):
        # The deliberate exception. The SSE ``id:`` field IS the reconnect
        # token; redacting it would leave the client unable to reconnect.
        generator = _event_generator(mock_request, secret_consumer, connection_metadata)

        events = [event async for event in generator]

        assert events[0]["id"] == SECRET_RECONNECT_TOKEN


@pytest.mark.asyncio
class TestNonDurableConsumerMetrics:
    async def test_records_success_when_event_processed_successfully(
        self,
        mock_request,
        mocked_non_durable_consumer,
        mock_record_success,
        mock_record_failure,
        connection_metadata,
    ):
        await _consume_events(
            mock_request, mocked_non_durable_consumer, connection_metadata
        )

        mock_record_success.assert_called_once()
        mock_record_failure.assert_not_called()

    async def test_records_failure_when_event_processing_fails(
        self,
        mock_request,
        mocked_non_durable_consumer,
        mock_record_success,
        mock_record_failure,
        connection_metadata,
        mocker,
    ):
        mocker.patch.object(
            ConsumeEvent, "__init__", side_effect=ValueError("Processing failed")
        )

        await _consume_events(
            mock_request, mocked_non_durable_consumer, connection_metadata
        )

        mock_record_failure.assert_called_once()
        mock_record_success.assert_not_called()


@pytest.mark.asyncio
class TestNonDurableSseMetrics:
    async def test_non_durable_events_stream_records_connection_start(
        self,
        mock_request,
        test_filter_groups,
        mock_sse_start_connection,
    ):
        await non_durable_events_stream(mock_request, test_filter_groups, None)

        mock_sse_start_connection.assert_called_once()

    async def test_event_generator_records_connection_events(
        self,
        mock_request,
        mocked_non_durable_consumer,
        mock_sse_record_event_sent,
        connection_metadata,
    ):
        await _consume_events(
            mock_request, mocked_non_durable_consumer, connection_metadata
        )

        mock_sse_record_event_sent.assert_any_call(connection_metadata, "connected")
        mock_sse_record_event_sent.assert_any_call(connection_metadata, "test_event")

    async def test_event_generator_records_connection_end_on_completion(
        self,
        mock_request,
        mocked_non_durable_consumer,
        mock_sse_end_connection,
        connection_metadata,
    ):
        await _consume_events(
            mock_request, mocked_non_durable_consumer, connection_metadata
        )

        mock_sse_end_connection.assert_called_once_with(
            metadata=connection_metadata,
            is_error_disconnect=False,
        )

    async def test_event_generator_records_error_on_exception(
        self,
        mock_request,
        mock_sse_end_connection,
        mock_sse_record_event_sent,
        connection_metadata,
        mocker,
    ):
        mock_consumer = MagicMock()
        mock_consumer.queue_id = "test-queue"
        mock_consumer.cleanup_channel = AsyncMock()
        faulty_iterator = AsyncIterator(error=ValueError("Test error"))
        mock_consumer.consume.return_value = faulty_iterator

        await _consume_events(mock_request, mock_consumer, connection_metadata)

        mock_sse_record_event_sent.assert_any_call(connection_metadata, "error")

        mock_sse_end_connection.assert_called_once_with(
            metadata=connection_metadata,
            is_error_disconnect=True,
        )
