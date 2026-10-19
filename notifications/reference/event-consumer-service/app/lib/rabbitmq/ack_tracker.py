# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import aio_pika
from aio_pika.abc import AbstractIncomingMessage

logger = logging.getLogger(__name__)

# The exceptions by which an AMQP operation reports that the channel it was
# issued on is dead. Catching one of these is proof that the channel
# incarnation the operation went to is gone, so the session's deliveries on
# it were requeued by the broker and its consumer no longer exists:
#
# * ChannelInvalidStateError -- the channel wrapper has no usable underlay.
# * ChannelClosed, with its per-reply-code subclasses such as
#   ChannelPreconditionFailed -- the broker closed the channel.
# * AMQPConnectionError, with ConnectionClosed and its subclasses -- the
#   connection carrying the channel closed while the operation was in
#   flight, which takes every channel on it down too. aiormq raises this
#   class at an in-flight operation only when the connection closes; its
#   other raisers are connect-time only. The builtin ConnectionError, its
#   base class, is deliberately NOT included: only aiormq's own family says
#   the AMQP connection closed.
#
# Kept as a named tuple so every `except` that uses it stays a name rather
# than an inline `except (A, B):`, which Black 25 strips the parens from on
# py3.14 (see b.8qg).
DEAD_CHANNEL_EXC = (
    aio_pika.exceptions.ChannelInvalidStateError,
    aio_pika.exceptions.ChannelClosed,
    aio_pika.exceptions.AMQPConnectionError,
)


