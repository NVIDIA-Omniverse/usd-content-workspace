# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Shared test doubles for the consumer-liveness poll (issue b.9gj).

aio-pika surfaces no notification when the broker cancels our consumer or
kills the channel it lives on, so the consume/browse loops poll a liveness
predicate off their idle path. The fakes here model the only three things that
predicate reads: whether the aio-pika channel is closed, whether our consumer
tag is still in the underlying aiormq channel's ``consumers`` registry, and
WHICH underlay incarnation that registry belongs to -- a RobustChannel reopens
in place, so the wrapper's own state cannot say whether the channel a consumer
was started on has since died.

They live here rather than in one test module because three loops share the
behaviour -- the two durable consume loops in ``rabbitmq_base`` and the DLQ
browse loop in ``dead_letter_consumer`` -- and their test modules cannot
import from each other. The pytest fixtures built on these doubles are in
``tests/app/lib/rabbitmq/conftest.py``.

``PumpedQueue`` is the queue end of the rigs that drive those loops end to
end, from the gRPC servicer tests and the REST stream tests alike.
"""

import asyncio
from unittest.mock import AsyncMock

import pytest

# The consumer tag the pumped-boundary fixtures hand the consume loops. The
# fakes must agree with it, otherwise a "tag present" case would silently be
# a "tag absent" one.
PUMP_CONSUMER_TAG = "consumer-tag-1"

LIVENESS_QUEUE_NAME = "event-queue:liveness-qid"

# The consume loops wrap ``messages_queue.get()`` in an ``asyncio.wait_for``
# keyed on ``settings.consumer_liveness_poll_seconds``, so any fixture that
# swaps the whole ``settings.settings`` singleton for a Mock must supply a
# REAL float or a MagicMock reaches ``wait_for`` as the timeout. This value is
# far longer than any unit test runs: those tests exercise the ack paths, so
# the liveness poll must never fire underneath them.
IDLE_POLL_NEVER_FIRES_SECONDS = 30.0

# The opposite knob, for the tests that ARE about the liveness poll: short
# enough that the idle branch is reached immediately.
IDLE_POLL_FIRES_IMMEDIATELY_SECONDS = 0.01

# A generator whose consumer is gone must terminate promptly. This ceiling is
# generous next to the poll interval, so it never flakes -- but it means a
# regression fails as a NAMED timeout instead of hanging the suite. That
# matters here: before the liveness poll landed, the loops blocked forever on
# ``messages_queue.get()``, which is exactly the hang this bounds.
CONSUMER_LOSS_TIMEOUT_S = 5.0


class FakeConnection:
    """
    The connector's shared AMQP connection, reduced to what liveness reads.

    ``connected`` is a REAL ``asyncio.Event`` on purpose. It is the
    load-bearing half of ``_connection_is_down``, and on a plain ``MagicMock``
    connection ``connection.connected.is_set()`` returns a truthy auto-created
    Mock -- so every case would read "connection up" and the down-path tests
    would pass without the predicate ever consulting the event.

    ``reconnecting`` is modelled because the predicate must NOT consult it:
    aio-pika holds that reconnect lock for a further ``reconnect_interval``
    after a successful reconnect, so it reads True on a connection that is
    already back up.
    """

    def __init__(self, *, is_closed=False, connected=True, reconnecting=False):
        self.is_closed = is_closed
        self.connected = asyncio.Event()
        if connected:
            self.connected.set()
        self.reconnecting = reconnecting


class FakeUnderlayChannel:
    """
    The aiormq channel behind an aio-pika one.

    Two things matter here. ``consumers`` is the registry aiormq pops our tag
    out of when it handles the broker's ``Basic.Cancel``, which is the sole
    in-process trace a broker-initiated cancel leaves behind. And the OBJECT
    ITSELF is the channel's incarnation identity: ``Channel._open`` builds a
    fresh underlay on every open, so a new instance here means the channel our
    consumer was living on is gone.
    """

    def __init__(self, consumer_tags):
        self.consumers = {tag: object() for tag in consumer_tags}


class FakeLivenessChannel:
    """An aio-pika channel exposing only what the liveness check reads."""

    def __init__(self, *, is_closed=False, consumer_tags=(), underlay_error=None):
        self.is_closed = is_closed
        # Named ``_channel`` to match aio-pika, where that private attribute IS
        # the underlay. ``RabbitMQBase._channel_incarnation`` reads it
        # synchronously -- an attribute read cannot suspend, which is what makes
        # the capture atomic with the consume it follows -- so a fake that
        # exposed the underlay only through ``get_underlay_channel`` would
        # report "incarnation unknown" forever and quietly degrade every loop
        # test to the no-incarnation fallback path.
        self._channel = FakeUnderlayChannel(consumer_tags)
        self._underlay_error = underlay_error
        self.underlay_call_count = 0

    async def get_underlay_channel(self):
        self.underlay_call_count += 1
        if self._underlay_error is not None:
            raise self._underlay_error
        return self._channel

    def cancel_consumer(self, consumer_tag):
        """Model the broker's Basic.Cancel: aiormq drops the tag."""
        self._channel.consumers.pop(consumer_tag, None)

    def kill(self):
        """
        Model a channel-fatal fault: the channel closes, its consumers die.

        The other half of the consumer-loss space, and the half that decides
        cleanup: the broker requeues every unacked delivery when it closes a
        channel, so the loops must NOT nack after this -- whereas after a bare
        ``cancel_consumer`` on a live channel they must.
        """
        self.is_closed = True
        self._channel.consumers.clear()

    def reopen(self, consumer_tags=()):
        """
        Model aio-pika's in-place RobustChannel reopen after a channel death.

        This is the half of the flagship ack-timeout kill that the wrapper
        hides. Measured against a live broker, the close is followed ~75 ms
        later by a reopen that reuses the SAME wrapper object, while the
        liveness poll runs on a ~1 s cadence -- so ``is_closed`` reads False
        again by the time anything looks. Only the underlay is new, and with
        production's ``robust=False`` queues its consumer registry comes back
        empty, so the tag stays absent.

        Defaults to reopening WITHOUT our tag, the production shape. Passing
        the tag back models the (contrived) case where only the incarnation
        distinguishes a dead channel from a live one.
        """
        self.is_closed = False
        self._channel = FakeUnderlayChannel(consumer_tags)


