# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Characterization: the CHANNEL-FATAL blast radius on a shared packed channel.

Companion to ``test_ac6_queue_delete_blast_radius.py``. That file covers the
BENIGN blast radius -- a plain ``queue.delete`` is a per-consumer
``basic.cancel`` that the AMQP spec guarantees targets one consumer-tag and
leaves the channel and its siblings intact. This file covers the SHARP one that
packing actually introduced (issue b.y7y, Part 2): a CHANNEL-LEVEL fault
(``ChannelPreconditionFailed`` / access-refused / bad redeclare) closes the
WHOLE shared channel, and every co-tenant stream packed onto it (up to the
shipped ``client_limit`` of 8) rides on that one channel.

The control forcing the fault is the documented one from the source
repository's internal channel-capacity design record (not published; its Blast
radius section): a bare
``queue.delete(if_unused=True)`` against a still-consumed queue raises
``PRECONDITION_FAILED`` and the channel dies -- the *channel-fatal* counterpart
to AC-6's channel-safe ``basic.cancel`` delete. We force it on the SHARED
packed channel and then measure what actually happens to the 7 co-tenants.

CHARACTERIZATION, not an invented contract
------------------------------------------
There is NO documented intended channel-fatal co-tenant behavior to assert
against, so this file does NOT assert a "must reconnect / must terminate"
contract. It RECORDS the measured behavior on the shipped stack (aio-pika
``RobustChannel`` / ``RobustQueue`` over a live ``rabbitmq:4.1.3`` broker) and
pins that measured answer so a regression -- or a library/version change that
alters it -- surfaces here. If a future measurement differs, correct these
assertions to the new measured answer rather than weakening them.

Measured behavior (host.docker.internal, rabbitmq:4.1.3, aio-pika RobustChannel;
b.y7y test-writer session, 2026-08-19)
--------------------------------------------------------------------------------
1. The fault IS channel-level: ``queue_delete(if_unused=True)`` on the shared
   channel against a still-consumed queue raises ``ChannelPreconditionFailed``,
   and the shared channel is observed CLOSED (``is_closed`` True) synchronously
   right after -- the whole channel died, not just the one consumer.
2. aio-pika's ``RobustChannel`` AUTO-REOPENS the SAME channel object within a
   fraction of a second (``_on_close`` -> ``restore`` -> ``reopen`` ->
   ``_on_open`` re-declares exchanges and restores every robust queue and its
   consumer). The pool never swaps the object, so ``ChannelPool._all_channels``
   still tracks the same instance and ``ChannelInfo.is_closed()`` reads False
   again -- so the pool's lazy ``is_closed()`` eviction never fires; the robust
   layer heals the channel before the next checkout.
3. NO pool checkout is reclaimed OR lost across the fault:
   ``pool_stats()["total_client_count"]`` stays at ``SIBLING_COUNT`` throughout.
   The co-tenants are NOT surfaced or terminated, and they do NOT leak.
4. The co-tenant buffers DO deliver again: once the robust restore re-declares
   the ``auto_delete`` queue and re-attaches the consumer, every survivor buffer
   -- and the faulted stream's own buffer -- resumes receiving. The one caveat
   is a brief DELIVERY GAP: a message published during the ~0.1 s reopen window,
   while the ``auto_delete`` queue is momentarily re-declaring, is dropped as
   unroutable. So the resume assertions publish-retry across that gap rather
   than asserting a single post-fault publish (which would be racy). Message
   CONTINUITY across a channel fault is therefore NOT guaranteed, but consumer
   RECOVERY is.

Net finding for b.y7y Part 2: on the shipped aio-pika robust stack a
channel-level fault on a packed channel is transparently RECOVERED -- the feared
"already-attached consumers are silently and permanently dead" outcome does NOT
occur for this fault class; the co-tenants briefly see a closed channel, then
resume, with a small message-continuity gap during the reopen. The residual
risk (the delivery gap; and whether every channel-fatal class auto-restores) is
recorded here for the deferred follow-up decision, not silently asserted away.