class AckTracker:
    """
    Tracks in-flight messages for client ack support.

    Each consume session gets its own AckTracker. Messages are tracked
    after being sent to the client, and removed when the client settles
    them -- with an ack, or with a nack that requeues the message for
    redelivery. On disconnect, all unsettled messages are nacked back to
    RabbitMQ for redelivery.

    A tracker also carries the session's channel-dead verdict (see
    :meth:`mark_channel_dead`), which is the settle side's half of the
    signal that decides how the consume loop cleans up.
    """

    def __init__(self):
        self._in_flight: dict[int, AbstractIncomingMessage] = {}
        # Parallel to _in_flight, keyed by the same delivery tag: the decoded
        # x-death-derived origin queue for a dead-lettered message. Populated
        # only when track() is given an origin_queue (the DLQ browse path), so
        # REPLAY can recover it without re-decoding the message body. Evicted in
        # lockstep with _in_flight on ack/nack/nack_all.
        self._origin_queue: dict[int, str] = {}
        # Whether the channel these delivery tags live on is known dead. Set
        # only by mark_channel_dead(); see it for why this is tracker state
        # rather than a local of whoever noticed.
        self._channel_dead = False

    @property
    def in_flight_count(self) -> int:
        return len(self._in_flight)

    @property
    def channel_dead(self) -> bool:
        """
        Whether the channel this session's delivery tags lived on is dead.

        Read by the consume loop's cleanup to narrow itself the same way a
        consumer-loss detection does (see
        ``RabbitMQBase._should_nack_outstanding``).
        """
        return self._channel_dead

    def mark_channel_dead(self) -> None:
        """
        Record that the channel carrying these delivery tags has died.

        A session learns its channel is gone in one of two places. Inside
        the consume generator, the loop's own idle liveness poll or a
        failing broker operation of its own (a skip-ack, a nack, an aged
        ack) records the loss as the loop's ``consumer_lost_exc``. Outside
        it, a client settle -- an ack, a nack, a DLQ action -- goes to the
        broker and fails, which happens FIRST whenever the client is
        settling at the moment of the kill (the flagship ack-timeout case).
        That discovery is made on the settle side of the stream's
        ``merge``, outside the consume generator, so it cannot reach the
        generator's cleanup as a raised exception. When the settle's error
        reaches ``merge``, the merge cancels the event side's pending read,
        if it has one, and whenever the generator is then torn down its
        ``finally`` schedules the cleanup with the loop's own
        ``consumer_lost_exc`` still None.
        Unmarked, that cleanup would believe the channel healthy: it would
        cancel a consumer the broker has already dropped and nack delivery
        tags the broker requeued when it closed the channel -- futile
        round trips that log failures on exactly the incident path.

        Marking the tracker is how that out-of-band verdict gets in-band:
        the tracker is the one object both the settle side and the consume
        loop hold. :meth:`ack` and :meth:`nack` mark it themselves, at the
        failing broker call and before the error leaves the tracker, so
        the mark is in place before ``merge`` can tear the event side
        down. That is direct evidence about the consume channel, not a
        guess: a delivery settles on the channel it arrived on, and
        deliveries only arrive on the incarnation the consumer was started
        on.

        This method is the only writer of the flag, everything runs on one
        event loop, and the flag only ever goes from False to True, so no
        lock is needed. Idempotent -- a second mark changes nothing.
        """
        self._channel_dead = True

    def track(
        self,
        message: AbstractIncomingMessage,
        origin_queue: str | None = None,
    ) -> int:
        """
        Track a message for later acking. Returns the delivery tag.

        :param origin_queue: For dead-lettered messages, the decoded
            x-death-derived origin queue. Cached keyed by delivery tag so a
            later REPLAY can recover it via :meth:`origin_queue` without a
            second full decode of the message body. This MUST be the decoded
            origin queue (never client input) to preserve the replay-target
            invariant in ``DeadLetterConsumer.replay``.
        """
        tag = message.delivery_tag
        self._in_flight[tag] = message
        if origin_queue is not None:
            self._origin_queue[tag] = origin_queue
        return tag

    def origin_queue(self, delivery_tag: int) -> str | None:
        """
        Return the cached origin queue for a tracked dead-lettered message.

        Returns ``None`` for an unknown / already-actioned delivery tag (or a
        message tracked without an origin queue) so the caller can skip without
        crashing the stream.
        """
        return self._origin_queue.get(delivery_tag)

    def get(self, delivery_tag: int) -> AbstractIncomingMessage:
        """
        Return a tracked message by its delivery tag without removing it.

        Used by REPLAY to read the raw message body for republish before
        deciding (based on the publish-confirm outcome) whether to ack or
        nack-requeue the DLQ copy.

        :raises KeyError: if the delivery tag is not tracked
        """
        return self._in_flight[delivery_tag]

    async def ack(self, delivery_tag: int) -> None:
        """
        Ack a tracked message by its delivery tag.

        The tag is popped before the broker call, so it leaves the tracker
        whether or not the ack succeeds, and an unknown tag raises before
        anything reaches the broker.

        :raises KeyError: if the delivery tag is not tracked. The tracker
            is not marked.
        :raises DEAD_CHANNEL_EXC: if the delivery's channel is dead. The
            tracker is marked (see :meth:`mark_channel_dead`) before the
            same exception object is re-raised.
        """
        message = self._in_flight.pop(delivery_tag)
        self._origin_queue.pop(delivery_tag, None)
        try:
            await message.ack()
        except DEAD_CHANNEL_EXC:
            self.mark_channel_dead()
            raise

    async def nack(self, delivery_tag: int, requeue: bool = True) -> None:
        """
        Nack a single tracked message by its delivery tag.

        Two callers, both with ``requeue=True``. A client nack on the durable
        consume-with-acks stream returns a message the client failed to
        process to its queue for redelivery. REPLAY's LEAVE-on-failure path
        nacks the DLQ copy when a republish cannot be confirmed, so it stays
        in the DLQ (no loss).

        The tag is popped before the message is nacked, so a client-nacked
        delivery cannot be nacked a second time by :meth:`nack_all` during
        stream cleanup, and an unknown tag raises before anything reaches
        the broker.

        :raises KeyError: if the delivery tag is not tracked. The tracker
            is not marked.
        :raises DEAD_CHANNEL_EXC: if the delivery's channel is dead. The
            tracker is marked (see :meth:`mark_channel_dead`) before the
            same exception object is re-raised.
        """
        message = self._in_flight.pop(delivery_tag)
        self._origin_queue.pop(delivery_tag, None)
        try:
            await message.nack(requeue=requeue)
        except DEAD_CHANNEL_EXC:
            self.mark_channel_dead()
            raise

    async def nack_all(self) -> None:
        """
        Nack all in-flight messages so they are requeued in RabbitMQ.
        Called on client disconnect or stream cleanup.

        Drains by popping rather than iterating ``_in_flight`` in place.
        Each nack awaits, and cleanup runs in a detached task, so a client
        ack or nack can land on the tracker during any of those awaits --
        iterating the live dict would raise ``RuntimeError: dictionary
        changed size during iteration`` and abandon every message not yet
        reached, unacked and unnacked. Popping each entry BEFORE its await
        also settles it exactly once: a concurrent client ack for the same
        tag now takes the tracker's unknown-tag KeyError path instead of
        double-settling a delivery.

        On a channel known dead there is nothing to nack: the broker
        requeued every unacked delivery when it closed the channel, and the
        tags themselves died with that incarnation. The messages are
        dropped instead. Cleanup already skips this call on that path (see
        ``RabbitMQBase._should_nack_outstanding``); the check here is what
        makes the tracker safe for any other caller to drain.
        """
        if self._channel_dead:
            logger.debug(
                "Abandoning %s in-flight message(s) on a dead channel; the "
                "broker requeued them when it closed the channel",
                len(self._in_flight),
            )
            self._in_flight.clear()
            self._origin_queue.clear()
            return

        while self._in_flight:
            tag, message = self._in_flight.popitem()
            self._origin_queue.pop(tag, None)
            try:
                await message.nack(requeue=True)
            except Exception:
                logger.exception(
                    "Failed to nack in-flight message with delivery_tag=%s",
                    tag,
                )
        self._origin_queue.clear()
