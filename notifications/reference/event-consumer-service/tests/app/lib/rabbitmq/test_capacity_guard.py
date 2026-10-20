# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import pytest

from notification_common.config.common_settings import (
    CommonSettings,
    common_settings,
)
from notification_common.errors.errors import (
    ChannelNotAvailableError,
)
from notification_common.rabbitmq.rabbitmq_connector import (
    RabbitMQConnector,
)

from app.config import settings
from app.lib.observability.admission_metrics import admission_metrics
from app.lib.rabbitmq import stream_memory_cap
from app.lib.rabbitmq.capacity_guard import (
    CapacityGuard,
    logger as capacity_guard_logger,
    re_routable_shed_threshold,
)

NON_DURABLE = RabbitMQConnector.NON_DURABLE_POOL_NAME
DURABLE = RabbitMQConnector.DURABLE_POOL_NAME

# Admission is keyed to STREAMS (clients), not channels: the guard compares one
# pool's ``total_client_count`` against that same pool's ``max_client_capacity``.
# The constants below are therefore stream counts unless named otherwise, and
# each equals ``max_pool_size * channel_client_limit`` for the sizing its name
# gives -- the same derivation ``ChannelPool.pool_stats()`` makes.
MAX_STREAMS_AT_LIMIT_1 = 100
MAX_STREAMS_AT_LIMIT_8 = 800
MAX_STREAMS_AT_LIMIT_16 = 1600

# The durable pool sized to 2 clients per channel, used where the two pools must
# be told apart by capacity as well as by usage.
DURABLE_MAX_STREAMS_AT_LIMIT_2 = 200
DURABLE_STREAMS_BELOW_CAPACITY = 150

# The factory's default ``max_pool_size``, in CHANNELS. It appears only in
# scenarios that deliberately set channel counts at odds with stream counts;
# no verdict in this file is derived from a channel number.
MAX_POOL_SIZE = 100
FEW_CHANNELS_IN_USE = 5

# A stream count comfortably below capacity at every packed sizing tested here,
# used for the "all channels checked out, streams still free" case.
SPARSELY_PACKED_STREAMS = 150

# A memory-safe stream cap well below the limit-8 pool's 800-stream capacity, so
# a rejection at this number cannot be explained by the pool bound.
#
# It lives up here with the shared constants rather than in the cap block below
# because, from Epic 7 Task 2 on, the cap is the ONLY bound that refuses a
# pinned non-durable stream (SDD SR-3.1 as amended): any test anywhere in this
# file that needs a genuine non-durable refusal has to drive one through a cap,
# not through the pool's own stream capacity.
BINDING_STREAM_CAP = 50

# Lifecycle states in which a pool cannot serve, whichever pool holds them.
UNUSABLE_POOL_STATES = ["missing-entry", "not-initialized", "closed"]

SURFACES = [
    pytest.param(False, id="non-durable-surface"),
    pytest.param(True, id="durable-surface"),
]


def _with_pool_state(stats, pool_name, state):
    """
    Put one pool of a ``pool_stats_by_pool()`` mapping into an unusable state.

    :param stats: The mapping to modify; the factory hands out a fresh deep
        copy per call, so mutating it in place cannot leak across tests.
    :param pool_name: The pool to break.
    :param state: One of ``UNUSABLE_POOL_STATES``.
    :returns: The same mapping.
    """
    if state == "missing-entry":
        stats.pop(pool_name)
    elif state == "not-initialized":
        stats[pool_name]["initialized"] = False
    elif state == "closed":
        stats[pool_name]["closed"] = True
    else:
        raise ValueError(f"unknown pool state: {state}")

    return stats


def _rejecting_stats(make_pool_stats_by_pool, set_stream_cap, pool_name, reason):
    """
    Build stats in which the named pool must be denied admission.

    DECIDED BEHAVIOR CHANGE (Epic 7, Task 2), ``at-capacity`` on the
    NON-DURABLE pool: filling the pool to its own ``max_client_capacity`` no
    longer denies anything there, because that number is now a report-only soft
    threshold and the memory-safe stream cap is the sole hard refusal for
    pinned traffic (SDD SR-3.1 as amended). A helper that kept handing back a
    full-but-admitting pool would have turned every ``at-capacity`` row of its
    consumers into a silent false green, so the non-durable refusal is now
    driven through a binding cap with the stream count sitting on it. The
    durable pool is untouched -- durable admission is single-tier and still
    refuses at its own pool bound.

    :param make_pool_stats_by_pool: The shared factory fixture's function.
    :param set_stream_cap: The ``set_stream_cap`` fixture, used to make the
        cap bind for the non-durable ``at-capacity`` case. Unused otherwise.
    :param pool_name: The pool to make unavailable.
    :param reason: ``at-capacity``, ``empty-stats``, or an
        ``UNUSABLE_POOL_STATES`` entry.
    :returns: A ``pool_stats_by_pool()`` return value.
    """
    if reason == "empty-stats":
        return {}

    stats = make_pool_stats_by_pool()

    if reason == "at-capacity":
        if pool_name == DURABLE:
            stats[pool_name]["total_client_count"] = MAX_STREAMS_AT_LIMIT_1
        else:
            set_stream_cap(BINDING_STREAM_CAP)
            stats[pool_name]["total_client_count"] = BINDING_STREAM_CAP
        return stats

    return _with_pool_state(stats, pool_name, reason)


@pytest.fixture
def mock_connector(mocker):
    # Establish the NOT-DRAINING default for the whole file. The guard reads
    # ``self._connector.draining`` on the non-durable path (Epic 5, Task 2 /
    # SR-6.1), and a bare ``mocker.Mock()`` returns a truthy child Mock for any
    # unset attribute -- so without this every pre-existing non-durable
    # admission row would silently flip to a draining refusal. ``draining`` is a
    # plain bool on ``RabbitMQConnector`` (False until ``begin_draining()``), so
    # the mock mirrors it as a plain False here. The draining rows in
    # ``TestDrainingRefusesAdmission`` opt back in with ``draining = True``.
    connector = mocker.Mock()
    connector.draining = False
    return connector


@pytest.fixture
def guard(mock_connector):
    return CapacityGuard(mock_connector)


@pytest.fixture
def pool_stats_by_pool(make_pool_stats_by_pool):
    """Both pools present, initialized, open, and idle."""
    return make_pool_stats_by_pool()


@pytest.fixture
def set_stream_cap(mocker):
    """
    Return a callable that pins ``memory_safe_stream_cap()`` to a value.

    Patches the operator override, which the cap function honors before it
    reads anything else, so the caller gets exactly the number it asks for.
    That patch also overrides the autouse ``non_binding_stream_memory_cap``
    fixture in ``tests/app/conftest.py`` for the test that asks for it --
    ``mocker`` applies patches in order and the later one wins.

    It sits with the shared fixtures rather than in the cap block below for the
    same reason ``BINDING_STREAM_CAP`` does: the cap is now the hard bound on
    the non-durable path, so tests throughout the file need it to express a
    non-durable refusal at all.

    :returns: ``set_stream_cap(cap)``, where ``cap`` is a stream count.
    """

    def _set(cap):
        mocker.patch.object(settings.settings, "max_non_durable_streams_per_pod", cap)

    return _set


@pytest.fixture
def set_shed_threshold(mocker):
    """
    Return a callable that pins the re-routable shed threshold to a value.

    Patches the operator override the same way ``set_stream_cap`` does, so the
    two admission knobs are configured through one shape. The value asked for
    is the value ``re_routable_shed_threshold()`` uses only while it sits
    inside ``[soft threshold, hard bound]``; outside that band the clamp moves
    it, which is why every caller below picks a number strictly between the
    pool's ``max_client_capacity`` and the configured cap. This fixture does
    not re-derive that clamp -- the arithmetic is pinned by
    ``re_routable_shed_threshold()``'s own unit tests.

    :returns: ``set_shed_threshold(threshold)``, where ``threshold`` is a
        stream count.
    """

    def _set(threshold):
        mocker.patch.object(
            settings.settings, "max_re_routable_streams_per_pod", threshold
        )

    return _set


