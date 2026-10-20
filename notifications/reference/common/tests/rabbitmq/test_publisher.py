# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import datetime
from unittest.mock import AsyncMock, call

import aio_pika
import pytest
import pytest_asyncio

from notification_common.observability import tracing
from notification_common.rabbitmq import routing
from notification_common.rabbitmq.omni_event import OmniEvent
from notification_common.rabbitmq.publisher import (
    SuccessfulPublishingResult,
    FailedPublishingResult,
    Publisher,
)


def omni_event():
    return OmniEvent(
        "test-event-type",
        datetime.datetime.now(datetime.UTC).isoformat().replace("+00:00", "Z"),
        "test-resource-id",
        {"test": "message"},
    )


@pytest.fixture
def event():
    return omni_event()


@pytest.fixture
def publisher():
    return Publisher()


@pytest_asyncio.fixture(autouse=True)
async def setup(publisher):
    await publisher.connect()
    yield


@pytest.fixture
def success_response(event):
    return SuccessfulPublishingResult(event=event)


@pytest.fixture
def failed_response(event):
    reason_msg = "Failed to publish event: ValueError - test-reason"
    return FailedPublishingResult(event=event, reason=reason_msg)


@pytest.mark.asyncio
class TestPublish:
    @pytest_asyncio.fixture
    async def mock_exchange(self):
        exchange = AsyncMock()
        return exchange

    @pytest_asyncio.fixture
    async def mock_channel(self):
        channel = AsyncMock()
        return channel

    @pytest_asyncio.fixture
    async def mock_channel_context(self, mock_channel):
        """Fixture that creates a mock context manager for get_channel()"""
        async_context_mock = AsyncMock()
        async_context_mock.__aenter__ = AsyncMock(return_value=mock_channel)
        async_context_mock.__aexit__ = AsyncMock(return_value=None)
        return async_context_mock

    @pytest_asyncio.fixture
    async def setup_publisher_mocks(
        self, publisher, mock_channel_context, mock_exchange, mocker
    ):
        """
        Fixture that sets up standard publisher mocking for get_channel and get_exchange
        """
        mocker.patch.object(publisher, "get_channel", return_value=mock_channel_context)
        mocker.patch.object(publisher, "get_exchange", return_value=mock_exchange)
        return {"channel_context": mock_channel_context, "exchange": mock_exchange}

    async def test_publishes_message(
        self, publisher, event, mock_channel, mocker, setup_publisher_mocks
    ):
        mocker.patch.object(publisher, "_message", return_value="Test Message Value")
        mocker.patch.object(routing, "routing_key", return_value="test-key")

        await publisher.publish(event)

        setup_publisher_mocks["exchange"].publish.assert_called_once_with(
            message="Test Message Value",
            routing_key="test-key",
        )
        publisher.get_exchange.assert_called_once_with(mock_channel)

    async def test_returns_success_response(
        self, publisher, event, setup_publisher_mocks
    ):
        results = await publisher.publish(event)
        assert results.success is True

    async def test_returns_failed_response_if_error_raised(
        self, publisher, event, setup_publisher_mocks
    ):
        setup_publisher_mocks["exchange"].publish.side_effect = Exception()

        results = await publisher.publish(event)
        assert results.success is False

    async def test_makes_routing_key_from_event_type_and_resource_id(
        self, publisher, event, mocker, setup_publisher_mocks
    ):
        spy = mocker.spy(routing, "routing_key")
        await publisher.publish(event)

        spy.assert_called_once_with("test-event-type", "test-resource-id")

    class TestWhileClosing:
        async def test_returns_failure_if_closes_before_publish(self, event, publisher):
            publisher.closing = True
            results = await publisher.publish(event)
            assert results.success is False

        async def test_does_not_get_channel_when_closing(
            self, event, publisher, mocker
        ):
            publisher.closing = True
            spy = mocker.patch.object(publisher, "get_channel")

            await publisher.publish(event)

            spy.assert_not_called()

        async def test_does_not_publish_if_closes_during_channel_access(
            self, event, publisher, mock_exchange, mock_channel, mocker
        ):
            # Mock context manager that sets closing during execution
            async def mock_context():
                publisher.closing = True
                return mock_channel

            async_context_mock = AsyncMock()
            async_context_mock.__aenter__ = mock_context
            async_context_mock.__aexit__ = AsyncMock(return_value=None)
            mocker.patch.object(
                publisher, "get_channel", return_value=async_context_mock
            )
            mocker.patch.object(publisher, "get_exchange", return_value=mock_exchange)

            result = await publisher.publish(event)

            assert result.success is False
            mock_exchange.publish.assert_not_called()

    class TestTracing:
        async def test_creates_span_during_publish(self, publisher, event, mocker):
            spy_trace_operation = mocker.spy(tracing, "trace_operation")

            await publisher.publish(event)

            spy_trace_operation.assert_called_once_with("publish_event")

        async def test_adds_span_event(self, publisher, event, mocker):
            spy_add_event = mocker.spy(tracing, "add_current_span_event")

            await publisher.publish(event)

            spy_add_event.assert_called_once()

        async def test_does_not_add_span_event_when_publish_fails(
            self, publisher, event, mocker, setup_publisher_mocks
        ):
            setup_publisher_mocks["exchange"].publish.side_effect = Exception(
                "Publish failed"
            )
            spy_add_event = mocker.spy(tracing, "add_current_span_event")

            await publisher.publish(event)

            spy_add_event.assert_not_called()


