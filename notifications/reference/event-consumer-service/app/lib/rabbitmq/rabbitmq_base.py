# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import json
import logging
import random
import re
import time
import uuid
from abc import ABC
from typing import AsyncGenerator, Callable, Awaitable

import aio_pika
from aio_pika.abc import AbstractQueue
from notification_common.errors.errors import (
    ClientFacingServerError,
    NotificationBaseException,
    QueueNotFoundError,
    UnprocessableError,
)
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings
from app.lib.binding_keys import binding_keys_from_filters
from app.lib.models.event_message import EventMessage
from app.lib.models.filter_group import FilterGroup
from app.lib.queue_names import (
    QUEUE_NAME_PREFIX,
    redact_queue_name,
    redact_reconnect_token,
)
from app.lib.rabbitmq.ack_tracker import DEAD_CHANNEL_EXC, AckTracker
from app.lib.rabbitmq.event_consumer_connector import EventConsumerConnector

logger = logging.getLogger(__name__)

# Single source of truth for client-supplied `queue_id` validation, shared by
# the gRPC servicer (through `validate_queue_id` below) and the REST Pydantic
# models and query parameters (through a declarative `pattern=`). The pattern
# excludes `:` (the `_queue_name` separator) and its `{1,200}` length bound
# keeps the full broker name under RabbitMQ's 255-byte limit: `event-queue:`
# plus a 200-byte id is 212 bytes for a durable queue, and 215 for a
# non-durable one once the `:nd` suffix is appended.
#
# The bound is enforced where every surface funnels rather than surface by
# surface: `DurableQueueConsumer` applies it in `create_queue` and in its
# `get_queue` override, which is the one door the consume, update, and delete
# paths all go through. The gRPC servicer and the REST Pydantic models check
# it again at their own boundary, so a malformed id is refused before it
# reaches a log line or a span attribute, and so the REST constraint appears
# in the generated OpenAPI document -- but those boundary checks are now a
# fast fail in front of the invariant rather than the invariant itself.
QUEUE_ID_PATTERN = r"^[A-Za-z0-9._-]{1,200}$"

# How much of an over-long queue id is echoed back in the rejection message.
# The value is client-controlled and unbounded until this check runs, and the
# message travels: `ExceptionInterceptor` re-logs it with `logger.exception`
# before it is returned. Echoing enough to recognise the id costs nothing;
# echoing all of it turns one rejected request into a log line as large as
# the request was.
_QUEUE_ID_ECHO_LIMIT = 64


def validate_queue_id(queue_id: str) -> None:
    """
    Reject a client-supplied durable queue id that fails the shared pattern.

    This is the gRPC half of the enforcement described above; the REST half is
    declarative, so both halves reject the same ids for the same reason.
    ``UnprocessableError`` is the class create has always raised here, and the
    shared error mapping renders it as gRPC ``INVALID_ARGUMENT`` (HTTP 422),
    matching what the Pydantic ``pattern`` produces on the REST surfaces.

    Validation always runs before the broker is contacted, and at the API
    boundary it runs before a channel is checked out to talk to it. An
    over-long id would otherwise die building the AMQP frame, where pamqp
    raises a bare ``ValueError("Max length exceeded for queue")`` -- a
    message about a broker-name limit the caller never chose, arriving from
    a layer that names neither the field nor the rule it broke.

    The rejected id is echoed, because a caller with several queues needs to
    know which one it got wrong, but only its first
    ``_QUEUE_ID_ECHO_LIMIT`` characters, with the real length appended so a
    length problem is still diagnosable from the message alone.

    :param queue_id: The client-supplied id to check.
    :raises UnprocessableError: If the id does not match ``QUEUE_ID_PATTERN``.
    """
    if not re.fullmatch(QUEUE_ID_PATTERN, queue_id):
        raise UnprocessableError(
            f"Invalid queue_id '{_truncate_queue_id(queue_id)}': "
            f"must match {QUEUE_ID_PATTERN}"
        )


def _truncate_queue_id(queue_id: str) -> str:
    """
    Render a rejected queue id short enough to log.

    :param queue_id: The client-supplied id being reported.

    :return: The id itself when it is already short, otherwise its first
        ``_QUEUE_ID_ECHO_LIMIT`` characters followed by an ellipsis and the
        length that was actually submitted.
    """
    if len(queue_id) <= _QUEUE_ID_ECHO_LIMIT:
        return queue_id
    return f"{queue_id[:_QUEUE_ID_ECHO_LIMIT]}... ({len(queue_id)} characters)"


# Single source of truth for the client-supplied `reconnect_token` bound,
# shared by the gRPC servicer (through `validate_reconnect_token` below) and
# the REST query parameter and header (through a declarative `pattern=`).
# A reconnect token IS a non-durable queue id, so it lands in a broker name
# the same way a durable id does: `event-queue:` plus the token plus `:nd` is
# 215 bytes at this bound, inside RabbitMQ's 255-byte limit. The number
# deliberately matches the `{1,200}` bound in QUEUE_ID_PATTERN so both kinds
# of id spend the same share of that byte budget.
RECONNECT_TOKEN_MAX_LENGTH = 200

# The full constraint: printable ASCII, within the bound above. The alphabet
# is deliberately wide -- a token is opaque to the client, which only ever
# echoes back one the server minted (a 36-char uuid4), so a token that is
# merely unrecognized must still reach the lookup and fail as "queue not
# found" rather than be rejected as malformed. What it rules out is the
# character a broker queue name cannot carry at all: RabbitMQ's queue name is
# a shortstr, and pamqp's `Queue.Declare.validate()` rejects anything outside
# printable ASCII with a bare `ValueError('Invalid value for queue')` --
# raised deep in the AMQP frame build, after a channel has been checked out,
# and naming neither the field nor the rule it broke. Constraining the
# alphabet here is what keeps a token this service cannot possibly have
# minted an INVALID_ARGUMENT that says which field and why, and keeps "an
# unrecognized token of legal shape is reported NOT_FOUND" true.
#
# The length bound is interpolated from the constant above so there is one
# number, not two that can drift.
RECONNECT_TOKEN_PATTERN = rf"^[\x21-\x7e]{{1,{RECONNECT_TOKEN_MAX_LENGTH}}}$"


