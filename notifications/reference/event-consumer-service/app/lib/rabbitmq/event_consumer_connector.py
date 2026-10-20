# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import aio_pika
from notification_common.rabbitmq.channel_pool import ChannelPool
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings

logger = logging.getLogger(__name__)


class _DeadLetterQueueArgsMismatch(Exception):
    """
    Internal signal that the DLQ already exists with different arguments.

    Raised in place of the broker's AMQP 406 PRECONDITION_FAILED (which
    aiormq surfaces as ``ChannelPreconditionFailed``) so the handler in
    ``_declare_dead_letter_topology`` reacts to a mismatch on the DLQ
    declare ONLY. A precondition failure anywhere else in the topology (the
    DLX, say) stays a plain ``ChannelPreconditionFailed`` and propagates.
    """


class EventConsumerConnector(RabbitMQConnector):
    """
    Event-Consumer-Service-specific RabbitMQ connector.

    Extends the shared ``RabbitMQConnector`` to provision this service's
    dead-letter topology (DLX, DLQ, and the DLX->DLQ binding) over AMQP at
    startup. The provisioning fires on the same ``connect()`` ->
    ``declare_exchange()`` path as the shared ``events`` exchange, so it is
    re-declared on every (re)connect and is idempotent across reconnects
    (RabbitMQ declares are idempotent when args match).

    The DLX/DLQ topology is event-consumer-service-specific, so it lives in
    this repo rather than in the shared ``notification_common`` connector,
    which other code paths use without this topology.
    """

    async def declare_exchange(self):
        """
        Declare the shared ``events`` exchange, then provision the
        dead-letter topology.
        """
        await super().declare_exchange()
        await self._declare_dead_letter_topology()

    async def _declare_dead_letter_topology(self):
        """
        Declare the DLX (durable direct exchange), the DLQ (durable quorum
        queue, self-bounded via x-message-ttl + x-max-length with
        x-overflow drop-head), and bind the DLQ to the DLX.

        The DLQ sets NO x-dead-letter-exchange/x-dead-letter-routing-key of
        its own, so there is no dead-letter cycle.

        Tolerant of argument drift: RabbitMQ queue arguments are immutable
        after declaration, so changing
        ``OMNI_EVENTS_DEAD_LETTER_QUEUE_MAX_LENGTH`` or
        ``OMNI_EVENTS_DEAD_LETTER_QUEUE_MESSAGE_TTL_MS`` against a broker
        whose DLQ already exists makes the active declare fail with AMQP 406
        PRECONDITION_FAILED. The catch keys on that failure rather than on
        which argument mismatched, so a live queue differing in any other
        argument (``x-queue-type``, when it is classic or was provisioned by
        another tool) takes the same path; the logged remedies cover both
        cases. Rather than letting the 406 escape through ``connect()`` and
        crash-loop the pod, the mismatch is logged at ERROR with the remedies
        and the topology is re-provisioned passively so startup completes on
        the existing queue.

        Acquires a durable-pool channel the same way ``declare_exchange``
        acquires a non-durable one, guarded by try/finally. The durable pool
        is the right home for this topology because it is dead-letter
        infrastructure for durable queues, and it keeps a startup
        infrastructure checkout out of the non-durable pool's stats.
        """
        # Reach into the pool directly rather than going through
        # get_channel(durable_pool=True): this runs from declare_exchange(),
        # which connect() awaits while holding its non-reentrant lock, and
        # get_channel() re-enters connect() whenever the connection looks
        # closed. That re-entry would deadlock startup permanently.
        #
        # Bind the pool to a local and pass it down rather than letting the
        # helper re-read the attribute: the drift path below puts a failed
        # broker round-trip and a channel close/reopen between this guard and
        # the second helper call, and RabbitMQConnector.close() nulls both
        # pools without taking the connection lock. A SIGTERM landing in that
        # window would otherwise make the helper's re-read return None and
        # raise AttributeError.
        durable_channel_pool = self._durable_channel_pool
        if not durable_channel_pool:
            return

        dlq_name = settings.settings.dead_letter_queue_name

        logger.info(
            "Declaring dead-letter exchange '%s' and queue '%s'",
            settings.settings.dead_letter_exchange_name,
            dlq_name,
        )

        try:
            await self._provision_dead_letter_topology(
                durable_channel_pool, passive=False
            )
            return
        except _DeadLetterQueueArgsMismatch as exc:
            arguments = self._dead_letter_queue_arguments()
            logger.error(
                "Dead-letter queue argument mismatch (AMQP 406): queue '%s' "
                "already exists with different arguments, so this service's "
                "arguments were NOT applied. Desired: x-queue-type=%s, "
                "x-message-ttl=%s, x-max-length=%s, x-overflow=%s. RabbitMQ "
                "queue arguments are immutable after declaration, so the live "
                "queue keeps the ones it was created with. Remedy depends on "
                "WHICH argument differs -- see the broker error at the end of "
                "this message for the offending one. For the numeric bounds "
                "(x-message-ttl, x-max-length): to LOWER a bound on the live "
                "queue, apply a broker policy to '%s' (argument-vs-policy "
                "resolution is min-wins on numeric keys; such a policy must "
                "omit 'overflow', which is policy-wins on quorum queues); to "
                "RAISE a bound, drain and delete the queue in a maintenance "
                "window so it is recreated from these settings. For ANY OTHER "
                "argument (x-queue-type, say, when the live queue is classic "
                "or was provisioned by another tool), no policy can help -- a "
                "queue's type is fixed at declaration and no policy can "
                "change it -- so the only remedy is to drain and delete the "
                "queue in a maintenance window and let it be recreated with "
                "the arguments above. Startup continues against the "
                "existing queue; this message repeats on every (re)connect "
                "while the drift persists. Broker error: %s",
                dlq_name,
                arguments["x-queue-type"],
                arguments["x-message-ttl"],
                arguments["x-max-length"],
                arguments["x-overflow"],
                dlq_name,
                exc,
            )

        # The 406 closed the channel before the DLX->DLQ bind could run, so
        # the topology is unfinished. Re-provision on a fresh channel --
        # genuinely fresh, because the mismatch path closed the 406-killed
        # channel explicitly (see _close_channel_killed_by_406) so the pool
        # evicted it on return. Obtain the existing queue passively this time
        # and issue the bind. A failure here (the queue being genuinely
        # absent, say) is a real broker problem rather than drift, so it
        # propagates.
        await self._provision_dead_letter_topology(durable_channel_pool, passive=True)

    async def _provision_dead_letter_topology(
        self, durable_channel_pool: ChannelPool, passive: bool
    ):
        """
        Declare the DLX, obtain the DLQ, and bind the DLQ to the DLX, all on
        a single channel from the given durable pool.

        :param durable_channel_pool: the durable pool to check a channel out
            of, passed in by ``_declare_dead_letter_topology`` rather than
            re-read from ``self`` so a concurrent ``close()`` -- which nulls
            the connector's pool attributes without taking the connection
            lock -- cannot turn it into ``None`` mid-provision.
        :param passive: when ``False``, declare the DLQ actively with this
            service's self-bounding arguments, raising
            ``_DeadLetterQueueArgsMismatch`` if the live queue was created
            with different ones. When ``True``, assert the existing queue
            without re-declaring its arguments -- ``robust=False`` matches
            ``DeadLetterConsumer._get_dead_letter_queue`` and keeps aio-pika's
            RobustChannel from storing the passive reference for re-declare
            on reconnect.
        """
        # The pool is used directly here for the deadlock reason spelled out
        # in _declare_dead_letter_topology.
        channel = await durable_channel_pool.get_channel()
        try:
            exchange = await channel.declare_exchange(
                settings.settings.dead_letter_exchange_name,
                type="direct",
                durable=True,
            )
            if passive:
                queue = await channel.declare_queue(
                    settings.settings.dead_letter_queue_name,
                    passive=True,
                    robust=False,
                )
            else:
                try:
                    queue = await channel.declare_queue(
                        settings.settings.dead_letter_queue_name,
                        durable=True,
                        arguments=self._dead_letter_queue_arguments(),
                    )
                except aio_pika.exceptions.ChannelPreconditionFailed as exc:
                    await self._close_channel_killed_by_406(channel)
                    raise _DeadLetterQueueArgsMismatch(str(exc)) from exc
            await queue.bind(exchange, settings.settings.dead_letter_routing_key)
        finally:
            await durable_channel_pool.return_channel(channel)

    @staticmethod
    async def _close_channel_killed_by_406(
        channel: aio_pika.abc.AbstractRobustChannel,
    ):
        """
        Explicitly close the channel the broker killed with the 406, so the
        pool evicts it deterministically when the caller's ``finally``
        returns it.

        Without this, the drift path leaks channels slowly. The broker's
        server-initiated close fires ``RobustChannel._on_close``, which
        AUTO-REOPENS the channel via ``await self.restore()``
        (aio_pika/robust_channel.py:100-121). ``ChannelPool.return_channel``
        keys eviction on ``channel.is_closed``, so it usually sees a
        restored, open channel and keeps it -- harmless. But when the return
        wins the race against the restore, the pool evicts and forgets the
        channel while ``restore()`` is still in flight, and the reopened
        channel is left open with nobody tracking it: one orphaned AMQP
        channel per drift-reconnect, a bounded slow leak for as long as the
        drift persists.

        ``RobustChannel.close()`` clears the restoration flag before
        delegating (aio_pika/robust_channel.py:123-129), which suppresses the
        auto-reopen and marks the channel closed, so ``return_channel``
        always takes its eviction branch and the orphan branch becomes
        unreachable.

        Closing an already-closed channel does not raise: ``UnderlayChannel
        .close()`` returns early once its one-shot close callback has fired,
        and ``aiormq``'s ``Base.close()`` returns ``None`` when
        ``is_closed``. A close that reaches the broker (the channel having
        already been restored) can still fail on a connection-level problem,
        though, and letting that escape would turn graceful drift handling
        back into the crash-loop this whole path exists to prevent. So the
        close is best-effort, mirroring ``ChannelPool._close_channel_info``.

        :param channel: the channel whose active DLQ declare took the 406.
        """
        try:
            await channel.close()
        except Exception as exc:
            logger.warning(
                "Failed to close the channel the broker killed with the "
                "dead-letter queue 406; the pool may retain or orphan it. "
                "Error: %s",
                exc,
            )

    @staticmethod
    def _dead_letter_queue_arguments() -> dict:
        """
        Build the DLQ's declare arguments from settings.

        These make the DLQ self-bounding on a broker nobody has configured
        (SDD SR-1.4), and they are also what the drift ERROR reports as the
        desired-but-unapplied values, so both read from this one place.
        """
        return {
            "x-queue-type": "quorum",
            "x-message-ttl": settings.settings.dead_letter_queue_message_ttl_ms,
            "x-max-length": settings.settings.dead_letter_queue_max_length,
            "x-overflow": "drop-head",
        }
