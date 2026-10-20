# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Named acceptance test for PRD AC-6: queue-delete blast radius on a shared channel.

AC-6 verbatim: "Deleting one stream's queue on a shared channel leaves sibling
streams delivering (verified by test at the default packing density)."

Packing turns this into a BLAST-RADIUS question. Before packing one stream owned
one channel and there were no siblings to harm; at the shipped ``client_limit``
of 8 a channel carries up to 8 streams, so deleting one stream's queue must not
disturb the other 7. The SDD records the underlying broker fact as an existing
contract -- "deletion of one queue cancels only its own consumer (broker CCN
semantics, measured to 1000 consumers/channel)" -- so the proof runs against a
REAL broker; a mocked channel would only assert our own fake's behaviour.

This file therefore SKIPS in the dev sandbox when no broker is reachable and
runs in CI's ``integration:`` job (``.gitlab-ci.yml``), which starts a
``rabbitmq:4.1.3`` service and sets ``OMNI_EVENTS_REQUIRE_LIVE_BROKER=1`` so the
``live_broker_connection`` fixture HARD-fails rather than skips if that broker
is absent (issue b.y7y).

Packed to the full shipped density
----------------------------------
Per the PM review addition, the shared channel is packed to 8 streams (the
shipped ``client_limit``), one queue is deleted, and the other 7 are asserted to
keep delivering -- full occupancy is where a per-channel error would do the most
damage and where the SDD's CCN contract is actually interesting.

ORPHAN-STREAM TERMINATION (Epic 5 / t1.tj2.hk; corrected by issue b.9gj)
-----------------------------------------------------------------------
This file also discharges the Epic's re-breakdown contract item 5. The code
reading recorded during breakdown established that a non-durable stream whose
queue disappeared server-side had NO SELF-termination path:
``RabbitMQBase._consume_with_client_acks`` / ``_consume_with_delayed_acks``
blocked at ``await messages_queue.get()``; that buffer is fed ONLY by the
``queue.consume`` callback installed in ``_start_consuming_from_queue``, and the
broker's ``basic.cancel`` was never turned into a buffer wake-up or an
exception. So the orphaned consume loop blocked forever, holding its pool
checkout, with no way to reclaim itself.

**That premise is now false, and this file records the correction.** Issue
b.9gj replaced the bare ``messages_queue.get()`` with ``_next_message``, which
polls a consumer-liveness predicate off the idle path every
``consumer_liveness_poll_seconds``; an orphan whose queue is gone now ends its
own generator with ``QueueNotFoundError``. Per the standing instruction that
lived in this docstring -- "if CI ever shows the orphan terminating WITHOUT the
drain, correct the assertion to that measured answer rather than weakening it"
-- ``TestOrphanedStreamTermination`` below asserts SELF-termination as the
primary outcome, measured by running the orphan through the REAL consume
generator rather than a raw message buffer.

