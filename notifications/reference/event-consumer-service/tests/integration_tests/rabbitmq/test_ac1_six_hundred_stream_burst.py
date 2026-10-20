# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Named acceptance test for PRD AC-1: 600 streams pack onto one pod.

AC-1 verbatim: "With default settings, a test client opening 600 concurrent
non-durable streams against a single pod succeeds, and the pod continues
serving other clients (no ``ChannelNotAvailableError``)."

This is the Bee's reason to exist. At the shipped packed defaults 600 sits
BELOW the 800 soft threshold, so this is plain packing -- not the SR-1.5
overflow band and not the memory-safe cap. The 600 checkouts land
deterministically as 75 channels x 8 clients: ``ChannelPool`` drains the 75
zero-client channels first, then packs onto the least-loaded channel under
``client_limit``, and grows the pool only when no channel is under the limit
(``notification_common/rabbitmq/channel_pool.py`` ``_get_best_available_channel``).
So ``total_channel_count == 75`` is a precise non-vacuity assertion: 600
checkouts on 600 separate channels would be the UNPACKED configuration and must
fail this test.

Three arms, one claim
---------------------
- ``TestSixHundredStreamsPackOntoOnePodOverAMockedConnection`` (always runs)
  proves the packing itself over a real ``ChannelPool`` on a mock connection:
  75 channels absorb 600 streams, and the 601st grows the pool (the pod keeps
  serving).
- ``TestSixHundredStreamsKeepThePodServing`` (always runs) proves the admission
  half over the SAME real pool objects: the guard admits at 600 held streams,
  and a durable checkout still succeeds -- the counted==actual proof taken from
  real pool state, which is the counting contract Task 1 hands this Task.
- ``TestSixHundredStreamsOnALiveBroker`` (SKIPS without a broker) proves a real
  broker accepts 75 real channels on one connection at the shipped packing
  density and fans one publish out to all 600 consumers. It runs in CI's
  ``integration:`` job (``.gitlab-ci.yml``), which starts a ``rabbitmq:4.1.3``
  service and sets ``OMNI_EVENTS_REQUIRE_LIVE_BROKER=1`` so a missing broker
  HARD-fails there; no broker is reachable in the dev sandbox, so it skips
  locally with the ``live_broker_connection`` fixture's message.

Latency NFR -- cited, not benchmarked
-------------------------------------
The PRD's consumer-path NFR ("p99 at the default packing density within 2x the
unpacked configuration") is already empirically established in the source
repository's internal channel-capacity design record (not published) and is NOT
re-measured here (the
project rule is not to redo documented empirical verifications). The measured
basis is that record's ``hol`` table -- the 10%-straggler head-of-line proof:
p99 **3.33 ms at 8 consumers/channel** against **3.11 ms at 1 consumer/channel**
(prefetch 15, RabbitMQ 4.1.3), i.e. the packed density is within ~1.07x of the
unpacked one, comfortably inside the 2x NFR. The density that measurement covers
is exactly this file's: 8 streams/channel at prefetch 15.

No test in this file makes a wall-clock latency assertion. A timing assertion
here would be flaky under CI load and, more importantly, would not measure the
in-cluster consumer path the NFR is actually about -- so the NFR is cited, not
re-asserted. The shipped defaults the citation depends on (client_limit 8,
prefetch 15, pool_size 100) are pinned by ``tests/app/test_packing_defaults_contract.py``;
a defaults drift that invalidated the citation would fail THERE.

Soft threshold -- pinned elsewhere
----------------------------------
SR-1.4's ">= 800 streams/pod at defaults" is owned by
``tests/app/test_packing_defaults_contract.py`` (subtask t3.tj2.xn.n9.5p), which
multiplies ``pool_size x client_limit`` to 800 and asserts the >= 800 floor.
This file references that pin rather than restating the arithmetic; it asserts
only that 600 fits under the 800 threshold it reports.

