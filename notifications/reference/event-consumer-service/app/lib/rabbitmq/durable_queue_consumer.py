# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging
from typing import AsyncGenerator

import aio_pika
from aio_pika.abc import AbstractQueue
from notification_common.auth import principal_id
from notification_common.config.auth_settings import auth_settings

from app.config import settings
from app.lib.models.filter_group import FilterGroup
from app.lib.models.event_message import EventMessage
from app.lib.permissions import permissions
from app.lib.queue_names import NO_PRINCIPAL
from app.lib.rabbitmq.ack_tracker import AckTracker
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase, validate_queue_id

logger = logging.getLogger(__name__)


class DurableQueueConsumer(RabbitMQBase):
    """
    Class for consuming from a durable queue in rabbitmq.
    """

    @classmethod
    async def get_queue(cls, channel, queue_id: str) -> AbstractQueue:
        """
        Resolve a durable queue id to its broker queue, validating the id.

        This is where the ``QUEUE_ID_PATTERN`` bound is actually enforced
        for every operation on an EXISTING durable queue. Consume, consume
        with acks, update, and delete all reach the broker through this one
        method, so validating here makes "no over-long id ever becomes a
        broker name" a property of the code rather than a claim about an
        enumerated list of call sites that a future surface could quietly
        fall off. ``create_queue`` is the only other door, and it validates
        directly.

        The check deliberately lives in this subclass rather than in
        ``RabbitMQBase.get_queue``: the non-durable reconnect path uses the
        base method with an id that is a reconnect token, which has its own,
        deliberately wider constraint in ``validate_reconnect_token`` and
        must NOT be held to the durable-id character pattern (an
        unrecognised token has to reach the lookup and fail as "queue not
        found").

        :param channel: The channel to declare the queue on.
        :param queue_id: The durable queue id to resolve.

        :raises UnprocessableError: If the id fails ``QUEUE_ID_PATTERN``.
        """
        validate_queue_id(queue_id)
        return await super().get_queue(channel, queue_id)

    @classmethod
    async def consume(cls, queue_id: str) -> AsyncGenerator[EventMessage, None]:
        """
        Consume from a durable queue with server-side delayed acking.

        Messages are acked to RabbitMQ after a short delay, giving the
        TCP stack time to deliver them to the client before the broker
        considers them done.

        :param queue_id: The "queue_id" returned from #create_durable_queue.
        :return: async generator of EventMessages
        """
        await permissions.raise_if_cant_consume_durable()

        channel = await cls.connector.get_channel_direct(durable_pool=True)
        # The set_qos / get_queue calls between checkout and the
        # consumer's own try/finally are unprotected and can fail
        # (broker error, queue not found, gRPC stream cancelled).
        # Without this guard the pool checkout leaks with
        # client_count=1 forever.
        try:
            await channel.set_qos(
                prefetch_count=settings.settings.durable_consumer_prefetch_count
            )
            queue = await cls.get_queue(channel, queue_id)
        except BaseException:
            await cls.connector.return_channel(channel)
            raise

        logger.debug(f"Consuming from queue {queue.name} with delayed acks")

        async def _return_channel():
            await cls.connector.return_channel(channel)

        consumer = cls._consume_with_delayed_acks(
            queue,
            permissions.can_consume_durable_queue_event,
            on_cleanup_complete=_return_channel,
        )
        try:
            async for message in consumer:
                yield message
        finally:
            await consumer.aclose()

    @classmethod
    async def consume_with_acks(
        cls, queue_id: str
    ) -> tuple[AsyncGenerator[EventMessage, None], AckTracker]:
        """
        Consume from a durable queue with client ack support.

        Returns a tuple of (event generator, ack tracker). Messages are
        not acked to RabbitMQ until the client explicitly acks them via
        the ack tracker. On disconnect, unacked messages are nacked back
        to RabbitMQ for redelivery.

        :param queue_id: The "queue_id" returned from #create_durable_queue.
        :return: (async generator of EventMessages, AckTracker instance)
        """
        await permissions.raise_if_cant_consume_durable()

        ack_tracker = AckTracker()
        channel = await cls.connector.get_channel_direct(durable_pool=True)

        # Override the channel's prefetch count. The same
        # durable_consumer_prefetch_count setting governs BOTH durable
        # consume paths (this with-acks path and the server-delayed-ack
        # consume path). Pool channels default to 500, but a lower
        # prefetch creates backpressure and limits the blast radius of a
        # disconnect: fewer in-flight messages to nack/redeliver, which
        # keeps quorum x-delivery-count churn clear of the delivery-limit.
        try:
            await channel.set_qos(
                prefetch_count=settings.settings.durable_consumer_prefetch_count
            )
            queue = await cls.get_queue(channel, queue_id)
        except BaseException:
            await cls.connector.return_channel(channel)
            raise

        logger.debug(f"Consuming from queue {queue.name} with client acks")

        async def _generator() -> AsyncGenerator[EventMessage, None]:
            async def _return_channel():
                await cls.connector.return_channel(channel)

            consumer = cls._consume_with_client_acks(
                queue,
                permissions.can_consume_durable_queue_event,
                ack_tracker,
                on_cleanup_complete=_return_channel,
            )
            try:
                async for message in consumer:
                    yield message
            finally:
                await consumer.aclose()

        return _generator(), ack_tracker

    @classmethod
    async def create_queue(
        cls, filter_groups: list[FilterGroup], queue_id: str | None = None
    ) -> tuple[str, bool]:
        """
        Creates a durable queue that will immediately start receiving any
        published messages for the event type.

        When ``queue_id`` is omitted, a server-side uuid4 is minted and a
        new queue is always created. When ``queue_id`` is supplied, the
        call is idempotent (ensure-exists): if a queue with that id
        already exists it is returned unchanged, with its existing
        bindings and regardless of who created it; otherwise it is
        created. Queue ids are therefore a deployment-wide namespace.

        The returned queue_id must be passed in to the consume method
        every time this client wants to consume events.

        :param filter_groups: The types of events to consume.
        :param queue_id: Optional client-supplied queue id. When omitted a
            server-side uuid4 is generated.

        :return: A tuple of (queue_id, created) where ``created`` is True
            when this call declared/bound the queue and False when an
            existing queue with the supplied id was found.
        """
        if queue_id is not None:
            validate_queue_id(queue_id)

        # The principal is needed here as a *value*: it is the attribution
        # carried by the create/attach log lines below. It is resolved BEFORE
        # the permissions round trip and any broker work, not at the log line
        # at the end, so that the one thing that can fail here -- the caller's
        # own credential -- fails as a pre-flight 401 with no side effects,
        # instead of after the queue has already been declared and bound.
        #
        # This is a cached read that can still raise; what it reports when it
        # does depends on the configured principal_source, stated once in
        # ``app.lib.auth.authenticate``'s docstring. The ordering stands under
        # every source -- nothing above it has a side effect to undo.
        principal = await cls._resolve_principal()

        await permissions.raise_if_cant_create_durable_queue()

        async with cls.connector.get_channel(durable_pool=True) as channel:
            if queue_id is None:
                queue_id, queue = await cls._create_queue(channel, filter_groups, True)
                cls._log_queue_created(queue_id, queue.name, principal)
                return queue_id, True

            # Lightweight passive existence probe for the ensure-exists
            # path. Unlike _assert_queue_exists, this does a single
            # declare_queue(passive=True) with NO retry/sleep loop: on a
            # fresh create a miss is the normal case, so the broker-restart
            # retry rationale does not apply and the loop would add latency
            # to every create-new call.
            #
            # robust=False prevents aio_pika's RobustChannel from caching
            # the passive queue reference (see _assert_queue_exists), and
            # the probe runs on a temporary channel so a ChannelClosed miss
            # does not poison the pool channel used for the declare/bind
            # below.
            name = await cls._queue_name(queue_id)
            try:
                async with cls.connector.get_temporary_channel() as temp_channel:
                    await temp_channel.declare_queue(
                        name,
                        passive=True,
                        robust=False,
                    )
                # The queue already exists; return without declaring or
                # binding.
                cls._log_queue_attached(queue_id, principal)
                return queue_id, False
            except aio_pika.exceptions.ChannelClosed:
                # Not found: fall through to create below.
                pass

            # The check-then-create race here is benign: declare_queue and
            # queue.bind are both idempotent for matching arguments, so
            # concurrent racers converge on the same queue. ``created`` is
            # best-effort — two racers may both report True. Accepted
            # limitation: two concurrent creates with the same id but
            # different filter_groups may union their bindings;
            # UpdateDurableQueue is the path for deterministic binding
            # changes.
            _, queue = await cls._create_queue(
                channel, filter_groups, True, queue_id=queue_id
            )
            cls._log_queue_created(queue_id, queue.name, principal)
            return queue_id, True

    @staticmethod
    async def _resolve_principal() -> str:
        """
        Name the principal making this request, for the create log line.

        Obtaining that name is the primary purpose of this call, and it is
        why ``create_queue`` makes it before it does anything with a side
        effect: the value is needed either way, and taking it first means a
        failure to produce it costs nothing. The transport-level resolution
        and the entry point's ``authenticate`` have both already resolved the
        token, so on the ordinary path this is a cached read -- one that can
        still raise, on the terms ``app.lib.auth.authenticate``'s docstring
        states for every in-request resolution.

        :return: The caller's principal id, or ``authn-disabled`` when
            authentication is off and there is no token to name one.

        :raises AuthenticationError: If the request's token is no longer
            usable.
        """
        if not auth_settings.authentication_enabled():
            return NO_PRINCIPAL

        principal = await principal_id.principal_id()
        return principal if principal is not None else NO_PRINCIPAL

    @staticmethod
    def _log_queue_created(queue_id: str, name: str, principal: str) -> None:
        """
        Record which principal created a durable queue, for operator triage.

        A broker queue name carries nothing about its creator, so this line is
        the only place the pair is written down: an operator looking at a queue
        in the broker finds who asked for it by searching the service logs for
        this line. It is emitted only for a queue this call actually declared;
        the ensure-exists path, where the queue belongs to whoever created it
        first, has its own line -- see ``_log_queue_attached``.

        The principal is passed in, resolved by ``create_queue`` before any of
        the work this line reports on. Reading it here instead would put a
        credential check after the queue exists: an in-request resolution
        raises on the terms ``app.lib.auth.authenticate``'s docstring states,
        and the stretch of work between that pre-flight resolution and this
        line -- the permissions round trip, the channel checkout, the
        existence probe and the declare-and-bind -- can outlast even the
        derived cache lifetime, so the raise would fail a create that had
        already happened.

        :param queue_id: The client-facing id of the queue just created.
        :param name: The broker queue name derived from that id.
        :param principal: The resolved creator, from ``_resolve_principal``.
        """
        logger.info(
            "Durable queue created: id=%s name=%s principal=%s",
            queue_id,
            name,
            principal,
        )

    @staticmethod
    def _log_queue_attached(queue_id: str, principal: str) -> None:
        """
        Record a create call that attached to a queue somebody else had
        already created.

        Queue ids are a deployment-wide namespace and ensure-exists is
        silent about it: the caller gets ``created: false`` and a working
        queue whether it is re-creating its own queue after a restart or has
        collided with another client's id. Only the service sees both sides,
        so this line is where an operator finds out that a queue has more
        than one principal creating against it -- the pairing that
        ``_log_queue_created`` records for the first caller, continued for
        every one after it.

        The broker name is deliberately absent: it is a pure function of the
        id, ``_log_queue_created`` already spells it out for this queue, and
        this branch has done nothing to it.

        :param queue_id: The client-supplied id that already existed.
        :param principal: The caller, from ``_resolve_principal``.
        """
        logger.info(
            "Durable queue attached (existing): id=%s principal=%s",
            queue_id,
            principal,
        )

    @classmethod
    async def delete_queue(cls, queue_id: str) -> None:
        """
        Deletes a durable queue
        """
        await permissions.raise_if_cant_delete_durable_queue()

        async with cls.connector.get_channel(durable_pool=True) as channel:
            queue = await cls.get_queue(channel, queue_id)

            logger.info(f"Deleting durable queue: {queue.name}")
            await queue.delete(if_unused=False, if_empty=False)
            logger.info(f"Durable queue deleted successfully: {queue.name}")

    @classmethod
    async def update_queue(
        cls,
        queue_id: str,
        current_filter_groups: list[FilterGroup],
        new_filter_groups: list[FilterGroup],
    ) -> None:
        """
        Updates the bindings on a durable queue
        """
        await permissions.raise_if_cant_create_durable_queue()

        async with cls.connector.get_channel(durable_pool=True) as channel:
            queue = await cls.get_queue(channel, queue_id)
            await cls._update_bindings(
                channel, queue, current_filter_groups, new_filter_groups
            )