class TestHasCapacityStreamAdmission:
    """
    Non-durable admission around the non-durable pool's stream capacity.

    That capacity is the SOFT threshold: the packing density the pool is
    provisioned for and the number the autoscaler reacts to, not a refusal
    point. Streams at or above it land in the pool's overflow band and keep
    being admitted, so every row here is about the pool's own stream counting
    -- which pool's numbers are read, and in what units -- and none of them is
    about a refusal. The autouse ``non_binding_stream_memory_cap`` fixture
    parks the hard bound (and, via the clamp, the shed threshold) far out of
    reach, so nothing else can answer these rows either.
    """

    @pytest.mark.parametrize(
        "client_limit, total_client_count, expected",
        [
            pytest.param(1, 0, True, id="limit-1-idle"),
            pytest.param(1, 99, True, id="limit-1-99-of-100-streams"),
            # DECIDED BEHAVIOR CHANGE (Epic 7, Task 2): the six rows at and past
            # the packing product were False through Epic 2 -- one-tier
            # admission refused there. Under SDD SR-3.1 as amended the packing
            # product is a report-only soft threshold and the memory-safe stream
            # cap is the sole hard refusal on the non-durable path, so a stream
            # crossing this number is admitted into the overflow band (SR-1.5)
            # instead of being rejected. This is the decided inversion, not
            # drift.
            pytest.param(1, 100, True, id="limit-1-100-of-100-streams"),
            pytest.param(1, 105, True, id="limit-1-105-of-100-streams"),
            pytest.param(8, 0, True, id="limit-8-idle"),
            pytest.param(8, 799, True, id="limit-8-799-of-800-streams"),
            pytest.param(8, 800, True, id="limit-8-800-of-800-streams"),
            pytest.param(8, 805, True, id="limit-8-805-of-800-streams"),
            pytest.param(16, 0, True, id="limit-16-idle"),
            pytest.param(16, 1599, True, id="limit-16-1599-of-1600-streams"),
            pytest.param(16, 1600, True, id="limit-16-1600-of-1600-streams"),
            pytest.param(16, 1605, True, id="limit-16-1605-of-1600-streams"),
        ],
    )
    def test_admission_tracks_streams_against_stream_capacity(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        client_limit,
        total_client_count,
        expected,
    ):
        # Capacity is 100 channels times the client limit, so the number being
        # crossed moves with the limit: 105 streams is past a limit-1 pool's
        # packing product and well inside a limit-8 one. That number is the SOFT
        # threshold, so crossing it is not a refusal point at any of the three
        # sizings -- the rows walk up to it, onto it, and past it, and the answer
        # stays yes. Each expected verdict is written out against the scenario's
        # own stream number rather than recomputed from the guard.
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=client_limit,
            non_durable_clients=total_client_count,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is expected

    def test_admits_at_limit_1_when_durable_use_holds_the_last_slot(
        self, guard, mock_connector, make_pool_stats_by_pool
    ):
        # DECIDED BEHAVIOR CHANGE (Epic 2, Task 1): this scenario returned False
        # under the interim combined-channel contract, where a durable checkout
        # spent a non-durable admission slot. Durable use is now accounted to the
        # durable pool alone, so 99 of 100 non-durable streams still admits.
        stats = make_pool_stats_by_pool(
            non_durable_clients=MAX_STREAMS_AT_LIMIT_1 - 1, durable_clients=1
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is True

    def test_admits_at_limit_1_when_the_non_durable_pool_is_at_its_soft_threshold(
        self, guard, mock_connector, make_pool_stats_by_pool
    ):
        # DECIDED BEHAVIOR CHANGE (Epic 7, Task 2): this returned False through
        # Epic 2, and its comment claimed the non-durable boundary at
        # client_limit 1 was unchanged from pre-Task behavior. Under SDD SR-3.1
        # as amended there is no boundary here to be unchanged: 100 streams over
        # a 100-stream packing product is the soft threshold, and the stream
        # crossing it is admitted into the overflow band (SR-1.5).
        #
        # The overflow band exists because a consumer stream is PINNED to the
        # pod it landed on for its whole life (PRD FR-11), so refusing at the
        # scaling threshold cannot be rescued by adding pods -- it is a hard
        # client failure, not a rebalance. Only the memory-safe cap, which is a
        # real physical bound, is allowed to refuse this traffic.
        stats = make_pool_stats_by_pool(
            non_durable_clients=MAX_STREAMS_AT_LIMIT_1, durable_clients=0
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is True


class TestHasCapacityIgnoresChannelCounts:
    """Verdicts must not be readable off channel counts in either direction."""

    @pytest.mark.parametrize("client_limit", [8, 16], ids=["limit-8", "limit-16"])
    def test_admits_when_all_channels_are_checked_out_but_streams_remain(
        self, guard, mock_connector, make_pool_stats_by_pool, client_limit
    ):
        # The case channel-keyed admission gets wrong. Every one of the 100
        # channels is checked out, so a check on in_use_channel_count would
        # report a full pool and 503 the request -- while the pool is really
        # carrying 150 streams of an 800- or 1600-stream budget.
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=client_limit,
            non_durable={
                "total_channel_count": MAX_POOL_SIZE,
                "available_channel_count": 0,
                "in_use_channel_count": MAX_POOL_SIZE,
            },
            non_durable_clients=SPARSELY_PACKED_STREAMS,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is True

    @pytest.mark.parametrize("client_limit", [8, 16], ids=["limit-8", "limit-16"])
    def test_rejects_when_streams_are_at_the_hard_bound_despite_idle_channels(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        set_stream_cap,
        client_limit,
    ):
        # The mirror: 95 of 100 channels are free, so a channel-keyed check would
        # admit, but the stream count has reached the bound that refuses. The
        # channel numbers here are deliberately incoherent with the stream number
        # -- a guard that reads either channel key cannot produce the expected
        # verdict, in this direction as well as the admitting one above.
        #
        # DECIDED BEHAVIOR CHANGE (Epic 7, Task 2) in the DRIVER, not the
        # property: this used to reach False by filling the pool to its own
        # max_client_capacity, which no longer refuses anyone (SDD SR-3.1 as
        # amended). The rejecting-direction mirror is now driven through the
        # memory-safe stream cap, the bound that does refuse. The cap of 50 sits
        # far below both pool sizings, so the pool bound cannot explain the
        # verdict either.
        set_stream_cap(BINDING_STREAM_CAP)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=client_limit,
            non_durable={
                "total_channel_count": MAX_POOL_SIZE,
                "available_channel_count": MAX_POOL_SIZE - FEW_CHANNELS_IN_USE,
                "in_use_channel_count": FEW_CHANNELS_IN_USE,
            },
            non_durable_clients=BINDING_STREAM_CAP,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is False


class TestHasCapacityPoolSeparation:
    """Each surface gates on the pool it consumes from, and only that pool."""

    def test_non_durable_soft_threshold_admits_non_durable_admission(
        self, guard, mock_connector, make_pool_stats_by_pool
    ):
        # DECIDED BEHAVIOR CHANGE (Epic 7, Task 2): this asserted False through
        # Epic 2, when the packing product was the non-durable refusal point.
        # Under SDD SR-3.1 as amended it is the report-only soft threshold, so
        # the non-durable surface keeps admitting there (SR-1.5's overflow
        # band); only the memory-safe cap refuses this traffic.
        stats = make_pool_stats_by_pool(
            non_durable_clients=MAX_STREAMS_AT_LIMIT_1, durable_clients=0
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is True

    def test_non_durable_soft_threshold_leaves_durable_admission_unaffected(
        self, guard, mock_connector, make_pool_stats_by_pool
    ):
        # Renamed for accuracy under Epic 7, Task 2 -- the expectation is
        # unchanged. A non-durable pool sitting on its packing product is at its
        # soft threshold, not saturated; genuine non-durable saturation is
        # cap-driven now and its isolation from the durable surface is pinned by
        # TestMemorySafeStreamCap and TestRaiseIfAtCapacity.
        stats = make_pool_stats_by_pool(
            non_durable_clients=MAX_STREAMS_AT_LIMIT_1, durable_clients=0
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=True) is True

    def test_durable_saturation_rejects_durable_admission(
        self, guard, mock_connector, make_pool_stats_by_pool
    ):
        stats = make_pool_stats_by_pool(
            non_durable_clients=0, durable_clients=MAX_STREAMS_AT_LIMIT_1
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=True) is False

    def test_durable_saturation_leaves_non_durable_admission_unaffected(
        self, guard, mock_connector, make_pool_stats_by_pool
    ):
        # DECIDED BEHAVIOR CHANGE (Epic 2, Task 1): the interim combined view
        # rejected here, because a saturated durable pool consumed the shared
        # budget. This replaces the old
        # "returns false when only the durable pool is saturated" test -- the
        # inversion is the decided split, not drift.
        stats = make_pool_stats_by_pool(
            non_durable_clients=0, durable_clients=MAX_STREAMS_AT_LIMIT_1
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is True

    def test_durable_surface_rejects_when_both_pools_are_at_their_own_numbers(
        self, guard, mock_connector, make_pool_stats_by_pool
    ):
        # DECIDED BEHAVIOR CHANGE (Epic 7, Task 2) in the TEST'S SHAPE: this and
        # the test below were one parametrize over SURFACES asserting a single
        # False. The two surfaces no longer agree -- durable admission is
        # byte-for-byte unchanged and still refuses at the durable pool bound,
        # while the non-durable surface admits at its soft threshold (SDD SR-3.1
        # as amended) -- and that asymmetry IS the point, so it is written out
        # per surface rather than hidden behind a shared expected literal.
        stats = make_pool_stats_by_pool(
            non_durable_clients=MAX_STREAMS_AT_LIMIT_1,
            durable_clients=MAX_STREAMS_AT_LIMIT_1,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=True) is False

    def test_non_durable_surface_admits_when_both_pools_are_at_their_own_numbers(
        self, guard, mock_connector, make_pool_stats_by_pool
    ):
        # DECIDED BEHAVIOR CHANGE (Epic 7, Task 2): the non-durable half of the
        # old both-surfaces parametrize asserted False. The identical 100 streams
        # over 100 mean different things on the two surfaces now -- a hard pool
        # bound on the durable side, a report-only soft threshold on the
        # non-durable one (SDD SR-3.1 as amended, PRD FR-11) -- so a saturated
        # durable pool alongside it changes nothing here.
        stats = make_pool_stats_by_pool(
            non_durable_clients=MAX_STREAMS_AT_LIMIT_1,
            durable_clients=MAX_STREAMS_AT_LIMIT_1,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is True

    def test_durable_admission_is_read_from_the_durable_entry(
        self, guard, mock_connector, make_pool_stats_by_pool, set_stream_cap
    ):
        # The two pools differ in both usage and capacity: durable holds 150 of
        # 200 stream slots while the non-durable pool is refusing. Only the
        # durable entry explains True -- reading the non-durable entry, or any
        # combined view of the two, yields False.
        #
        # DECIDED BEHAVIOR CHANGE (Epic 7, Task 2) in the DRIVER, not the
        # property: the non-durable side used to be made refusing by filling it
        # to its own 100-stream capacity, which now admits (SDD SR-3.1 as
        # amended) and would leave this test green even for a guard that read
        # the wrong entry. It is driven through a binding cap instead, the only
        # bound that still refuses pinned non-durable traffic.
        set_stream_cap(BINDING_STREAM_CAP)
        stats = make_pool_stats_by_pool(
            non_durable_clients=BINDING_STREAM_CAP,
            durable_client_limit=2,
            durable_clients=DURABLE_STREAMS_BELOW_CAPACITY,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=True) is True

    def test_durable_rejection_is_read_from_the_durable_entry(
        self, guard, mock_connector, make_pool_stats_by_pool
    ):
        # The converse: durable is full at its own capacity of 200 while the
        # non-durable pool is idle, so only the durable entry explains False.
        stats = make_pool_stats_by_pool(
            non_durable_clients=0,
            durable_client_limit=2,
            durable_clients=DURABLE_MAX_STREAMS_AT_LIMIT_2,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=True) is False

    def test_admits_durable_when_the_pool_holds_no_open_channels(
        self, guard, mock_connector, make_pool_stats_by_pool
    ):
        # The min_size=0 trap. rabbitmq_durable_channel_pool_min_size is 0, so a
        # healthy idle durable pool is initialized and open while reporting zero
        # total and zero available channels. Deriving durable availability from
        # either channel key would 503 every durable RPC on a healthy service.
        stats = make_pool_stats_by_pool(
            durable={
                "total_channel_count": 0,
                "available_channel_count": 0,
            },
            durable_in_use=0,
            durable_clients=0,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=True) is True


class TestHasCapacityFailsClosed:
    """An unusable pool denies its own surface and only its own surface."""

    @pytest.mark.parametrize("state", UNUSABLE_POOL_STATES)
    @pytest.mark.parametrize("durable_pool", SURFACES)
    def test_rejects_when_the_checked_pool_is_unusable(
        self, guard, mock_connector, make_pool_stats_by_pool, durable_pool, state
    ):
        # DECIDED BEHAVIOR CHANGE (Epic 2, Task 1) on the durable surface: a
        # missing or closed durable pool used to leave the combined view
        # admitting, since only the non-durable entry carried capacity. Each
        # pool now fails closed on its own surface.
        pool_name = DURABLE if durable_pool else NON_DURABLE
        stats = _with_pool_state(make_pool_stats_by_pool(), pool_name, state)
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=durable_pool) is False

    @pytest.mark.parametrize(
        "by_pool",
        [pytest.param({}, id="empty-stats"), pytest.param(None, id="no-stats")],
    )
    @pytest.mark.parametrize("durable_pool", SURFACES)
    def test_rejects_when_no_pool_stats_are_reported(
        self, guard, mock_connector, durable_pool, by_pool
    ):
        mock_connector.pool_stats_by_pool.return_value = by_pool

        assert guard.has_capacity(durable_pool=durable_pool) is False

    @pytest.mark.parametrize("state", UNUSABLE_POOL_STATES)
    @pytest.mark.parametrize("durable_pool", SURFACES)
    def test_admits_when_only_the_other_pool_is_unusable(
        self, guard, mock_connector, make_pool_stats_by_pool, durable_pool, state
    ):
        # Lifecycle is per pool: a not-initialized, closed, or absent durable
        # pool must not deny non-durable admission, and the same in reverse.
        other_pool = NON_DURABLE if durable_pool else DURABLE
        stats = _with_pool_state(make_pool_stats_by_pool(), other_pool, state)
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=durable_pool) is True


class TestHasCapacityExhaustionLog:
    """
    The refusal log is the operator-facing statement of the re-key.

    DECIDED BEHAVIOR CHANGE (Epic 7, Task 2): the two surfaces no longer share
    one fingerprint, so this class no longer parametrizes over them. The
    durable surface still refuses at its pool bound and still says so with the
    ``pool exhausted`` line. The non-durable surface cannot reach that line at
    its packing product any more -- it admits there under SDD SR-3.1 as amended
    -- so the fingerprint of a non-durable refusal is now the memory-safe cap
    line, and that is what the second test pins.
    """

    def test_logs_the_durable_pool_and_its_stream_counts_when_exhausted(
        self, guard, mock_connector, make_pool_stats_by_pool, caplog
    ):
        # Both pools are saturated and identically sized, so the emitted line
        # differs only by pool name -- naming the wrong pool fails this. The
        # numbers are streams, not channels: 800 of 800 over 100 channels.
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8,
            non_durable_clients=MAX_STREAMS_AT_LIMIT_8,
            durable_client_limit=8,
            durable_clients=MAX_STREAMS_AT_LIMIT_8,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        with caplog.at_level(logging.INFO, logger=capacity_guard_logger.name):
            guard.has_capacity(durable_pool=True)

        assert (
            f"Capacity check: {DURABLE} pool exhausted "
            f"(streams in use: {MAX_STREAMS_AT_LIMIT_8}, "
            f"max streams: {MAX_STREAMS_AT_LIMIT_8})" in caplog.text
        )

    def test_logs_the_cap_and_the_stream_count_when_non_durable_refuses(
        self, guard, mock_connector, make_pool_stats_by_pool, set_stream_cap, caplog
    ):
        # The non-durable half of the old two-row table, re-keyed to the line a
        # non-durable refusal now emits. An operator reading a 503 on this
        # surface has to be told which number was hit, and past the soft
        # threshold that number is the cap, not the pool's capacity: the pool is
        # deliberately 800 streams wide here while the refusal happens at 50.
        set_stream_cap(BINDING_STREAM_CAP)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=BINDING_STREAM_CAP
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        with caplog.at_level(logging.INFO, logger=capacity_guard_logger.name):
            guard.has_capacity()

        assert (
            f"Capacity check: memory-safe stream cap reached "
            f"(streams: {BINDING_STREAM_CAP}, cap: {BINDING_STREAM_CAP})" in caplog.text
        )


class TestRaiseIfAtCapacity:

    @pytest.mark.parametrize("durable_pool", SURFACES)
    def test_returns_none_when_the_checked_pool_admits(
        self, guard, mock_connector, pool_stats_by_pool, durable_pool
    ):
        mock_connector.pool_stats_by_pool.return_value = pool_stats_by_pool

        assert guard.raise_if_at_capacity(durable_pool=durable_pool) is None

    @pytest.mark.parametrize(
        "reason",
        ["at-capacity", "empty-stats"] + UNUSABLE_POOL_STATES,
    )
    @pytest.mark.parametrize("durable_pool", SURFACES)
    def test_raises_channel_not_available_on_every_rejection_reason(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        set_stream_cap,
        durable_pool,
        reason,
    ):
        # The meaning of this test is unchanged under Epic 7, Task 2; what a
        # rejecting state IS on the non-durable surface changed underneath it,
        # and ``_rejecting_stats`` absorbs that (see its docstring). Every other
        # reason -- empty stats and the three unusable pool states -- refuses on
        # both surfaces exactly as before, since lifecycle is checked ahead of
        # any threshold and is not tiered.
        pool_name = DURABLE if durable_pool else NON_DURABLE
        stats = _rejecting_stats(
            make_pool_stats_by_pool, set_stream_cap, pool_name, reason
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        with pytest.raises(ChannelNotAvailableError) as exc_info:
            guard.raise_if_at_capacity(durable_pool=durable_pool)

        assert type(exc_info.value) is ChannelNotAvailableError

    @pytest.mark.parametrize("durable_pool", SURFACES)
    def test_does_not_raise_when_only_the_other_pool_is_at_capacity(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        set_stream_cap,
        durable_pool,
    ):
        # DECIDED BEHAVIOR CHANGE (Epic 2, Task 1) on the non-durable surface:
        # a saturated durable pool used to raise here. Each entry point now
        # raises only for its own pool's exhaustion.
        #
        # DECIDED BEHAVIOR CHANGE (Epic 7, Task 2) in the DRIVER, not the
        # property: the durable-surface direction used to put the non-durable
        # pool "at capacity" by filling it to its packing product, which is no
        # longer a refusing state (SDD SR-3.1 as amended) and so no longer poses
        # the question. Both directions now go through ``_rejecting_stats``, so
        # the other pool is genuinely refusing on each -- the durable pool at its
        # own bound, the non-durable pool at a binding cap.
        other_pool = NON_DURABLE if durable_pool else DURABLE
        stats = _rejecting_stats(
            make_pool_stats_by_pool, set_stream_cap, other_pool, "at-capacity"
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.raise_if_at_capacity(durable_pool=durable_pool) is None


# --- Memory-safe stream cap (Epic 2, Task 4; re-scoped Epic 7, Task 2) -------
#
# Everything below drives the cap through the operator override
# (``max_non_durable_streams_per_pod``), which ``memory_safe_stream_cap()``
# returns unconditionally -- the budget-derived arithmetic is pinned in
# ``tests/app/lib/rabbitmq/test_stream_memory_cap.py``, not here. The guard
# re-reads the cap on every admission decision, so patching the setting inside a
# test takes effect on the next call and no guard has to be rebuilt. These
# patches also override the autouse ``non_binding_stream_memory_cap`` fixture in
# ``tests/app/conftest.py``, which otherwise keeps the cap out of the way of the
# pool-sizing tables above.
#
# Since Epic 7, Task 2 those patches decide the HARD threshold rather than a
# secondary one: on the non-durable path the pool's own stream capacity is a
# report-only soft threshold, so a test that sets the cap here is choosing the
# only number at which a pinned stream is refused (SDD SR-3.1 as amended). The
# same override is also the top of the band ``re_routable_shed_threshold()``
# splits, so raising it moves the shed line too -- which is why the autouse
# fixture's absolute value neutralizes both tiers at once, and why the shared
# ``set_stream_cap`` fixture and ``BINDING_STREAM_CAP`` now live at the top of
# this file with the rest of the shared scaffolding.

# Far above every capacity these fixtures can express, so nothing the cap does
# can move a boundary.
GENEROUS_STREAM_CAP = 100_000

# Channel counts chosen to be incoherent with the stream counts they accompany,
# so a guard that compared the cap against ``in_use_channel_count`` would return
# the opposite verdict in each direction.
CHANNELS_HOLDING_THE_CAPPED_STREAMS = 7
STREAMS_WELL_UNDER_THE_CAP = 10


@pytest.fixture
def no_stream_memory_cap(mocker):
    """
    Configure the real "no memory backstop" case, where
    ``memory_safe_stream_cap()`` returns None.

    Reached by clearing the override and configuring an unbounded prefetch,
    which is the production route to a None cap: the per-stream footprint is
    undefined, so no cap can be computed. Driving it through the settings the
    way production would is the point -- a stub returning None would not prove
    the fall-through survives the branch that actually produces it.

    The warn-once cache is cleared on both sides so this neither inherits nor
    leaves behind a suppressed warning for the sibling tests that assert on it.
    """
    mocker.patch.object(settings.settings, "max_non_durable_streams_per_pod", None)
    mocker.patch.object(common_settings, "rabbitmq_channel_prefetch_count", 0)
    stream_memory_cap._warn_unbounded_prefetch_once.cache_clear()
    yield
    stream_memory_cap._warn_unbounded_prefetch_once.cache_clear()


class TestMemorySafeStreamCap:
    """
    The hard bound on non-durable admission: the memory-safe stream cap.

    DECIDED BEHAVIOR CHANGE (Epic 7, Task 2): the cap is no longer a BACKSTOP
    behind the pool bound. Under SDD SR-3.1 as amended it is the SOLE hard
    refusal on the non-durable path -- the pool's own stream capacity became a
    report-only soft threshold, so there is nothing left for the cap to sit
    behind and no case in which a smaller pool bound hides it. What must hold
    now is that it binds wherever it is configured, that it refuses ALONE (the
    pool bound below it refuses nobody), and that it stays invisible on the
    durable path, which is single-tier and uncapped.

    The one surviving pool-bound refusal is the incomputable-cap fallback, and
    it is pinned by ``test_incomputable_cap_falls_back_to_the_strict_pool_bound``
    below.
    """

    @pytest.mark.parametrize(
        "total_client_count, expected",
        [
            pytest.param(BINDING_STREAM_CAP - 1, True, id="49-under-a-cap-of-50"),
            pytest.param(BINDING_STREAM_CAP, False, id="50-at-a-cap-of-50"),
            pytest.param(BINDING_STREAM_CAP + 10, False, id="60-over-a-cap-of-50"),
        ],
    )
    def test_cap_binds_when_it_is_below_the_pool_bound(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        set_stream_cap,
        total_client_count,
        expected,
    ):
        # The pool has 800 stream slots and every row here is nowhere near them,
        # so the pool bound admits throughout and only the cap can produce the
        # False rows. Each verdict is written against the row's own stream count
        # and the configured cap of 50, not recomputed from the guard.
        set_stream_cap(BINDING_STREAM_CAP)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=total_client_count
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is expected

    @pytest.mark.parametrize(
        "total_client_count, expected",
        [
            pytest.param(MAX_STREAMS_AT_LIMIT_8 - 1, True, id="799-of-800-streams"),
            pytest.param(MAX_STREAMS_AT_LIMIT_8, True, id="800-of-800-streams"),
        ],
    )
    def test_generous_cap_leaves_nothing_refusing_below_it(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        set_stream_cap,
        total_client_count,
        expected,
    ):
        # DECIDED BEHAVIOR CHANGE (Epic 7, Task 2): the 800-of-800 row was False
        # and this test was named for the backstop property -- a generous cap
        # left the pool's own 800 as the reject boundary. Under SDD SR-3.1 as
        # amended the pool bound refuses nobody while a cap is computable, so
        # with the cap at 100,000 there is nothing left below it that can refuse:
        # both rows admit, and a pod carrying 800 streams against a cap of
        # 100,000 keeps taking clients into its overflow band.
        set_stream_cap(GENEROUS_STREAM_CAP)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=total_client_count
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is expected

    @pytest.mark.parametrize(
        "total_client_count, expected",
        [
            pytest.param(MAX_STREAMS_AT_LIMIT_8 - 1, True, id="799-of-800-streams"),
            pytest.param(MAX_STREAMS_AT_LIMIT_8, False, id="800-of-800-streams"),
        ],
    )
    def test_cap_equal_to_the_pool_bound_refuses_at_that_shared_number(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        set_stream_cap,
        total_client_count,
        expected,
    ):
        # RE-JUSTIFIED (Epic 7, Task 2), rows unchanged. The old name and
        # rationale said the cap "behaves like no cap" here because the two
        # bounds coincide at 800 and either could account for the rejection.
        # That is now false in the other direction: with a computable cap the
        # pool bound refuses nobody, so the CAP is what rejects at 800 and it is
        # the only thing that can. The rows survive the re-tiering by
        # coincidence of value, not because the pool bound still has a say.
        set_stream_cap(MAX_STREAMS_AT_LIMIT_8)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=total_client_count
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is expected

    @pytest.mark.parametrize(
        "total_client_count, expected",
        [
            pytest.param(MAX_STREAMS_AT_LIMIT_8 - 1, True, id="799-of-800-streams"),
            pytest.param(MAX_STREAMS_AT_LIMIT_8, False, id="800-of-800-streams"),
        ],
    )
    def test_incomputable_cap_falls_back_to_the_strict_pool_bound(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        no_stream_memory_cap,
        total_client_count,
        expected,
    ):
        # PROMOTED (Epic 7, Task 2), rows unchanged. This was the fall-through
        # case -- "no computable cap must not fail closed" -- back when the pool
        # bound refused on every non-durable request. It is now the ONLY
        # surviving pool-bound refusal on this path: with a prefetch of 0 the
        # per-stream broker footprint is undefined and no cap can be computed,
        # so the pool bound becomes the hard bound and 800 of 800 rejects (SDD
        # SR-3.5's always-on invariant).
        #
        # The point is fail-safe, never unbounded. Admission on a pod whose
        # memory cost per stream is unknown has to stay bounded by a real
        # number, so the guard falls BACK to the pool's own capacity rather than
        # falling THROUGH to no bound at all -- and the band is degenerate
        # there, so that refusal is class-blind, the documented exception to
        # PRD FR-5.
        #
        # The ``no_stream_memory_cap`` fixture drives the real production route
        # to a None cap through the prefetch setting rather than stubbing the
        # cap function, which is what makes this the fallback's pin rather than
        # a restatement of the guard's own branch.
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=total_client_count
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is expected

    def test_cap_counts_streams_not_the_channels_holding_them(
        self, guard, mock_connector, make_pool_stats_by_pool, set_stream_cap
    ):
        # The discriminating case. 50 streams packed 8 to a channel occupy 7 of
        # 100 channels, so a cap compared against in_use_channel_count would see
        # 7 against 50 and admit. The cap is in stream units, so this rejects.
        set_stream_cap(BINDING_STREAM_CAP)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8,
            non_durable={
                "total_channel_count": MAX_POOL_SIZE,
                "available_channel_count": (
                    MAX_POOL_SIZE - CHANNELS_HOLDING_THE_CAPPED_STREAMS
                ),
                "in_use_channel_count": CHANNELS_HOLDING_THE_CAPPED_STREAMS,
            },
            non_durable_clients=BINDING_STREAM_CAP,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is False

    def test_cap_admits_when_channels_reach_it_but_streams_do_not(
        self, guard, mock_connector, make_pool_stats_by_pool, set_stream_cap
    ):
        # The mirror, which a channel-keyed cap gets wrong in the other
        # direction: 50 channels checked out against a cap of 50 would reject,
        # while the 10 streams they carry are far under it.
        set_stream_cap(BINDING_STREAM_CAP)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8,
            non_durable={
                "total_channel_count": MAX_POOL_SIZE,
                "available_channel_count": MAX_POOL_SIZE - BINDING_STREAM_CAP,
                "in_use_channel_count": BINDING_STREAM_CAP,
            },
            non_durable_clients=STREAMS_WELL_UNDER_THE_CAP,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is True

    def test_durable_admission_ignores_a_cap_its_usage_already_exceeds(
        self, guard, mock_connector, make_pool_stats_by_pool, set_stream_cap
    ):
        # DECIDED SCOPE (Epic 2, Task 4): the memory-safe backstop applies to the
        # NON-DURABLE path only. The handful of long-lived durable queues are not
        # the fan-out memory risk the cap exists to contain, and durable clients
        # are gated on the durable pool's own budget -- capping them would 503
        # durable RPCs for a reason that has nothing to do with them, the exact
        # coupling Task 1 removed. A cap of 1 against one durable client would
        # reject on the non-durable side; here it must admit.
        set_stream_cap(1)
        stats = make_pool_stats_by_pool(durable_clients=1)
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=True) is True

    def test_durable_admission_ignores_the_cap_well_inside_its_own_budget(
        self, guard, mock_connector, make_pool_stats_by_pool, set_stream_cap
    ):
        # The same decided scope at a realistic durable load: 150 of the durable
        # pool's own 200 stream slots, against a cap of 50. The durable pool has
        # room, so the answer is yes; a cap that leaked onto this path would see
        # 150 over 50 and reject.
        set_stream_cap(BINDING_STREAM_CAP)
        stats = make_pool_stats_by_pool(
            durable_client_limit=2, durable_clients=DURABLE_STREAMS_BELOW_CAPACITY
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=True) is True

    def test_generous_cap_does_not_rescue_an_unusable_non_durable_pool(
        self, guard, mock_connector, make_pool_stats_by_pool, set_stream_cap
    ):
        # Lifecycle still wins: the cap is a ceiling on admission, never a
        # licence to admit against a pool that cannot serve.
        set_stream_cap(GENEROUS_STREAM_CAP)
        stats = _with_pool_state(make_pool_stats_by_pool(), NON_DURABLE, "closed")
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is False

    def test_binding_cap_does_not_change_durable_fail_closed(
        self, guard, mock_connector, make_pool_stats_by_pool, set_stream_cap
    ):
        # And the cap changes nothing on the durable surface in either
        # direction: an uninitialized durable pool still rejects.
        set_stream_cap(BINDING_STREAM_CAP)
        stats = _with_pool_state(make_pool_stats_by_pool(), DURABLE, "not-initialized")
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=True) is False

    def test_cap_driven_rejection_raises_channel_not_available(
        self, guard, mock_connector, make_pool_stats_by_pool, set_stream_cap
    ):
        # Shedding for memory has to reach the client as the same 503 /
        # RESOURCE_EXHAUSTED retry-with-backoff contract as pool exhaustion, so
        # the exact type is asserted -- a cap-specific subclass would be an
        # API-visible change this Task did not authorize.
        set_stream_cap(BINDING_STREAM_CAP)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=BINDING_STREAM_CAP
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        with pytest.raises(ChannelNotAvailableError) as exc_info:
            guard.raise_if_at_capacity()

        assert type(exc_info.value) is ChannelNotAvailableError

    def test_cap_shed_logs_the_cap_line_at_warning_and_says_nothing_else(
        self, guard, mock_connector, make_pool_stats_by_pool, set_stream_cap, caplog
    ):
        """
        JUSTIFIED LOGGING TEST: pin the cap shed's own operator-facing line.

        The runbook's first move on a shed is to tell one refusal reason from
        another, because the remedies are opposite -- add pods for saturation,
        raise the budget (or accept the shed) for the configured backstop. The
        cap line is what carries that for a cap shed, so its phrase, its level,
        and the fact that it is the ONLY record emitted are all pinned here.

        DECIDED BEHAVIOR CHANGE (Epic 7, Task 2): this test used to carry a
        second half that drove the non-durable ``pool exhausted`` INFO line at
        800 of 800 and contrasted the two. That call now ADMITS and logs
        nothing (SDD SR-3.1 as amended), and the two lines an operator must
        tell apart on this surface are no longer the cap and pool exhaustion --
        they are the cap WARNING and the new shed INFO. That second half is
        RETIRED here rather than re-pointed: the WARNING-versus-shed-INFO
        pairing has a single unconditional owner in
        ``TestShedLogsDistinctlyFromTheCap``, so asserting it here as well would
        pin it twice, and dropping it without that owner would pin it zero
        times.
        """
        set_stream_cap(BINDING_STREAM_CAP)
        capped = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=BINDING_STREAM_CAP
        )
        mock_connector.pool_stats_by_pool.return_value = capped

        with caplog.at_level(logging.INFO, logger=capacity_guard_logger.name):
            guard.has_capacity()

        assert [(record.levelno, record.getMessage()) for record in caplog.records] == [
            (
                logging.WARNING,
                f"Capacity check: memory-safe stream cap reached "
                f"(streams: {BINDING_STREAM_CAP}, cap: {BINDING_STREAM_CAP})",
            )
        ]


# --- Re-routable shed threshold (Epic 7, Task 2) ----------------------------
#
# The middle tier of non-durable admission (SDD SR-3.7, PRD FR-12). Everything
# below drives it through the operator override
# (``max_re_routable_streams_per_pod``) via the ``set_shed_threshold`` fixture,
# never by letting the default midpoint fall where it may: the clamp arithmetic
# -- ``soft <= shed <= hard bound`` and the degenerate-band re-anchor -- belongs
# to ``re_routable_shed_threshold()``'s own unit tests, and re-deriving it here
# would make these verdicts restatements of the function rather than pins on
# what the GUARD does with the resolved number.
#
# The scenario is one three-tier band, sized so its three numbers are distinct
# and none can stand in for another: a ``client_limit`` 8 non-durable pool
# (soft threshold 800), the cap patched to 1000, and the shed threshold patched
# to 900, comfortably inside the band. A verdict explained by the soft
# threshold, or by the cap, cannot also be explained by the shed line.

# The hard bound of the shed-tier scenario. Above the limit-8 pool's 800-stream
# packing product, so the overflow band it tops is real rather than inverted.
SHED_BAND_STREAM_CAP = 1000

# The shed line inside that band: 100 streams above the soft threshold and 100
# below the cap, so neither outer tier can account for a refusal here.
SHED_THRESHOLD_IN_BAND = 900

# One stream below the shed line -- the anti-vacuity row, where both classes
# must still be admitted.
STREAMS_BELOW_THE_SHED_THRESHOLD = SHED_THRESHOLD_IN_BAND - 1

# Inside the band ABOVE the shed line and below the cap: the range in which the
# two classes disagree, used to pin the default class of an unclassified
# caller.
STREAMS_RESERVED_FOR_PINNED_STREAMS = 950

# A shed threshold below every pool bound these fixtures express. The clamp
# raises it to the pool's own ``max_client_capacity``, so it is the most
# aggressive shed line a configuration can produce -- which is exactly what the
# durable path has to be shown ignoring.
SHED_THRESHOLD_BELOW_EVERY_POOL_BOUND = 1

# A shed threshold well below the limit-8 pool's 800-stream bound, used under
# the incomputable-cap fallback where that bound is also the soft threshold.
# A configured shed line that low would bite everywhere if the degenerate band
# did not re-anchor it onto the pool bound.
SHED_THRESHOLD_BELOW_THE_POOL_BOUND = 400


def _verdicts_by_stream_class(guard):
    """
    Return ``(re_routable_verdict, pinned_verdict)`` for the current stats.

    Both calls read the same ``pool_stats_by_pool()`` return value and differ
    in exactly one argument, so a pair whose halves disagree can only be the
    caller's stream class talking -- and a pair whose halves agree is a
    class-blind bound. That contrast is the property every class below is
    about, which is why the two verdicts are compared as a tuple rather than
    asserted one per test.

    :param guard: The ``CapacityGuard`` under test.
    :returns: ``(has_capacity(re_routable=True), has_capacity())`` -- the
        second call passing no class argument on purpose, so the pinned half
        of every pair also exercises the parameter's default.
    """
    return (guard.has_capacity(re_routable=True), guard.has_capacity())


class TestShedThresholdRefusesReRoutableStreams:
    """
    SDD SR-3.7's core clause: the shed threshold refuses re-routable streams
    and only re-routable streams.

    Inside the pool's overflow band a RE-ROUTABLE stream -- one whose client
    retry can land on another pod -- is refused at the shed line, while a
    PINNED stream keeps being admitted up to the hard bound. That reserves the
    top of the band for traffic no amount of scale-out can rescue (PRD FR-12
    over FR-11). The hard bound itself is class-blind and refuses both.

    Each row drives ONE stats report through both classes, so the two verdicts
    of a pair differ only by the caller's class, and every expected verdict is
    written as a literal against the row's own stream count rather than
    recomputed from the guard.
    """

    def test_re_routable_is_refused_at_the_shed_threshold_while_pinned_is_admitted(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        set_stream_cap,
        set_shed_threshold,
    ):
        # The headline pair. 900 streams is past the shed line, 100 clear of
        # the soft threshold below it and 100 clear of the cap above it, so
        # neither outer tier can produce either half of this answer: the soft
        # threshold refuses nobody and the cap has not been reached.
        set_stream_cap(SHED_BAND_STREAM_CAP)
        set_shed_threshold(SHED_THRESHOLD_IN_BAND)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=SHED_THRESHOLD_IN_BAND
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert _verdicts_by_stream_class(guard) == (False, True)

    def test_both_classes_are_admitted_one_stream_below_the_shed_threshold(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        set_stream_cap,
        set_shed_threshold,
    ):
        # Anti-vacuity for the row above: at 899 the re-routable stream is
        # admitted, so the refusal at 900 is the shed threshold's doing and not
        # a blanket refusal of re-routable traffic. This is also the row that
        # fails if the comparison is ever written as ``>`` instead of ``>=``
        # in the wrong direction -- the shed line is inclusive of the threshold
        # and exclusive of everything under it.
        set_stream_cap(SHED_BAND_STREAM_CAP)
        set_shed_threshold(SHED_THRESHOLD_IN_BAND)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8,
            non_durable_clients=STREAMS_BELOW_THE_SHED_THRESHOLD,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert _verdicts_by_stream_class(guard) == (True, True)

    def test_both_classes_are_refused_at_the_cap(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        set_stream_cap,
        set_shed_threshold,
    ):
        # The hard bound is class-blind: at 1000 the pinned stream that was
        # still admitted at 900 is refused too. The shed threshold is left
        # configured at 900 here on purpose -- the cap must win over a shed
        # line that is also crossed, and it must win for BOTH classes.
        set_stream_cap(SHED_BAND_STREAM_CAP)
        set_shed_threshold(SHED_THRESHOLD_IN_BAND)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=SHED_BAND_STREAM_CAP
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert _verdicts_by_stream_class(guard) == (False, False)


class TestShedThresholdDoesNotTouchDurableAdmission:
    """
    SDD SR-3.7's scope clause, and the durable-untouched half of this Task's
    Success Criteria: the shed tier exists on the non-durable path only.

    Durable admission is single-tier -- gated on the durable pool's own stream
    budget, with no overflow band, no cap, and no class -- so the most
    aggressive shed threshold a configuration can produce must leave it
    admitting. Asserted at the same layer as
    ``test_durable_admission_ignores_a_cap_its_usage_already_exceeds``: a
    ``has_capacity(durable_pool=True)`` call whose durable pool has room.

    Both tests pass ``re_routable=True`` explicitly. The durable path ignores
    the argument entirely, so a guard that let the class leak across surfaces
    fails here rather than silently 503-ing durable RPCs for a non-durable
    pod's crowding.
    """

    def test_an_aggressive_shed_threshold_leaves_an_idle_durable_pool_admitting(
        self, guard, mock_connector, make_pool_stats_by_pool, set_shed_threshold
    ):
        # The shed threshold is configured below every pool bound, and the
        # non-durable pool is sitting on its own 100-stream capacity, which the
        # clamp makes the effective shed line -- so every re-routable
        # non-durable call against these same stats refuses. The durable pool
        # is idle, and idle is idle regardless.
        set_shed_threshold(SHED_THRESHOLD_BELOW_EVERY_POOL_BOUND)
        stats = make_pool_stats_by_pool(
            non_durable_clients=MAX_STREAMS_AT_LIMIT_1, durable_clients=0
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=True, re_routable=True) is True

    def test_an_aggressive_shed_threshold_leaves_a_busy_durable_pool_admitting(
        self, guard, mock_connector, make_pool_stats_by_pool, set_shed_threshold
    ):
        # The same aggressive shed line at a realistic durable load: 150 of the
        # durable pool's own 200 stream slots. The durable pool has room, so
        # the answer is yes; a shed tier that leaked onto this path would see
        # 150 over a shed line of 200 -- or, worse, over the non-durable pool's
        # 100 -- and refuse.
        set_shed_threshold(SHED_THRESHOLD_BELOW_EVERY_POOL_BOUND)
        stats = make_pool_stats_by_pool(
            non_durable_clients=MAX_STREAMS_AT_LIMIT_1,
            durable_client_limit=2,
            durable_clients=DURABLE_STREAMS_BELOW_CAPACITY,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=True, re_routable=True) is True


class TestUnclassifiedCallersAreTreatedAsPinned:
    """
    SDD SR-3.7's default clause: a caller that names no class is PINNED.

    The default is the conservative side of the tier. Misclassifying a
    re-routable stream as pinned costs only band fairness; misclassifying a
    pinned stream as re-routable rejects traffic nothing can rescue, which is
    the exact failure PRD FR-11 exists to prevent.

    It is also the whole suite's safety net. Every pre-existing call site in
    this file -- and every call site in the service that has not been
    classified yet -- reaches the guard as a bare ``has_capacity()``, so if the
    default were "re-routable" a configured shed threshold would silently start
    refusing all of them. Pinning the default here is what makes it a contract
    rather than an accident of the signature.
    """

    def test_a_call_with_no_class_argument_is_admitted_above_the_shed_threshold(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        set_stream_cap,
        set_shed_threshold,
    ):
        # 950 streams sits in the range where the two classes disagree: above
        # the shed line of 900 and below the cap of 1000. A re-routable stream
        # is refused here (pinned by
        # ``TestShedThresholdRefusesReRoutableStreams``), so an admitting
        # verdict from a call that passes no class argument can only mean the
        # default resolved to pinned.
        set_stream_cap(SHED_BAND_STREAM_CAP)
        set_shed_threshold(SHED_THRESHOLD_IN_BAND)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8,
            non_durable_clients=STREAMS_RESERVED_FOR_PINNED_STREAMS,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is True


class TestShedLogsDistinctlyFromTheCap:
    """
    JUSTIFIED LOGGING TESTS: the operator-facing half of SR-3.7.

    Justified in the sense this file already documents for the cap line: the
    log record IS the contract here, not an implementation detail. The Epic 6
    runbook's decision tree branches on which shed an operator is looking at,
    and the two remedies are opposite -- a class shed at the shed threshold is
    a healthy pod doing what it was configured to do (raise
    ``max_re_routable_streams_per_pod``, or accept it), while a cap shed means
    the pod is at its memory bound (more broker RAM, a lower prefetch, more
    replicas). A refusal an operator cannot attribute sends them to the wrong
    remedy, so the phrase, the level, and the fact that each refusal emits
    exactly ONE record are all pinned.

    This class is the single unconditional owner of the WARNING-versus-shed-
    INFO pairing. ``test_cap_shed_logs_the_cap_line_at_warning_and_says_nothing
    _else`` above retired the second half that used to contrast the cap line
    with non-durable pool exhaustion, because that call now admits and logs
    nothing (SDD SR-3.1 as amended).
    """

    @pytest.fixture
    def at_the_shed_threshold(self, make_pool_stats_by_pool):
        """The limit-8 pool sitting exactly on the shed line of 900."""
        return make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=SHED_THRESHOLD_IN_BAND
        )

    @pytest.fixture
    def at_the_cap(self, make_pool_stats_by_pool):
        """The same pool sitting on the cap of 1000, 100 streams higher."""
        return make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=SHED_BAND_STREAM_CAP
        )

    def test_a_class_shed_emits_only_the_shed_line_at_info(
        self,
        guard,
        mock_connector,
        set_stream_cap,
        set_shed_threshold,
        at_the_shed_threshold,
        caplog,
    ):
        # A class shed is a healthy, configured outcome on a pod that is not
        # near its memory bound, so it is INFO-grade, not a warning -- and it
        # carries all three band numbers, because the operator's next question
        # is whether to move the shed line and they cannot answer it without
        # knowing where the band's ends are.
        set_stream_cap(SHED_BAND_STREAM_CAP)
        set_shed_threshold(SHED_THRESHOLD_IN_BAND)
        mock_connector.pool_stats_by_pool.return_value = at_the_shed_threshold

        with caplog.at_level(logging.INFO, logger=capacity_guard_logger.name):
            guard.has_capacity(re_routable=True)

        assert [(record.levelno, record.getMessage()) for record in caplog.records] == [
            (
                logging.INFO,
                f"Capacity check: shedding re-routable stream at the shed "
                f"threshold (streams: {SHED_THRESHOLD_IN_BAND}, "
                f"shed threshold: {SHED_THRESHOLD_IN_BAND}, "
                f"soft threshold: {MAX_STREAMS_AT_LIMIT_8}, "
                f"hard bound: {SHED_BAND_STREAM_CAP})",
            )
        ]

    def test_a_cap_shed_emits_only_the_cap_line_at_warning(
        self,
        guard,
        mock_connector,
        set_stream_cap,
        set_shed_threshold,
        at_the_cap,
        caplog,
    ):
        # The same re-routable caller, 100 streams higher, with the shed line
        # still configured at 900 and therefore also crossed. The cap check
        # runs first precisely so this reads as a memory event: an operator
        # must see the WARNING, not an INFO that would send them to adjust a
        # threshold that is not what refused.
        set_stream_cap(SHED_BAND_STREAM_CAP)
        set_shed_threshold(SHED_THRESHOLD_IN_BAND)
        mock_connector.pool_stats_by_pool.return_value = at_the_cap

        with caplog.at_level(logging.INFO, logger=capacity_guard_logger.name):
            guard.has_capacity(re_routable=True)

        assert [(record.levelno, record.getMessage()) for record in caplog.records] == [
            (
                logging.WARNING,
                f"Capacity check: memory-safe stream cap reached "
                f"(streams: {SHED_BAND_STREAM_CAP}, cap: {SHED_BAND_STREAM_CAP})",
            )
        ]

    def test_the_two_shed_reasons_do_not_produce_the_same_message(
        self,
        guard,
        mock_connector,
        set_stream_cap,
        set_shed_threshold,
        at_the_shed_threshold,
        at_the_cap,
        caplog,
    ):
        # The property the two tests above cannot state on their own: over one
        # logger, in one test, the two refusals are DISTINGUISHABLE. The levels
        # are already pinned above, so this asserts on the text -- the stricter
        # of the two forms, since a future author who collapses the two call
        # sites into one shared line fails here even if the levels still
        # differ.
        set_stream_cap(SHED_BAND_STREAM_CAP)
        set_shed_threshold(SHED_THRESHOLD_IN_BAND)

        with caplog.at_level(logging.INFO, logger=capacity_guard_logger.name):
            mock_connector.pool_stats_by_pool.return_value = at_the_shed_threshold
            guard.has_capacity(re_routable=True)
            shed_message = caplog.records[-1].getMessage()
            mock_connector.pool_stats_by_pool.return_value = at_the_cap
            guard.has_capacity(re_routable=True)
            cap_message = caplog.records[-1].getMessage()

        assert shed_message != cap_message