@pytest.mark.asyncio
class TestBatchPublish:
    @pytest_asyncio.fixture
    async def mock_exchange(self):
        exchange = AsyncMock()
        return exchange

    @pytest_asyncio.fixture
    async def mock_channel(self):
        channel = AsyncMock()
        return channel

    @pytest_asyncio.fixture
    async def mock_channel_context(self, mock_channel):
        """Fixture that creates a mock context manager for get_channel()"""
        async_context_mock = AsyncMock()
        async_context_mock.__aenter__ = AsyncMock(return_value=mock_channel)
        async_context_mock.__aexit__ = AsyncMock(return_value=None)
        return async_context_mock

    @pytest_asyncio.fixture
    async def setup_batch_publisher_mocks(
        self, publisher, mock_channel_context, mock_exchange, mocker
    ):
        """
        Fixture that sets up standard publisher mocking for batch operations
        """
        mocker.patch.object(publisher, "get_channel", return_value=mock_channel_context)
        mocker.patch.object(publisher, "get_exchange", return_value=mock_exchange)
        return {"channel_context": mock_channel_context, "exchange": mock_exchange}

    async def test_returns_empty_list_for_empty_events(self, publisher):
        results = await publisher.batch_publish([])
        assert results == []

    async def test_returns_list_of_results(
        self, publisher, event, setup_batch_publisher_mocks
    ):
        results = await publisher.batch_publish([event, event])

        assert len(results) == 2
        assert all(result.success for result in results)
        assert all(result.event == event for result in results)

    async def test_publishes_all_events_in_order(
        self, publisher, event, mock_channel, mocker, setup_batch_publisher_mocks
    ):
        mocker.patch.object(publisher, "_message", return_value="Test Message Value")
        mocker.patch.object(routing, "routing_key", return_value="test-key")

        other_event = omni_event()
        await publisher.batch_publish([event, other_event])

        # Verify all events were published in order
        assert setup_batch_publisher_mocks["exchange"].publish.call_count == 2
        setup_batch_publisher_mocks["exchange"].publish.assert_has_calls(
            [
                call(message="Test Message Value", routing_key="test-key"),
                call(message="Test Message Value", routing_key="test-key"),
            ]
        )

    async def test_continues_processing_after_individual_event_failure(
        self, publisher, event, setup_batch_publisher_mocks
    ):
        # Make the first publish fail, second succeed
        setup_batch_publisher_mocks["exchange"].publish.side_effect = [
            Exception("First event failed"),
            None,  # Second event succeeds
        ]

        results = await publisher.batch_publish([event, event])

        assert len(results) == 2
        assert results[0].success is False
        assert results[1].success is True
        assert "First event failed" in results[0].reason

    async def test_returns_all_failed_results_when_channel_access_fails(
        self, publisher, event, mocker
    ):
        mocker.patch.object(
            publisher, "get_channel", side_effect=Exception("Channel failed")
        )

        results = await publisher.batch_publish([event, event])

        assert len(results) == 2
        assert all(not result.success for result in results)
        assert all("Channel failed" in result.reason for result in results)


class TestMessage:
    def test_uses_encoded_event_to_create_message(self, event, freezer, mocker):
        freezer.move_to(datetime.datetime.now())
        spy = mocker.spy(aio_pika, "Message")
        Publisher._message(event)
        spy.assert_called_once_with(
            event.encoded_for_publish(),
            delivery_mode=aio_pika.DeliveryMode.PERSISTENT,
        )

    def test_returns_api_pika_message_object(self, publisher, event, mocker):
        mocker.patch.object(aio_pika, "Message", return_value="Test Message Value")
        assert publisher._message(event) == "Test Message Value"