class FakeLivenessQueue:
    """An aio-pika queue reduced to the ``channel``/``name`` pair."""

    def __init__(self, channel, name=LIVENESS_QUEUE_NAME):
        self.channel = channel
        self.name = name


class PumpedQueue:
    """
    An aio-pika queue reduced to what the REAL consume loops drive, for
    rigs that run them end to end through the servicer or the REST stream.

    ``consume`` records the delivery callback the loop registers -- the real
    ``_start_consuming_from_queue`` builds the pump queue that callback
    feeds -- so the test delivers by calling it, as aiormq would. ``cancel``
    is the ``basic.cancel`` the loop's cleanup sends on a healthy channel. It
    is a plain ``AsyncMock`` that completes without suspending, which a real
    cancel's RPC does not.
    """

    def __init__(self, name):
        self.name = name
        # Read by the loop's channel-incarnation capture at consume start.
        self.channel = FakeLivenessChannel(consumer_tags=(PUMP_CONSUMER_TAG,))
        self.cancel = AsyncMock()
        self.consuming = asyncio.Event()
        self.deliver = None

    async def consume(self, callback):
        self.deliver = callback
        self.consuming.set()
        return PUMP_CONSUMER_TAG


def close_connector_on_dequeue(messages_queue, connector, mocker, n=2):
    """
    Flip ``connector`` into shutdown as the ``n``-th delivery is dequeued.

    The deliveries before it are handled normally. The ``n``-th is then
    dequeued with the connector already closing, which is a loop's
    shutdown-nack path. The seam is ``get_nowait`` because a queue with a
    buffered delivery takes ``_next_message``'s fast path and never reaches
    ``get``.

    :param messages_queue: The pump queue a pumped-boundary fixture hands
        the loop.
    :param connector: The connector the loop under test reads ``closing``
        from.
    :param mocker: The test's ``mocker``, which undoes the patch.
    :param n: Which dequeue, counting from 1, starts the shutdown.
    :returns: ``messages_queue``, for the test to enqueue deliveries on.
    """
    original_get_nowait = messages_queue.get_nowait
    dequeued = []

    def _get_nowait_then_close():
        message = original_get_nowait()
        dequeued.append(message)
        if len(dequeued) == n:
            connector.closing = True
        return message

    mocker.patch.object(
        messages_queue, "get_nowait", side_effect=_get_nowait_then_close
    )
    return messages_queue