class TestShedUnderTheIncomputableCapFallback:
    """
    SDD SR-3.7's documented exception, at the guard layer: under the
    incomputable-cap fallback the refusal is CLASS-BLIND.

    When the prefetch is 0 the per-stream broker footprint is undefined, no cap
    can be computed, and the non-durable pool's own bound becomes the hard
    bound so admission stays bounded by a real number (SR-3.5's always-on
    invariant, satisfied by falling back rather than falling through). The pool
    bound and the soft threshold are the SAME number there, so the overflow
    band is degenerate: there is no room between them to reserve for pinned
    streams, the shed threshold re-anchors onto the hard bound, and both
    classes refuse at that one value. That is the documented exception to PRD
    FR-5, and the price of never running unbounded on a pod whose memory cost
    per stream is unknown.

    Both tests configure an aggressive shed threshold of 400, far below the
    pool bound. If the degenerate band did not re-anchor it, that number would
    refuse re-routable streams 400 deep into a pool that is still admitting
    pinned ones -- so the class-blindness asserted here is a real outcome of
    the re-anchor and not a vacuous one.

    The ``no_stream_memory_cap`` fixture drives the real production route to a
    None cap through the prefetch setting rather than stubbing the cap
    function, which is what makes these pins on the fallback rather than
    restatements of the guard's own branch.
    """

    @pytest.mark.parametrize(
        "total_client_count, expected",
        [
            pytest.param(
                MAX_STREAMS_AT_LIMIT_8 - 1, (True, True), id="799-of-800-streams"
            ),
            pytest.param(
                MAX_STREAMS_AT_LIMIT_8, (False, False), id="800-of-800-streams"
            ),
        ],
    )
    def test_both_classes_receive_the_same_verdict_under_the_fallback(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        no_stream_memory_cap,
        set_shed_threshold,
        total_client_count,
        expected,
    ):
        # Both halves of each pair are written out as literals: below the pool
        # bound both classes are admitted, and AT it both are refused -- a
        # pinned stream included, which is the FR-5 exception. There is no
        # stream count under this fallback at which the two classes disagree,
        # because there is no band for them to disagree in.
        set_shed_threshold(SHED_THRESHOLD_BELOW_THE_POOL_BOUND)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=total_client_count
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert _verdicts_by_stream_class(guard) == expected

    def test_the_fallback_refusal_says_pool_exhausted_and_never_the_shed_line(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        no_stream_memory_cap,
        set_shed_threshold,
        caplog,
    ):
        # The operator-facing half of the same exception, and the reason this
        # is not just a verdict test: a re-routable caller refused at the
        # fallback bound must be told the POOL is exhausted, because the
        # remedy is capacity, not the shed threshold they configured at 400.
        # Emitting the shed line here would point them at a knob that cannot
        # move this refusal. The record list is asserted whole, so "no shed
        # INFO line" is stated rather than inferred.
        #
        # Records are filtered to the guard's own logger: the fallback path
        # necessarily also emits the unbounded-prefetch WARNING from
        # ``stream_memory_cap``, which is that module's contract and is pinned
        # in ``tests/app/lib/rabbitmq/test_stream_memory_cap.py``.
        set_shed_threshold(SHED_THRESHOLD_BELOW_THE_POOL_BOUND)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=MAX_STREAMS_AT_LIMIT_8
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        with caplog.at_level(logging.INFO, logger=capacity_guard_logger.name):
            guard.has_capacity(re_routable=True)

        guard_records = [
            record
            for record in caplog.records
            if record.name == capacity_guard_logger.name
        ]

        assert [(record.levelno, record.getMessage()) for record in guard_records] == [
            (
                logging.INFO,
                f"Capacity check: {NON_DURABLE} pool exhausted "
                f"(streams in use: {MAX_STREAMS_AT_LIMIT_8}, "
                f"max streams: {MAX_STREAMS_AT_LIMIT_8})",
            )
        ]