Runs in CI's ``integration:`` job (``.gitlab-ci.yml``) against a
``rabbitmq:4.1.3`` service with ``OMNI_EVENTS_REQUIRE_LIVE_BROKER=1`` so a
missing broker HARD-fails; skips cleanly in the dev sandbox with no broker.
"""

import asyncio
import uuid
from types import SimpleNamespace

import aio_pika
import pytest
import pytest_asyncio
from aio_pika.exceptions import ChannelPreconditionFailed

from notification_common.rabbitmq.channel_pool import ChannelPool, ChannelPoolConfig
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase

# Reuse the AC-6 packing rig's shared constants/helpers rather than restating
# them: the density under test is identical (the shipped ``client_limit`` of 8
# streams on one channel); only the fault class differs (channel-fatal vs
# ``basic.cancel``).
from tests.integration_tests.rabbitmq.test_ac6_queue_delete_blast_radius import (
    SIBLING_COUNT,
    _message_body,
    _receive,
    _routing_key,
)

# The stream whose queue we force the channel-fatal fault against; the other 7
# are the co-tenants whose blast-radius we characterize.
VICTIM_INDEX = 0

# Bounded baseline wait -- a hung baseline fails as a named timeout.
BASELINE_TIMEOUT_S = 15

# The shared channel is force-closed by the fault; aio-pika's RobustChannel
# reopens it in a fraction of a second (measured ~0.1 s). This grace is generous
# so CI load cannot make the reopen look like a non-reopen, while still bounding
# the wait so a genuine never-reopen fails as a timeout rather than hanging.
REOPEN_GRACE_S = 15
REOPEN_POLL_S = 0.1

# Co-tenant delivery resumes once the robust restore re-attaches the consumer,
# but a publish landing in the ~0.1 s reopen window is dropped as unroutable
# (the auto_delete queue is momentarily re-declaring). So resume is proven by
# publish-retry across that gap: up to RESUME_ATTEMPTS publishes, each awaited
# for RESUME_PER_ATTEMPT_S. The product is a generous ceiling; a buffer that
# never resumes exhausts every attempt and is recorded as not-resumed.
RESUME_ATTEMPTS = 30
RESUME_PER_ATTEMPT_S = 0.5


def _survivor_indices():
    """The co-tenant indices -- every stream but the one we fault."""
    return [index for index in range(SIBLING_COUNT) if index != VICTIM_INDEX]


async def _await_reopen(channel, timeout, poll):
    """Poll ``channel.is_closed`` until it reopens, returning True on reopen.

    Returns False if the channel is still closed after ``timeout`` -- the
    never-reopened outcome, recorded rather than hung on.
    """
    loop = asyncio.get_running_loop()
    deadline = loop.time() + timeout
    while loop.time() < deadline:
        if not channel.is_closed:
            return True
        await asyncio.sleep(poll)
    return not channel.is_closed


class ChannelFatalObservations(SimpleNamespace):
    """The measured record of one channel-fatal episode on a packed channel."""


@pytest_asyncio.fixture
async def channel_fatal_characterization(live_broker_connection):
    """
    Pack 8 streams onto one channel, force a CHANNEL-FATAL fault, measure.

    Runs the full episode once against the live broker and records every
    measured value into a :class:`ChannelFatalObservations`; the test methods
    below assert those recorded fields (the record-then-assert pattern of
    ``test_drain_sigterm_sequence.py``'s ``drain_episode``), so the expensive
    live sequence runs once per test rather than the broker being poked from
    each assertion.

    Steps, all against the live broker:

    1. Build a ``ChannelPool`` with ``pool_size=1`` / ``min_size=1`` so all
       ``SIBLING_COUNT`` checkouts land on the SAME channel; ``client_limit`` is
       the shipped default (8). Declare a NON-``auto_delete`` topic exchange
       (persistent so post-fault publishes still route) and, per stream, an
       ``auto_delete`` queue bound to a distinct routing key, consumed via the
       PRODUCTION helper ``RabbitMQBase._start_consuming_from_queue`` (exercising
       its real setup).
    2. Baseline: publish one message per routing key; record how many buffers
       receive their own message (non-vacuity control).
    3. Force the channel-fatal fault: ``queue_delete(if_unused=True)`` on the
       SHARED channel against the still-consumed victim queue -> the documented
       ``PRECONDITION_FAILED``. Record the exception type and, synchronously,
       whether the shared channel is now closed.
    4. Recovery: poll for the RobustChannel to auto-reopen; record whether it
       reopened, whether the pool still tracks the SAME object, the
       ``ChannelInfo.is_closed()`` reading, and the pool's client count after
       reopen.
    5. Resume: publish-retry each survivor (and the victim) across the reopen
       gap; record how many resume and the client count afterwards.
    """
    connection = live_broker_connection

    pool = ChannelPool(
        connection,
        ChannelPoolConfig(pool_size=1, min_size=1),
        name=RabbitMQConnector.NON_DURABLE_POOL_NAME,
    )
    await pool.initialize()

    exchange_name = f"chfatal-{uuid.uuid4()}"
    setup_channel = None
    try:
        setup_channel = await connection.channel()
        # NON auto_delete: it must survive the shared channel's death so
        # post-fault publishes still route (an auto_delete exchange would vanish
        # with the packed queues and confound the resume measurement).
        exchange = await setup_channel.declare_exchange(
            exchange_name, type="topic", durable=False, auto_delete=False
        )

        checked_out_channels = []
        routing_keys = []
        queue_names = []
        buffers = []
        for index in range(SIBLING_COUNT):
            channel = await pool.get_channel()
            checked_out_channels.append(channel)

            routing_key = _routing_key(index)
            queue_name = f"chfatal-q-{index}-{uuid.uuid4()}"
            queue = await channel.declare_queue(
                queue_name, durable=False, auto_delete=True
            )
            await queue.bind(exchange_name, routing_key)

            buffer, _tag = await RabbitMQBase._start_consuming_from_queue(queue)
            routing_keys.append(routing_key)
            queue_names.append(queue_name)
            buffers.append(buffer)

        shared_channel = checked_out_channels[0]
        all_on_one_channel = all(c is shared_channel for c in checked_out_channels)

        # (2) Baseline non-vacuity: every sibling delivers before the fault.
        for index in range(SIBLING_COUNT):
            await exchange.publish(
                aio_pika.Message(body=_message_body(routing_keys[index])),
                routing_key=routing_keys[index],
            )
        baseline_delivered = 0
        for index in range(SIBLING_COUNT):
            message = await _receive(
                buffers[index],
                BASELINE_TIMEOUT_S,
                f"baseline delivery to sibling {index}",
            )
            if message.body == _message_body(routing_keys[index]):
                baseline_delivered += 1

        clients_before_fault = pool.pool_stats()["total_client_count"]

        # (3) Force the CHANNEL-FATAL fault on the shared channel itself.
        fault_exc_type = None
        try:
            await shared_channel.queue_delete(queue_names[VICTIM_INDEX], if_unused=True)
        except Exception as exc:  # noqa: BLE001 - characterizing the fault class
            fault_exc_type = type(exc).__name__
        # Synchronous read BEFORE any await yields to the loop, so the reopen
        # cannot race ahead of this "closed by the fault" observation.
        closed_immediately = shared_channel.is_closed
        clients_immediately_after = pool.pool_stats()["total_client_count"]

        # (4) Recovery: let the RobustChannel auto-reopen.
        reopened = await _await_reopen(shared_channel, REOPEN_GRACE_S, REOPEN_POLL_S)
        pool_still_tracks_same_object = shared_channel in pool._all_channels
        channel_info = pool._all_channels.get(shared_channel)
        info_closed_after_reopen = (
            channel_info.is_closed() if channel_info is not None else None
        )
        clients_after_reopen = pool.pool_stats()["total_client_count"]

        # (5) Resume: publish-retry across the reopen gap.
        async def _resume(index):
            for _attempt in range(RESUME_ATTEMPTS):
                await exchange.publish(
                    aio_pika.Message(body=_message_body(routing_keys[index])),
                    routing_key=routing_keys[index],
                )
                try:
                    await asyncio.wait_for(
                        buffers[index].get(), timeout=RESUME_PER_ATTEMPT_S
                    )
                    return True
                except asyncio.TimeoutError:
                    continue
            return False

        survivor_indices = _survivor_indices()
        survivors_resumed = 0
        for index in survivor_indices:
            if await _resume(index):
                survivors_resumed += 1
        victim_resumed = await _resume(VICTIM_INDEX)
        clients_after_resume = pool.pool_stats()["total_client_count"]

        yield ChannelFatalObservations(
            all_on_one_channel=all_on_one_channel,
            baseline_delivered=baseline_delivered,
            fault_exc_type=fault_exc_type,
            closed_immediately=closed_immediately,
            clients_before_fault=clients_before_fault,
            clients_immediately_after=clients_immediately_after,
            reopened=reopened,
            pool_still_tracks_same_object=pool_still_tracks_same_object,
            info_closed_after_reopen=info_closed_after_reopen,
            clients_after_reopen=clients_after_reopen,
            survivor_count=len(survivor_indices),
            survivors_resumed=survivors_resumed,
            victim_resumed=victim_resumed,
            clients_after_resume=clients_after_resume,
        )
    finally:
        await pool.close_all()
        if setup_channel is not None:
            try:
                await setup_channel.exchange_delete(exchange_name)
            finally:
                await setup_channel.close()


class TestPackingPremise:
    """The eight checkouts really do share one channel and delivered a baseline."""

    @pytest.mark.asyncio
    async def test_all_eight_streams_packed_onto_one_channel(
        self, channel_fatal_characterization
    ):
        # Without a shared channel there is no channel-fatal blast radius to
        # characterize, so this premise fails loudly if selection ever spread
        # the streams across channels.
        assert channel_fatal_characterization.all_on_one_channel is True

    @pytest.mark.asyncio
    async def test_every_stream_delivered_before_the_fault(
        self, channel_fatal_characterization
    ):
        # Non-vacuity: "co-tenants resume" is meaningless if they never
        # delivered to begin with.
        assert channel_fatal_characterization.baseline_delivered == SIBLING_COUNT


class TestTheFaultIsChannelLevel:
    """The forced fault really is channel-fatal, not a benign basic.cancel."""

    @pytest.mark.asyncio
    async def test_the_fault_raises_channel_precondition_failed(
        self, channel_fatal_characterization
    ):
        # This is the control distinguishing this file from AC-6: an
        # ``if_unused=True`` delete against a consumed queue is PRECONDITION
        # FAILED, a channel-level error, not a per-consumer cancel.
        assert (
            channel_fatal_characterization.fault_exc_type
            == ChannelPreconditionFailed.__name__
        )

    @pytest.mark.asyncio
    async def test_the_whole_shared_channel_was_closed_by_the_fault(
        self, channel_fatal_characterization
    ):
        # Measured: the channel-level error closed the ENTIRE shared channel
        # (every co-tenant rides on it), unlike AC-6's channel-safe delete.
        assert channel_fatal_characterization.closed_immediately is True


class TestTheRobustChannelAutoReopens:
    """
    Measured: aio-pika's RobustChannel heals the packed channel in place.

    Recorded characterization, not an intended contract: the shipped robust
    stack reopens the SAME channel object and restores its queues/consumers, so
    the pool's object-identity tracking and lazy ``is_closed()`` eviction see a
    healthy channel again rather than a dead one.
    """

    @pytest.mark.asyncio
    async def test_the_shared_channel_auto_reopens_within_the_grace(
        self, channel_fatal_characterization
    ):
        assert channel_fatal_characterization.reopened is True

    @pytest.mark.asyncio
    async def test_the_pool_still_tracks_the_same_channel_object(
        self, channel_fatal_characterization
    ):
        # RobustChannel reopens IN PLACE -- the pool never swapped the object,
        # so ``_all_channels`` object-identity tracking still holds it.
        assert channel_fatal_characterization.pool_still_tracks_same_object is True

    @pytest.mark.asyncio
    async def test_channel_info_reports_the_channel_healthy_after_reopen(
        self, channel_fatal_characterization
    ):
        # The pool's lazy eviction key: ``ChannelInfo.is_closed()`` reads False
        # after the reopen, so the dead channel is never evicted -- the robust
        # layer healed it before the next checkout would have swept it.
        assert channel_fatal_characterization.info_closed_after_reopen is False


class TestNoCheckoutIsReclaimedOrLost:
    """
    Measured: the co-tenants are neither terminated nor leaked across the fault.

    ``total_client_count`` stays at ``SIBLING_COUNT`` from before the fault
    through the reopen and the resume -- the pool neither reclaims the 7
    co-tenant checkouts nor loses count of them.
    """

    @pytest.mark.asyncio
    async def test_client_count_held_before_the_fault(
        self, channel_fatal_characterization
    ):
        assert channel_fatal_characterization.clients_before_fault == SIBLING_COUNT

    @pytest.mark.asyncio
    async def test_client_count_unchanged_immediately_after_the_fault(
        self, channel_fatal_characterization
    ):
        assert channel_fatal_characterization.clients_immediately_after == SIBLING_COUNT

    @pytest.mark.asyncio
    async def test_client_count_unchanged_after_reopen(
        self, channel_fatal_characterization
    ):
        assert channel_fatal_characterization.clients_after_reopen == SIBLING_COUNT

    @pytest.mark.asyncio
    async def test_client_count_unchanged_after_resume(
        self, channel_fatal_characterization
    ):
        assert channel_fatal_characterization.clients_after_resume == SIBLING_COUNT


class TestCoTenantsResumeDelivery:
    """
    Measured: every co-tenant buffer delivers again after the reopen.

    Recorded characterization: once the robust restore re-declares the queue and
    re-attaches the consumer, all ``SIBLING_COUNT - 1`` survivor buffers -- and
    the faulted stream's own buffer -- resume receiving (proven by publish-retry
    across the brief reopen delivery gap). The feared "already-attached
    consumers silently stop" outcome does NOT occur for this fault class.
    """

    @pytest.mark.asyncio
    async def test_all_co_tenant_buffers_resume_delivery(
        self, channel_fatal_characterization
    ):
        obs = channel_fatal_characterization
        assert obs.survivors_resumed == obs.survivor_count

    @pytest.mark.asyncio
    async def test_the_faulted_streams_own_buffer_also_resumes(
        self, channel_fatal_characterization
    ):
        # The victim's ``if_unused=True`` delete FAILED (precondition), so its
        # queue survived and its consumer is restored on reopen too.
        assert channel_fatal_characterization.victim_resumed is True