Capacity numbers are installed, never inherited
-----------------------------------------------
There is no autouse memory-cap neutralizer under ``tests/integration_tests/``
(that fixture lives in ``tests/app/conftest.py``), so the memory-safe cap is
LIVE here. Every capacity input is pinned explicitly against a stray developer
``.env`` (``pinned_shipped_config``): budget 1024 MiB, 10 replicas, prefetch 15,
which computes to a cap of 3342 -- far above 600.
"""

import asyncio
import uuid

import aio_pika
import pytest
import pytest_asyncio
from unittest.mock import AsyncMock, MagicMock

from notification_common.config.common_settings import CommonSettings, common_settings
from notification_common.rabbitmq.channel_pool import (
    ChannelPool,
    ChannelPoolConfig,
    DurableChannelPoolConfig,
)
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings
from app.lib.rabbitmq.capacity_guard import CapacityGuard

# --- Module constants: each a literal with its derivation in a comment ----

# The PRD's measured worst-case desktop client (AC-1 / G-1): one scene load
# opens ~600 streams on ONE connection.
WORST_CASE_CLIENT_BURST = 600

# The shipped non-durable pool geometry (common_settings defaults).
POOL_SIZE = 100  # rabbitmq_channel_pool_size
POOL_MIN_SIZE = 75  # rabbitmq_channel_pool_min_size
CLIENT_LIMIT = 8  # rabbitmq_channel_client_limit
PREFETCH = 15  # rabbitmq_channel_prefetch_count

# The soft packing threshold: pool_size 100 x client_limit 8 = 800 streams/pod.
# 600 sits below it, so this file is plain packing, not the overflow band.
SOFT_THRESHOLD = 800

# 600 streams / 8 per channel = 75 channels, with no pool growth. This is the
# packing non-vacuity number: 600 separate channels would be the unpacked shape.
EXPECTED_CHANNELS = 75

# The memory-safe cap inputs and the cap they compute to. Derivation, checked by
# hand and NOT recomputed from production code:
#   budget 1024 MiB / (15 x 1800 + 5120) B = 33,429 fleet / 10 replicas = 3,342
BUDGET_MIB = 1024
MAX_REPLICAS = 10
CAP = 3_342

# How long the live-broker fan-out arm waits for each of the 600 consumers to
# receive the single published message. A hung fan-out in CI must fail as a
# timeout, not be killed, so every wait is bounded and names what it awaited.
FANOUT_TIMEOUT_S = 30

# The single message the live-broker arm fans out to all 600 queues.
_FANOUT_BODY = b'{"event_type": "ac1-fanout"}'


def _fresh_mock_channel():
    """A fresh mock channel, as a real connection hands out per ``channel()``."""
    channel = AsyncMock()
    channel.is_closed = False
    channel.set_qos = AsyncMock()
    channel.close = AsyncMock()
    return channel


def _mock_connection():
    """
    A mock connection that hands out a FRESH channel per ``channel()`` call.

    The pool grows by calling ``connection.channel()``; a connection that
    returned one shared channel would collapse the 75-channel packing this file
    asserts into a single object. Mirrors the ``mock_channel_factory`` idiom in
    ``tests/app/lib/rabbitmq/test_durable_pool_isolation.py``.
    """
    connection = MagicMock()
    connection.is_closed = False
    connection.channel = AsyncMock(side_effect=lambda: _fresh_mock_channel())
    return connection


@pytest.fixture
def pinned_shipped_config(mocker):
    """
    Pin the shipped non-durable geometry and the memory-cap inputs.

    The always-run arms build an ``unmodified`` ``ChannelPoolConfig()`` (no
    per-pool overrides), which late-binds to ``common_settings`` on every read.
    A developer's untracked ``.env`` re-seeds those globals, so the four packing
    numbers and the three cap inputs are pinned here explicitly -- the same
    "installed, never inherited" posture the AC-11/AC-12 acceptance files take.

    The cap is driven through the COMPUTED path (override ``None``), so this file
    really exercises the prefetch-15 cap of 3342 rather than a hand-set number.
    """
    mocker.patch.object(common_settings, "rabbitmq_channel_pool_size", POOL_SIZE)
    mocker.patch.object(
        common_settings, "rabbitmq_channel_pool_min_size", POOL_MIN_SIZE
    )
    mocker.patch.object(common_settings, "rabbitmq_channel_client_limit", CLIENT_LIMIT)
    mocker.patch.object(common_settings, "rabbitmq_channel_prefetch_count", PREFETCH)

    # Computed-cap path: override None, and pin the three inputs that compute
    # the 3342 cap so it does not ride on the ambient environment.
    mocker.patch.object(settings.settings, "max_non_durable_streams_per_pod", None)
    mocker.patch.object(
        settings.settings, "broker_stream_memory_budget_mib", BUDGET_MIB
    )
    mocker.patch.object(settings.settings, "scaler_max_replicas", MAX_REPLICAS)


class TestShippedDefaultsGuard:
    """
    Pin this module's four packing constants to the DECLARED settings defaults.

    Read from ``model_fields[...].default`` rather than a resolved ``settings``
    instance: a developer's untracked ``.env`` re-seeds these vars on reload, so
    a resolved-value assertion would be green in CI and red on that machine (the
    hazard recorded in ``tests/app/config/test_settings.py``). Without this the
    file could silently stop testing "default settings".
    """

    def test_pool_size_matches_the_declared_default(self):
        assert (
            POOL_SIZE
            == CommonSettings.model_fields["rabbitmq_channel_pool_size"].default
        )

    def test_pool_min_size_matches_the_declared_default(self):
        assert (
            POOL_MIN_SIZE
            == CommonSettings.model_fields["rabbitmq_channel_pool_min_size"].default
        )

    def test_client_limit_matches_the_declared_default(self):
        assert (
            CLIENT_LIMIT
            == CommonSettings.model_fields["rabbitmq_channel_client_limit"].default
        )

    def test_prefetch_matches_the_declared_default(self):
        assert (
            PREFETCH
            == CommonSettings.model_fields["rabbitmq_channel_prefetch_count"].default
        )

    def test_the_burst_fits_under_the_soft_threshold(self):
        # The premise of the whole file: 600 is plain packing, below the 800
        # soft threshold, so no overflow band and no cap are in play.
        assert WORST_CASE_CLIENT_BURST < SOFT_THRESHOLD


class TestSixHundredStreamsPackOntoOnePodOverAMockedConnection:
    """
    600 non-durable checkouts pack onto 75 channels over a mock connection.

    A real ``ChannelPool`` at the shipped config, over a connection that hands
    out a fresh channel per call, absorbs the whole 600-stream burst by packing
    -- not by opening 600 channels. The 601st checkout is the first that must
    grow the pool, which is the "pod keeps serving other clients" half.
    """

    @pytest_asyncio.fixture
    async def packed_pool(self, pinned_shipped_config):
        """
        A real non-durable pool holding 600 concurrent checkouts.

        Reaching the ``yield`` without ``ChannelNotAvailableError`` IS the "600
        succeeds" proof; the checkouts are held (never returned) so the 600
        streams are concurrent, not sequential-then-freed.
        """
        pool = ChannelPool(
            _mock_connection(),
            ChannelPoolConfig(),
            name=RabbitMQConnector.NON_DURABLE_POOL_NAME,
        )
        await pool.initialize()

        for _unused_stream in range(WORST_CASE_CLIENT_BURST):
            await pool.get_channel()

        try:
            yield pool
        finally:
            await pool.close_all()

    @pytest.mark.asyncio
    async def test_six_hundred_checkouts_all_succeed(self, packed_pool):
        # The fixture already took all 600 without raising; a client count of
        # exactly 600 confirms every one was admitted rather than dropped.
        assert packed_pool.pool_stats()["total_client_count"] == WORST_CASE_CLIENT_BURST

    @pytest.mark.asyncio
    async def test_the_burst_packs_onto_seventy_five_channels(self, packed_pool):
        # The packing non-vacuity assertion: 600 on 75 channels is packing; 600
        # on 600 channels would be the unpacked configuration this test rejects.
        assert packed_pool.pool_stats()["total_channel_count"] == EXPECTED_CHANNELS

    @pytest.mark.asyncio
    async def test_the_pool_reports_the_eight_hundred_soft_threshold(self, packed_pool):
        assert packed_pool.pool_stats()["max_client_capacity"] == SOFT_THRESHOLD

    @pytest.mark.asyncio
    async def test_every_open_channel_carries_exactly_eight_clients(self, packed_pool):
        # Ownership/client counts are only visible through the pool's own channel
        # map; per-channel evenness is what makes "packed to the limit" true
        # rather than "some channel absorbed a lopsided share".
        client_counts = [
            info.client_count for info in packed_pool._all_channels.values()
        ]

        assert client_counts == [CLIENT_LIMIT] * EXPECTED_CHANNELS

    @pytest.mark.asyncio
    async def test_the_601st_checkout_grows_the_pool_and_succeeds(self, packed_pool):
        # PM review addition: "would admit" -> "did serve". With all 75
        # min_size channels at exactly 8, the 601st is the first checkout that
        # must GROW the pool past min_size. Growth-past-min_size under packing is
        # precisely the path a packing regression would break.
        channel = await packed_pool.get_channel()

        assert channel is not None
        assert packed_pool.pool_stats()["total_channel_count"] == EXPECTED_CHANNELS + 1


class TestSixHundredStreamsKeepThePodServing:
    """
    The admission half of AC-1, decided from REAL pool state.

    The guard reads ``pool_stats_by_pool()`` off two real ``ChannelPool``
    instances -- not a synthetic dict -- so the verdict is counted==actual: 600
    real checkouts, and the guard still admits. The "keeps serving other
    clients" clause is proven by a durable checkout succeeding while all 600
    non-durable streams are held.
    """

    @pytest_asyncio.fixture
    async def real_pools_and_guard(self, pinned_shipped_config, mocker):
        """
        Two real pools -- non-durable packed to 600 -- and a guard over them.

        The connector is a ``spec``'d ``MagicMock`` whose ``pool_stats_by_pool``
        rebuilds each call from the two live pools, so the guard's verdict
        tracks real pool state. ``spec=RabbitMQConnector`` makes a rename of the
        method fail loudly here rather than silently hand the guard a stats-less
        mock.
        """
        non_durable = ChannelPool(
            _mock_connection(),
            ChannelPoolConfig(),
            name=RabbitMQConnector.NON_DURABLE_POOL_NAME,
        )
        durable = ChannelPool(
            _mock_connection(),
            DurableChannelPoolConfig(),
            name=RabbitMQConnector.DURABLE_POOL_NAME,
        )
        await non_durable.initialize()
        await durable.initialize()

        for _unused_stream in range(WORST_CASE_CLIENT_BURST):
            await non_durable.get_channel()

        connector = mocker.MagicMock(spec=RabbitMQConnector)
        # Not draining: the guard reads ``connector.draining`` on the
        # non-durable path (Epic 5, Task 2 / SR-6.1). It is a plain instance
        # attribute absent from the class spec, so it must be set explicitly or
        # the guard's read raises ``AttributeError``. This scenario proves the
        # pod keeps serving at 600 held streams, not the draining short-circuit.
        connector.draining = False
        connector.pool_stats_by_pool.side_effect = lambda: {
            RabbitMQConnector.NON_DURABLE_POOL_NAME: non_durable.pool_stats(),
            RabbitMQConnector.DURABLE_POOL_NAME: durable.pool_stats(),
        }
        guard = CapacityGuard(connector=connector)

        try:
            yield non_durable, durable, guard
        finally:
            await non_durable.close_all()
            await durable.close_all()

    @pytest.mark.asyncio
    async def test_the_guard_admits_at_six_hundred_held_streams(
        self, real_pools_and_guard
    ):
        non_durable, _durable, guard = real_pools_and_guard

        # Non-vacuity: the pool really is holding the full burst, below the 3342
        # cap and below the 800 soft threshold, so admission here is the packing
        # verdict AC-1 is about.
        assert non_durable.pool_stats()["total_client_count"] == WORST_CASE_CLIENT_BURST

        # Admission is the absence of ChannelNotAvailableError; a raise fails the
        # test, which is the AC-1 regression this file exists to prevent.
        guard.raise_if_at_capacity()

    @pytest.mark.asyncio
    async def test_a_durable_checkout_succeeds_while_the_burst_is_held(
        self, real_pools_and_guard
    ):
        # "The pod continues serving other clients": a durable stream is a
        # different pool and must be unaffected by the non-durable burst.
        _non_durable, durable, guard = real_pools_and_guard

        channel = await durable.get_channel()

        assert channel is not None
        # And the guard in front of durable entry points agrees, reading the
        # durable pool's own budget rather than the exhausted non-durable view.
        guard.raise_if_at_capacity(durable_pool=True)

    @pytest.mark.asyncio
    async def test_a_601st_non_durable_stream_is_still_served(
        self, real_pools_and_guard
    ):
        # The non-durable "keeps serving" half at the pool layer: one more
        # checkout past the 600 grows the pool and succeeds.
        non_durable, _durable, _guard = real_pools_and_guard

        channel = await non_durable.get_channel()

        assert channel is not None
        assert non_durable.pool_stats()["total_channel_count"] == EXPECTED_CHANNELS + 1


class TestSixHundredStreamsOnALiveBroker:
    """
    A real broker accepts 75 channels on one connection and fans out to 600.

    Skips cleanly when no broker is reachable (the ``live_broker_connection``
    fixture's documented skip); CI's ``integration:`` job runs it against a
    ``rabbitmq:4.1.3`` service (hard-failing on a missing broker). One publish
    fanning out to 600 auto-delete queues is deliberately the cheap shape, and
    it mirrors the production fan-out that caused the 2026-08-11 incident. Every
    resource is uuid-named so ``pytest-xdist`` workers do not collide, and every
    wait is bounded.
    """

    @pytest.mark.asyncio
    async def test_six_hundred_streams_pack_and_receive_one_fanned_out_publish(
        self, live_broker_connection, pinned_shipped_config
    ):
        connection = live_broker_connection
        pool = ChannelPool(
            connection,
            ChannelPoolConfig(),
            name=RabbitMQConnector.NON_DURABLE_POOL_NAME,
        )
        await pool.initialize()

        exchange_name = f"ac1-fanout-{uuid.uuid4()}"
        routing_key = "ac1.fanout"
        received: asyncio.Queue = asyncio.Queue()

        async def _on_message(message: aio_pika.abc.AbstractIncomingMessage):
            received.put_nowait(message)

        setup_channel = None
        try:
            setup_channel = await connection.channel()
            exchange = await setup_channel.declare_exchange(
                exchange_name, type="topic", durable=False, auto_delete=True
            )

            # 600 streams: each takes its own pool checkout, declares an
            # auto_delete queue on that checked-out channel, binds it to the
            # SAME routing key, and starts a no-ack consumer reporting into
            # ``received``.
            for _unused_stream in range(WORST_CASE_CLIENT_BURST):
                channel = await pool.get_channel()
                queue = await channel.declare_queue(
                    f"ac1-q-{uuid.uuid4()}", durable=False, auto_delete=True
                )
                await queue.bind(exchange_name, routing_key)
                await queue.consume(_on_message, no_ack=True)

            # Non-vacuity: a real broker accepted 75 channels on ONE connection
            # carrying all 600 streams -- the packing density under test.
            assert pool.pool_stats()["total_channel_count"] == EXPECTED_CHANNELS
            assert pool.pool_stats()["total_client_count"] == WORST_CASE_CLIENT_BURST

            # One publish, fanned out to all 600 queues.
            await exchange.publish(
                aio_pika.Message(body=_FANOUT_BODY), routing_key=routing_key
            )

            for stream_index in range(WORST_CASE_CLIENT_BURST):
                try:
                    await asyncio.wait_for(received.get(), timeout=FANOUT_TIMEOUT_S)
                except asyncio.TimeoutError:  # pragma: no cover - CI-only failure
                    pytest.fail(
                        "timed out waiting for the fan-out publish to reach all "
                        f"{WORST_CASE_CLIENT_BURST} consumers; only "
                        f"{stream_index} of {WORST_CASE_CLIENT_BURST} received "
                        f"within {FANOUT_TIMEOUT_S}s"
                    )
        finally:
            # Closing the pool cancels the consumers; the auto_delete queues go
            # with them. The exchange is deleted explicitly.
            await pool.close_all()
            cleanup_channel = await connection.channel()
            try:
                await cleanup_channel.exchange_delete(exchange_name)
            finally:
                await cleanup_channel.close()
            if setup_channel is not None:
                await setup_channel.close()