# --- Shed-threshold clamp arithmetic (Epic 7, Task 2) ------------------------
#
# ``re_routable_shed_threshold()`` on its own: arithmetic over three inputs --
# the soft threshold, the hard bound, and the operator override -- with no
# guard, no pool stats, and no admission verdict in between. The behavioral
# classes above deliberately configure a shed line strictly inside the band so
# the clamp never moves it, and say so; this is the one place the clamp itself
# is pinned, so a formula change lands here and nowhere else.
#
# Every expectation below is a literal reasoned from that row's own three
# inputs. No test here re-evaluates the clamp expression, because a test that
# recomputes a formula agrees with the formula whatever the formula says.
#
# The rows are pure arithmetic and pass their bounds in directly, which is what
# lets them state bands the fixtures cannot express -- notably the inverted one.
# Only the two rows whose INPUT is itself a production number reach for
# plumbing: the incomputable-cap fallback (driven through the real prefetch-0
# route) and the shipped-default pin (driven through the real cap computation).

# The band the in-band rows clamp into. Both ends are the numbers the
# behavioral classes above run on -- the limit-8 pool's 800-stream packing
# product under a hard bound of 1000 -- so the two halves of the tier are
# reasoned about on one band.
CLAMP_SOFT_THRESHOLD = 800
CLAMP_HARD_BOUND = 1000

