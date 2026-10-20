# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import pytest
from grpc_health.v1 import health_pb2

from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.grpc.channel_pool_health_servicer import (
    ChannelPoolHealthServicer,
)

SERVING = health_pb2.HealthCheckResponse.SERVING
NOT_SERVING = health_pb2.HealthCheckResponse.NOT_SERVING

NON_DURABLE = RabbitMQConnector.NON_DURABLE_POOL_NAME
DURABLE = RabbitMQConnector.DURABLE_POOL_NAME

SERVICER_LOGGER = "app.grpc.channel_pool_health_servicer"

# Deliberately not the library default (100): a surface that hardcoded the
# default instead of reading the non-durable pool's own value would still
# pass every threshold assertion here if this were 100.
MAX_POOL_SIZE = 75

# The readiness reserve, in STREAM slots. It is intentionally a different
# threshold from the CapacityGuard: the guard admits per pool against that
# pool's full stream capacity, while readiness trips earlier -- by this
# reserve -- on the non-durable pool alone.
MIN_AVAILABLE_STREAMS = 10


@pytest.fixture
def mock_connector(mocker):
    connector = mocker.Mock(spec=RabbitMQConnector)
    # Pin the not-draining default. ``RabbitMQConnector.draining`` is a plain
    # boolean the servicer reads synchronously and short-circuits to
    # NOT_SERVING when truthy; a fresh connector has it ``False``. It is an
    # instance-only attribute set in ``__init__``, so it is absent from the
    # class and therefore from a ``Mock(spec=RabbitMQConnector)`` -- reading
    # ``.draining`` on the spec'd mock would raise ``AttributeError`` and
    # loudly ERROR every row in this file. Pinning ``draining = False`` here is
    # what keeps the existing (non-draining) rows valid; draining rows opt in
    # by setting ``draining = True``.
    connector.draining = False
    return connector


@pytest.fixture
def min_available():
    return MIN_AVAILABLE_STREAMS


@pytest.fixture
def servicer(mock_connector, min_available):
    return ChannelPoolHealthServicer(
        mock_connector, min_available_streams=min_available
    )


@pytest.fixture
def set_pool_stats(mock_connector):
    """
    Publish one ``make_pool_stats_by_pool`` result on BOTH stats accessors.

    ``mocker.Mock(spec=RabbitMQConnector)`` returns a truthy ``Mock`` from any
    accessor a test forgot to stub, so a file that drove only the accessor the
    servicer does not call could pass vacuously. Setting ``pool_stats()`` (the
    flat, non-durable-only view) and ``pool_stats_by_pool()`` from the same
    factory result removes that failure mode: whichever one the servicer reads,
    it sees the same non-durable numbers, and neither returns a bare ``Mock``.
    """

    def _set(stats_by_pool):
        mock_connector.pool_stats_by_pool.return_value = stats_by_pool
        mock_connector.pool_stats.return_value = stats_by_pool.get(NON_DURABLE) or {}
        return stats_by_pool

    return _set


@pytest.fixture
def pool_stats(make_pool_stats_by_pool):
    """
    Both pools present, initialized, open and idle.

    Sized at one client per channel so the entry's stream capacity is
    ``MAX_POOL_SIZE``; sizing comes from ``non_durable_client_limit=`` rather
    than a hand-set ``channel_client_limit`` / ``max_client_capacity`` pair so
    the two can never disagree.
    """
    return make_pool_stats_by_pool(
        non_durable={"max_pool_size": MAX_POOL_SIZE},
        non_durable_client_limit=1,
    )


class TestCheck:
    @pytest.mark.asyncio
    async def test_returns_serving_when_pool_is_healthy(
        self, servicer, set_pool_stats, pool_stats
    ):
        set_pool_stats(pool_stats)

        response = await servicer.Check(None, None)

        assert response.status == SERVING

    @pytest.mark.asyncio
    async def test_returns_not_serving_when_pool_stats_empty(
        self, servicer, set_pool_stats
    ):
        set_pool_stats({})

        response = await servicer.Check(None, None)

        assert response.status == NOT_SERVING

    @pytest.mark.asyncio
    async def test_returns_not_serving_when_pool_not_initialized(
        self, servicer, set_pool_stats, pool_stats
    ):
        pool_stats[NON_DURABLE]["initialized"] = False
        set_pool_stats(pool_stats)

        response = await servicer.Check(None, None)

        assert response.status == NOT_SERVING

    @pytest.mark.asyncio
    async def test_returns_not_serving_when_pool_is_closed(
        self, servicer, set_pool_stats, pool_stats
    ):
        pool_stats[NON_DURABLE]["closed"] = True
        set_pool_stats(pool_stats)

        response = await servicer.Check(None, None)

        assert response.status == NOT_SERVING