def validate_reconnect_token(reconnect_token: str) -> None:
    """
    Reject a client-supplied reconnect token that fails the shared pattern.

    This is the gRPC half of the enforcement described above; the REST half is
    declarative, so both halves reject the same tokens for the same reason.
    ``UnprocessableError`` is what the shared error mapping renders as gRPC
    ``INVALID_ARGUMENT``, matching the 422 the REST ``pattern`` produces.

    As with an over-long queue id, this runs before a channel is checked out
    to talk to the broker. A token the broker cannot name a queue with would
    otherwise die building the AMQP frame, where pamqp raises a bare
    ``ValueError`` -- a message about a broker-name limit the caller never
    chose, naming neither the field nor the rule it broke.

    Unlike the queue-id message, this one does not echo the offending value.
    A reconnect token is not a credential: presenting it requires a valid
    bearer token and every delivered event is authorized per principal by the
    permissions policy. It is the queue's identifier, though, so an authorized
    principal that obtains it could attach to the same stream and take a share
    of its events; it is kept out of telemetry as defense in depth -- and an
    error message travels into logs on both sides of the call. Everywhere else
    that needs to name a token -- a log line, a span attribute, a metric
    attribute -- goes
    through ``app.lib.queue_names.redact_reconnect_token`` for the same
    reason. The message therefore names the field and the rule instead, which
    is everything a caller needs and nothing a log reader should not see.

    A caller whose surface cannot tell an absent token from an empty one --
    proto3, where a string field defaults to "" -- must not call this for an
    absent token: the empty string names no queue and is rejected here, so
    the gRPC servicer checks for a token before asking whether it is a valid
    one.

    :param reconnect_token: The client-supplied token to check.
    :raises UnprocessableError: If the token does not match
        ``RECONNECT_TOKEN_PATTERN``.
    """
    if not re.fullmatch(RECONNECT_TOKEN_PATTERN, reconnect_token):
        raise UnprocessableError(
            f"Invalid reconnect_token: must be 1 to "
            f"{RECONNECT_TOKEN_MAX_LENGTH} printable ASCII characters "
            f"(no spaces, no control or non-ASCII characters)"
        )


class ConsumerCancelledError(ClientFacingServerError):
    """
    Raised when the broker cancelled our consumer, or killed the channel
    (or the connection) it lived on.

    Raising it does not prove the queue still exists. The
    consumer-liveness check confirms the queue still exists when it can
    reach the broker, and a deleted queue surfaces there as
    ``QueueNotFoundError`` instead. When the connection is down, or the
    check cannot reach the broker, it raises this error without checking.
    The consume loops (when one of their own broker operations finds the
    channel dead) and the servicer (when a client-driven operation does)
    also raise it without checking: a dead-channel error leaves no doubt
    about the channel, and a round trip to check the queue would cost
    every stream a temporary channel during a kill wave. If the queue was
    in fact deleted, the client learns it as ``NOT_FOUND`` on the
    reconnect this error asks for.

    The client's stream cannot recover in place: the AMQP consumer is
    gone, and its outstanding delivery tags died with the channel. The
    stream is therefore ended with this terminal error so the client
    reconnects rather than waiting on a queue nothing is consuming.

    It deliberately inherits the 500 status of its base rather than 503:
    503 is the capacity-shed code that the Envoy retry policy and the
    capacity metrics key on, and this is not a capacity condition.

    ``channel_closed`` records, AT DETECTION TIME, whether the channel
    incarnation the consumer was started on had died. That fact decides
    how the consume loops clean up, and it is deliberately NOT the
    wrapper's ``is_closed`` at cleanup time: the channel is a
    RobustChannel, so it may have silently reopened, and the fresh
    incarnation's ``is_closed`` describes a channel that never carried
    our delivery tags. Carrying the fact on the exception is what keeps
    the cleanup decision pinned to the moment the loss was observed.
    """

    def __init__(self, queue_name: str, channel_closed: bool):
        # The broker queue name (`event-queue:<queue_id>`) is an internal
        # identifier, so it goes to the log-only `internal_message` and
        # the client gets generic guidance -- the same split
        # QueueNotFoundError uses. NotificationBaseException is called
        # directly because ClientFacingServerError.__init__ accepts no
        # `internal_message`; the "Internal Server Error: " prefix is
        # restated here because that is exactly what it would have added.
        #
        # "Log-only" is precisely why the name is redacted first: a
        # non-durable name embeds the stream's reconnect token, and this
        # message is logged on every stream the broker cancels.
        NotificationBaseException.__init__(
            self,
            "Internal Server Error: The broker cancelled this stream's "
            "consumer. Reconnect to resume consuming.",
            internal_message=f"queue_name: {redact_queue_name(queue_name)}",
        )
        self.channel_closed = channel_closed


rabbitmq_connector: RabbitMQConnector = EventConsumerConnector()