# Overrides outside that band, one on each side, each far enough out that a
# clamp applied in the wrong direction cannot land on the expected answer by
# coincidence.
OVERRIDE_BELOW_THE_SOFT_THRESHOLD = 500
OVERRIDE_ABOVE_THE_HARD_BOUND = 5000

# Overrides one stream inside each end of the band. They are the rows that
# separate ``<`` from ``<=`` in either comparison: an off-by-one at the bottom
# moves 801 and leaves 999 alone, and one at the top does the reverse.
OVERRIDE_JUST_INSIDE_THE_SOFT_THRESHOLD = 801
OVERRIDE_JUST_INSIDE_THE_HARD_BOUND = 999

# A band with an ODD width (800..1001, 201 wide), used to pin the direction the
# unset-override midpoint rounds. Floor division puts it at 900; rounding up
# would put it at 901.
ODD_WIDTH_BAND_HARD_BOUND = 1001
ODD_WIDTH_BAND_MIDPOINT = 900

# A degenerate band: the soft threshold and the hard bound are the same number,
# reachable whenever the computed cap lands exactly on the packing product.
DEGENERATE_BAND_VALUE = 300

# A WIDTH-1 band: the hard bound is exactly one stream above the soft threshold.
# The floor-division midpoint degenerates here to the soft threshold itself
# (800 + (801 - 800) // 2 = 800 + 0 = 800), so an unset override sheds
# re-routable traffic right at the packing product.
WIDTH_1_BAND_SOFT_THRESHOLD = 800
WIDTH_1_BAND_HARD_BOUND = 801