async def drain(gen):
    """Read ``gen`` to its end and return every record it yielded."""
    return [record async for record in gen]


async def expect_terminal_error(gen, expected_error):
    """Require the generator's next step to raise ``expected_error`` promptly."""
    try:
        with pytest.raises(expected_error):
            await asyncio.wait_for(anext(gen), timeout=CONSUMER_LOSS_TIMEOUT_S)
    except asyncio.TimeoutError:
        pytest.fail(
            f"the consume generator did not end with {expected_error.__name__} "
            f"within {CONSUMER_LOSS_TIMEOUT_S}s; a stream whose AMQP consumer "
            "is gone must not stay open (issue b.9gj)"
        )


async def cancel_while_polling(gen):
    """
    Cancel the stream while its loop is parked inside the idle liveness poll.

    This is how a quiet stream normally ends in production: the client goes
    away and the task driving the generator is cancelled, mid-wait. It must
    stay an ORDINARY exit -- the consumer is alive, so cleanup owes the broker
    a real ``basic.cancel`` -- and the poll must not turn a cancellation into
    a consumer-loss error.

    The sleep is what puts the cancellation inside the wait rather than before
    the loop ever reached it, so the case cannot silently degrade into
    "cancelled a generator that had not started".
    """
    step = asyncio.ensure_future(anext(gen))
    await asyncio.sleep(IDLE_POLL_FIRES_IMMEDIATELY_SECONDS * 3)
    step.cancel()

    done, _pending = await asyncio.wait({step}, timeout=CONSUMER_LOSS_TIMEOUT_S)
    if not done:
        pytest.fail(
            "the consume generator did not unwind within "
            f"{CONSUMER_LOSS_TIMEOUT_S}s of the stream being cancelled while "
            "it waited on the idle liveness poll"
        )
    return step


async def drain_while_shutting_down(gen, connector):
    """
    Start a graceful shutdown while the loop sits inside the idle poll.

    This is how a quiet stream ends on every pod rollout: nothing is flowing,
    so the loop is parked in ``_next_message``'s wait when ``start_servers``
    marks the connector closing. The stream must simply END there -- an idle
    stream has no delivery to hand back and no error to report -- rather than
    re-entering the wait and polling until gRPC cancels it.

    The sleep is what puts the flag flip inside the wait rather than before
    the loop ever reached it, so the case cannot silently degrade into
    "returned at the top-of-loop check", which a different test already covers.

    :returns: The records yielded after the shutdown began, which must be none.
    """
    remaining = asyncio.ensure_future(drain(gen))
    await asyncio.sleep(IDLE_POLL_FIRES_IMMEDIATELY_SECONDS * 3)
    connector.closing = True

    try:
        return await asyncio.wait_for(remaining, timeout=CONSUMER_LOSS_TIMEOUT_S)
    except asyncio.TimeoutError:
        pytest.fail(
            "the consume generator did not end within "
            f"{CONSUMER_LOSS_TIMEOUT_S}s of the connector being marked closing "
            "while it waited on the idle liveness poll; an idle stream must "
            "honour a graceful shutdown rather than poll until gRPC cancels it"
        )


async def expect_cleanup(background_tasks):
    """
    Await the loop's cleanup task, bounded so a stall fails by name.

    Every loop schedules cleanup with ``asyncio.create_task`` and the
    delayed-ack loop cancels its idle timer first, so the last captured task
    is always the cleanup one.
    """
    try:
        await asyncio.wait_for(background_tasks[-1], timeout=CONSUMER_LOSS_TIMEOUT_S)
    except asyncio.TimeoutError:
        pytest.fail(
            f"the consume loop's cleanup task did not finish within "
            f"{CONSUMER_LOSS_TIMEOUT_S}s; the channel it releases would leak"
        )