class RabbitMQBase(ABC):
    """
    Abstract base class for rabbitmq consumer clients.

    This module creates an instance, and it is that instance that should be
    imported in other modules.
    """

    connector: RabbitMQConnector = rabbitmq_connector

    _QUEUE_EXISTS_RETRIES = 3
    _QUEUE_EXISTS_RETRY_DELAY_S = 1

    # The two terminal outcomes of the consumer-liveness check, kept as a
    # named tuple attribute so every `except` below stays a name rather
    # than an inline `except (A, B):` that Black 25 strips the parens from
    # on py3.14 (see b.8qg).
    _CONSUMER_LOST_EXC = (ConsumerCancelledError, QueueNotFoundError)

    # Fraction by which each idle liveness poll's deadline is stretched,
    # so the streams sharing a connection do not all wake at once. The
    # band is upward-only -- see _liveness_poll_timeout.
    _LIVENESS_POLL_JITTER = 0.15

    @classmethod
    async def get_queue(cls, channel, queue_id: str) -> AbstractQueue:
        queue_name = await cls._queue_name(queue_id)

        # This is the LOOKUP path, so `queue_id` is client input -- on the
        # non-durable reconnect path it is the token straight off the wire --
        # and nothing here has established that it names a real queue. That
        # costs the rendering nothing: `redact_queue_name` decides by the
        # name's SHAPE and always hashes the id it finds, so a client that
        # presented `sha256-<12 hex>` as its token is recorded under the
        # digest of that value, not under the value itself.
        log_name = redact_queue_name(queue_name)

        # Check existence with a temporary channel first
        # because pool channels may have the queue cached
        # due to the way aio_pika works.
        await cls._assert_queue_exists(queue_name)

        # Both calls use robust=False to prevent aio_pika's
        # RobustChannel from storing passive queue references
        # in _queues. Without this, a RabbitMQ restart causes
        # an infinite reconnect loop (see robust=False comment
        # in _assert_queue_exists).
        try:
            return await channel.declare_queue(  # type: ignore
                queue_name,
                passive=True,
                robust=False,
            )
        except aio_pika.exceptions.ChannelClosed:
            raise QueueNotFoundError(queue_name=log_name)

    @classmethod
    async def _assert_queue_exists(cls, queue_name: str) -> None:
        """
        Verify a queue exists on RabbitMQ using a temporary
        channel.

        Retries a few times to handle the brief window after
        a broker restart where durable queues may not yet be
        available while RabbitMQ is still loading its mnesia
        database from disk.

        Uses robust=False so that aio_pika's RobustChannel
        does not store the passive queue reference in its
        internal _queues dict. Without this, on reconnection
        after a RabbitMQ restart, RobustConnection tries to
        restore these passive references by sending
        queue_declare(passive=True). For non-durable queues
        that no longer exist, this returns NOT_FOUND and
        kills the entire connection in an infinite reconnect
        loop.

        The retry log line and the not-found error name the queue, and a
        non-durable name embeds the stream's reconnect token, so the name is
        redacted here for both. Deciding by the name's shape is safe on
        every path into this method, including the lookup path where the name
        was built from an id the broker has not confirmed:
        ``redact_queue_name`` always hashes the id it finds.

        :param queue_name: The broker queue name to assert, verbatim, as it
            must be sent to the broker.
        """
        log_name = redact_queue_name(queue_name)
        last = cls._QUEUE_EXISTS_RETRIES - 1
        for attempt in range(cls._QUEUE_EXISTS_RETRIES):
            try:
                async with cls.connector.get_temporary_channel() as temp_channel:
                    await temp_channel.declare_queue(
                        queue_name,
                        passive=True,
                        robust=False,
                    )
                    return
            except aio_pika.exceptions.ChannelClosed:
                if attempt == last:
                    raise QueueNotFoundError(queue_name=log_name)
                logger.debug(
                    "Queue '%s' not found on attempt " "%d, retrying in %ds",
                    log_name,
                    attempt + 1,
                    cls._QUEUE_EXISTS_RETRY_DELAY_S,
                )
                await asyncio.sleep(cls._QUEUE_EXISTS_RETRY_DELAY_S)

    @classmethod
    async def _create_queue(
        cls,
        channel,
        filter_groups: list[FilterGroup],
        durable: bool,
        queue_id: str | None = None,
    ) -> tuple[str, AbstractQueue]:
        if not filter_groups:
            raise UnprocessableError("Cannot consume events without any filters")

        keys = binding_keys_from_filters(filter_groups)

        # Use the client-supplied queue_id when provided; otherwise mint a
        # server-side uuid4 exactly as before.
        queue_id = queue_id if queue_id is not None else str(uuid.uuid4())
        name = await cls._queue_name(queue_id)

        # A non-durable queue's id is the reconnect token the client is
        # about to be handed, so neither it nor the name derived from it can
        # be written down. A durable id is an ordinary identifier and is
        # logged verbatim, which is what an operator needs to find the queue
        # in the broker.
        #
        # The name is rendered by `redact_queue_name`, exactly as on the
        # lookup path: one rule, keyed on the name's shape, everywhere. The
        # id beside it is rendered to match -- redacted iff the name was.
        log_name = redact_queue_name(name)
        redacted = log_name != name
        log_queue_id = redact_reconnect_token(queue_id) if redacted else queue_id

        creation_log_message = "durable" if durable else "non-durable"
        creation_log_message = (
            f"Creating {creation_log_message} "
            f"queue with id {log_queue_id} and name {log_name}"
        )
        arguments = {}

        if not durable:
            arguments = {
                "x-expires": settings.settings.non_durable_queue_ttl_seconds * 1000
            }
            logger.debug(creation_log_message)
        else:
            arguments = {
                "x-queue-type": "quorum",
                "x-dead-letter-exchange": settings.settings.dead_letter_exchange_name,
                "x-dead-letter-routing-key": settings.settings.dead_letter_routing_key,
            }
            # A count, not the groups: they carry the client's resource ids
            # (folder paths), which do not belong in an INFO log line.
            logger.info(f"{creation_log_message}: filter_groups={len(filter_groups)}")

        # robust=False prevents aio_pika's RobustChannel from storing
        # a reference in its internal _queues dict. Without this, channel
        # recovery re-declares every queue that was ever created through
        # the channel — even ones that were already deleted — because
        # queue.delete() does not remove the _queues entry.
        queue = await channel.declare_queue(  # type: ignore
            name,
            arguments=arguments,  # type: ignore
            durable=durable,
            robust=False,
        )
        exchange = await cls.connector.get_exchange(channel)

        for binding_key in keys:
            await queue.bind(exchange, binding_key)

        logger.debug(f"Queue {log_name} bound to " f"exchange with bindings: {keys}")

        return str(queue_id), queue

    @classmethod
    async def _parse_and_authorize(
        cls,
        message,
        authz_check: Callable[[dict], Awaitable[bool]],
        consumer_tag: str,
        queue: AbstractQueue,
        messages_queue: asyncio.Queue,
    ) -> EventMessage | None:
        """
        Parse a raw RabbitMQ message and run the authorization check.

        Returns an EventMessage if the message is authorized, or None
        if the message was unauthorized (and already acked to skip it).

        Only the authorization check itself is guarded. When it raises,
        the message and all prefetched messages are nacked, the consumer
        is cancelled, and the check's exception is re-raised. The
        skip-ack runs outside that guard, so a failed skip-ack is not an
        authorization failure: it escapes as itself, untouched by the
        authorization-failure branch. A dead-channel error from the
        skip-ack or from that branch's own nack (see ``DEAD_CHANNEL_EXC``)
        reaches the calling consume loop, which records it as consumer
        loss.
        """
        message_string = message.body.decode("utf-8")
        parsed_message = json.loads(message_string)
        event_message = EventMessage(parsed_message)

        try:
            authorized = await authz_check(event_message.message)
        except Exception:
            await message.nack(requeue=True)
            await cls._cancel_queue_consumer(consumer_tag, queue)
            await cls._nack_prefetched_messages(messages_queue)
            raise

        if not authorized:
            await message.ack()
            logger.debug(
                "Event skipped due to authorization: event_type=%s, resource_id=%s",
                event_message.message.get("event_type"),
                event_message.message.get("resource_id"),
            )
            return None

        return event_message

    @classmethod
    async def _consume_with_delayed_acks(
        cls,
        queue: AbstractQueue,
        authz_check: Callable[[dict], Awaitable[bool]],
        on_cleanup_complete: Callable[[], Awaitable[None]] | None = None,
    ) -> AsyncGenerator[EventMessage, None]:
        """
        Consume events from a RabbitMQ queue with delayed acking.

        Instead of acking messages immediately (which risks losing a
        message if the client connection drops before the message
        reaches the client), this method delays the ack by
        ``server_ack_delay_seconds``. This gives the TCP stack time to
        deliver the message to the client before we tell RabbitMQ
        the message is done.

        **How it works:**

        Messages are yielded to the caller immediately, but the
        underlying RabbitMQ ack is deferred. Two mechanisms ensure
        every message eventually gets acked:

        1. **Pre-message flush** — Before processing each new
           message from the queue, any previously yielded messages
           whose delay has elapsed are acked.

        2. **Debounced idle timer** — After each message is yielded,
           a background task is scheduled to fire after
           ``server_ack_delay_seconds + 5s``. If another message arrives
           before the timer fires, the timer is cancelled and
           rescheduled (debounced). This handles the "last message
           before a quiet period" case — when no new messages
           arrive to trigger the pre-message flush.

        On disconnect or shutdown, all un-acked messages are nacked
        so RabbitMQ requeues them for redelivery, unless the channel is
        already known dead, in which case the broker has requeued them
        itself and the nacks are skipped.

        :param queue: The queue to consume from
        :param authz_check: Callable that takes the message dict and
                            returns True if the user is authorized
        The ack delay is controlled by the ``server_ack_delay_seconds``
        setting.
        """
        # Messages waiting to be acked: (raw_message, yielded_at_time)
        pending_acks: list[tuple] = []
        idle_timer_task: asyncio.Task | None = None
        # Initialize before the try block so the finally can run even
        # if _start_consuming_from_queue raises. on_cleanup_complete
        # MUST run on every exit path so the caller's pool-channel
        # release fires; otherwise the pool checkout leaks.
        messages_queue: asyncio.Queue | None = None
        consumer_tag: str | None = None
        # Set when the stream ends because the AMQP consumer itself died:
        # either the idle liveness poll classified the loss, or one of the
        # loop's own broker operations on a delivery it holds failed with a
        # dead-channel error. Written only by the loop, in its two `except`
        # handlers below, before its `finally` schedules the cleanup. The
        # exception is kept rather than a bare flag because it carries the
        # channel state observed at DETECTION time, which is what decides
        # the cleanup path (see _cleanup).
        consumer_lost_exc: Exception | None = None

        async def _ack_aged_messages():
            """
            Ack all pending messages older than the configured delay.

            Each message is popped before its ack, so two overlapping
            flushes (the loop's and the idle timer's) never ack the same
            delivery tag. A failed ack is logged and the flush moves on,
            EXCEPT a dead-channel error: that one propagates, because it
            says the channel is gone rather than that one ack failed, and
            every remaining ack would fail the same way. The caller decides
            what it means -- see the loop's dead-channel handler and
            ``_idle_timer_callback``.
            """
            now = time.monotonic()
            cutoff = now - settings.settings.server_ack_delay_seconds
            while pending_acks and pending_acks[0][1] <= cutoff:
                msg, _ = pending_acks.pop(0)
                try:
                    await msg.ack()
                except DEAD_CHANNEL_EXC:
                    raise
                except Exception:
                    logger.exception("Failed to ack aged message")

        async def _idle_timer_callback():
            """
            Fires after a quiet period to ack any remaining
            messages. This is the fallback for when no new
            messages arrive to trigger the pre-message flush.

            A dead-channel error from the flush ends the timer and nothing
            else. The timer is a background task: it cannot end the stream
            and does not write ``consumer_lost_exc``. The loop's next own
            broker operation or its idle liveness poll finds the same dead
            channel and ends the stream.
            """
            try:
                await asyncio.sleep(settings.settings.server_ack_delay_seconds + 5)
                await _ack_aged_messages()
            except asyncio.CancelledError:
                pass
            except DEAD_CHANNEL_EXC:
                logger.debug(
                    "Idle ack flush found the channel dead for queue %s; "
                    "the consume loop ends the stream",
                    redact_queue_name(queue.name),
                )

        def _reset_idle_timer():
            """Cancel the existing idle timer and start a new one."""
            nonlocal idle_timer_task
            if idle_timer_task is not None:
                idle_timer_task.cancel()
            idle_timer_task = asyncio.create_task(_idle_timer_callback())

        async def _nack_pending():
            """Nack all un-acked messages on disconnect/shutdown."""
            for msg, _ in pending_acks:
                try:
                    await msg.nack(requeue=True)
                except Exception:
                    logger.exception("Failed to nack pending message during cleanup")
            pending_acks.clear()

        try:
            messages_queue, consumer_tag = await cls._start_consuming_from_queue(queue)
            # Pin the channel incarnation the consumer is attached to,
            # with no await in between, so the liveness poll can tell a
            # channel that died and reopened from one that never died.
            started_incarnation = cls._channel_incarnation(queue)
            while True:
                if cls.connector.closing:
                    return

                try:
                    message = await cls._next_message(
                        messages_queue, queue, consumer_tag, started_incarnation
                    )
                except cls._CONSUMER_LOST_EXC as exc:
                    # A shutdown that lands mid-classification is a
                    # clean end, not a consumer-loss error -- surfacing
                    # one would turn an ordinary pod shutdown into a
                    # spurious INTERNAL for the client. Mirrors the
                    # `except Exception` handler below; the dead-channel
                    # handler there records its verdict even on a
                    # shutdown, for the reason given at it.
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

                    # Flush any messages whose delay has elapsed
                    await _ack_aged_messages()

                    event_message = await cls._parse_and_authorize(
                        message,
                        authz_check,
                        consumer_tag,
                        queue,
                        messages_queue,
                    )
                    if event_message is None:
                        continue

                    # Track the message for delayed acking
                    pending_acks.append((message, time.monotonic()))
                    yield event_message

                    # Reset the idle timer after each yielded message
                    _reset_idle_timer()

                except DEAD_CHANNEL_EXC as exc:
                    # One of the loop's own broker operations on a delivery
                    # it holds -- the shutdown nack, the aged-ack flush, the
                    # skip-ack, or the authorization-failure nack -- found
                    # the channel dead. Those deliveries arrived on the
                    # consumer's own channel, so this is direct evidence the
                    # consumer's incarnation is gone: the broker requeued
                    # its deliveries and dropped the consumer. It is
                    # recorded as consumer loss so the cleanup narrows, and
                    # the stream ends with the same reconnect guidance as
                    # every other dead-channel path. No queue-existence
                    # check is made first; a deleted queue surfaces as
                    # NOT_FOUND on the reconnect.
                    #
                    # The verdict is recorded BEFORE the shutdown check, the
                    # opposite of the liveness handler above. That handler's
                    # classification can be produced by the shutdown itself,
                    # so on a shutdown it is not evidence of anything. A
                    # dead-channel error proves the channel is gone whatever
                    # the reason the stream is ending, so the futile cancel
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
            # Cancel the idle timer if running
            if idle_timer_task is not None:
                idle_timer_task.cancel()

            # Cancel the consumer before nacking. If we nack first,
            # RabbitMQ requeues the message then immediately
            # re-delivers it to our still-active consumer.
            #
            # Runs in a background task to ensure completion. There
            # is a race condition where requests end before cleanup
            # calls can be completed, so asyncio.create_task
            # guarantees the coroutine runs to completion
            # independently.
            #
            # When the consumer itself was lost -- the liveness poll
            # classified the loss, or one of the loop's own broker
            # operations found the channel dead -- the cleanup narrows to
            # the steps that can still do something. The cancel is
            # always futile -- the broker has already dropped our tag
            # from its registry -- so it is skipped on every
            # consumer-lost path. The nacks are skipped only where the
            # broker has already disposed of the deliveries itself: on
            # a channel close (it requeues them) or a deleted queue
            # (they died with it). A per-consumer cancel on the SAME
            # channel incarnation requeues nothing, so those nacks DO run.
            # on_cleanup_complete still runs, so the channel is released
            # on every path.
            async def _cleanup():
                try:
                    if consumer_tag is not None:
                        if cls._should_cancel_consumer(consumer_lost_exc):
                            await cls._cancel_queue_consumer(consumer_tag, queue)
                        if cls._should_nack_outstanding(consumer_lost_exc):
                            await _nack_pending()
                            await cls._nack_prefetched_messages(messages_queue)
                finally:
                    if on_cleanup_complete:
                        await on_cleanup_complete()

            asyncio.create_task(_cleanup())

    @classmethod
    async def _consume_with_client_acks(
        cls,
        queue: AbstractQueue,
        authz_check: Callable[[dict], Awaitable[bool]],
        ack_tracker: AckTracker,
        on_cleanup_complete: Callable[[], Awaitable[None]] | None = None,
    ) -> AsyncGenerator[EventMessage, None]:
        """
        Consume events from a RabbitMQ queue without auto-acking.

        Messages are tracked in the ack_tracker and must be explicitly
        acked by the client. On disconnect or shutdown, all unacked
        messages are nacked back to RabbitMQ for redelivery, unless the
        channel is already known dead, in which case the broker has
        requeued them itself and the nacks are skipped.

        :param queue: The queue to consume from
        :param authz_check: Callable that takes the message dict and returns
                            True if the user is authorized to consume it
        :param ack_tracker: Tracker for in-flight messages awaiting client acks
        :param on_cleanup_complete: Optional async callback invoked after
                                    cleanup finishes (e.g. to return a channel
                                    to the pool)
        """
        # Initialize before the try block so the finally can run even
        # if _start_consuming_from_queue raises. on_cleanup_complete
        # MUST run on every exit path so the caller's pool-channel
        # release fires; otherwise the pool checkout leaks.
        messages_queue: asyncio.Queue | None = None
        consumer_tag: str | None = None
        # Set when the stream ends because the AMQP consumer itself died:
        # either the idle liveness poll classified the loss, or one of the
        # loop's own broker operations on a delivery it holds failed with a
        # dead-channel error. Written only by the loop, in its two `except`
        # handlers below, before its `finally` schedules the cleanup. The
        # exception is kept rather than a bare flag because it carries the
        # channel state observed at DETECTION time, which is what decides
        # the cleanup path (see _cleanup).
        consumer_lost_exc: Exception | None = None

        try:
            messages_queue, consumer_tag = await cls._start_consuming_from_queue(queue)
            # Pin the channel incarnation the consumer is attached to,
            # with no await in between, so the liveness poll can tell a
            # channel that died and reopened from one that never died.
            started_incarnation = cls._channel_incarnation(queue)
            while True:
                if cls.connector.closing:
                    return

                try:
                    message = await cls._next_message(
                        messages_queue, queue, consumer_tag, started_incarnation
                    )
                except cls._CONSUMER_LOST_EXC as exc:
                    # A shutdown that lands mid-classification is a
                    # clean end, not a consumer-loss error -- surfacing
                    # one would turn an ordinary pod shutdown into a
                    # spurious INTERNAL for the client. Mirrors the
                    # `except Exception` handler below; the dead-channel
                    # handler there records its verdict even on a
                    # shutdown, for the reason given at it.
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

                    event_message = await cls._parse_and_authorize(
                        message,
                        authz_check,
                        consumer_tag,
                        queue,
                        messages_queue,
                    )
                    if event_message is None:
                        continue

                    delivery_tag = ack_tracker.track(message)
                    event_message.delivery_tag = delivery_tag
                    yield event_message

                except DEAD_CHANNEL_EXC as exc:
                    # One of the loop's own broker operations on a delivery
                    # it holds -- the shutdown nack, the skip-ack, or the
                    # authorization-failure nack -- found the channel dead.
                    # Those deliveries arrived on the consumer's own
                    # channel, so this is direct evidence the consumer's
                    # incarnation is gone: the broker requeued its
                    # deliveries and dropped the consumer. It is recorded
                    # as consumer loss so the cleanup narrows, and the
                    # stream ends with the same reconnect guidance as every
                    # other dead-channel path. No queue-existence check is
                    # made first; a deleted queue surfaces as NOT_FOUND on
                    # the reconnect. Client settles do not come through
                    # here: they run on the settle side of the servicer's
                    # merge and mark the tracker instead (see _cleanup).
                    #
                    # The verdict is recorded BEFORE the shutdown check, the
                    # opposite of the liveness handler above. That handler's
                    # classification can be produced by the shutdown itself,
                    # so on a shutdown it is not evidence of anything. A
                    # dead-channel error proves the channel is gone whatever
                    # the reason the stream is ending, so the futile cancel
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
            # The cleanup must be explicitly performed when consumption
            # is finished or interrupted: cancel the consumer, nack
            # all in-flight and prefetched messages back to RabbitMQ,
            # then return the channel.
            #
            # The consumer MUST be cancelled before nacking. If we
            # nack first, RabbitMQ requeues the message to Ready
            # then immediately re-delivers it to our still-active
            # consumer, putting it right back to Unacked.
            #
            # Running the cleanup in a separate asyncio.create_task
            # ensures it completes. There is a race condition where
            # requests end before the cleanup calls can be completed.
            #
            # When the consumer itself was lost -- the liveness poll
            # classified the loss, or one of the loop's own broker
            # operations found the channel dead -- the cleanup narrows to
            # the steps that can still do something. The cancel is
            # always futile -- the broker has already dropped our tag
            # from its registry -- so it is skipped on every
            # consumer-lost path. The nacks are skipped only where the
            # broker has already disposed of the deliveries itself: on
            # a channel close (it requeues them) or a deleted queue
            # (they died with it). A per-consumer cancel on the SAME
            # channel incarnation requeues nothing, so those nacks DO run.
            # on_cleanup_complete still runs, so the channel is released
            # on every path.
            # The loss is not always detected HERE. A client ack or nack
            # that fails against a dead channel discovers it first, on the
            # settle side of the servicer's merge. The tracker marks itself
            # at that failing settle, before the merge cancels this
            # generator's pending read, so the verdict reaches cleanup as
            # ``ack_tracker.channel_dead`` and narrows it identically. It
            # is read inside the task rather than when the task is
            # scheduled, so a mark from a settle that fails in between
            # still counts.
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

    @classmethod
    async def _next_message(
        cls,
        messages_queue: asyncio.Queue,
        queue: AbstractQueue,
        consumer_tag: str,
        started_incarnation: object | None = None,
    ) -> aio_pika.abc.AbstractIncomingMessage | None:
        """
        Await the next delivery, checking consumer liveness while idle.

        aio-pika surfaces no notification when the broker cancels our
        consumer or kills the channel it lives on: aiormq handles the
        broker's Basic.Cancel by silently dropping the consumer from its
        registry, and a per-consumer cancel leaves the channel open so
        the channel close callbacks never fire either. A plain
        ``messages_queue.get()`` therefore blocks forever on a dead
        consumer, leaving the client's stream open and apparently
        healthy against a queue nothing is attached to. Polling the
        liveness predicate off the idle path turns that silent zombie
        into a prompt terminal error, at the cost of at most one check
        per ``consumer_liveness_poll_seconds`` of quiet.

        Cancelling ``asyncio.Queue.get()`` is item-safe: the cancelled
        getter is removed from the waiter list and the next waiter is
        re-woken, so a delivery that races the timeout stays queued and
        is returned by the next iteration.

        :param started_incarnation: The channel incarnation the consumer
            was started on (``_channel_incarnation``), passed through to
            the liveness check so a channel that died and was reopened
            underneath the poll is recognised as a death rather than as
            a live-channel cancel.
        :return: The next delivery, or ``None`` when the connector went
            into shutdown while this wait was idle. ``None`` means
            "clean shutdown, end the stream" -- an idle stream has no
            delivery to wait for and no error to report, and returning
            it here is what lets it honour ``start_servers``' contract
            that marking the connector closing makes the consume loops
            return. Without it an idle stream would keep polling until
            gRPC cancelled it.
        """
        while True:
            # Fast path for a stream that is actually flowing: a buffered
            # delivery needs none of the timer machinery `wait_for` sets
            # up and tears down per call. The liveness poll is a cost
            # paid only while genuinely idle.
            if not messages_queue.empty():
                return messages_queue.get_nowait()

            try:
                return await asyncio.wait_for(
                    messages_queue.get(),
                    timeout=cls._liveness_poll_timeout(),
                )
            except asyncio.TimeoutError:
                # Shutdown is not consumer loss: hand the loop the
                # clean-shutdown sentinel instead of classifying, so an
                # idle stream ends here rather than re-entering the wait
                # and polling until gRPC cancels it.
                if cls.connector.closing:
                    return None
                await cls._raise_if_consumer_lost(
                    queue, consumer_tag, started_incarnation
                )

    @classmethod
    def _liveness_poll_timeout(cls) -> float:
        """
        The idle-poll deadline for one ``_next_message`` wait, jittered.

        Every stream on the pod shares one AMQP connection, so a
        connection-level event leaves them all idle from the same
        instant. On an unjittered deadline their liveness checks would
        then land together, and the paths that do classify against the
        broker would arrive as one synchronized burst. Spreading each
        deadline over a ``_LIVENESS_POLL_JITTER``-wide band costs nothing
        and turns that burst back into a trickle.

        The band is UPWARD-ONLY -- ``[poll, poll * 1.15)`` -- because two
        properties have to hold at once and a symmetric band cannot hold
        both at the setting's own 1.0-second floor. The spread is the
        anti-stampede property; the floor is the anti-spin property, the
        setting's refusal to poll sub-second for broker-timescale events.
        A symmetric band needs a ``max(1.0, ...)`` clamp to keep the
        floor, and at ``poll=1.0`` that clamp swallows the whole downward
        half: measured, the deadlines collapse to a 0.007 s spread at
        poll=1.0 against 0.55 s at poll=2.0, i.e. the jitter goes inert
        at exactly the setting the tightest deployments would choose.
        Stretching upward instead keeps the full 15% spread at every
        value including the floor, and makes the clamp unnecessary --
        the result is never below ``poll``, which the setting already
        bounds at 1.0. The cost is a mean deadline 7.5% above the
        configured window, which is detection latency the setting is
        free to buy back.
        """
        jitter = cls._LIVENESS_POLL_JITTER
        poll_seconds = settings.settings.consumer_liveness_poll_seconds
        return poll_seconds * random.uniform(1.0, 1.0 + jitter)

    @staticmethod
    def _should_cancel_consumer(
        consumer_lost_exc: Exception | None,
        channel_dead: bool = False,
    ) -> bool:
        """
        Whether cleanup should still cancel the AMQP consumer.

        Only on a normal exit. Every consumer-lost outcome means the
        broker has already dropped our tag from its own registry --
        that absence is what the liveness check detected, and a dead
        channel takes its consumers with it -- so
        ``basic.cancel`` is a round trip that can only fail, and on a
        reopened RobustChannel it would be a cancel for a tag that
        incarnation never knew.

        :param consumer_lost_exc: The consumer-loss exception that ended
            the stream, or None if it ended for any other reason.
        :param channel_dead: The settle side's verdict, from
            ``AckTracker.channel_dead``. See
            :meth:`_should_nack_outstanding` for why the two detection
            sites are both consulted here.
        """
        if channel_dead:
            return False
        return consumer_lost_exc is None

    @staticmethod
    def _should_nack_outstanding(
        consumer_lost_exc: Exception | None,
        channel_dead: bool = False,
    ) -> bool:
        """
        Whether cleanup should still nack the outstanding deliveries.

        The nacks are futile only where the broker has already disposed
        of those deliveries itself:

        * ``QueueNotFoundError`` -- the queue is gone, and every delivery
          outstanding against it went with it.
        * ``ConsumerCancelledError(channel_closed=True)`` -- the broker
          requeued every unacked delivery when it closed the channel.
          Note this covers the channel that closed and was then reopened
          in place by aio-pika before the poll observed it (the flagship
          ack-timeout kill): ``channel_closed`` is an incarnation
          comparison, so a reopen does not disguise the death, and the
          nacks are correctly skipped rather than aimed at delivery tags
          the dead incarnation took with it.

        A per-consumer ``basic.cancel`` on the SAME channel incarnation
        the consumer was started on (a quorum leader failover, an
        operator cancel) requeues nothing, so those deliveries are still
        ours to nack and must be, or the channel goes back to the pool
        with its prefetch budget permanently burned by unacked ghosts.

        A channel death has TWO possible witnesses, and either one alone
        is conclusive. ``consumer_lost_exc`` is the event side's, written
        inside the consume generator: either the idle liveness poll
        noticed the incarnation was gone, or one of the loop's own broker
        operations on a delivery it holds (the shutdown nack, the aged-ack
        flush, the skip-ack, the authorization-failure nack) came back
        with a dead-channel error. ``channel_dead`` is the settle side's:
        a client ack/nack went to the broker and came back with a
        dead-channel error, which is what happens first whenever the
        client is settling at the moment of the kill. That one arrives via
        the tracker rather than as a raised exception, because it is
        discovered outside the generator: the error travels up through the
        servicer's ``merge``, which, when the generator has a read
        pending, tears it down by cancelling that read -- so
        ``consumer_lost_exc`` stays None on precisely the path where the
        channel is most certainly dead. The tracker marks itself at the
        failing settle, before any teardown, so the verdict is in place
        when cleanup reads it. Both witnesses land on this one predicate
        so the cleanup decision has a single definition.

        :param consumer_lost_exc: The consumer-loss exception that ended
            the stream, or None if it ended for any other reason.
        :param channel_dead: The settle side's verdict, from
            ``AckTracker.channel_dead``. False for a consume path with no
            client-settle surface, which has only the event-side witness.
        """
        if channel_dead:
            return False
        if consumer_lost_exc is None:
            return True
        if isinstance(consumer_lost_exc, ConsumerCancelledError):
            return not consumer_lost_exc.channel_closed
        return False

    @classmethod
    def _connection_is_down(cls) -> bool:
        """
        Whether the connector's shared AMQP connection is currently down.

        The load-bearing check is the ``connected`` event
        (``AbstractConnection.connected: asyncio.Event``), NOT
        ``is_closed``. ``is_closed`` resolves only on an explicit local
        ``close()``, so a broker-side drop leaves a RobustConnection
        reading ``is_closed=False`` while it is plainly not connected
        (verified against the pinned aio-pika: after a blip,
        ``is_closed=False connected=False``). aio-pika clears
        ``connected`` on every disconnect and re-sets it once a
        reconnect attempt succeeds, which is exactly the liveness
        question being asked here. ``is_closed`` is still read, since it
        is the signal for a connection that is down for good.

        ``RobustConnection.reconnecting`` is deliberately NOT consulted:
        it is the reconnect lock, which stays held for a further
        ``reconnect_interval`` after a successful reconnect, so it reads
        True on a connection that is already back up.

        Read straight off the connector's connection handle because the
        connector exposes no public predicate for this, and its
        ``connection()`` accessor is unusable here -- it RECONNECTS as a
        side effect, which is the opposite of a question.
        """
        connection = cls.connector._connection
        if connection is None or connection.is_closed:
            return True
        return not connection.connected.is_set()

    @staticmethod
    def _channel_incarnation(queue: AbstractQueue) -> object | None:
        """
        Identify the channel incarnation a consumer is living on.

        Pool channels are ``RobustChannel``s: when the broker closes one
        (an ack-timeout enforcement, any other channel-fatal error, a
        connection blip) aio-pika reopens it in place within
        milliseconds, reusing the same wrapper object. Everything the
        stream held on the dead incarnation -- its consumer, its
        outstanding delivery tags -- died with it, but the wrapper's
        ``is_closed`` is back to False long before the next liveness poll
        a second later, so the wrapper alone cannot answer the question
        that matters: "did the channel our consumer was started on die?"
        The underlay CAN answer it. ``Channel._open`` builds a fresh
        underlay on every open, so a different object is a different
        incarnation, and comparing the current one against the one
        captured at consume start is the whole test.

        Read synchronously off ``Channel._channel`` rather than through
        the public ``get_underlay_channel()`` because the capture has to
        be ATOMIC with the consume it follows: on a RobustChannel that
        coroutine first awaits ``connection.ready()``, and a suspension
        point between the ``basic.consume`` and the capture would let the
        very close being guarded against land in the gap, pinning the
        wrong incarnation. An attribute read cannot suspend. (Same
        reasoning as ``_connection_is_down``, which reads the connector's
        connection handle directly.)

        Reading a private attribute means the answer is best-effort, so
        it is read in a way that cannot RAISE: this runs at consume start
        for every stream, and a pinned-library change here must degrade
        the liveness check back to its coarser fallback rather than fail
        streams that would otherwise work.

        :param queue: The queue whose channel is being identified.
        :return: An opaque identity for the channel's current
            incarnation, or None when there is none to identify -- an
            uninitialized channel, or a channel that no longer exposes
            its underlay. None means "unknown", and the liveness check
            then falls back to the wrapper's own closed state.
        """
        channel = getattr(queue, "channel", None)
        return getattr(channel, "_channel", None)

    @classmethod
    async def _raise_if_consumer_lost(
        cls,
        queue: AbstractQueue,
        consumer_tag: str,
        started_incarnation: object | None = None,
    ) -> None:
        """
        Terminate the stream if our AMQP consumer is no longer attached.

        The predicate is purely in-process — no broker round trip — so
        it is cheap enough to run on every idle poll. The consumer is
        lost if the channel incarnation it was started on is gone, or if
        our tag is absent from that same incarnation's consumer
        registry. That one predicate covers all four ways the consumer
        dies without the stream noticing: an ack-timeout channel kill,
        any other channel-fatal error, a connection blip, and a
        broker-initiated ``basic.cancel`` from the queue being deleted.
        Production declares queues with ``robust=False``, so aio-pika's
        channel recovery does NOT restore these consumers — the channel
        reopens with an empty registry and our tag stays absent.

        Only once the consumer is known lost do we pay for a broker
        round trip, to classify the cause: a deleted queue raises
        ``QueueNotFoundError`` (404 / NOT_FOUND) so the client knows to
        reprovision, while a surviving queue raises
        ``ConsumerCancelledError`` so the client knows to reconnect.

        Whether the channel died is captured HERE, at detection time,
        and carried on ``ConsumerCancelledError``, because it decides how
        the consume loops clean up (see ``_should_nack_outstanding``).
        It is an INCARNATION comparison, not a reading of the wrapper's
        ``is_closed``: measured against a live broker, an ack-timeout
        close is followed by a RobustChannel reopen ~75 ms later, so a
        poll on the ~1 s cadence reads ``is_closed`` False off a channel
        that is brand new. Believing that reading classified the flagship
        ack-timeout kill as a live-channel cancel, and cleanup then
        nacked delivery tags belonging to the dead incarnation — tags the
        broker had already requeued when it closed the channel — logging
        one caught ``ChannelInvalidStateError`` per outstanding delivery,
        up to ``durable_consumer_prefetch_count`` of them per timed-out
        client. ``channel_closed=False`` now means only what it says: a
        ``basic.cancel`` against a consumer whose channel is the SAME
        one it was started on, and whose deliveries are therefore still
        ours to nack.

        :param queue: The queue the consumer was started on.
        :param consumer_tag: The tag returned by ``basic.consume``.
        :param started_incarnation: The channel incarnation the consumer
            was started on, from ``_channel_incarnation``. None when the
            caller did not capture one, in which case the check degrades
            to the wrapper's current closed state and cannot tell a
            reopened channel from one that never closed.
        """
        # Every warning below names the queue, and this runs on non-durable
        # streams too, whose name embeds the reconnect token -- so the name
        # is redacted once here and the redacted form is what gets logged.
        log_name = redact_queue_name(queue.name)

        channel = queue.channel
        channel_closed = channel.is_closed
        if channel_closed:
            lost = True
        else:
            try:
                underlay = await channel.get_underlay_channel()
            except aio_pika.exceptions.ChannelInvalidStateError:
                # The channel is mid-reopen and has no underlay yet, so
                # it is certainly not carrying our consumer -- and the
                # incarnation our deliveries arrived on is already gone,
                # which is a channel close by any other name.
                lost = True
                channel_closed = True
            else:
                # Re-read the incarnation AFTER that await, not before:
                # the await is the only window in which the channel can
                # flip under us, and reading on the far side of it keeps
                # the registry lookup below and this comparison talking
                # about the same incarnation.
                reopened = (
                    started_incarnation is not None
                    and cls._channel_incarnation(queue) is not started_incarnation
                )
                if reopened:
                    # The wrapper reads open, but this is a NEW
                    # incarnation: the one the consumer was started on
                    # closed, and aio-pika reopened in place. That close
                    # took the consumer with it and requeued every
                    # delivery outstanding on it, so it is a channel
                    # death however healthy the wrapper looks now.
                    lost = True
                    channel_closed = True
                else:
                    lost = consumer_tag not in underlay.consumers

        if not lost:
            return

        if cls._connection_is_down():
            # The classifying round trip opens a temporary channel, and
            # `_assert_queue_exists` retries it three times a second
            # apart. Against a connection that is itself down that can
            # only fail, and the connection is shared by every stream on
            # the pod -- so this is precisely the case where the
            # classification storms the broker while it is least able to
            # answer. The cause is not in doubt anyway.
            logger.warning(
                "Consumer liveness check failed: consumer '%s' on queue "
                "'%s' is gone and the broker connection is down; ending "
                "the stream so the client reconnects",
                consumer_tag,
                log_name,
            )
            # The broker requeues every unacked delivery on a connection
            # it has lost, so the deliveries are already back on the
            # queue regardless of what this channel handle reports.
            raise ConsumerCancelledError(queue.name, channel_closed=True)

        try:
            await cls._assert_queue_exists(queue.name)
        except QueueNotFoundError:
            logger.warning(
                "Consumer liveness check failed: consumer '%s' is gone and "
                "queue '%s' no longer exists; ending the stream so the "
                "client reprovisions the queue",
                consumer_tag,
                log_name,
            )
            raise
        except Exception as exc:
            # `_assert_queue_exists` converts only a ChannelClosed into
            # QueueNotFoundError; a connection-level failure, an invalid
            # channel state or a timeout escapes as itself. Letting it
            # propagate would end the stream with a bare, unactionable
            # 500 and -- because it is not a consumer-loss type -- send
            # cleanup down the full cancel-and-nack path against a broker
            # that just failed to answer. The consumer is already known
            # lost at this point; only the cause is unknown, so it is
            # reported as a cancel and logged with the real error.
            logger.warning(
                "Consumer liveness check failed: consumer '%s' is gone and "
                "the check for queue '%s' could not reach the broker; "
                "ending the stream so the client reconnects",
                consumer_tag,
                log_name,
                exc_info=True,
            )
            raise ConsumerCancelledError(
                queue.name, channel_closed=channel_closed
            ) from exc

        logger.warning(
            "Consumer liveness check failed: consumer '%s' is no longer "
            "registered on a live channel (broker cancel or channel close) "
            "while queue '%s' still exists; ending the stream so the client "
            "reconnects",
            consumer_tag,
            log_name,
        )
        raise ConsumerCancelledError(queue.name, channel_closed=channel_closed)

    @staticmethod
    async def _start_consuming_from_queue(
        queue: AbstractQueue,
    ) -> tuple[asyncio.Queue, str]:
        """
        Initializes message consumption from the given RabbitMQ queue.

        This method sets up an internal asyncio.Queue to buffer incoming messages
        from RabbitMQ, defines a callback to populate this buffer, and starts
        the consumption process on the provided RabbitMQ queue.

        We initialize it this way to get the consumer tag, which is
        used to explicitly cancel the consumer.
        """
        messages_queue = asyncio.Queue()

        async def callback(message: aio_pika.abc.AbstractIncomingMessage):
            await messages_queue.put(message)

        # The consumer acknowledgement timeout is NOT set here. RabbitMQ
        # accepts it only as a queue argument, as the `consumer-timeout`
        # policy key, or in the server config -- a basic.consume argument
        # is silently ignored, so passing it here bought nothing while
        # reading as if hung clients were bounded. The chart's queue-policy
        # hook carries the real lever
        # (policyHook.policy.definition.consumerTimeoutMs).
        consumer_tag = await queue.consume(callback)
        return messages_queue, consumer_tag

    @staticmethod
    async def _cancel_queue_consumer(consumer_tag: str, queue: AbstractQueue):
        """
        Cancel the consumer for the given queue using the provided consumer tag.
        """
        # Both lines name the queue and both run on non-durable streams,
        # whose name embeds the reconnect token, so the redacted form is
        # what is written down.
        log_name = redact_queue_name(queue.name)

        # noinspection PyBroadException
        try:
            await queue.cancel(consumer_tag)
            logger.debug(
                f"Successfully cancelled consumer '{consumer_tag}' on '{log_name}'"
            )
        except Exception:
            logger.exception(f"Failed to cancel consumer on queue '{log_name}'")

    @staticmethod
    async def _nack_prefetched_messages(messages_queue: asyncio.Queue):
        """
        Drain and nack all remaining prefetched messages from the asyncio queue.

        This ensures that messages which were prefetched by RabbitMQ but not
        yet processed are properly nacked and requeued.

        :param messages_queue: The asyncio.Queue contains prefetched messages
        """
        while not messages_queue.empty():
            # noinspection PyBroadException
            try:
                message = messages_queue.get_nowait()
                # noinspection PyUnresolvedReferences
                await message.nack(requeue=True)
            except asyncio.QueueEmpty:
                break
            except Exception:
                logger.exception("Failed to nack prefetched message")

    @classmethod
    async def _update_bindings(
        cls,
        channel,
        queue: AbstractQueue,
        current_filter_groups: list[FilterGroup],
        new_filter_groups: list[FilterGroup],
    ) -> None:
        if not current_filter_groups and not new_filter_groups:
            return

        groups_to_add = FilterGroup.added_groups(
            current_filter_groups, new_filter_groups
        )

        groups_to_remove = FilterGroup.removed_groups(
            current_filter_groups, new_filter_groups
        )

        binding_keys_to_add = binding_keys_from_filters(groups_to_add)
        binding_keys_to_remove = binding_keys_from_filters(groups_to_remove)

        logger.debug(
            "Updating bindings. "
            f"Adding: {binding_keys_to_add}, and removing: {binding_keys_to_remove}."
        )

        exchange = await cls.connector.get_exchange(channel)

        for binding_key in binding_keys_to_add:
            await queue.bind(exchange, binding_key)

        for binding_key in binding_keys_to_remove:
            await queue.unbind(exchange, binding_key)

    @staticmethod
    async def _queue_name(queue_id) -> str:
        """
        Map a queue id onto its broker queue name.

        The name is derived from the queue id alone, so the same id always
        resolves to the same broker queue no matter who asks. Nothing about
        the caller's identity is mixed in: a broker name is immutable once
        declared, so deriving it from an identity string would let any
        change to that string (a different principal-id claim, an identity
        provider migration, a re-created service account) silently orphan
        every queue the client owns.

        It stays a coroutine because subclasses override it, callers await
        it on the lookup path, and the tests patch it as an async mock.
        """
        return f"{QUEUE_NAME_PREFIX}:{queue_id}"