# An INVERTED band: a soft threshold of 800 ABOVE a hard bound of 118. This is
# the SR-3.5 operator-set-absolute-cap case -- an operator may cap the pod below
# the packing product outright when pod compute rather than broker memory is the
# binding constraint. It is NO LONGER the shipped configuration: at the packed
# defaults the computed cap is 3,342 (prefetch 15), well ABOVE the 800 packing
# product, so the shipped band is not inverted. The synthetic 800/118 numbers
# are kept because the resolver's inverted-band behavior still needs covering.
INVERTED_BAND_SOFT_THRESHOLD = 800
INVERTED_BAND_HARD_BOUND = 118

# The non-durable pool bound under the incomputable-cap fallback, where it is
# both the soft threshold and the hard bound.
FALLBACK_POOL_BOUND = MAX_STREAMS_AT_LIMIT_8

# Overrides spanning that fallback bound -- under it, on it, and over it.
OVERRIDE_UNDER_THE_FALLBACK_BOUND = 400
OVERRIDE_OVER_THE_FALLBACK_BOUND = 1600

# The shipped configuration's band and the shed line the shipped default puts
# in it. The soft threshold is the shipped PACKED packing product
# (``rabbitmq_channel_pool_size`` 100 x the flipped ``rabbitmq_channel_client_
# limit`` 8 = 800), the hard bound is the cap computed at the shipped budget,
# prefetch 15 and replica count -- 3,342, the number pinned in
# ``test_stream_memory_cap.py`` -- and 2,071 is the midpoint of 800..3,342 that
# an unset override lands on. Hand-check (not computed in code):
# 800 + (3342 - 800) // 2 = 800 + 1271 = 2071. That 2,071 is the number Task 6's
# operator docs and the Epic 6 runbook quote (the runbook currently quotes the
# pre-flip 109), so it is written here as a literal rather than derived.
SHIPPED_SOFT_THRESHOLD = 800
SHIPPED_SHED_THRESHOLD = 2071