Epic 5 Task 1's (t2.tj2.hk.n7) SIGTERM drain is retained as the belt-and-braces
path, because self-termination and drain-driven reclaim answer different
questions: the stream ending is not the same as its POOL CHECKOUT coming back
(the checkout is held by the caller, not the generator). On SIGTERM
``start_servers._graceful_shutdown`` runs, in order,
``connector.begin_draining()`` (flips the synchronously-readable ``draining``
signal) -> ``await connector.drain_non_durable(window, poll)`` (waits up to the
bounded window for the non-durable pool's ``total_client_count`` to reach 0) ->
``prepare_to_shutdown()`` -> ``shutdown_event.set()`` -> (finally)
``connector.close()`` / ``close_all()``. A checkout still held at window expiry
is force-reclaimed by that subsequent ``close_all()``: it force-CLOSES channels
rather than waiting on checkouts to return.
"""

import asyncio
import uuid
from dataclasses import dataclass

import aio_pika
import pytest
import pytest_asyncio

from notification_common.errors.errors import QueueNotFoundError
from notification_common.rabbitmq.channel_pool import ChannelPool, ChannelPoolConfig
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings
from app.lib.rabbitmq.ack_tracker import AckTracker
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase

# Captured at import time, before the autouse ``mock_queue_iterator`` fixture in
# ``tests/integration_tests/conftest.py`` replaces the attribute with a canned
# generator. The orphan runs through this REAL loop, which is what makes its
# self-termination a measurement rather than a restatement of a fake.
_REAL_CONSUME_WITH_CLIENT_ACKS = RabbitMQBase._consume_with_client_acks

# The shared channel is packed to the shipped client_limit (8 streams). The
# density is the point (PRD AC-6 "at the default packing density"), so the pool
# is built with pool_size=1 / min_size=1 and client_limit is INHERITED from the
# shipped default rather than overridden.
SIBLING_COUNT = 8

# The stream whose queue is deleted server-side; the other 7 are the survivors.
ORPHAN_INDEX = 0

# Bounded waits so a hung acceptance test in CI fails as a timeout naming what it
# awaited, rather than being killed.
BASELINE_TIMEOUT_S = 15
SURVIVOR_TIMEOUT_S = 15

# The orphan's consumer-liveness poll, held AT the settings floor
# (``consumer_liveness_poll_seconds`` is ``ge=1.0``) so the latency measured here
# is one a real deployment could configure.
ORPHAN_POLL_SECONDS = 1.0

# The ceiling on the orphan ending its own generator. Generous next to the poll
# so broker or CI latency cannot flake it, but bounded so the PRE-b.9gj
# behaviour -- a stream that stays open forever -- fails as a named timeout
# instead of hanging CI.
ORPHAN_SELF_TERMINATION_TIMEOUT_S = 20

# The bounded drain window Task 1's ``drain_non_durable()`` waits for the
# non-durable pool's ``total_client_count`` to reach 0. Every checkout is held
# by the FIXTURE, not by the consume loops, so self-termination does not return
# one: the count never reaches 0, the drain returns AT the window, and the
# subsequent ``close()`` force-reclaims. Kept short so CI is fast; the window
# bounds the wait, so a hang is impossible.
ORPHAN_DRAIN_WINDOW_S = 2
ORPHAN_DRAIN_POLL_S = 0.1

# A hard ceiling on the force-close reclaim so a stuck broker close fails as a
# named timeout rather than being killed.
RECLAIM_TIMEOUT_S = 15


def _routing_key(index: int) -> str:
    """A distinct routing key per sibling stream."""
    return f"ac6.rk.{index}"


def _message_body(routing_key: str) -> bytes:
    """A body that names its own routing key, so a buffer owning its own
    message is checkable rather than assumed."""
    return f'{{"rk": "{routing_key}"}}'.encode("utf-8")


@dataclass
class BlastRadiusScenario:
    """
    The state after one sibling's queue has been deleted from a shared channel.

    :param shared_channel: The single channel all ``SIBLING_COUNT`` streams were
        packed onto.
    :param checked_out_channels: The channel object returned by each of the
        ``SIBLING_COUNT`` pool checkouts, in order -- all expected identical.
    :param exchange: The topic exchange the siblings are bound to (used to
        publish further messages).
    :param routing_keys: One distinct routing key per sibling, by index.
    :param buffers: The per-sibling ``asyncio.Queue`` message buffers returned by
        ``_start_consuming_from_queue``, by index. ``None`` at ``ORPHAN_INDEX``,
        whose stream is driven by ``orphan_generator`` instead.
    :param orphan_generator: The REAL ``_consume_with_client_acks`` generator for
        the stream whose queue is deleted. The orphan runs through the production
        consume loop (not a raw buffer) so its SELF-termination is a measurement
        of shipped behaviour rather than of the test's own plumbing.
    :param deleted_routing_key: The routing key of the orphaned (deleted) queue.
    :param pool: The live non-durable ``ChannelPool`` the streams are checked out
        of, so the orphan-termination test can drive Task 1's drain sequence
        over real pool state (``total_client_count``) and the force-close reclaim.
    """

    shared_channel: object
    checked_out_channels: list
    exchange: object
    routing_keys: list
    buffers: list
    orphan_generator: object
    deleted_routing_key: str
    pool: object


async def _receive(buffer, timeout, what):
    """Await one message from ``buffer``, failing as a named timeout."""
    try:
        return await asyncio.wait_for(buffer.get(), timeout=timeout)
    except asyncio.TimeoutError:
        pytest.fail(f"timed out waiting for {what} within {timeout}s")


@pytest_asyncio.fixture
async def blast_radius_scenario(live_broker_connection, mocker):
    """
    Pack 8 streams onto one channel, prove baseline delivery, delete one queue.

    Steps, all against the live broker:

    1. Build a real ``ChannelPool`` with ``pool_size=1`` / ``min_size=1`` so
       every checkout is forced onto the SAME channel; ``client_limit`` is left
       to the shipped default (8). Take ``SIBLING_COUNT`` checkouts.
    2. Declare a uuid-named topic exchange and, on the shared channel, one
       uuid-named ``auto_delete`` queue per sibling bound to a DISTINCT routing
       key. The seven SURVIVORS are consumed through the PRODUCTION helper
       ``RabbitMQBase._start_consuming_from_queue`` (which is not patched by the
       autouse ``mock_queue_iterator``), so the real consumer setup is
       exercised; the ORPHAN is consumed through the REAL
       ``_consume_with_client_acks`` generator, because its self-termination is
       the thing ``TestOrphanedStreamTermination`` measures.
    3. Baseline delivery: publish one message per routing key and assert each
       stream receives its OWN message. This is the non-vacuity control --
       "siblings still deliver" is meaningless on a channel that never
       delivered anything, and neither is "the orphan terminated" on a stream
       that never started.
    4. Delete the orphan's queue from a SEPARATE channel, modelling the
       server-side disappearance the AC is about without the delete itself
       perturbing the shared channel.

    The consumer-liveness poll is shortened to ``ORPHAN_POLL_SECONDS`` and
    ``RabbitMQBase.connector`` is pointed at the live connection, because the
    liveness check's classifying step (``_assert_queue_exists``) opens a
    temporary channel off the connector.

    Yields a :class:`BlastRadiusScenario`. Tears down the pool (cancelling the
    consumers; the ``auto_delete`` queues go with them), deletes the exchange,
    and closes the helper channels.
    """
    connection = live_broker_connection

    mocker.patch.object(
        settings.settings, "consumer_liveness_poll_seconds", ORPHAN_POLL_SECONDS
    )
    mocker.patch.object(RabbitMQBase.connector, "_connection", connection)
    mocker.patch.object(RabbitMQBase.connector, "closing", False)

    pool = ChannelPool(
        connection,
        ChannelPoolConfig(pool_size=1, min_size=1),
        name=RabbitMQConnector.NON_DURABLE_POOL_NAME,
    )
    await pool.initialize()

    exchange_name = f"ac6-blast-{uuid.uuid4()}"
    setup_channel = None
    delete_channel = None
    orphan_generator = None
    try:
        setup_channel = await connection.channel()
        exchange = await setup_channel.declare_exchange(
            exchange_name, type="topic", durable=False, auto_delete=True
        )

        async def _authz(_message):
            return True

        checked_out_channels = []
        routing_keys = []
        queue_names = []
        buffers = []
        for index in range(SIBLING_COUNT):
            channel = await pool.get_channel()
            checked_out_channels.append(channel)

            routing_key = _routing_key(index)
            queue_name = f"ac6-q-{index}-{uuid.uuid4()}"
            queue = await channel.declare_queue(
                queue_name, durable=False, auto_delete=True
            )
            await queue.bind(exchange_name, routing_key)

            if index == ORPHAN_INDEX:
                # The orphan runs the REAL consume loop, so its termination is
                # shipped behaviour rather than the test's own plumbing.
                orphan_generator = _REAL_CONSUME_WITH_CLIENT_ACKS(
                    queue, _authz, AckTracker()
                )
                buffers.append(None)
            else:
                # Real production consumer setup.
                buffer, _consumer_tag = await RabbitMQBase._start_consuming_from_queue(
                    queue
                )
                buffers.append(buffer)

            routing_keys.append(routing_key)
            queue_names.append(queue_name)

        shared_channel = checked_out_channels[0]

        # Baseline delivery (non-vacuity control): every stream receives its own
        # message before any deletion.
        for index in range(SIBLING_COUNT):
            await exchange.publish(
                aio_pika.Message(body=_message_body(routing_keys[index])),
                routing_key=routing_keys[index],
            )
        for index in _survivor_indices():
            message = await _receive(
                buffers[index],
                BASELINE_TIMEOUT_S,
                f"baseline delivery to sibling {index}",
            )
            assert message.body == _message_body(routing_keys[index])

        try:
            orphan_record = await asyncio.wait_for(
                anext(orphan_generator), timeout=BASELINE_TIMEOUT_S
            )
        except asyncio.TimeoutError:
            pytest.fail(
                "timed out waiting for baseline delivery through the orphan's "
                f"real consume generator within {BASELINE_TIMEOUT_S}s"
            )
        assert orphan_record.message == {"rk": routing_keys[ORPHAN_INDEX]}

        # Delete the orphan's queue from a SEPARATE channel.
        delete_channel = await connection.channel()
        await delete_channel.queue_delete(queue_names[ORPHAN_INDEX])

        yield BlastRadiusScenario(
            shared_channel=shared_channel,
            checked_out_channels=checked_out_channels,
            exchange=exchange,
            routing_keys=routing_keys,
            buffers=buffers,
            orphan_generator=orphan_generator,
            deleted_routing_key=routing_keys[ORPHAN_INDEX],
            pool=pool,
        )
    finally:
        if orphan_generator is not None:
            await orphan_generator.aclose()
        await pool.close_all()
        if delete_channel is not None:
            await delete_channel.close()
        if setup_channel is not None:
            try:
                await setup_channel.exchange_delete(exchange_name)
            finally:
                await setup_channel.close()


def _survivor_indices():
    """The indices of the streams that survive the delete (all but the orphan)."""
    return [index for index in range(SIBLING_COUNT) if index != ORPHAN_INDEX]


@dataclass
class OrphanTermination:
    """
    How the orphaned stream's own generator ended.

    :param error: The exception the generator ended with, or None if it ended
        cleanly or not at all.
    :param timed_out: True if it was STILL OPEN at the ceiling -- the pre-b.9gj
        zombie, and the outcome the whole class exists to rule out.
    :param elapsed: Seconds from the first step to that end.
    """

    error: BaseException | None
    timed_out: bool
    elapsed: float


@pytest_asyncio.fixture
async def orphan_termination(blast_radius_scenario):
    """
    Step the orphan's REAL generator once and RECORD how it ended.

    Recording rather than asserting is what lets each test below read the one
    field it is about. Driven here once per rig instead of inside three test
    bodies, each of which otherwise re-awaits the same termination and pays
    its own liveness-poll wait for it.

    Nothing is published to the deleted queue's routing key, so the orphan's
    loop is purely idle -- and idle is precisely where it used to block
    forever.
    """
    loop = asyncio.get_running_loop()
    started = loop.time()
    try:
        await asyncio.wait_for(
            anext(blast_radius_scenario.orphan_generator),
            timeout=ORPHAN_SELF_TERMINATION_TIMEOUT_S,
        )
    except asyncio.TimeoutError:
        return OrphanTermination(
            error=None, timed_out=True, elapsed=loop.time() - started
        )
    # Any other exception IS the observation under test, so it is recorded
    # rather than allowed to error the fixture.
    except Exception as exc:  # noqa: BLE001
        return OrphanTermination(
            error=exc, timed_out=False, elapsed=loop.time() - started
        )
    return OrphanTermination(error=None, timed_out=False, elapsed=loop.time() - started)


class TestSharedChannelPremise:
    """The eight checkouts really do share one channel -- the file's premise."""

    @pytest.mark.asyncio
    async def test_all_eight_checkouts_land_on_the_same_channel(
        self, blast_radius_scenario
    ):
        # If a future selection change spread these across channels there would
        # be no shared channel to bound a blast radius on, so this fails loudly.
        channels = blast_radius_scenario.checked_out_channels

        assert len(channels) == SIBLING_COUNT
        assert all(channel is channels[0] for channel in channels)


class TestSiblingsSurviveTheDelete:
    """AC-6: deleting one queue leaves the other seven delivering."""

    @pytest.mark.asyncio
    async def test_the_seven_survivors_keep_delivering_after_the_delete(
        self, blast_radius_scenario
    ):
        # Publish AFTER the deletion, to the surviving routing keys only, and
        # require every survivor to receive the new message. A channel-level
        # error from the delete would have taken all seven down together -- that
        # is precisely the blast radius being bounded.
        scenario = blast_radius_scenario

        for index in _survivor_indices():
            await scenario.exchange.publish(
                aio_pika.Message(body=_message_body(scenario.routing_keys[index])),
                routing_key=scenario.routing_keys[index],
            )

        for index in _survivor_indices():
            message = await _receive(
                scenario.buffers[index],
                SURVIVOR_TIMEOUT_S,
                f"post-delete delivery to surviving sibling {index}",
            )
            assert message.body == _message_body(scenario.routing_keys[index])


class TestSharedChannelSurvivesTheDelete:
    """The shared channel stays open and usable after a sibling's queue is
    deleted."""

    @pytest.mark.asyncio
    async def test_the_shared_channel_is_still_open(self, blast_radius_scenario):
        # A channel-level error would have closed the channel and taken every
        # sibling on it down; it must still be open.
        assert blast_radius_scenario.shared_channel.is_closed is False

    @pytest.mark.asyncio
    async def test_the_shared_channel_is_still_usable(self, blast_radius_scenario):
        # Beyond "still open": the channel can still carry a further
        # declare/bind/consume and deliver on it, proving it is genuinely
        # healthy rather than merely not-yet-closed.
        scenario = blast_radius_scenario
        probe_routing_key = f"ac6.probe.{uuid.uuid4()}"

        probe_queue = await scenario.shared_channel.declare_queue(
            f"ac6-probe-{uuid.uuid4()}", durable=False, auto_delete=True
        )
        await probe_queue.bind(scenario.exchange, probe_routing_key)
        probe_buffer, _tag = await RabbitMQBase._start_consuming_from_queue(probe_queue)

        await scenario.exchange.publish(
            aio_pika.Message(body=_message_body(probe_routing_key)),
            routing_key=probe_routing_key,
        )

        message = await _receive(
            probe_buffer, SURVIVOR_TIMEOUT_S, "delivery on the reused shared channel"
        )
        assert message.body == _message_body(probe_routing_key)


class TestOrphanedStreamTermination:
    """
    The orphaned stream SELF-terminates; the drain remains the backstop.

    The premise this class shipped with -- "the orphaned stream has NO
    SELF-termination path" -- was true when it was written and is false now.
    Issue b.9gj replaced the bare ``messages_queue.get()`` with
    ``_next_message``, which polls a consumer-liveness predicate off the idle
    path, so an orphan whose queue was deleted ends its own generator with
    ``QueueNotFoundError``. Following this file's own standing instruction --
    correct the assertion to the MEASURED answer rather than weakening it --
    the primary assertion is now self-termination, driven through the REAL
    ``_consume_with_client_acks`` generator against the live broker.

    Epic 5 Task 1's (t2.tj2.hk.n7) drain is kept as the belt-and-braces path,
    because the two answer different questions. The stream ending does NOT
    return the pool checkout: the checkout is held by the CALLER (here, the
    fixture) for the whole stream lifecycle, so a channel still held at drain
    expiry is reclaimed by ``close_all()`` force-closing it, exactly as before.

    The termination itself is driven ONCE, by the ``orphan_termination``
    fixture, and every test here reads a field off that record -- rather than
    each body re-awaiting the same ending and paying its own poll wait for it.
    """

    @pytest.mark.asyncio
    async def test_the_orphan_does_not_stay_open(self, orphan_termination):
        # THE corrected assertion. Idle is exactly where the orphan used to
        # block forever, holding its pool checkout with no way to reclaim
        # itself.
        assert orphan_termination.timed_out is False, (
            "the orphaned stream was still open "
            f"{ORPHAN_SELF_TERMINATION_TIMEOUT_S}s after its queue was "
            "deleted; this is the zombie stream issue b.9gj reports"
        )

    @pytest.mark.asyncio
    async def test_the_orphan_ends_with_queue_not_found(self, orphan_termination):
        # NOT_FOUND is what tells the client to REPROVISION rather than keep
        # waiting on a queue that is gone. A generator that merely stopped
        # would leave it reconnecting forever.
        assert isinstance(orphan_termination.error, QueueNotFoundError), (
            "expected QueueNotFoundError after the orphan's queue was deleted, "
            f"got {orphan_termination.error!r}"
        )

    @pytest.mark.asyncio
    async def test_the_orphans_termination_leaves_the_shared_channel_open(
        self, blast_radius_scenario, orphan_termination
    ):
        # Self-termination must stay a per-stream event. If ending the orphan
        # closed or errored the shared channel it would take the other seven
        # streams with it -- reintroducing the blast radius AC-6 bounds.
        assert blast_radius_scenario.shared_channel.is_closed is False

    @pytest.mark.asyncio
    async def test_the_orphans_termination_leaves_siblings_delivering(
        self, blast_radius_scenario, orphan_termination
    ):
        scenario = blast_radius_scenario

        survivor_index = _survivor_indices()[0]
        await scenario.exchange.publish(
            aio_pika.Message(body=_message_body(scenario.routing_keys[survivor_index])),
            routing_key=scenario.routing_keys[survivor_index],
        )
        message = await _receive(
            scenario.buffers[survivor_index],
            SURVIVOR_TIMEOUT_S,
            f"delivery to surviving sibling {survivor_index} after the orphan ended",
        )
        assert message.body == _message_body(scenario.routing_keys[survivor_index])

    @pytest.mark.asyncio
    async def test_the_drain_still_reclaims_the_orphans_checkout(
        self, blast_radius_scenario, orphan_termination
    ):
        # Belt and braces. Self-termination ends the STREAM; it does not return
        # the pool CHECKOUT, which the caller holds. Task 1's drain remains the
        # thing that reclaims a channel still held at shutdown, so it is still
        # asserted here -- now against a stream that has already ended rather
        # than one hung forever.
        scenario = blast_radius_scenario
        pool = scenario.pool

        # The ended stream did NOT hand its checkout back: all SIBLING_COUNT
        # are still out. This is what keeps the drain load-bearing.
        assert pool.pool_stats()["total_client_count"] == SIBLING_COUNT

        # Drive Task 1's drain sequence over the LIVE non-durable pool via the
        # real connector surface (the same begin_draining / drain_non_durable /
        # close path start_servers._graceful_shutdown runs on SIGTERM). A real
        # RabbitMQConnector is used with the fixture's live pool injected so the
        # genuine drain logic runs against real pool state -- pool_stats_by_pool()
        # reads this pool, and NON_DURABLE_POOL_NAME is the real key.
        connector = RabbitMQConnector()
        connector._channel_pool = pool

        # (1) Engage draining: the synchronously-readable signal Task 2's
        # readiness/admission surfaces consume. It must not close anything.
        connector.begin_draining()
        assert connector.draining is True
        assert connector.closing is False
        assert scenario.shared_channel.is_closed is False

        # A surviving sibling is unaffected UP TO the close: draining and the
        # drain window do not perturb live streams. Publish after begin_draining
        # and require a survivor to still receive it.
        survivor_index = _survivor_indices()[0]
        await scenario.exchange.publish(
            aio_pika.Message(body=_message_body(scenario.routing_keys[survivor_index])),
            routing_key=scenario.routing_keys[survivor_index],
        )
        message = await _receive(
            scenario.buffers[survivor_index],
            SURVIVOR_TIMEOUT_S,
            f"delivery to surviving sibling {survivor_index} during draining",
        )
        assert message.body == _message_body(scenario.routing_keys[survivor_index])

        # (2) Drain: wait up to the bounded window for total_client_count to
        # reach 0. The fixture holds every checkout for the whole test, so the
        # count never reaches 0 and the drain returns AT the window -- bounded,
        # not hung.
        await connector.drain_non_durable(
            window_seconds=ORPHAN_DRAIN_WINDOW_S,
            poll_interval_seconds=ORPHAN_DRAIN_POLL_S,
        )
        assert pool.pool_stats()["total_client_count"] == SIBLING_COUNT

        # (3) Reclaim: close() -> close_all() force-closes the shared channel at
        # window expiry, NOT gated on any checkout being returned. Bounded so a
        # stuck broker close fails as a named timeout.
        try:
            await asyncio.wait_for(connector.close(), timeout=RECLAIM_TIMEOUT_S)
        except asyncio.TimeoutError:
            pytest.fail(
                "timed out waiting for the drain force-close to reclaim the "
                f"orphaned checkout within {RECLAIM_TIMEOUT_S}s"
            )

        assert pool.pool_stats()["total_client_count"] == 0
        assert scenario.shared_channel.is_closed is True
