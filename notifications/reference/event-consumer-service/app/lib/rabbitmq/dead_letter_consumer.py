# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Manual-ack browse consumer over the single global dead-letter queue (DLQ).

SR-3.5 fixes the browse primitive as a manual-ack consumer on the quorum DLQ
with a bounded prefetch (a small page), NOT ``basic_get``: a consumer gives
stable per-message session delivery tags for REPLAY / DISCARD / LEAVE,
preserves requeue-on-LEAVE, and composes with the ``merge`` pattern
(``app.lib.streams``). Concurrent operators and multiple replicas are competing
consumers on the single global DLQ (each message delivered to one session at a
time); the prefetch bound caps how many DLQ messages one session holds
in-flight.

Epic 1 already declared the DLQ (durable quorum, self-bounded, bound to the
DLX) in ``EventConsumerConnector._declare_dead_letter_topology``. This consumer
**browses** that existing queue with ``passive=True`` and MUST NOT re-declare
it or its arguments.

Unlike the durable consume path, the DLQ browse path does NOT run a per-event
``authz_check`` while browsing — the coarse permission gates the whole RPC.
Per-event ``authz_check`` still applies on the eventual re-consume after a
REPLAY, unchanged and elsewhere.
"""

import asyncio
import enum
import logging
from dataclasses import dataclass
from typing import AsyncGenerator

import aio_pika
from notification_common.errors.errors import QueueNotFoundError

from app.config import settings
from app.lib.rabbitmq.ack_tracker import DEAD_CHANNEL_EXC, AckTracker
from app.lib.rabbitmq.dead_letter import (
    DecodedDeadLetter,
    decode_dead_lettered_message,
)
from app.lib.rabbitmq.rabbitmq_base import ConsumerCancelledError, RabbitMQBase

logger = logging.getLogger(__name__)


class ReplayStatus(enum.Enum):
    """
    Discriminator for the structured outcome of a single REPLAY action.

    The REPLAY handler returns the outcome rather than logging-and-swallowing
    so the servicer can surface an operator-facing ``ActionOutcome`` on the
    stream (SR-3.4). The servicer maps each status to its proto ``Outcome``:

    - :attr:`REPLAYED` -> ``Outcome.REPLAYED``
    - :attr:`ORIGIN_QUEUE_DELETED` -> ``Outcome.ORIGIN_QUEUE_DELETED``
    - :attr:`REPLAY_FAILED` -> ``Outcome.REPLAY_FAILED``
    """

    REPLAYED = "replayed"
    ORIGIN_QUEUE_DELETED = "origin_queue_deleted"
    REPLAY_FAILED = "replay_failed"


@dataclass
class ReplayResult:
    """
    Structured, operator-facing outcome of a single REPLAY action.

    The handler's return value is the operator-facing signal: it lets the
    servicer distinguish REPLAYED vs ORIGIN_QUEUE_DELETED vs REPLAY_FAILED so
    it can emit the right ``ActionOutcome`` report. No REPLAY failure path is
    silently logged-only — every path returns one of these.

    :param status: Which terminal state the REPLAY reached.
    :param origin_queue: The origin queue name (from ``x-death``) the republish
        targeted. Carried on every result; it is the missing queue name on the
        :attr:`ReplayStatus.ORIGIN_QUEUE_DELETED` path.
    :param detail: A short human-readable note. ``None`` on the success path;
        carries the failure description on the
        :attr:`ReplayStatus.REPLAY_FAILED` path.
    """

    status: ReplayStatus
    origin_queue: str
    detail: str | None = None


class ActionStatus(enum.Enum):
    """
    Discriminator for the structured outcome of a single DISCARD or LEAVE.

    Parallels :class:`ReplayStatus` so the servicer can map every non-replay
    per-tag action to a proto ``Outcome`` uniformly (SR-3.4):

    - :attr:`DISCARDED` -> ``Outcome.DISCARDED`` (DISCARD acked the DLQ copy)
    - :attr:`LEFT` -> ``Outcome.LEFT`` (LEAVE nacked-requeued the DLQ copy)
    - :attr:`UNKNOWN_TAG` -> the servicer surfaces or skips an
      unknown/already-actioned delivery tag without crashing the stream
    """

    DISCARDED = "discarded"
    LEFT = "left"
    UNKNOWN_TAG = "unknown_tag"


@dataclass
class ActionResult:
    """
    Structured, operator-facing outcome of a single DISCARD or LEAVE action.

    Modeled on :class:`ReplayResult` so the servicer maps DISCARD ->
    ``Outcome.DISCARDED`` and LEAVE -> ``Outcome.LEFT`` the same way it maps a
    :class:`ReplayResult`. An unknown/already-actioned delivery tag yields an
    :attr:`ActionStatus.UNKNOWN_TAG` result (no exception escapes the handler),
    matching the ack-unknown-tag behavior of ``_listen_for_durable_acks``: it
    is logged and the result is returned so the servicer can surface or skip it
    without crashing the stream.

    :param status: Which terminal state the action reached.
    :param delivery_tag: The session delivery tag the action targeted.
    :param detail: A short human-readable note. ``None`` on the success paths;
        carries the description on the :attr:`ActionStatus.UNKNOWN_TAG` path.
    """

    status: ActionStatus
    delivery_tag: int
    detail: str | None = None


class DeadLetterConsumer(RabbitMQBase):
    """
    Manual-ack browse consumer over the single global DLQ.

    Kept separate from ``DurableQueueConsumer`` so DLQ-specific semantics
    (browse a passively-declared global queue by name, no per-event authz,
    requeue-on-disconnect) stay distinct from the durable-queue consume path,
    while still reusing ``RabbitMQBase``'s channel / consume / cleanup helpers.
    """

    @classmethod
    async def browse(
        cls,
    ) -> tuple[
        AsyncGenerator[DecodedDeadLetter, None],
        AckTracker,
        aio_pika.abc.AbstractChannel,
    ]:
        """
        Browse the DLQ as a manual-ack consumer with a bounded prefetch.

        Returns a tuple of (decoded dead-letter generator, ack tracker, browse
        channel). Each yielded :class:`DecodedDeadLetter` carries its
        session-scoped ``delivery_tag`` (the handle the servicer uses for
        REPLAY / DISCARD / LEAVE via the tracker). Messages are NOT acked while
        browsing; on stream exit, anything still in-flight is nacked with
        ``requeue=True`` so un-actioned dead-letters return to the DLQ (LEAVE
        semantics).

        The browse ``channel`` is surfaced to the caller so REPLAY can
        republish on the SAME publisher-confirm channel (see :meth:`replay`)
        without opening a second one. The generator retains sole ownership of
        the channel's lifecycle: it returns the channel to the pool on cleanup
        (via ``on_cleanup_complete``). The caller MUST NOT close or return the
        channel itself; it only uses it to drive :meth:`replay` while the
        generator is live.

        :return: (async generator of DecodedDeadLetter, AckTracker instance,
            the publisher-confirm browse channel for REPLAY)
        """
        ack_tracker = AckTracker()
        channel = await cls.connector.get_channel_direct(durable_pool=True)

        # The set_qos / get-queue calls between checkout and the consumer's
        # own try/finally are unprotected and can fail (broker error, queue
        # not found, gRPC stream cancelled). Without this guard the pool
        # checkout leaks with client_count=1 forever.
        try:
            await channel.set_qos(
                prefetch_count=settings.settings.dead_letter_browse_prefetch_count
            )
            queue = await cls._get_dead_letter_queue(channel)
        except BaseException:
            await cls.connector.return_channel(channel)
            raise

        logger.debug("Browsing dead-letter queue '%s' with client acks", queue.name)

        async def _generator() -> AsyncGenerator[DecodedDeadLetter, None]:
            async def _return_channel():
                await cls.connector.return_channel(channel)

            consumer = cls._browse_with_client_acks(
                queue,
                ack_tracker,
                on_cleanup_complete=_return_channel,
            )
            try:
                async for record in consumer:
                    yield record
            finally:
                await consumer.aclose()

        return _generator(), ack_tracker, channel

    @classmethod
    async def replay(
        cls,
        channel: aio_pika.abc.AbstractChannel,
        ack_tracker: AckTracker,
        delivery_tag: int,
        origin_queue: str,
    ) -> ReplayResult:
        """
        Republish a dead-letter to its origin queue, ack-on-confirm (REPLAY).

        Implements SR-3.3 (no-loss republish) and SR-3.4 (no-loss when the
        origin queue is gone). The DLQ copy is acked ONLY after the broker
        confirms the republish; on any failure the DLQ copy is left in place
        (nacked with ``requeue=True``) so the message is never lost, and a
        structured :class:`ReplayResult` is returned so the servicer can emit
        the matching operator-facing report.

        Mechanics (verified against the installed aio-pika):

        - The republish reuses the SAME browse ``channel``, which is already in
          publisher-confirm mode (``publisher_confirms=True``). Awaiting
          ``Exchange.publish`` therefore awaits the broker's ``Basic.Ack``: a
          normal return IS the publish-confirmed signal; a ``Basic.Nack`` (or
          unroutable-with-mandatory) raises.
        - We publish to the channel's ready-made ``default_exchange`` (the
          nameless ``""`` default exchange) with ``routing_key`` = the origin
          queue name. The default exchange auto-binds every queue to its own
          name, so this routes iff the origin queue still exists.
        - ``mandatory=True`` makes an unroutable publish (origin queue deleted /
          no binding) raise :class:`aio_pika.exceptions.DeliveryError` — the
          precise origin-queue-deleted detector.
        - The republished message carries ``delivery_mode=PERSISTENT`` and the
          ORIGINAL body bytes, matching ``Publisher._message``.

        Replay-target invariant
        ------------------------
        ``origin_queue`` MUST be the value decoded from the message's
        ``x-death`` entry — never a client-supplied value. That is what
        confines a replay to the one queue the message was dead-lettered
        from: the caller names a delivery tag, not a destination, so it
        cannot steer a replayed message into a queue of its choosing. The
        coarse ``inspect-and-replay-all-dlq`` permission grants
        visibility and action across every queue's dead letters by design
        (per-queue scoping is deferred), and the eventual re-consume still
        runs the existing per-event ``authz_check`` unchanged.

        Note what that guarantee is and is not. It is "the replay caller
        cannot choose the destination"; it is NOT "a replay cannot cross
        clients". ``origin_queue`` is ``event-queue:<id>`` in a flat namespace
        any client holding ``create-durable-queues`` can claim, so if the
        original queue goes away — deleted explicitly, or reaped after the
        idle-expiry window — and a third party then creates a queue with the
        same id, a later replay delivers the original owner's dead-lettered
        messages into that party's queue. Closing that gap needs per-queue
        authorization, which is tracked separately; the name alone cannot do
        it, because a broker name that encoded the owner would be invalidated
        by any change to the owner's identity string.

        :param channel: The browse channel (already in publisher-confirm mode).
        :param ack_tracker: The session tracker holding the raw aio-pika message
            keyed by ``delivery_tag``.
        :param delivery_tag: The session delivery tag of the DLQ copy to replay.
        :param origin_queue: The origin queue name decoded from ``x-death``
            (NOT client input).
        :return: A :class:`ReplayResult` the servicer maps to an ``Outcome``.
        """
        # Read (do not yet pop) the raw aio-pika message so we still hold it for
        # an ack-on-confirm or a nack-on-failure decision.
        message = ack_tracker.get(delivery_tag)

        republish = aio_pika.Message(
            message.body,
            delivery_mode=aio_pika.DeliveryMode.PERSISTENT,
        )

        try:
            # Awaiting under publisher confirms blocks for the broker's
            # Basic.Ack; mandatory=True turns an unroutable (origin queue gone)
            # publish into a DeliveryError instead of a silent drop.
            await channel.default_exchange.publish(
                republish,
                routing_key=origin_queue,
                mandatory=True,
            )
        except aio_pika.exceptions.DeliveryError:
            # Origin queue no longer exists (or no binding for the routing key):
            # LEAVE the DLQ copy in place (no loss) and report the condition so
            # the servicer can surface ORIGIN_QUEUE_DELETED.
            logger.warning(
                "REPLAY origin queue '%s' is gone (unroutable); leaving "
                "dead-letter in place",
                origin_queue,
            )
            await ack_tracker.nack(delivery_tag, requeue=True)
            return ReplayResult(
                status=ReplayStatus.ORIGIN_QUEUE_DELETED,
                origin_queue=origin_queue,
            )
        except Exception as exc:
            # Any other publish failure (Nack, channel/broker error): LEAVE the
            # DLQ copy in place (no loss) and report REPLAY_FAILED. Never
            # silently log-and-swallow — the caller needs the signal. The
            # detail goes back to the client, so it names only the exception
            # class; the exception's text, which can carry broker or
            # connection internals, is in the log line below.
            detail = f"republish to the origin queue failed ({type(exc).__name__})"
            logger.exception(
                "REPLAY publish to origin queue '%s' failed; leaving "
                "dead-letter in place",
                origin_queue,
            )
            await ack_tracker.nack(delivery_tag, requeue=True)
            return ReplayResult(
                status=ReplayStatus.REPLAY_FAILED,
                origin_queue=origin_queue,
                detail=detail,
            )

        # Publish confirmed: only now is it safe to ack the DLQ copy.
        await ack_tracker.ack(delivery_tag)
        return ReplayResult(
            status=ReplayStatus.REPLAYED,
            origin_queue=origin_queue,
        )

    @classmethod
    async def discard(
        cls,
        ack_tracker: AckTracker,
        delivery_tag: int,
    ) -> ActionResult:
        """
        Drop a dead-letter from the DLQ by acking its DLQ copy (DISCARD).

        Implements SR-3.3's DISCARD action: ack the tracked DLQ message so the
        broker removes it from the DLQ, and remove it from the in-flight
        registry. Returns a structured :class:`ActionResult` the servicer maps
        to ``Outcome.DISCARDED`` so DISCARD reports like REPLAY (SR-3.4).

        An unknown/already-actioned delivery tag is handled gracefully: the
        ``KeyError`` from the tracker is caught and logged (matching the
        ack-unknown-tag behavior of ``_listen_for_durable_acks``), and an
        :attr:`ActionStatus.UNKNOWN_TAG` result is returned so the servicer can
        surface or skip it without crashing the stream.

        :param ack_tracker: The session tracker holding the raw aio-pika message
            keyed by ``delivery_tag``.
        :param delivery_tag: The session delivery tag of the DLQ copy to drop.
        :return: An :class:`ActionResult` the servicer maps to an ``Outcome``.
        """
        try:
            await ack_tracker.ack(delivery_tag)
        except KeyError:
            logger.warning(
                "DISCARD for unknown delivery_tag=%s (already actioned?)",
                delivery_tag,
            )
            return ActionResult(
                status=ActionStatus.UNKNOWN_TAG,
                delivery_tag=delivery_tag,
                detail="unknown or already-actioned delivery tag",
            )
        return ActionResult(status=ActionStatus.DISCARDED, delivery_tag=delivery_tag)

    @classmethod
    async def leave(
        cls,
        ack_tracker: AckTracker,
        delivery_tag: int,
    ) -> ActionResult:
        """
        Return a dead-letter to the DLQ by nack-requeueing its copy (LEAVE).

        Implements SR-3.3's LEAVE action: nack the tracked DLQ message with
        ``requeue=True`` so the broker returns it to the DLQ (no loss), and
        remove it from the in-flight registry. Returns a structured
        :class:`ActionResult` the servicer maps to ``Outcome.LEFT`` so LEAVE
        reports like REPLAY (SR-3.4).

        An unknown/already-actioned delivery tag is handled gracefully: the
        ``KeyError`` from the tracker is caught and logged (matching the
        ack-unknown-tag behavior of ``_listen_for_durable_acks``), and an
        :attr:`ActionStatus.UNKNOWN_TAG` result is returned so the servicer can
        surface or skip it without crashing the stream.

        :param ack_tracker: The session tracker holding the raw aio-pika message
            keyed by ``delivery_tag``.
        :param delivery_tag: The session delivery tag of the DLQ copy to leave.
        :return: An :class:`ActionResult` the servicer maps to an ``Outcome``.
        """
        try:
            await ack_tracker.nack(delivery_tag, requeue=True)
        except KeyError:
            logger.warning(
                "LEAVE for unknown delivery_tag=%s (already actioned?)",
                delivery_tag,
            )
            return ActionResult(
                status=ActionStatus.UNKNOWN_TAG,
                delivery_tag=delivery_tag,
                detail="unknown or already-actioned delivery tag",
            )
        return ActionResult(status=ActionStatus.LEFT, delivery_tag=delivery_tag)

    @classmethod
    async def _get_dead_letter_queue(cls, channel) -> aio_pika.abc.AbstractQueue:
        """
        Get the existing global DLQ by configured name, passively.

        Unlike ``RabbitMQBase.get_queue``, the DLQ has a fixed global name
        (not a per-queue-id name), so this declares it directly
        by ``settings.dead_letter_queue_name``. ``passive=True`` asserts the
        queue exists without re-declaring it or its arguments; ``robust=False``
        keeps aio-pika's RobustChannel from storing the passive reference in
        its internal ``_queues`` (matching ``get_queue``'s robustness handling
        and avoiding an infinite reconnect loop after a broker restart).
        """
        queue_name = settings.settings.dead_letter_queue_name
        try:
            return await channel.declare_queue(  # type: ignore
                queue_name,
                passive=True,
                robust=False,
            )
        except aio_pika.exceptions.ChannelClosed:
            raise QueueNotFoundError(queue_name=queue_name)

    @classmethod
    async def _browse_with_client_acks(
        cls,
        queue: aio_pika.abc.AbstractQueue,
        ack_tracker: AckTracker,
        on_cleanup_complete=None,
    ) -> AsyncGenerator[DecodedDeadLetter, None]:
        """
        Consume from the DLQ without auto-acking, decoding each message.

        Mirrors ``RabbitMQBase._consume_with_client_acks`` but for the DLQ
        browse path: no per-event authz, and each message is decoded into a
        :class:`DecodedDeadLetter` carrying its session delivery tag. The raw
        aio-pika message stays held by the ``AckTracker`` so the servicer can
        ack (DISCARD), nack-requeue (LEAVE), or republish-then-ack (REPLAY) it
        later by delivery tag. On disconnect or shutdown, all in-flight
        messages are nacked back to the DLQ for redelivery, unless the
        channel is already known dead, in which case the broker has
        requeued them itself and the nacks are skipped.

        :param queue: The DLQ to browse from.
        :param ack_tracker: Tracker for in-flight messages awaiting client
                            action.
        :param on_cleanup_complete: Optional async callback invoked after
                                    cleanup finishes (e.g. to return a channel
                                    to the pool).
        """
        # Initialize before the try block so the finally can run even if
        # _start_consuming_from_queue raises. on_cleanup_complete MUST run on
        # every exit path so the caller's pool-channel release fires;
        # otherwise the pool checkout leaks.
        messages_queue: asyncio.Queue | None = None
        consumer_tag: str | None = None
        # Set when the browse ends because the AMQP consumer itself died:
        # either the idle liveness poll classified the loss, or the loop's
        # own shutdown nack failed with a dead-channel error. Written only by
        # the loop, in its two `except` handlers below, before its `finally`
        # schedules the cleanup. The exception is kept rather than a bare
        # flag because it carries the channel state observed at DETECTION
        # time, which is what decides the cleanup path (see _cleanup).
        consumer_lost_exc: Exception | None = None

        try:
            messages_queue, consumer_tag = await cls._start_consuming_from_queue(queue)
            # Pin the channel incarnation the consumer is attached to, with no
            # await in between, so the liveness poll can tell a channel that
            # died and reopened from one that never died.
            started_incarnation = cls._channel_incarnation(queue)
            while True:
                if cls.connector.closing:
                    return

                try:
                    message = await cls._next_message(
                        messages_queue, queue, consumer_tag, started_incarnation
                    )
                except cls._CONSUMER_LOST_EXC as exc:
                    # A shutdown that lands mid-classification is a clean end,
                    # not a consumer-loss error -- surfacing one would turn an
                    # ordinary pod shutdown into a spurious INTERNAL for the
                    # client. Mirrors the `except Exception` handler below;
                    # the dead-channel handler there records its verdict even
                    # on a shutdown, for the reason given at it.
                    if cls.connector.closing:
                        return
                    consumer_lost_exc = exc
                    raise

                if message is None:
                    # The connector went into shutdown while idle.
                    return

                try:
                    if cls.connector.closing:
                        await message.nack(requeue=True)
                        return

                    record = decode_dead_lettered_message(message)
                    record.delivery_tag = ack_tracker.track(
                        message, record.origin_queue
                    )
                    yield record

                except DEAD_CHANNEL_EXC as exc:
                    # The loop's own broker operation on a delivery it holds
                    # found the channel dead. Its only such operation is the
                    # shutdown nack, so today this records the verdict and
                    # returns; the raise is kept so the browse follows the
                    # same rule as the consume loops in RabbitMQBase. That
                    # delivery arrived on the consumer's own channel, so this
                    # is direct evidence the consumer's incarnation is gone,
                    # and it is recorded as consumer loss so the cleanup
                    # narrows. Client DLQ actions do not come through here:
                    # they run on the settle side of the servicer's merge and
                    # mark the tracker instead (see _cleanup).
                    #
                    # The verdict is recorded BEFORE the shutdown check, the
                    # opposite of the liveness handler above. That handler's
                    # classification can be produced by the shutdown itself,
                    # so on a shutdown it is not evidence of anything. A
                    # dead-channel error proves the channel is gone whatever
                    # the reason the browse is ending, so the futile cancel
                    # and nacks are skipped either way; only the client-
                    # facing error is withheld on a shutdown.
                    consumer_lost_exc = ConsumerCancelledError(
                        queue.name, channel_closed=True
                    )
                    if cls.connector.closing:
                        return
                    raise consumer_lost_exc from exc

                except Exception:
                    if cls.connector.closing:
                        return
                    raise

                if cls.connector.closing:
                    return
        finally:
            # The cleanup must be explicitly performed when browsing is
            # finished or interrupted: cancel the consumer, nack all in-flight
            # and prefetched messages back to the DLQ, then return the channel.
            #
            # The consumer MUST be cancelled before nacking. If we nack first,
            # RabbitMQ requeues the message to Ready then immediately
            # re-delivers it to our still-active consumer, putting it right
            # back to Unacked.
            #
            # Running the cleanup in a separate asyncio.create_task ensures it
            # completes. There is a race condition where requests end before
            # the cleanup calls can be completed.
            #
            # When the consumer itself was lost -- the liveness poll
            # classified the loss, or the loop's own shutdown nack found the
            # channel dead -- the cleanup narrows to the steps that can still
            # do something. The cancel is always futile
            # -- the broker has already dropped our tag from its registry --
            # so it is skipped on every consumer-lost path. The nacks are
            # skipped only where the broker has already disposed of the
            # deliveries itself: on a channel close (it requeues them) or a
            # deleted queue (they died with it). A per-consumer cancel on the
            # SAME channel incarnation requeues nothing, so those nacks DO run.
            # on_cleanup_complete still runs, so the channel is released on
            # every path.
            #
            # The loss is not always detected HERE. A DLQ action whose
            # settle fails against a dead channel discovers it first, on
            # the settle side of the servicer's merge. This session's
            # tracker marks itself at that failing settle, before the merge
            # cancels this generator's pending read, so the verdict reaches
            # cleanup as ``ack_tracker.channel_dead`` and narrows it
            # identically. It is read inside the task rather than when the
            # task is scheduled, so a mark from a settle that fails in
            # between still counts.
            async def _cleanup():
                try:
                    if consumer_tag is not None:
                        channel_dead = ack_tracker.channel_dead
                        if cls._should_cancel_consumer(consumer_lost_exc, channel_dead):
                            await cls._cancel_queue_consumer(consumer_tag, queue)
                        if cls._should_nack_outstanding(
                            consumer_lost_exc, channel_dead
                        ):
                            await ack_tracker.nack_all()
                            await cls._nack_prefetched_messages(messages_queue)
                finally:
                    if on_cleanup_complete:
                        await on_cleanup_complete()

            asyncio.create_task(_cleanup())
