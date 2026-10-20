# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Unit tests for the dead-letter consume/replay logic.

Covers the per-tag REPLAY / DISCARD / LEAVE actions on ``DeadLetterConsumer``.
These exercise the logic at the RabbitMQ layer: a real ``AckTracker`` holds the
(mock) dead-lettered message, and the channel / default-exchange publish /
publisher-confirm boundary is hand-faked. No gRPC server is spun up (that is the
integration Subtask). The pure ``decode_dead_lettered_message`` decode is
covered in ``test_dead_letter.py``.
"""

import asyncio
import logging
from types import SimpleNamespace
from unittest.mock import AsyncMock

import aio_pika
import pytest
import pytest_asyncio
from notification_common.errors.errors import QueueNotFoundError

from app.config import settings as _settings_module
from app.lib.rabbitmq import dead_letter_consumer
from app.lib.rabbitmq.ack_tracker import AckTracker
from app.lib.rabbitmq.dead_letter_consumer import (
    ActionStatus,
    DeadLetterConsumer,
    ReplayStatus,
)
from app.lib.rabbitmq.rabbitmq_base import ConsumerCancelledError, RabbitMQBase
from tests.fixtures.consumer_liveness import (
    CONSUMER_LOSS_TIMEOUT_S,
    LIVENESS_QUEUE_NAME,
    PUMP_CONSUMER_TAG,
    cancel_while_polling,
    close_connector_on_dequeue,
    drain,
    drain_while_shutting_down,
    expect_cleanup,
    expect_terminal_error,
)
from tests.fixtures.dead_channel import dead_channel_failures, unrelated_failures

#: The text of a republish failure; it must reach the log but not the client.
_INTERNAL_FAILURE_TEXT = "broker nacked the publish on amqp://internal-host:5672"
_DEAD_LETTER_LOGGER = dead_letter_consumer.logger.name


def _delivery_error() -> aio_pika.exceptions.DeliveryError:
    """An unroutable-with-mandatory DeliveryError (origin queue deleted)."""
    return aio_pika.exceptions.DeliveryError(None, None)


class _FakeDefaultExchange:
    """
    Records publishes to the channel's default exchange.

    ``on_publish`` is an optional hook invoked inside the awaited publish (after
    args are captured, before the publish resolves) so a test can observe the
    confirm-then-ack ordering or inject a publish failure.
    """

    def __init__(self, on_publish=None):
        self.publish_calls = []
        self._on_publish = on_publish

    async def publish(self, message, routing_key=None, mandatory=None):
        self.publish_calls.append(
            {
                "message": message,
                "routing_key": routing_key,
                "mandatory": mandatory,
            }
        )
        if self._on_publish is not None:
            self._on_publish()


class _FakeChannel:
    """A channel exposing only the ``default_exchange`` REPLAY republishes on."""

    def __init__(self, default_exchange):
        self.default_exchange = default_exchange


class TestDiscard:
    """DISCARD acks (drops) the DLQ copy; no republish."""

    @pytest.mark.asyncio
    async def test_acks_the_dlq_copy_once(self, dead_lettered_message):
        tracker = AckTracker()
        tag = tracker.track(dead_lettered_message)

        await DeadLetterConsumer.discard(tracker, tag)

        dead_lettered_message.ack.assert_awaited_once()

    @pytest.mark.asyncio
    async def test_does_not_nack(self, dead_lettered_message):
        tracker = AckTracker()
        tag = tracker.track(dead_lettered_message)

        await DeadLetterConsumer.discard(tracker, tag)

        dead_lettered_message.nack.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_returns_discarded_status(self, dead_lettered_message):
        tracker = AckTracker()
        tag = tracker.track(dead_lettered_message)

        result = await DeadLetterConsumer.discard(tracker, tag)

        assert result.status is ActionStatus.DISCARDED

    @pytest.mark.asyncio
    async def test_unknown_tag_returns_unknown_status(self):
        tracker = AckTracker()

        result = await DeadLetterConsumer.discard(tracker, 999)

        assert result.status is ActionStatus.UNKNOWN_TAG


class TestLeave:
    """LEAVE nacks requeue=True; no ack, no republish."""

    @pytest.mark.asyncio
    async def test_nacks_with_requeue_true_once(self, dead_lettered_message):
        tracker = AckTracker()
        tag = tracker.track(dead_lettered_message)

        await DeadLetterConsumer.leave(tracker, tag)

        dead_lettered_message.nack.assert_awaited_once_with(requeue=True)

    @pytest.mark.asyncio
    async def test_does_not_ack(self, dead_lettered_message):
        tracker = AckTracker()
        tag = tracker.track(dead_lettered_message)

        await DeadLetterConsumer.leave(tracker, tag)

        dead_lettered_message.ack.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_returns_left_status(self, dead_lettered_message):
        tracker = AckTracker()
        tag = tracker.track(dead_lettered_message)

        result = await DeadLetterConsumer.leave(tracker, tag)

        assert result.status is ActionStatus.LEFT

    @pytest.mark.asyncio
    async def test_unknown_tag_returns_unknown_status(self):
        tracker = AckTracker()

        result = await DeadLetterConsumer.leave(tracker, 999)

        assert result.status is ActionStatus.UNKNOWN_TAG


class TestReplayHappyPath:
    """REPLAY republishes to the origin queue then acks on confirm."""

    @pytest.mark.asyncio
    async def test_publishes_with_origin_queue_routing_key(
        self, make_dead_lettered_message
    ):
        message = make_dead_lettered_message(queue="origin-queue", delivery_tag=5)
        tracker = AckTracker()
        tracker.track(message)
        exchange = _FakeDefaultExchange()
        channel = _FakeChannel(exchange)

        await DeadLetterConsumer.replay(channel, tracker, 5, "origin-queue")

        assert exchange.publish_calls[0]["routing_key"] == "origin-queue"

    @pytest.mark.asyncio
    async def test_publishes_with_mandatory_true(self, make_dead_lettered_message):
        message = make_dead_lettered_message(delivery_tag=5)
        tracker = AckTracker()
        tracker.track(message)
        exchange = _FakeDefaultExchange()
        channel = _FakeChannel(exchange)

        await DeadLetterConsumer.replay(channel, tracker, 5, "origin-queue")

        assert exchange.publish_calls[0]["mandatory"] is True

    @pytest.mark.asyncio
    async def test_publishes_original_body_bytes(self, make_dead_lettered_message):
        message = make_dead_lettered_message(delivery_tag=5)
        tracker = AckTracker()
        tracker.track(message)
        exchange = _FakeDefaultExchange()
        channel = _FakeChannel(exchange)

        await DeadLetterConsumer.replay(channel, tracker, 5, "origin-queue")

        assert exchange.publish_calls[0]["message"].body == message.body

    @pytest.mark.asyncio
    async def test_acks_only_after_publish_confirmed(self, make_dead_lettered_message):
        message = make_dead_lettered_message(delivery_tag=5)
        tracker = AckTracker()
        tracker.track(message)
        ack_count_at_publish = []

        def _record_ack_state():
            # Captured inside the awaited publish: the DLQ copy must NOT be
            # acked yet (confirm-then-ack ordering).
            ack_count_at_publish.append(message.ack.await_count)

        exchange = _FakeDefaultExchange(on_publish=_record_ack_state)
        channel = _FakeChannel(exchange)

        await DeadLetterConsumer.replay(channel, tracker, 5, "origin-queue")

        assert ack_count_at_publish == [0]

    @pytest.mark.asyncio
    async def test_acks_the_dlq_copy_after_confirm(self, make_dead_lettered_message):
        message = make_dead_lettered_message(delivery_tag=5)
        tracker = AckTracker()
        tracker.track(message)
        exchange = _FakeDefaultExchange()
        channel = _FakeChannel(exchange)

        await DeadLetterConsumer.replay(channel, tracker, 5, "origin-queue")

        message.ack.assert_awaited_once()

    @pytest.mark.asyncio
    async def test_returns_replayed_status(self, make_dead_lettered_message):
        message = make_dead_lettered_message(delivery_tag=5)
        tracker = AckTracker()
        tracker.track(message)
        exchange = _FakeDefaultExchange()
        channel = _FakeChannel(exchange)

        result = await DeadLetterConsumer.replay(channel, tracker, 5, "origin-queue")

        assert result.status is ReplayStatus.REPLAYED


class TestReplayPublishFailure:
    """A publish that is not confirmed must not lose the DLQ copy."""

    @pytest.mark.asyncio
    async def test_does_not_ack_on_publish_failure(self, make_dead_lettered_message):
        message = make_dead_lettered_message(delivery_tag=5)
        tracker = AckTracker()
        tracker.track(message)

        def _fail():
            raise RuntimeError("broker nacked the publish")

        exchange = _FakeDefaultExchange(on_publish=_fail)
        channel = _FakeChannel(exchange)

        await DeadLetterConsumer.replay(channel, tracker, 5, "origin-queue")

        message.ack.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_leaves_dlq_copy_in_place_on_failure(
        self, make_dead_lettered_message
    ):
        message = make_dead_lettered_message(delivery_tag=5)
        tracker = AckTracker()
        tracker.track(message)

        def _fail():
            raise RuntimeError("broker nacked the publish")

        exchange = _FakeDefaultExchange(on_publish=_fail)
        channel = _FakeChannel(exchange)

        await DeadLetterConsumer.replay(channel, tracker, 5, "origin-queue")

        message.nack.assert_awaited_once_with(requeue=True)

    @pytest.mark.asyncio
    async def test_reports_replay_failed_status(self, make_dead_lettered_message):
        message = make_dead_lettered_message(delivery_tag=5)
        tracker = AckTracker()
        tracker.track(message)

        def _fail():
            raise RuntimeError("broker nacked the publish")

        exchange = _FakeDefaultExchange(on_publish=_fail)
        channel = _FakeChannel(exchange)

        result = await DeadLetterConsumer.replay(channel, tracker, 5, "origin-queue")

        assert result.status is ReplayStatus.REPLAY_FAILED

    @pytest.fixture
    def failed_replay(self, make_dead_lettered_message, caplog):
        """Replay a message whose republish raises; return the result."""

        async def _run():
            tracker = AckTracker()
            tracker.track(make_dead_lettered_message(delivery_tag=5))

            def _fail():
                raise RuntimeError(_INTERNAL_FAILURE_TEXT)

            channel = _FakeChannel(_FakeDefaultExchange(on_publish=_fail))
            with caplog.at_level(logging.ERROR, logger=_DEAD_LETTER_LOGGER):
                return await DeadLetterConsumer.replay(
                    channel, tracker, 5, "origin-queue"
                )

        return _run

    @pytest.mark.asyncio
    async def test_replay_failed_detail_names_the_exception_class(self, failed_replay):
        # The failure is reported, not silently swallowed.
        result = await failed_replay()

        assert "RuntimeError" in result.detail

    @pytest.mark.asyncio
    async def test_replay_failed_detail_omits_the_exception_text(self, failed_replay):
        # The detail is returned to the client; the exception text can carry
        # broker or connection internals.
        result = await failed_replay()

        assert _INTERNAL_FAILURE_TEXT not in result.detail

    @pytest.mark.asyncio
    async def test_replay_failure_is_logged_with_the_exception(
        self, failed_replay, caplog
    ):
        await failed_replay()

        assert any(
            r.exc_info and _INTERNAL_FAILURE_TEXT in str(r.exc_info[1])
            for r in caplog.records
        )


class TestReplayOriginQueueDeleted:
    """SR-3.4: origin queue gone -> no loss + ORIGIN_QUEUE_DELETED reported."""

    @pytest.mark.asyncio
    async def test_does_not_ack_when_origin_deleted(self, make_dead_lettered_message):
        message = make_dead_lettered_message(delivery_tag=5)
        tracker = AckTracker()
        tracker.track(message)

        def _unroutable():
            raise _delivery_error()

        exchange = _FakeDefaultExchange(on_publish=_unroutable)
        channel = _FakeChannel(exchange)

        await DeadLetterConsumer.replay(channel, tracker, 5, "gone-queue")

        message.ack.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_leaves_dlq_copy_in_place_when_origin_deleted(
        self, make_dead_lettered_message
    ):
        message = make_dead_lettered_message(delivery_tag=5)
        tracker = AckTracker()
        tracker.track(message)

        def _unroutable():
            raise _delivery_error()

        exchange = _FakeDefaultExchange(on_publish=_unroutable)
        channel = _FakeChannel(exchange)

        await DeadLetterConsumer.replay(channel, tracker, 5, "gone-queue")

        message.nack.assert_awaited_once_with(requeue=True)

    @pytest.mark.asyncio
    async def test_reports_origin_queue_deleted_status(
        self, make_dead_lettered_message
    ):
        message = make_dead_lettered_message(delivery_tag=5)
        tracker = AckTracker()
        tracker.track(message)

        def _unroutable():
            raise _delivery_error()

        exchange = _FakeDefaultExchange(on_publish=_unroutable)
        channel = _FakeChannel(exchange)

        result = await DeadLetterConsumer.replay(channel, tracker, 5, "gone-queue")

        assert result.status is ReplayStatus.ORIGIN_QUEUE_DELETED

    @pytest.mark.asyncio
    async def test_reports_the_missing_origin_queue_name(
        self, make_dead_lettered_message
    ):
        message = make_dead_lettered_message(delivery_tag=5)
        tracker = AckTracker()
        tracker.track(message)

        def _unroutable():
            raise _delivery_error()

        exchange = _FakeDefaultExchange(on_publish=_unroutable)
        channel = _FakeChannel(exchange)

        result = await DeadLetterConsumer.replay(channel, tracker, 5, "gone-queue")

        assert result.origin_queue == "gone-queue"


@pytest.mark.asyncio
class TestGetDeadLetterQueue:
    """``_get_dead_letter_queue`` passively asserts the global DLQ exists."""

    async def test_declares_queue_passively_by_configured_name(self, mocker):
        channel = mocker.MagicMock()
        channel.declare_queue = mocker.AsyncMock()

        await DeadLetterConsumer._get_dead_letter_queue(channel)

        channel.declare_queue.assert_awaited_once_with(
            _settings_module.settings.dead_letter_queue_name,
            passive=True,
            robust=False,
        )

    async def test_returns_the_declared_queue(self, mocker):
        channel = mocker.MagicMock()
        queue = mocker.MagicMock()
        channel.declare_queue = mocker.AsyncMock(return_value=queue)

        result = await DeadLetterConsumer._get_dead_letter_queue(channel)

        assert result is queue

    async def test_channel_closed_raises_queue_not_found(self, mocker):
        channel = mocker.MagicMock()
        channel.declare_queue = mocker.AsyncMock(
            side_effect=aio_pika.exceptions.ChannelClosed(404, "NOT_FOUND")
        )

        with pytest.raises(QueueNotFoundError):
            await DeadLetterConsumer._get_dead_letter_queue(channel)


@pytest.fixture
def dlq_queue(mocker):
    """A mock DLQ aio-pika queue with a name."""
    queue = mocker.MagicMock(spec=aio_pika.abc.AbstractQueue)
    queue.name = "events.dlq"
    return queue


@pytest.fixture
def mock_browse_channel(mocker, dlq_queue):
    """
    Mock the connector boundary ``browse`` uses for channel checkout.

    Wires ``get_channel_direct`` -> a channel mock, a no-op ``return_channel``,
    and points ``_get_dead_letter_queue`` at the DLQ queue so ``browse`` does
    not touch the real broker.

    ``get_channel_direct`` is patched with ``autospec=True`` (not a bare
    ``AsyncMock``) so the pool-selection kwarg is checked against the real
    connector signature: a misspelled ``durable_pool`` fails here instead of
    being silently swallowed.
    """
    channel = mocker.MagicMock()
    channel.set_qos = mocker.AsyncMock()
    mocker.patch.object(
        DeadLetterConsumer.connector,
        "get_channel_direct",
        autospec=True,
        return_value=channel,
    )
    mocker.patch.object(DeadLetterConsumer.connector, "return_channel")
    mocker.patch.object(
        DeadLetterConsumer,
        "_get_dead_letter_queue",
        new_callable=AsyncMock,
        return_value=dlq_queue,
    )
    return channel


@pytest.fixture
def mock_browse_with_client_acks(mocker):
    """Replace the inner browse generator so ``browse`` setup can be tested."""

    async def mock_gen(_queue, _ack_tracker, on_cleanup_complete=None):
        try:
            for _ in range(3):
                yield object()
        finally:
            if on_cleanup_complete:
                await on_cleanup_complete()

    mocker.patch.object(
        DeadLetterConsumer,
        "_browse_with_client_acks",
        side_effect=mock_gen,
    )


@pytest.mark.asyncio
class TestBrowseSetup:
    """``browse`` checks out a channel, sets prefetch, and finds the DLQ."""

    @pytest_asyncio.fixture(autouse=True)
    async def connect(self):
        await DeadLetterConsumer.connector.connect()

    async def test_returns_generator_tracker_and_channel(
        self, mock_browse_channel, mock_browse_with_client_acks
    ):
        generator, ack_tracker, channel = await DeadLetterConsumer.browse()

        assert isinstance(ack_tracker, AckTracker)
        assert channel is mock_browse_channel

        await generator.aclose()

    async def test_acquires_browse_channel_from_durable_pool(
        self, mock_browse_channel, mock_browse_with_client_acks
    ):
        generator, _, _ = await DeadLetterConsumer.browse()
        await generator.aclose()

        DeadLetterConsumer.connector.get_channel_direct.assert_awaited_once_with(
            durable_pool=True
        )

    async def test_full_browse_lifecycle_takes_exactly_one_checkout(
        self, mock_browse_channel, mock_browse_with_client_acks
    ):
        """
        REPLAY republishes on the browse channel, so the whole browse lifecycle
        must hold exactly ONE durable-pool checkout and release it once.
        """
        generator, _, _ = await DeadLetterConsumer.browse()
        async for _ in generator:
            pass
        await generator.aclose()

        assert DeadLetterConsumer.connector.get_channel_direct.await_count == 1
        assert DeadLetterConsumer.connector.return_channel.call_count == 1

    async def test_sets_configured_prefetch_count(
        self, mock_browse_channel, mock_browse_with_client_acks, mocker
    ):
        mocker.patch.object(
            _settings_module.settings, "dead_letter_browse_prefetch_count", 3
        )

        generator, _, _ = await DeadLetterConsumer.browse()
        await generator.aclose()

        mock_browse_channel.set_qos.assert_awaited_once_with(prefetch_count=3)

    async def test_returns_channel_to_pool_on_cleanup(
        self, mock_browse_channel, mock_browse_with_client_acks
    ):
        generator, _, _ = await DeadLetterConsumer.browse()
        await anext(generator)
        await generator.aclose()

        DeadLetterConsumer.connector.return_channel.assert_called_once_with(
            mock_browse_channel
        )

    async def test_releases_channel_when_set_qos_raises(self, mock_browse_channel):
        mock_browse_channel.set_qos.side_effect = RuntimeError("qos boom")

        with pytest.raises(RuntimeError, match="qos boom"):
            await DeadLetterConsumer.browse()

        DeadLetterConsumer.connector.return_channel.assert_called_once_with(
            mock_browse_channel
        )

    async def test_releases_channel_when_get_queue_raises(
        self, mock_browse_channel, mocker
    ):
        mocker.patch.object(
            DeadLetterConsumer,
            "_get_dead_letter_queue",
            new_callable=AsyncMock,
            side_effect=QueueNotFoundError(queue_name="events.dlq"),
        )

        with pytest.raises(QueueNotFoundError):
            await DeadLetterConsumer.browse()

        DeadLetterConsumer.connector.return_channel.assert_called_once_with(
            mock_browse_channel
        )


@pytest.fixture
def mock_consume_boundary(mocker, dlq_queue):
    """
    Drive the REAL ``_browse_with_client_acks`` against a fake message pump.

    ``_start_consuming_from_queue`` is replaced with one that returns a real
    ``asyncio.Queue`` (pre-loaded by the test) plus a consumer tag; the
    cancel/prefetch-nack cleanup helpers are stubbed so no broker is touched.
    Returns the asyncio.Queue so a test can enqueue mock messages.
    """
    messages_queue = asyncio.Queue()

    async def fake_start(_queue):
        return messages_queue, PUMP_CONSUMER_TAG

    mocker.patch.object(
        DeadLetterConsumer,
        "_start_consuming_from_queue",
        new_callable=AsyncMock,
        side_effect=fake_start,
    )
    mocker.patch.object(
        DeadLetterConsumer, "_cancel_queue_consumer", new_callable=AsyncMock
    )
    mocker.patch.object(
        DeadLetterConsumer, "_nack_prefetched_messages", new_callable=AsyncMock
    )
    return messages_queue


@pytest.mark.asyncio
class TestBrowseWithClientAcks:
    """The real DLQ browse generator decodes, tracks, and yields records."""

    @pytest_asyncio.fixture(autouse=True)
    async def not_closing(self):
        RabbitMQBase.connector.closing = False

    async def test_yields_decoded_dead_letter_record(
        self, mock_consume_boundary, dlq_queue, dead_lettered_message
    ):
        mock_consume_boundary.put_nowait(dead_lettered_message)
        tracker = AckTracker()
        gen = DeadLetterConsumer._browse_with_client_acks(dlq_queue, tracker)

        record = await anext(gen)
        await gen.aclose()

        assert record.origin_queue == "origin-queue"
        assert record.reason == "REASON_DELIVERY_LIMIT"

    async def test_assigns_delivery_tag_from_tracker(
        self, mock_consume_boundary, dlq_queue, make_dead_lettered_message
    ):
        message = make_dead_lettered_message(delivery_tag=42)
        mock_consume_boundary.put_nowait(message)
        tracker = AckTracker()
        gen = DeadLetterConsumer._browse_with_client_acks(dlq_queue, tracker)

        record = await anext(gen)
        await gen.aclose()

        assert record.delivery_tag == 42

    async def test_tracks_the_message_in_flight(
        self, mock_consume_boundary, dlq_queue, dead_lettered_message
    ):
        mock_consume_boundary.put_nowait(dead_lettered_message)
        tracker = AckTracker()
        gen = DeadLetterConsumer._browse_with_client_acks(dlq_queue, tracker)

        await anext(gen)
        await gen.aclose()

        # The raw message is held so the servicer can later action it.
        assert tracker.in_flight_count == 1

    async def test_returns_immediately_when_connector_closing(
        self, mock_consume_boundary, dlq_queue, mocker
    ):
        mocker.patch.object(DeadLetterConsumer.connector, "closing", True)
        tracker = AckTracker()
        gen = DeadLetterConsumer._browse_with_client_acks(dlq_queue, tracker)

        records = [record async for record in gen]

        assert records == []

    async def test_nacks_in_flight_message_on_cleanup(
        self, mock_consume_boundary, dlq_queue, dead_lettered_message
    ):
        mock_consume_boundary.put_nowait(dead_lettered_message)
        tracker = AckTracker()
        gen = DeadLetterConsumer._browse_with_client_acks(dlq_queue, tracker)

        await anext(gen)
        await gen.aclose()
        # The cleanup runs in a background task; let it complete.
        await asyncio.sleep(0)

        dead_lettered_message.nack.assert_awaited_once_with(requeue=True)

    async def test_runs_cleanup_callback_on_exit(
        self, mock_consume_boundary, dlq_queue, dead_lettered_message
    ):
        mock_consume_boundary.put_nowait(dead_lettered_message)
        tracker = AckTracker()
        cleanup = AsyncMock()
        gen = DeadLetterConsumer._browse_with_client_acks(
            dlq_queue, tracker, on_cleanup_complete=cleanup
        )

        await anext(gen)
        await gen.aclose()
        await asyncio.sleep(0)

        cleanup.assert_awaited_once()

    async def test_nacks_message_when_closing_after_dequeue(
        self, mock_consume_boundary, dlq_queue, dead_lettered_message, mocker
    ):
        # The connector starts open so the top-of-loop check passes, then flips
        # to closing as soon as the message is dequeued: the dequeued message
        # must be nacked back to the DLQ rather than yielded. The seam is
        # ``get_nowait`` because a queue with a buffered delivery takes
        # ``_next_message``'s fast path and never reaches ``get``.
        original_get_nowait = mock_consume_boundary.get_nowait

        def get_nowait_then_close():
            message = original_get_nowait()
            DeadLetterConsumer.connector.closing = True
            return message

        mocker.patch.object(
            mock_consume_boundary, "get_nowait", side_effect=get_nowait_then_close
        )
        mock_consume_boundary.put_nowait(dead_lettered_message)
        tracker = AckTracker()
        gen = DeadLetterConsumer._browse_with_client_acks(dlq_queue, tracker)

        records = [record async for record in gen]

        assert records == []
        dead_lettered_message.nack.assert_awaited_once_with(requeue=True)

    async def test_returns_at_bottom_of_loop_when_closing_after_yield(
        self, mock_consume_boundary, dlq_queue, make_dead_lettered_message
    ):
        # After a record is successfully yielded, the bottom-of-loop re-check
        # must observe the connector closing and stop the browse generator
        # without dequeuing the next message. Enqueue two messages but flip
        # closing right after consuming the first record: only one is yielded.
        first = make_dead_lettered_message(delivery_tag=1)
        second = make_dead_lettered_message(delivery_tag=2)
        mock_consume_boundary.put_nowait(first)
        mock_consume_boundary.put_nowait(second)
        tracker = AckTracker()
        gen = DeadLetterConsumer._browse_with_client_acks(dlq_queue, tracker)

        records = []
        record = await anext(gen)
        records.append(record)
        # Flip closing so the bottom-of-loop check returns instead of
        # dequeuing the second message.
        DeadLetterConsumer.connector.closing = True
        with pytest.raises(StopAsyncIteration):
            await anext(gen)

        assert [r.delivery_tag for r in records] == [1]

    async def test_reraises_decode_error_when_not_closing(
        self, mock_consume_boundary, dlq_queue, dead_lettered_message, mocker
    ):
        # A decode failure while the stream is live must propagate so the
        # servicer surfaces it (no silent swallow).
        mocker.patch(
            "app.lib.rabbitmq.dead_letter_consumer.decode_dead_lettered_message",
            side_effect=ValueError("bad body"),
        )
        mock_consume_boundary.put_nowait(dead_lettered_message)
        tracker = AckTracker()
        gen = DeadLetterConsumer._browse_with_client_acks(dlq_queue, tracker)

        with pytest.raises(ValueError, match="bad body"):
            await anext(gen)

    async def test_swallows_decode_error_when_closing(
        self, mock_consume_boundary, dlq_queue, dead_lettered_message, mocker
    ):
        # If the stream is being torn down, a decode error during shutdown is
        # swallowed and the generator exits cleanly instead of raising.
        def raise_after_closing(_message):
            DeadLetterConsumer.connector.closing = True
            raise ValueError("bad body during shutdown")

        mocker.patch(
            "app.lib.rabbitmq.dead_letter_consumer.decode_dead_lettered_message",
            side_effect=raise_after_closing,
        )
        mock_consume_boundary.put_nowait(dead_lettered_message)
        tracker = AckTracker()
        gen = DeadLetterConsumer._browse_with_client_acks(dlq_queue, tracker)

        records = [record async for record in gen]

        assert records == []


@pytest.mark.asyncio
class TestBrowseWithClientAcksConsumerLoss:
    """
    A broker-cancelled consumer ends the DLQ browse stream (issue b.9gj).

    The browse loop shares the durable consume loops' defect and their fix:
    without the liveness poll it blocked forever on ``messages_queue.get()``
    while the operator's browse stream reported healthy against a DLQ nothing
    was attached to. The cases mirror ``TestConsumerLoss`` in
    ``test_rabbitmq_base.py``; the two modules cannot import each other, so
    this arm is kept in place rather than parameterized across all three.
    """

    @pytest_asyncio.fixture(autouse=True)
    async def not_closing(self):
        DeadLetterConsumer.connector.closing = False

    @pytest_asyncio.fixture
    async def browsing(
        self, mock_consume_boundary, cancellable_queue, dead_lettered_message
    ):
        """
        A live browse stream that has already yielded, with its copy in flight.

        A cleanup assertion about the outstanding DLQ copy is vacuous unless
        there IS one outstanding, and every case here turns on what happens to
        it.
        """
        generator = DeadLetterConsumer._browse_with_client_acks(
            cancellable_queue, AckTracker()
        )
        mock_consume_boundary.put_nowait(dead_lettered_message)
        record = await anext(generator)

        yield SimpleNamespace(
            generator=generator,
            queue=cancellable_queue,
            channel=cancellable_queue.channel,
            message=dead_lettered_message,
            record=record,
        )

    async def test_the_stream_ends_with_a_reconnect_signal_on_a_broker_cancel(
        self, browsing, fast_liveness_poll, queue_exists
    ):
        browsing.channel.cancel_consumer(PUMP_CONSUMER_TAG)

        await expect_terminal_error(browsing.generator, ConsumerCancelledError)

    async def test_the_stream_ends_with_not_found_when_the_dlq_was_deleted(
        self, browsing, fast_liveness_poll, queue_deleted
    ):
        browsing.channel.cancel_consumer(PUMP_CONSUMER_TAG)

        await expect_terminal_error(browsing.generator, QueueNotFoundError)

    async def test_cleanup_skips_the_consumer_cancel(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        browsing.channel.cancel_consumer(PUMP_CONSUMER_TAG)

        await expect_terminal_error(browsing.generator, ConsumerCancelledError)
        await expect_cleanup(background_tasks)

        DeadLetterConsumer._cancel_queue_consumer.assert_not_awaited()

    async def test_cleanup_nacks_the_in_flight_dlq_copy_on_a_live_channel(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        # A per-consumer cancel on a channel that is still OPEN requeues
        # NOTHING, so the DLQ copy is still ours to hand back -- and a
        # dead-lettered message stranded unacked is one an operator can never
        # browse again on this channel.
        browsing.channel.cancel_consumer(PUMP_CONSUMER_TAG)

        await expect_terminal_error(browsing.generator, ConsumerCancelledError)
        await expect_cleanup(background_tasks)

        browsing.message.nack.assert_awaited_once_with(requeue=True)

    async def test_cleanup_skips_the_nacks_when_the_channel_was_closed(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The broker requeued the DLQ copy when it killed the channel, so the
        # nack is a guaranteed failure rather than a safeguard against loss.
        browsing.channel.kill()

        await expect_terminal_error(browsing.generator, ConsumerCancelledError)
        await expect_cleanup(background_tasks)

        browsing.message.nack.assert_not_awaited()

    async def test_cleanup_skips_the_nacks_when_the_channel_reopened_underneath(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The browse loop pins its channel incarnation at consume start for the
        # same reason the durable loops do: an ack-timeout kill or any other
        # channel-fatal fault is followed by an in-place aio-pika reopen well
        # inside one liveness poll, so the wrapper reads open again by the time
        # anything looks. Without the incarnation the DLQ copy would be nacked
        # against a dead underlay.
        browsing.channel.kill()
        browsing.channel.reopen()

        await expect_terminal_error(browsing.generator, ConsumerCancelledError)
        await expect_cleanup(background_tasks)

        browsing.message.nack.assert_not_awaited()

    async def test_cleanup_skips_the_nacks_when_the_dlq_is_gone(
        self, browsing, fast_liveness_poll, queue_deleted, background_tasks
    ):
        browsing.channel.cancel_consumer(PUMP_CONSUMER_TAG)

        await expect_terminal_error(browsing.generator, QueueNotFoundError)
        await expect_cleanup(background_tasks)

        browsing.message.nack.assert_not_awaited()

    async def test_cleanup_still_releases_the_browse_channel(
        self,
        mock_consume_boundary,
        cancellable_queue,
        fast_liveness_poll,
        queue_exists,
        dead_lettered_message,
        background_tasks,
    ):
        # ``browse`` returns its durable-pool checkout from this callback, so
        # skipping it would trade a zombie stream for a leaked channel.
        on_cleanup = AsyncMock()
        generator = DeadLetterConsumer._browse_with_client_acks(
            cancellable_queue, AckTracker(), on_cleanup_complete=on_cleanup
        )
        mock_consume_boundary.put_nowait(dead_lettered_message)
        await anext(generator)
        cancellable_queue.channel.cancel_consumer(PUMP_CONSUMER_TAG)

        await expect_terminal_error(generator, ConsumerCancelledError)
        await expect_cleanup(background_tasks)

        on_cleanup.assert_awaited_once()

    async def test_normal_close_still_cancels_the_consumer(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The control for the skips above: with the consumer still alive, an
        # ordinary browse teardown takes the unchanged path.
        await browsing.generator.aclose()
        await expect_cleanup(background_tasks)

        DeadLetterConsumer._cancel_queue_consumer.assert_awaited_once()

    async def test_a_browse_cancelled_mid_poll_is_not_consumer_loss(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        # An operator closing a quiet browse stream ends it exactly this way.
        # The consumer is alive, so cleanup owes the broker a real
        # basic.cancel.
        await cancel_while_polling(browsing.generator)
        await expect_cleanup(background_tasks)

        DeadLetterConsumer._cancel_queue_consumer.assert_awaited_once()

    async def test_a_browse_cancelled_mid_poll_still_releases_the_channel(
        self,
        mock_consume_boundary,
        cancellable_queue,
        fast_liveness_poll,
        queue_exists,
        dead_lettered_message,
        background_tasks,
    ):
        on_cleanup = AsyncMock()
        generator = DeadLetterConsumer._browse_with_client_acks(
            cancellable_queue, AckTracker(), on_cleanup_complete=on_cleanup
        )
        mock_consume_boundary.put_nowait(dead_lettered_message)
        await anext(generator)

        await cancel_while_polling(generator)
        await expect_cleanup(background_tasks)

        on_cleanup.assert_awaited_once()

    async def test_a_browse_cancelled_mid_poll_reports_only_the_cancellation(
        self, browsing, fast_liveness_poll, queue_exists
    ):
        # The caller must see its own CancelledError, not a
        # ConsumerCancelledError manufactured by a poll that happened to be in
        # flight.
        step = await cancel_while_polling(browsing.generator)

        assert step.cancelled()


@pytest.mark.asyncio
class TestBrowseWithClientAcksShutdownRace:
    """
    A pod rollout that lands on an idle browse stream ends it cleanly.

    The browse loop shares this race with the durable consume loops: the
    connector can start closing while the loop is parked in the idle wait, or
    while the liveness check is mid-classification. Neither is a zombie
    consumer, so neither may reach the operator as an INTERNAL. Mirrors
    ``TestConsumerLossRacedByShutdown`` in ``test_rabbitmq_base.py``; the two
    modules cannot import each other, so this arm is kept in place.
    """

    @pytest_asyncio.fixture(autouse=True)
    async def not_closing(self):
        DeadLetterConsumer.connector.closing = False

    @pytest_asyncio.fixture
    async def browsing(
        self, mock_consume_boundary, cancellable_queue, dead_lettered_message
    ):
        """A live browse stream that has already yielded, with its copy in flight."""
        generator = DeadLetterConsumer._browse_with_client_acks(
            cancellable_queue, AckTracker()
        )
        mock_consume_boundary.put_nowait(dead_lettered_message)
        record = await anext(generator)

        yield SimpleNamespace(
            generator=generator, message=dead_lettered_message, record=record
        )

    @pytest.fixture(params=["broker cancel", "dlq deleted"])
    def lost_while_shutting_down(self, request, browsing, mocker):
        """
        The liveness check raises just as the connector goes into shutdown.

        Patched at ``_next_message`` because the flip has to land INSIDE the
        call: were the connector already closing, the loop's top-of-loop check
        would return before the check ever ran.
        """
        if request.param == "broker cancel":
            error = ConsumerCancelledError(LIVENESS_QUEUE_NAME, channel_closed=False)
        else:
            error = QueueNotFoundError(queue_name=LIVENESS_QUEUE_NAME)

        async def raise_as_shutdown_begins(*_args):
            DeadLetterConsumer.connector.closing = True
            raise error

        return mocker.patch.object(
            RabbitMQBase,
            "_next_message",
            new_callable=AsyncMock,
            side_effect=raise_as_shutdown_begins,
        )

    async def test_an_idle_browse_ends_without_an_error(
        self, browsing, fast_liveness_poll, queue_exists
    ):
        records = await drain_while_shutting_down(
            browsing.generator, DeadLetterConsumer.connector
        )

        assert records == []

    async def test_an_idle_shutdown_still_cancels_the_consumer(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        await drain_while_shutting_down(
            browsing.generator, DeadLetterConsumer.connector
        )
        await expect_cleanup(background_tasks)

        DeadLetterConsumer._cancel_queue_consumer.assert_awaited_once()

    async def test_an_idle_shutdown_still_nacks_the_in_flight_dlq_copy(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        # A dead-lettered message left unacked on shutdown is one no operator
        # can browse again until the broker's ack timeout releases it.
        await drain_while_shutting_down(
            browsing.generator, DeadLetterConsumer.connector
        )
        await expect_cleanup(background_tasks)

        browsing.message.nack.assert_awaited_once_with(requeue=True)

    async def test_a_consumer_loss_raced_by_shutdown_is_not_surfaced(
        self, browsing, lost_while_shutting_down
    ):
        records = [record async for record in browsing.generator]

        assert records == []

    async def test_a_consumer_loss_raced_by_shutdown_still_cancels_the_consumer(
        self, browsing, lost_while_shutting_down, background_tasks
    ):
        [record async for record in browsing.generator]
        await expect_cleanup(background_tasks)

        DeadLetterConsumer._cancel_queue_consumer.assert_awaited_once()

    async def test_a_consumer_loss_raced_by_shutdown_still_nacks(
        self, browsing, lost_while_shutting_down, background_tasks
    ):
        [record async for record in browsing.generator]
        await expect_cleanup(background_tasks)

        browsing.message.nack.assert_awaited_once_with(requeue=True)


@pytest.mark.asyncio
class TestBrowseCleanupOnAMarkedTracker:
    """
    A tracker marked dead narrows the BROWSE loop's cleanup too (issues
    b.qsa, b.xr7).

    The DLQ browse has the same merge-of-settles shape as the durable
    consume-with-acks stream: a REPLAY / DISCARD / LEAVE that settles against
    a channel the broker has killed fails on the settle side of the
    servicer's merge, ahead of this loop's next idle liveness poll. THIS
    session's tracker (``browse`` hands out one per session) marks itself at
    that failing settle, and the merge then tears this generator down with
    ``consumer_lost_exc`` still None. Mirrors
    ``TestClientAckLoopCleanupOnAMarkedTracker`` in ``test_rabbitmq_base.py``;
    the two modules cannot import each other, so this arm is kept in place.

    These arms mark the tracker and then close the generator with
    ``aclose()``, which pins how the cleanup reads the verdict, not the
    order production reaches it in. The production order -- an action
    failing while this loop is waiting on its next dead-letter -- is pinned
    end to end by ``TestASettleSideChannelDeathNarrowsTheEventSideCleanup``
    in ``tests/app/grpc/test_event_consumer_servicer.py``.
    """

    @pytest_asyncio.fixture(autouse=True)
    async def not_closing(self):
        DeadLetterConsumer.connector.closing = False

    @pytest_asyncio.fixture
    async def browsing(
        self, mock_consume_boundary, cancellable_queue, dead_lettered_message
    ):
        """
        A live browse stream that has already yielded, with its DLQ copy in
        flight and the session tracker exposed.

        The tracker is the seam under test, so it is built here rather than
        inline in the generator call as the neighbouring fixtures do.
        """
        tracker = AckTracker()
        generator = DeadLetterConsumer._browse_with_client_acks(
            cancellable_queue, tracker
        )
        mock_consume_boundary.put_nowait(dead_lettered_message)
        record = await anext(generator)

        yield SimpleNamespace(
            generator=generator,
            tracker=tracker,
            message=dead_lettered_message,
            record=record,
        )

    async def test_cleanup_skips_the_consumer_cancel(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        browsing.tracker.mark_channel_dead()

        await browsing.generator.aclose()
        await expect_cleanup(background_tasks)

        DeadLetterConsumer._cancel_queue_consumer.assert_not_awaited()

    async def test_cleanup_skips_the_nack_of_the_in_flight_dlq_copy(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The broker requeued the DLQ copy when it closed the channel, so the
        # LEAVE-semantics nack here is a guaranteed failure rather than the
        # safeguard against loss it is on a live channel.
        browsing.tracker.mark_channel_dead()

        await browsing.generator.aclose()
        await expect_cleanup(background_tasks)

        browsing.message.nack.assert_not_awaited()

    async def test_cleanup_skips_the_prefetched_nacks(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        browsing.tracker.mark_channel_dead()

        await browsing.generator.aclose()
        await expect_cleanup(background_tasks)

        DeadLetterConsumer._nack_prefetched_messages.assert_not_awaited()

    async def test_cleanup_still_releases_the_browse_channel(
        self,
        mock_consume_boundary,
        cancellable_queue,
        fast_liveness_poll,
        queue_exists,
        dead_lettered_message,
        background_tasks,
    ):
        # ``browse`` returns its durable-pool checkout from this callback, so
        # a narrowing that also skipped it would leak a channel per incident.
        tracker = AckTracker()
        on_cleanup = AsyncMock()
        generator = DeadLetterConsumer._browse_with_client_acks(
            cancellable_queue, tracker, on_cleanup_complete=on_cleanup
        )
        mock_consume_boundary.put_nowait(dead_lettered_message)
        await anext(generator)
        tracker.mark_channel_dead()

        await generator.aclose()
        await expect_cleanup(background_tasks)

        on_cleanup.assert_awaited_once()

    async def test_an_unmarked_tracker_still_cancels_the_consumer(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The control arm: same teardown, tracker left alone, full cleanup.
        await browsing.generator.aclose()
        await expect_cleanup(background_tasks)

        DeadLetterConsumer._cancel_queue_consumer.assert_awaited_once()

    async def test_an_unmarked_tracker_still_nacks_the_in_flight_dlq_copy(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        await browsing.generator.aclose()
        await expect_cleanup(background_tasks)

        browsing.message.nack.assert_awaited_once_with(requeue=True)

    async def test_a_mark_landing_after_the_generator_closed_still_counts(
        self, browsing, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The verdict is read inside the cleanup task, not captured when the
        # task is scheduled: the generator's finally reaches
        # ``create_task(_cleanup())`` and returns before the task body runs.
        # A DLQ action that fails at the same moment the browse is torn down
        # for another reason marks the tracker in exactly that window.
        await browsing.generator.aclose()
        browsing.tracker.mark_channel_dead()

        await expect_cleanup(background_tasks)

        browsing.message.nack.assert_not_awaited()


@pytest.fixture
def shutdown_on_second_dequeue(mock_consume_boundary, mocker):
    """The first DLQ copy stays in flight; the second meets a shutdown."""
    return close_connector_on_dequeue(
        mock_consume_boundary, DeadLetterConsumer.connector, mocker
    )


@pytest_asyncio.fixture
async def browse_shutdown_nack_failure(
    shutdown_on_second_dequeue,
    cancellable_queue,
    make_dead_lettered_message,
    background_tasks,
):
    """
    Run a browse into a shutdown whose nack of the dequeued copy raises.

    A third copy stays buffered behind the failing one, so a healthy
    cleanup would owe it a drain as well as the in-flight copy a nack.

    :returns: A callable taking the injected failure and returning what the
        browse did with it, once the cleanup task has finished.
    """

    async def _run(failure):
        in_flight = make_dead_lettered_message(delivery_tag=1)
        at_shutdown = make_dead_lettered_message(delivery_tag=2)
        at_shutdown.nack.side_effect = failure
        buffered = make_dead_lettered_message(delivery_tag=3)
        for message in (in_flight, at_shutdown, buffered):
            shutdown_on_second_dequeue.put_nowait(message)
        on_cleanup = AsyncMock()
        generator = DeadLetterConsumer._browse_with_client_acks(
            cancellable_queue, AckTracker(), on_cleanup_complete=on_cleanup
        )

        await anext(generator)
        remaining = await asyncio.wait_for(
            drain(generator), timeout=CONSUMER_LOSS_TIMEOUT_S
        )
        await expect_cleanup(background_tasks)

        return SimpleNamespace(
            remaining=remaining, in_flight=in_flight, on_cleanup=on_cleanup
        )

    return _run


@pytest.mark.asyncio
class TestBrowseShutdownNackOnADeadChannel:
    """
    A shutdown nack that finds the DLQ browse's channel dead ends the browse
    cleanly but still narrows the cleanup (issue b.7yv).

    The shutdown nack is the browse loop's only broker operation of its own
    on a delivery it holds: client DLQ actions settle on the servicer's side
    of the merge instead. The connector is closing, so the operator gets no
    error, but the dead-channel error is proof the channel is gone, so the
    cancel and the nacks are skipped. Mirrors
    ``TestShutdownNackOnADeadChannel`` in ``test_rabbitmq_base.py``; the two
    modules cannot import each other, so this arm is kept in place.
    """

    @dead_channel_failures
    async def test_the_browse_ends_without_an_error(
        self, browse_shutdown_nack_failure, make_failure
    ):
        browsing = await browse_shutdown_nack_failure(make_failure())

        assert browsing.remaining == []

    @dead_channel_failures
    async def test_cleanup_skips_the_consumer_cancel(
        self, browse_shutdown_nack_failure, make_failure
    ):
        await browse_shutdown_nack_failure(make_failure())

        DeadLetterConsumer._cancel_queue_consumer.assert_not_awaited()

    @dead_channel_failures
    async def test_cleanup_skips_the_nack_of_the_in_flight_dlq_copy(
        self, browse_shutdown_nack_failure, make_failure
    ):
        browsing = await browse_shutdown_nack_failure(make_failure())

        browsing.in_flight.nack.assert_not_awaited()

    @dead_channel_failures
    async def test_cleanup_skips_the_prefetched_nacks(
        self, browse_shutdown_nack_failure, make_failure
    ):
        await browse_shutdown_nack_failure(make_failure())

        DeadLetterConsumer._nack_prefetched_messages.assert_not_awaited()

    @dead_channel_failures
    async def test_cleanup_still_releases_the_browse_channel(
        self, browse_shutdown_nack_failure, make_failure
    ):
        browsing = await browse_shutdown_nack_failure(make_failure())

        browsing.on_cleanup.assert_awaited_once()

    @unrelated_failures
    async def test_another_shutdown_nack_failure_keeps_the_cleanup_whole(
        self, browse_shutdown_nack_failure, make_failure
    ):
        # The control arm: the channel is alive, so the in-flight DLQ copy
        # still has to go back on the DLQ for the next browse to find.
        browsing = await browse_shutdown_nack_failure(make_failure())

        browsing.in_flight.nack.assert_awaited_once_with(requeue=True)