class TestCheckStreamReserveBoundary:
    """
    The reserve is counted in non-durable STREAM slots, at any packing density.

    Stream capacity is ``max_pool_size * channel_client_limit``, so the same
    75-channel pool holds 75, 600 or 1200 clients at a client limit of 1, 8 or
    16. Every active-stream number below is written as a literal, independently
    counted from the capacity and the reserve; deriving it here from
    ``max_client_capacity - MIN_AVAILABLE_STREAMS`` would re-implement the
    servicer's own arithmetic and still pass if that arithmetic were wrong.
    """

    @pytest.mark.parametrize(
        "client_limit, active_streams, expected",
        [
            # client_limit 1: capacity 75 * 1 = 75, so the reserve boundary
            # sits at 75 - 10 = 65 active streams. These three rows are
            # numerically identical to the pre-Epic channel-unit thresholding
            # (64 / 65 / 66 in use against a 75-channel pool with a
            # 10-channel reserve) -- at one client per channel a stream IS a
            # channel, so the re-key preserves today's verdicts exactly.
            (1, 64, SERVING),
            (1, 65, SERVING),
            (1, 66, NOT_SERVING),
            # client_limit 8: capacity 75 * 8 = 600, boundary 600 - 10 = 590.
            (8, 589, SERVING),
            (8, 590, SERVING),
            (8, 591, NOT_SERVING),
            # client_limit 16: capacity 75 * 16 = 1200, boundary
            # 1200 - 10 = 1190.
            (16, 1189, SERVING),
            (16, 1190, SERVING),
            (16, 1191, NOT_SERVING),
        ],
    )
    @pytest.mark.asyncio
    async def test_verdict_at_the_reserve_boundary(
        self,
        servicer,
        set_pool_stats,
        make_pool_stats_by_pool,
        client_limit,
        active_streams,
        expected,
    ):
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable={"max_pool_size": MAX_POOL_SIZE},
                non_durable_client_limit=client_limit,
                non_durable_clients=active_streams,
            )
        )

        response = await servicer.Check(None, None)

        assert response.status == expected


class TestCheckWithIdleDurablePool:
    """
    Behavior preservation: an idle durable pool changes no verdict.

    The durable pool is idle in the steady state (it opens no channels until
    a durable checkout needs one), so every readiness verdict must match the
    pre-split behavior when the servicer read the non-durable pool alone.
    """

    @pytest.mark.parametrize(
        "non_durable_active_streams, expected",
        [
            # Client limit 1 against a 75-channel pool, reserve 10: the
            # boundary is 65 active streams, the same numbers the pre-split
            # channel-unit servicer used.
            (66, NOT_SERVING),
            (65, SERVING),
            (64, SERVING),
        ],
    )
    @pytest.mark.asyncio
    async def test_threshold_verdicts_match_pre_split_behavior(
        self,
        servicer,
        set_pool_stats,
        make_pool_stats_by_pool,
        non_durable_active_streams,
        expected,
    ):
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable={"max_pool_size": MAX_POOL_SIZE},
                non_durable_client_limit=1,
                non_durable_clients=non_durable_active_streams,
                durable_clients=0,
                durable_in_use=0,
            )
        )

        response = await servicer.Check(None, None)

        assert response.status == expected