class TestReRoutableShedThreshold:
    """
    The shed threshold's own arithmetic: the operator override clamped into
    ``[soft threshold, hard bound]``, and the default that sits in the middle.

    Neither end of the band is knowable when settings are validated -- the soft
    threshold comes from live pool stats and the hard bound from the prefetch,
    budget and replica settings -- so an out-of-band value is ACCEPTED and moved
    at admission rather than rejected at startup (SDD SR-3.7). That makes the
    clamp the only thing standing between a plausible-looking configmap line and
    a shed line that refuses traffic the pod would otherwise have carried, which
    is why it is pinned at both ends, inside, and in both collapsed bands.

    Two of the clamped results are load-bearing postures rather than mere
    corrections:

    - ``shed == soft threshold`` is the floor. The shed tier lives in the
      OVERFLOW band; a shed line under the packing product would refuse
      re-routable streams the pod admitted happily before the band existed,
      turning an availability feature into a regression.
    - ``shed == hard bound`` is a supported configuration, not an error
      (SR-3.7 audit addition (c)): it disables the shed tier for a fleet with
      nowhere to re-route to -- a single-replica deployment, or one that is
      uniformly hot -- and every class then refuses together at the cap.
    """

    @pytest.fixture
    def fallback_hard_bound(self, no_stream_memory_cap):
        """
        The hard bound the guard hands the resolver when the cap is incomputable.

        Produced the way production produces it: ``no_stream_memory_cap``
        configures the prefetch of 0 that leaves the per-stream footprint
        undefined, ``memory_safe_stream_cap()`` really returns None over that
        configuration, and the non-durable pool's own bound takes the cap's
        place (SDD SR-3.7 audit addition (b), Epic audit note 4). A stubbed
        None would prove nothing about the route that produces it, so the
        precondition is asserted here rather than assumed.

        :returns: The pool bound, standing in for the cap.
        """
        assert stream_memory_cap.memory_safe_stream_cap() is None

        return FALLBACK_POOL_BOUND

    @pytest.fixture
    def shipped_hard_bound(self, mocker):
        """
        The hard bound at the shipped configuration, with BOTH operator
        overrides cleared and the computed-cap inputs pinned to their DECLARED
        defaults.

        Clearing ``max_non_durable_streams_per_pod`` undoes the autouse
        neutralizer in ``tests/app/conftest.py`` so the cap is really computed
        from the shipped budget, prefetch and replica count instead of being
        read back off a fixture; clearing ``max_re_routable_streams_per_pod``
        is what puts the resolver on its default branch at all. Without both,
        the shipped-default pin would assert on fixture values and could not
        catch the regression it exists to catch.

        The three computed-cap inputs (prefetch, budget, replica divisor) are
        additionally patched to their DECLARED defaults, read from
        ``model_fields`` rather than the resolved settings: an untracked
        developer ``.env`` can set any of them (the hazard
        ``test_stream_memory_cap.py`` documents), which would make this pin
        machine-dependent. The cap is still genuinely COMPUTED -- from pinned
        inputs, not stubbed.

        :returns: The computed memory-safe stream cap at shipped defaults.
        """
        mocker.patch.object(settings.settings, "max_re_routable_streams_per_pod", None)
        mocker.patch.object(settings.settings, "max_non_durable_streams_per_pod", None)
        mocker.patch.object(
            common_settings,
            "rabbitmq_channel_prefetch_count",
            CommonSettings.model_fields["rabbitmq_channel_prefetch_count"].default,
        )
        mocker.patch.object(
            settings.settings,
            "broker_stream_memory_budget_mib",
            settings.Settings.model_fields["broker_stream_memory_budget_mib"].default,
        )
        mocker.patch.object(
            settings.settings,
            "scaler_max_replicas",
            settings.Settings.model_fields["scaler_max_replicas"].default,
        )

        return stream_memory_cap.memory_safe_stream_cap()

    def test_an_override_below_the_soft_threshold_resolves_up_to_it(
        self, set_shed_threshold
    ):
        # A shed line of 500 under a packing product of 800 would shed
        # re-routable streams 300 deep inside the density the pool is
        # PROVISIONED to pack at -- traffic the pod admitted before the
        # overflow band existed. The shed tier is a rule about the band, so the
        # floor is the band's own bottom: 800.
        set_shed_threshold(OVERRIDE_BELOW_THE_SOFT_THRESHOLD)

        assert re_routable_shed_threshold(CLAMP_SOFT_THRESHOLD, CLAMP_HARD_BOUND) == 800

    def test_an_override_above_the_hard_bound_resolves_down_to_it(
        self, set_shed_threshold
    ):
        # A shed line of 5000 over a hard bound of 1000 is unreachable: every
        # class already refuses at 1000, so nothing would ever be measured
        # against it. Rather than leave a dead number configured, the clamp
        # resolves it onto the bound, which is the same value an operator gets
        # by asking for the shed tier off -- a supported posture per SR-3.7
        # audit addition (c), and therefore a resolved value rather than an
        # error.
        set_shed_threshold(OVERRIDE_ABOVE_THE_HARD_BOUND)

        assert (
            re_routable_shed_threshold(CLAMP_SOFT_THRESHOLD, CLAMP_HARD_BOUND) == 1000
        )

    @pytest.mark.parametrize(
        "override, expected",
        [
            pytest.param(
                OVERRIDE_JUST_INSIDE_THE_SOFT_THRESHOLD, 801, id="801-just-above-800"
            ),
            pytest.param(
                OVERRIDE_JUST_INSIDE_THE_HARD_BOUND, 999, id="999-just-below-1000"
            ),
        ],
    )
    def test_an_override_inside_the_band_passes_through_untouched(
        self, set_shed_threshold, override, expected
    ):
        # Both ends of the open interval, one stream in from each. An off-by-one
        # in the lower comparison moves the 801 row and leaves 999 alone; one in
        # the upper comparison does the reverse -- so exactly one row flips,
        # which names the broken end.
        set_shed_threshold(override)

        assert (
            re_routable_shed_threshold(CLAMP_SOFT_THRESHOLD, CLAMP_HARD_BOUND)
            == expected
        )

    def test_an_unset_override_lands_on_the_midpoint_without_rounding_up(
        self, set_shed_threshold
    ):
        # Unset means "split the band evenly between the two classes". Over the
        # 800..1001 band, 201 wide, the exact midpoint is 900.5, and the answer
        # is 900: the midpoint floors, so it can never step past the hard bound
        # on an odd-width band and turn the shed tier into a second hard
        # refusal.
        set_shed_threshold(None)

        assert (
            re_routable_shed_threshold(CLAMP_SOFT_THRESHOLD, ODD_WIDTH_BAND_HARD_BOUND)
            == ODD_WIDTH_BAND_MIDPOINT
        )

    def test_a_width_1_band_sheds_at_the_soft_threshold_when_unset(
        self, set_shed_threshold
    ):
        # The operationally load-bearing case the settings description now warns
        # about (see max_re_routable_streams_per_pod's Field description): on a
        # band one stream wide the floor-division midpoint is the soft threshold
        # itself, so an operator who sets nothing sheds re-routable traffic right
        # at the packing product -- the shed tier is effectively at the soft
        # threshold. 800 + (801 - 800) // 2 = 800.
        set_shed_threshold(None)

        assert (
            re_routable_shed_threshold(
                WIDTH_1_BAND_SOFT_THRESHOLD, WIDTH_1_BAND_HARD_BOUND
            )
            == WIDTH_1_BAND_SOFT_THRESHOLD
        )

    @pytest.mark.parametrize(
        "override",
        [
            pytest.param(None, id="unset"),
            pytest.param(100, id="below-the-band"),
            pytest.param(DEGENERATE_BAND_VALUE, id="on-the-band"),
            pytest.param(900, id="above-the-band"),
        ],
    )
    def test_a_degenerate_band_collapses_every_override_onto_its_one_value(
        self, set_shed_threshold, override
    ):
        # When the computed cap lands exactly on the packing product there is no
        # overflow band to reserve, so there is nothing for the shed line to be
        # except that single value -- for every configured input, the unset one
        # included. The unset row is the one that matters most: a midpoint taken
        # over a zero-width band is 300 too, but only because the collapse
        # happens BEFORE the override is ever read.
        set_shed_threshold(override)

        assert (
            re_routable_shed_threshold(DEGENERATE_BAND_VALUE, DEGENERATE_BAND_VALUE)
            == 300
        )

    @pytest.mark.parametrize(
        "override",
        [
            pytest.param(None, id="unset"),
            pytest.param(50, id="below-the-hard-bound"),
            pytest.param(400, id="between-the-hard-bound-and-the-soft-threshold"),
            pytest.param(900, id="above-the-soft-threshold"),
        ],
    )
    def test_an_inverted_band_resolves_every_override_to_the_hard_bound(
        self, set_shed_threshold, override
    ):
        # The ordering pathology: a soft threshold of 800 ABOVE a hard bound of
        # 118 (an operator's absolute cap set below the packing product because
        # pod compute binds first -- NOT the shipped configuration, whose packed
        # cap of 3,342 sits above the 800 product and does not invert).
        # Every row resolves to 118, which is both halves of what this case
        # needs: deterministic, and never ABOVE the hard bound. The unset row is
        # the trap -- a midpoint over an inverted band takes a negative width,
        # 800 + (118 - 800) // 2 = 459, a shed line 341 streams past a bound at
        # which everything already refuses.
        set_shed_threshold(override)

        assert (
            re_routable_shed_threshold(
                INVERTED_BAND_SOFT_THRESHOLD, INVERTED_BAND_HARD_BOUND
            )
            == 118
        )

    @pytest.mark.parametrize(
        "override",
        [
            pytest.param(OVERRIDE_UNDER_THE_FALLBACK_BOUND, id="400-under-the-bound"),
            pytest.param(FALLBACK_POOL_BOUND, id="800-on-the-bound"),
            pytest.param(OVERRIDE_OVER_THE_FALLBACK_BOUND, id="1600-over-the-bound"),
        ],
    )
    def test_the_incomputable_cap_re_anchors_the_clamp_onto_the_pool_bound(
        self, set_shed_threshold, fallback_hard_bound, override
    ):
        # With no computable cap the pool bound takes the cap's place as the
        # clamp's upper end -- and it is ALSO the soft threshold, so the band is
        # degenerate and every configured override, wherever it sits, resolves
        # to that one number. There is deliberately no "passes through" row
        # here: the interval to pass through in is empty.
        #
        # That is the arithmetic half of the class-blind fallback refusal (PRD
        # FR-5's documented exception): the class shed is structurally
        # unreachable on this path, so a 400 an operator configured cannot bite
        # 400 streams into a pool that is still admitting pinned traffic.
        set_shed_threshold(override)

        assert (
            re_routable_shed_threshold(FALLBACK_POOL_BOUND, fallback_hard_bound) == 800
        )

    def test_the_shipped_default_resolves_to_the_shipped_shed_line(
        self, shipped_hard_bound
    ):
        # The pin on the shipped number, in the same spirit as the shipped-cap
        # pin of 118 in ``test_stream_memory_cap.py``: at the shipped packing
        # product of 100 streams (pool size 100 x client limit 1) under the
        # shipped computed cap, an operator who configures nothing gets a shed
        # line of 109. Both inputs are real -- the cap is computed here, not
        # patched -- so a change to the budget, the prefetch, the replica
        # divisor, or the midpoint rule moves this number and fails here, which
        # is what stops the docs and the runbook quoting a shed line the code no
        # longer produces.
        assert (
            re_routable_shed_threshold(SHIPPED_SOFT_THRESHOLD, shipped_hard_bound)
            == SHIPPED_SHED_THRESHOLD
        )


# --- Draining refusal (Epic 5, Task 2 / SR-6.1) ------------------------------
#
# A draining pod refuses every NEW non-durable stream, CLASS-BLIND, ahead of
# the soft/shed/hard tiers and irrespective of ``re_routable``: the pod is
# going away, so there is no overflow band to absorb a pinned stream into and
# both classes are refused. The signal is ``connector.draining`` (Task 1: a
# plain bool on ``RabbitMQConnector``, True from ``begin_draining()`` onward and
# independent of ``closing``), consumed through that exact surface -- the same
# attribute the shared ``mock_connector`` fixture defaults to False. The
# refusal rides the existing ``ChannelNotAvailableError`` (503 / gRPC
# RESOURCE_EXHAUSTED), so only its TRIGGER gains a draining case; the contract
# shape is unchanged. The short-circuit lives INSIDE the non-durable branch, so
# the durable path is byte-for-byte unchanged and still admits while draining.
#
# The autouse ``non_binding_stream_memory_cap`` fixture parks the cap and, via
# the clamp, the shed threshold far out of reach, so every pool below admits on
# all three tiers when NOT draining. That is what makes the draining refusal
# provably the SIGNAL talking and not a tier -- the anti-vacuity rows run the
# same stats with ``draining`` False and get the opposite verdict. This axis is
# orthogonal to the tier tables above, so those are not re-covered here.