class TestCheckIgnoresDurableUsage:
    """
    Durable usage does not consume the readiness reserve. DECIDED change.

    This inverts the pre-Epic ``TestCheckCountsDurableCheckouts``, which
    asserted that a durable checkout ate into the readiness reserve because
    the two pools were modelled as one shared channel budget. They are not:
    durable admission is gated on the durable pool's OWN stream budget (Spec
    Bee SDD decision on ``t1.38g.gs``), so a busy durable pool takes nothing
    away from the non-durable pool's stream slots. The readiness reserve now
    protects non-durable stream slots only, and readiness must stay SERVING
    while the non-durable pool has room no matter what the durable pool is
    doing. Re-keyed by ``t3.tj2.nm.nv.86`` (Plan Bee ``b.tj2``, Epic 2,
    Task 2).
    """

    @pytest.mark.asyncio
    async def test_serving_when_durable_pool_consumes_its_entire_budget(
        self, servicer, set_pool_stats, make_pool_stats_by_pool
    ):
        # The durable pool is fully spent -- 100 channels, 100 clients, its
        # whole 100-stream budget -- while the non-durable pool is idle.
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable={"max_pool_size": MAX_POOL_SIZE},
                non_durable_client_limit=1,
                non_durable_clients=0,
                durable={"total_channel_count": 100},
                durable_client_limit=1,
                durable_clients=100,
                durable_in_use=100,
            )
        )

        response = await servicer.Check(None, None)

        assert response.status == SERVING

    @pytest.mark.asyncio
    async def test_serving_when_summing_the_pools_would_cross_the_reserve(
        self, servicer, set_pool_stats, make_pool_stats_by_pool
    ):
        # 60 non-durable streams of 75 leaves 15 available, above the reserve
        # of 10. A servicer that added the durable pool's 6 clients would see
        # 66 of 75 -- 9 available -- and trip. Reading the non-durable pool
        # alone keeps this SERVING.
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable={"max_pool_size": MAX_POOL_SIZE},
                non_durable_client_limit=1,
                non_durable_clients=60,
                durable_clients=6,
                durable_in_use=6,
            )
        )

        response = await servicer.Check(None, None)

        assert response.status == SERVING

    @pytest.mark.asyncio
    async def test_serving_when_the_durable_pool_is_busy_at_a_high_client_limit(
        self, servicer, set_pool_stats, make_pool_stats_by_pool
    ):
        # A densely packed, heavily loaded durable pool -- 1500 of its 1600
        # stream slots taken -- alongside an idle non-durable pool.
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable={"max_pool_size": MAX_POOL_SIZE},
                non_durable_client_limit=16,
                non_durable_clients=0,
                durable={"max_pool_size": 100},
                durable_client_limit=16,
                durable_clients=1500,
                durable_in_use=100,
            )
        )

        response = await servicer.Check(None, None)

        assert response.status == SERVING


class TestCheckOverCapacity:
    """
    More active streams than stream capacity degrades, it does not explode.

    The stream-unit successor to the pre-Epic negative-available case: a pool
    reporting more clients than its capacity leaves availability negative, and
    readiness has to report NOT_SERVING without raising or wrapping around.
    """

    @pytest.fixture
    def over_capacity_stats(self, make_pool_stats_by_pool):
        # Client limit 16 against a 75-channel pool is 1200 stream slots;
        # 1400 active streams is 200 past it, so availability is -200.
        return make_pool_stats_by_pool(
            non_durable={"max_pool_size": MAX_POOL_SIZE},
            non_durable_client_limit=16,
            non_durable_clients=1400,
            non_durable_in_use=MAX_POOL_SIZE,
        )

    @pytest.mark.asyncio
    async def test_not_serving_when_active_streams_exceed_stream_capacity(
        self, servicer, set_pool_stats, over_capacity_stats
    ):
        set_pool_stats(over_capacity_stats)

        response = await servicer.Check(None, None)

        assert response.status == NOT_SERVING

    @pytest.mark.asyncio
    async def test_not_serving_log_reports_availability_in_stream_units(
        self, servicer, set_pool_stats, over_capacity_stats, caplog
    ):
        """
        The one log assertion in this file, and a knowing exception to the
        test-writing guide's "don't test logging output" rule: this line is an
        operator-facing signature the production diagnostics runbook keys on
        (the source repository's internal runbook, which is not published),
        and the units it
        reports changed with this Epic. The over-capacity case is used so the
        negative availability value is covered too. Asserts the unit word and
        the number, not the exact sentence.
        """
        set_pool_stats(over_capacity_stats)

        with caplog.at_level(logging.INFO, logger=SERVICER_LOGGER):
            await servicer.Check(None, None)

        messages = [
            record.getMessage()
            for record in caplog.records
            if record.name == SERVICER_LOGGER and record.levelno == logging.INFO
        ]

        assert any("streams" in message for message in messages)
        assert any("-200" in message for message in messages)


class TestCheckLifecycleSourcePool:
    """
    SDD SR-2.5: readiness follows the non-durable pool's lifecycle only.

    The durable pool must never gate readiness -- it initializes with zero
    channels, so a servicer that took ``initialized`` from the durable pool
    (or from an AND of both pools) would leave the pod permanently
    NOT_SERVING. The non-durable mirror cases below restate the lifecycle
    checks with the durable entry explicitly in the opposite state, so
    together they prove which pool the flags are read from rather than only
    that some pool's flag was read.
    """

    @pytest.mark.asyncio
    async def test_serving_when_durable_pool_is_not_initialized(
        self, servicer, set_pool_stats, make_pool_stats_by_pool
    ):
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable={
                    "max_pool_size": MAX_POOL_SIZE,
                    "initialized": True,
                },
                non_durable_client_limit=1,
                durable={"initialized": False},
            )
        )

        response = await servicer.Check(None, None)

        assert response.status == SERVING

    @pytest.mark.asyncio
    async def test_serving_when_durable_pool_is_closed(
        self, servicer, set_pool_stats, make_pool_stats_by_pool
    ):
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable={
                    "max_pool_size": MAX_POOL_SIZE,
                    "closed": False,
                },
                non_durable_client_limit=1,
                durable={"closed": True},
            )
        )

        response = await servicer.Check(None, None)

        assert response.status == SERVING

    @pytest.mark.asyncio
    async def test_not_serving_when_non_durable_pool_is_not_initialized(
        self, servicer, set_pool_stats, make_pool_stats_by_pool
    ):
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable={
                    "max_pool_size": MAX_POOL_SIZE,
                    "initialized": False,
                },
                non_durable_client_limit=1,
                durable={"initialized": True},
            )
        )

        response = await servicer.Check(None, None)

        assert response.status == NOT_SERVING

    @pytest.mark.asyncio
    async def test_not_serving_when_non_durable_pool_is_closed(
        self, servicer, set_pool_stats, make_pool_stats_by_pool
    ):
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable={
                    "max_pool_size": MAX_POOL_SIZE,
                    "closed": True,
                },
                non_durable_client_limit=1,
                durable={"closed": False},
            )
        )

        response = await servicer.Check(None, None)

        assert response.status == NOT_SERVING


class TestCheckWhileDraining:
    """
    SR-6.1: a draining pod reports NOT_SERVING regardless of pool capacity.

    Graceful drain (Task 1) sets ``connector.draining = True`` on SIGTERM. The
    servicer short-circuits on that signal ahead of every capacity/lifecycle
    check, so a pod that is going away stops being a readiness target even
    while it still has free non-durable stream slots -- internal traffic is
    steered off it before its pool is anywhere near exhausted.

    The draining signal is consumed verbatim as ``connector.draining`` (the
    surface recorded in Task 1's completion notes); the not-draining default
    lives in the ``mock_connector`` fixture, and these rows opt in explicitly.
    Every pool below is deep inside SERVING territory (well under the reserve),
    so only the draining signal explains the NOT_SERVING verdict -- the
    not-draining companion proves the same pool serves absent the signal, so
    the verdict is the signal talking and not a blanket NOT_SERVING.
    """

    @pytest.mark.parametrize(
        "client_limit, active_streams",
        [
            # client_limit 1: capacity 75, idle -- 75 free slots, far above
            # the 10-stream reserve. This is exactly the pool the SERVING
            # companion below (and TestCheck's healthy row) uses.
            (1, 0),
            # client_limit 8: capacity 600, only 8 active -- 592 free slots,
            # a packed sizing sitting deep inside SERVING territory. The
            # verdict cannot be read off capacity here either.
            (8, 8),
        ],
    )
    @pytest.mark.asyncio
    async def test_not_serving_while_draining_despite_available_capacity(
        self,
        servicer,
        mock_connector,
        set_pool_stats,
        make_pool_stats_by_pool,
        client_limit,
        active_streams,
    ):
        mock_connector.draining = True
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable={"max_pool_size": MAX_POOL_SIZE},
                non_durable_client_limit=client_limit,
                non_durable_clients=active_streams,
            )
        )

        response = await servicer.Check(None, None)

        assert response.status == NOT_SERVING

    @pytest.mark.asyncio
    async def test_serving_when_not_draining_and_pool_healthy(
        self, servicer, set_pool_stats, pool_stats
    ):
        # Anti-vacuity: the same healthy pool, with the fixture's not-draining
        # default in place, serves. Proves the NOT_SERVING rows above are the
        # draining signal and not a fixture that always trips.
        set_pool_stats(pool_stats)

        response = await servicer.Check(None, None)

        assert response.status == SERVING


class TestCheckWithPartialPoolStats:
    """
    The durable pool may be absent from the per-pool stats entirely.

    ``pool_stats_by_pool()`` omits a pool that does not exist yet, so the
    readiness surface has to tolerate a single-key dict rather than assume
    both entries are present.
    """

    @pytest.mark.asyncio
    async def test_serving_when_durable_pool_entry_is_missing(
        self, servicer, set_pool_stats, make_pool_stats_by_pool
    ):
        # 60 of 75 stream slots taken leaves 15 available, above the reserve.
        stats = make_pool_stats_by_pool(
            non_durable={"max_pool_size": MAX_POOL_SIZE},
            non_durable_client_limit=1,
            non_durable_clients=60,
        )
        set_pool_stats({NON_DURABLE: stats[NON_DURABLE]})

        response = await servicer.Check(None, None)

        assert response.status == SERVING

    @pytest.mark.asyncio
    async def test_not_serving_when_non_durable_pool_entry_is_missing(
        self, servicer, set_pool_stats, make_pool_stats_by_pool
    ):
        stats = make_pool_stats_by_pool()
        set_pool_stats({DURABLE: stats[DURABLE]})

        response = await servicer.Check(None, None)

        assert response.status == NOT_SERVING