class TestDrainingRefusesAdmission:
    """
    SR-6.1 at the guard layer: the draining signal refuses NEW non-durable
    streams, class-blind, and leaves the durable path and the non-draining
    tiers untouched.
    """

    @pytest.mark.parametrize(
        "client_limit, total_client_count",
        [
            pytest.param(1, 0, id="limit-1-idle"),
            pytest.param(8, 0, id="limit-8-idle"),
            pytest.param(8, MAX_STREAMS_AT_LIMIT_8, id="limit-8-at-soft-threshold"),
        ],
    )
    def test_draining_refuses_both_classes_over_an_otherwise_admitting_pool(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        client_limit,
        total_client_count,
    ):
        # Across two pool sizings (limit-1 and packed limit-8) and both stream
        # classes: a pool that admits on every tier when not draining -- idle,
        # or sitting on its report-only soft threshold, both well below the
        # parked cap and shed line -- is refused for the re-routable AND the
        # pinned (default-arg) call once draining is engaged. The (False, False)
        # tuple is the class-blindness: the verdict cannot be a tier's, because
        # the identical stats admit both classes when not draining (pinned by
        # ``test_the_same_pool_admits_both_classes_when_not_draining`` below).
        mock_connector.draining = True
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=client_limit,
            non_durable_clients=total_client_count,
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert _verdicts_by_stream_class(guard) == (False, False)

    @pytest.mark.parametrize(
        "re_routable", [True, False], ids=["re-routable", "pinned"]
    )
    def test_draining_raises_channel_not_available_for_both_classes(
        self, guard, mock_connector, make_pool_stats_by_pool, re_routable
    ):
        # The draining refusal reaches the client as the SAME 503 /
        # RESOURCE_EXHAUSTED contract as pool exhaustion, exact type asserted
        # for both classes -- draining adds a trigger, not a new error shape. A
        # draining-specific subclass would be an API-visible change this Task
        # did not authorize.
        mock_connector.draining = True
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=0
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        with pytest.raises(ChannelNotAvailableError) as exc_info:
            guard.raise_if_at_capacity(re_routable=re_routable)

        assert type(exc_info.value) is ChannelNotAvailableError

    def test_the_same_pool_admits_both_classes_when_not_draining(
        self, guard, mock_connector, make_pool_stats_by_pool
    ):
        # Anti-vacuity for the refusal rows above. The SAME otherwise-admitting
        # pool, with draining NOT engaged (the shared fixture's default),
        # admits both classes and raises nothing -- so the (False, False)
        # verdict above is the draining signal talking and not a pool that was
        # refusing anyway. This is also the row that would fail if the shared
        # not-draining default were ever dropped.
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=0
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert _verdicts_by_stream_class(guard) == (True, True)
        assert guard.raise_if_at_capacity() is None
        assert guard.raise_if_at_capacity(re_routable=True) is None

    def test_not_draining_still_admits_at_the_soft_threshold(
        self, guard, mock_connector, make_pool_stats_by_pool
    ):
        # Tier-independence, admitting side: the draining short-circuit must not
        # perturb the non-draining tiers. With draining not engaged, the
        # non-durable surface still admits at its report-only soft threshold
        # (SDD SR-3.1 as amended) exactly as ``TestHasCapacityStreamAdmission``
        # pins it -- one row is enough here, since the full tables live above.
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=MAX_STREAMS_AT_LIMIT_8
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is True

    def test_not_draining_still_refuses_at_the_hard_cap(
        self, guard, mock_connector, make_pool_stats_by_pool, set_stream_cap
    ):
        # Tier-independence, refusing side: with draining not engaged the
        # memory-safe cap still refuses at its bound, so the short-circuit did
        # not swallow the hard tier. The pool is 800 streams wide and the cap
        # binds at 50, so only the cap can explain the refusal -- the same
        # driver ``TestMemorySafeStreamCap`` uses.
        set_stream_cap(BINDING_STREAM_CAP)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=BINDING_STREAM_CAP
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity() is False

    def test_draining_does_not_refuse_durable_admission(
        self, guard, mock_connector, make_pool_stats_by_pool
    ):
        # DURABLE-PATH PIN: the draining short-circuit lives inside the
        # non-durable branch (``if not durable_pool:``), so the durable path is
        # byte-for-byte unchanged and keeps admitting while the connector is
        # draining. Durable queues are long-lived and not the stop-new-arrivals
        # concern SR-6.1 addresses. ``re_routable=True`` is passed to prove the
        # durable path ignores both the class and the draining signal, and the
        # refusal that would fire on the non-durable surface for these same
        # engaged-draining stats is exactly what this asserts does NOT happen
        # here. ``raise_if_at_capacity(durable_pool=True)`` returning None is
        # the same statement in the raising contract.
        mock_connector.draining = True
        stats = make_pool_stats_by_pool(durable_clients=0)
        mock_connector.pool_stats_by_pool.return_value = stats

        assert guard.has_capacity(durable_pool=True, re_routable=True) is True
        assert guard.has_capacity(durable_pool=True) is True
        assert guard.raise_if_at_capacity(durable_pool=True) is None


# --- Admission telemetry wiring (Epic 7 / b.3c9) -----------------------------
#
# The guard records one admission-counter increment per admission DECISION,
# beside the same branch that emits the operator log line: a shed on each
# refusal (labelled with the decision's ``reason`` and, for the class-aware
# shed, the stream ``class``) and an admitted on the success path (labelled by
# class). The counter arithmetic and its gating on ``metrics_enabled`` belong to
# ``AdmissionMetrics``'s own unit tests
# (``tests/app/lib/observability/test_admission_metrics.py``); everything below
# pins only that the GUARD calls the right method, at the right site, with the
# right labels -- and that it calls exactly one of them per decision.
#
# The two methods are spied through the module-level ``admission_metrics``
# singleton the guard imports, so a call the guard makes is seen here. The log
# fingerprints beside these calls are pinned unchanged by
# ``TestHasCapacityExhaustionLog``, ``TestShedLogsDistinctlyFromTheCap``, and
# the cap/fallback log tests above -- those assert the emitted record list whole
# and would fail if a metrics call ever leaked a log line, so recording adds
# telemetry without touching the operator-facing text.


@pytest.fixture
def admission_spy(mocker):
    """
    Spy on the guard's admission counters through the shared singleton.

    :returns: ``(record_admitted_mock, record_shed_mock)``.
    """
    admitted = mocker.patch.object(admission_metrics, "record_admitted")
    shed = mocker.patch.object(admission_metrics, "record_shed")
    return admitted, shed


class TestAdmissionMetricsShedWiring:
    """Each non-durable refusal records exactly one shed with its own reason."""

    def test_memory_cap_refusal_records_a_memory_cap_shed(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        set_stream_cap,
        admission_spy,
    ):
        admitted, shed = admission_spy
        set_stream_cap(BINDING_STREAM_CAP)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=BINDING_STREAM_CAP
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        guard.has_capacity()

        shed.assert_called_once_with(reason="memory_cap")
        admitted.assert_not_called()

    def test_pool_exhausted_fallback_records_a_pool_exhausted_shed(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        no_stream_memory_cap,
        admission_spy,
    ):
        # Under the incomputable-cap fallback the pool bound is the hard bound;
        # a refusal there is a pool-exhaustion shed, class-blind.
        admitted, shed = admission_spy
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=MAX_STREAMS_AT_LIMIT_8
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        guard.has_capacity()

        shed.assert_called_once_with(reason="pool_exhausted")
        admitted.assert_not_called()

    def test_class_shed_records_a_shed_threshold_re_routable_shed(
        self,
        guard,
        mock_connector,
        make_pool_stats_by_pool,
        set_stream_cap,
        set_shed_threshold,
        admission_spy,
    ):
        admitted, shed = admission_spy
        set_stream_cap(SHED_BAND_STREAM_CAP)
        set_shed_threshold(SHED_THRESHOLD_IN_BAND)
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=SHED_THRESHOLD_IN_BAND
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        guard.has_capacity(re_routable=True)

        shed.assert_called_once_with(
            reason="shed_threshold", stream_class="re_routable"
        )
        admitted.assert_not_called()

    def test_draining_refusal_records_a_draining_shed(
        self, guard, mock_connector, make_pool_stats_by_pool, admission_spy
    ):
        admitted, shed = admission_spy
        mock_connector.draining = True
        stats = make_pool_stats_by_pool(
            non_durable_client_limit=8, non_durable_clients=0
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        guard.has_capacity()

        shed.assert_called_once_with(reason="draining")
        admitted.assert_not_called()


class TestAdmissionMetricsAdmittedWiring:
    """The non-durable success path records one admitted, labelled by class."""

    def test_admits_a_re_routable_stream_as_re_routable(
        self, guard, mock_connector, pool_stats_by_pool, admission_spy
    ):
        admitted, shed = admission_spy
        mock_connector.pool_stats_by_pool.return_value = pool_stats_by_pool

        guard.has_capacity(re_routable=True)

        admitted.assert_called_once_with(True)
        shed.assert_not_called()

    def test_admits_an_unclassified_stream_as_pinned(
        self, guard, mock_connector, pool_stats_by_pool, admission_spy
    ):
        # A bare call defaults to pinned, so the class label follows the guard's
        # own default rather than being asserted independently of it.
        admitted, shed = admission_spy
        mock_connector.pool_stats_by_pool.return_value = pool_stats_by_pool

        guard.has_capacity()

        admitted.assert_called_once_with(False)
        shed.assert_not_called()

    def test_one_stream_open_records_exactly_one_admission(
        self, guard, mock_connector, pool_stats_by_pool, admission_spy
    ):
        # ``raise_if_at_capacity`` is the once-per-stream-open entry point and
        # calls ``has_capacity`` once, so a single stream open increments the
        # counter exactly once -- nothing rides the per-message path.
        admitted, shed = admission_spy
        mock_connector.pool_stats_by_pool.return_value = pool_stats_by_pool

        guard.raise_if_at_capacity()

        assert admitted.call_count == 1
        shed.assert_not_called()


class TestAdmissionMetricsScope:
    """Admission counters are non-durable-only and never fire before the tiers."""

    def test_durable_admission_records_no_admission_metric(
        self, guard, mock_connector, pool_stats_by_pool, admission_spy
    ):
        admitted, shed = admission_spy
        mock_connector.pool_stats_by_pool.return_value = pool_stats_by_pool

        guard.has_capacity(durable_pool=True)

        admitted.assert_not_called()
        shed.assert_not_called()

    def test_durable_refusal_records_no_admission_metric(
        self, guard, mock_connector, make_pool_stats_by_pool, admission_spy
    ):
        admitted, shed = admission_spy
        stats = make_pool_stats_by_pool(
            non_durable_clients=0, durable_clients=MAX_STREAMS_AT_LIMIT_1
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        guard.has_capacity(durable_pool=True)

        admitted.assert_not_called()
        shed.assert_not_called()

    def test_lifecycle_refusal_records_no_admission_metric(
        self, guard, mock_connector, make_pool_stats_by_pool, admission_spy
    ):
        # A pool that fails the lifecycle checks is refused ahead of the
        # admission tiers, so neither counter fires -- the counters live only in
        # the tier logic they instrument.
        admitted, shed = admission_spy
        stats = _with_pool_state(
            make_pool_stats_by_pool(), NON_DURABLE, "not-initialized"
        )
        mock_connector.pool_stats_by_pool.return_value = stats

        guard.has_capacity()

        admitted.assert_not_called()
        shed.assert_not_called()
