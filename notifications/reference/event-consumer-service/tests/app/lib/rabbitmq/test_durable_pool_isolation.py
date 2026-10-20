# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Isolation between the durable and non-durable channel pools.

This file carries the Epic acceptance criterion for the pool split (SDD
SR-2.1 / SR-2.2): a durable checkout's ``set_qos`` can never affect a channel
the non-durable pool hands out. The durable consume paths and the DLQ browse
path each lower a pooled channel's prefetch per checkout and never reset it on
return, so before the split a non-durable borrower could inherit a durable
borrower's prefetch as soon as the non-durable ``client_limit`` rose above 1.
The structural fix is that the two paths draw from two different pools, and
this file is where that is proven end to end.

Unlike the per-call-site tests -- which assert a call site passes
``durable_pool=True`` against a *mocked* connector -- every test here runs two
REAL ``ChannelPool`` instances over a mocked connection, so the property is
proven by channel identity and recorded ``set_qos`` calls rather than by a
kwarg spy.

The module-level ``mock_conn`` override below is load-bearing: the root
``mock_conn`` (tests/conftest.py) resolves ``connection.channel()`` to one
shared channel object, under which both pools would alias the same channel and
disjointness would be untestable. The override hands out a fresh channel per
call, matching notification_common's ``mock_channel_factory`` idiom.

As of Epic 3 (t1.tj2.xn) this file runs at the shipped PACKED density by
default: the non-durable ``client_limit`` ships at 8 and nothing here patches
it, so every isolation assertion now exercises genuinely SHARED non-durable
channels -- the configuration under which an inherited durable prefetch is
actually observable. Task 2 (t3.tj2.xn.n9.nn) RE-BASED the file onto those
defaults; Task 4 (t3.tj2.xn.ax.mh) then EXTENDED it with the AC-8-under-packing
coverage that only becomes testable at ``client_limit > 1``. That extension is
``TestDurablePathUnderPackedNonDurableLoad`` below: it proves durable prefetch
(byte-for-byte), durable delivery, and durable queue CRUD are all unaffected
while the non-durable pool is genuinely PACKED (several borrowers per channel,
loaded to the shipped client_limit of 8 and into the SR-1.5 overflow band),
including one arm that runs both paths CONCURRENTLY via ``asyncio.gather``.
"""

import asyncio
from collections import Counter
from unittest.mock import AsyncMock, MagicMock

import pytest
import pytest_asyncio
from notification_common.config.common_settings import common_settings
from notification_common.errors.errors import ChannelNotAvailableError
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings as _settings_module
from app.lib.rabbitmq.capacity_guard import CapacityGuard
from app.lib.rabbitmq.dead_letter_consumer import DeadLetterConsumer
from app.lib.rabbitmq.durable_queue_consumer import DurableQueueConsumer
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase
from tests.app.lib.rabbitmq.test_rabbitmq_base import MockMessage
from tests.fixtures.mock_filter_group import filter_group

# The three prefetch values this file distinguishes between. They are
# deliberately distinct so an assertion about one is unambiguous: a channel
# carrying DURABLE_PREFETCH or DLQ_BROWSE_PREFETCH cannot have got there via
# pool initialization, which always applies POOL_INIT_PREFETCH. After Epic 3's
# flip the three values are 15 (pool init), 20 (durable) and 10 (DLQ browse) --
# still pairwise distinct, so every _channels_carrying(...) / _prefetch_calls
# (...) assertion keeps its meaning and the :297 constants guard keeps its
# point. The margin is now narrow, though: 15 / 20 / 10 sit within a factor of
# two of each other, where it used to be 500 against 20 and 10, so a future
# prefetch-tuning change is materially more likely to collide them -- which is
# precisely what that guard exists to catch.
DURABLE_PREFETCH = 20  # settings.durable_consumer_prefetch_count
DLQ_BROWSE_PREFETCH = 10  # settings.dead_letter_browse_prefetch_count
POOL_INIT_PREFETCH = 15  # common_settings.rabbitmq_channel_prefetch_count

# Both pools are shrunk to these limits so "take every channel slot the pool is
# sized for" is a handful of checkouts rather than 100.
NON_DURABLE_POOL_SIZE = 4
NON_DURABLE_MIN_SIZE = 1
DURABLE_POOL_SIZE = 4
DURABLE_MIN_SIZE = 0

# The non-durable pool's SOFT THRESHOLD (packing product): NON_DURABLE_POOL_SIZE
# 4 channels x the shipped rabbitmq_channel_client_limit of 8 = 32 stream slots.
# The pool packs up to this density before entering the SR-1.5 overflow band.
NON_DURABLE_SOFT_THRESHOLD = 32

# The non-durable client limit used to prove isolation at a packing density
# deliberately DIFFERENT from both 1 and the shipped 8, so
# test_isolation_holds_when_the_non_durable_client_limit_is_above_one proves the
# property at an arbitrary shared density rather than only at whatever ships.
NON_DEFAULT_CLIENT_LIMIT = 3

# A memory-safe stream cap deliberately set ABOVE the non-durable pool's soft
# threshold (NON_DURABLE_POOL_SIZE x the shipped client_limit of 8 = 32), so the
# gap between the two IS the overflow band. A pod driven to this number has
# packed two streams past the threshold and is at the hard bound, which is the
# one tier that refuses a pinned stream. Used by the single test that opts out
# of the root autouse non_binding_stream_memory_cap fixture.
BINDING_STREAM_CAP = 34

# The fixed message set a delivering fake queue feeds a durable consumer, used
# by the AC-8 delivery arm. Distinct, deterministic bodies with a monotonic
# ``seq`` so the delivered EventMessage sequence is identical between a baseline
# run and a run under packed non-durable load -- which is exactly the
# "byte-for-byte unaffected" claim the arm compares. The delivery tags the fake
# stamps (1..N) flow through AckTracker.track into each yielded
# EventMessage.delivery_tag, so the comparison covers tags as well as bodies.
_DELIVERED_BODIES = [
    b'{"event_type": "e1", "seq": 1}',
    b'{"event_type": "e1", "seq": 2}',
    b'{"event_type": "e2", "seq": 3}',
]
DELIVERED_MESSAGE_COUNT = len(_DELIVERED_BODIES)

# A hard ceiling on any durable drain in this file so a mis-wired fake fails
# fast at ``messages_queue.get()`` instead of hanging the suite.
DURABLE_DRAIN_TIMEOUT_S = 5


def _make_delivering_queue():
    """
    Build a fake queue whose ``consume(callback)`` feeds ``_DELIVERED_BODIES``.

    Mirrors the ``mock_queue`` idiom in ``tests/app/lib/rabbitmq/conftest.py``
    (the :277-283 block): ``consume`` invokes the production callback once per
    body -- populating the consumer's internal ``asyncio.Queue`` -- and returns
    a consumer tag. The other queue methods the durable CRUD paths reach
    (``bind`` / ``unbind`` / ``delete`` / ``cancel``) are the ``AsyncMock``
    attributes ``AsyncMock()`` supplies for free, so a ``create_queue`` /
    ``update_queue`` / ``delete_queue`` runs to completion against it.
    """
    queue = AsyncMock()
    queue.name = "durable-queue-under-test"

    async def _consume(callback, **_kwargs):
        for index, body in enumerate(_DELIVERED_BODIES):
            await callback(MockMessage(body, delivery_tag=index + 1))
        return "consumer-tag"

    queue.consume = _consume
    return queue


@pytest.fixture
def mock_channel_factory():
    """
    Build a fresh mock channel per call, as a real connection would.

    Each channel's ``declare_queue`` returns a fresh DELIVERING queue (see
    ``_make_delivering_queue``) rather than a bare ``AsyncMock``, so the AC-8
    delivery and queue-CRUD arms can drive the real durable consume / CRUD paths
    -- both of which reach a queue via ``declare_queue`` -- to completion. This
    is inert for every pre-existing test: those never iterate a durable
    generator (so ``consume`` is never called) and assert only on ``set_qos``
    call lists and channel identity, neither of which this touches.
    """

    def create_mock_channel():
        channel = AsyncMock()
        channel.is_closed = False
        channel.set_qos = AsyncMock()
        channel.close = AsyncMock()
        channel.declare_queue = AsyncMock(
            side_effect=lambda *_args, **_kwargs: _make_delivering_queue()
        )
        return channel

    return create_mock_channel


@pytest_asyncio.fixture
async def mock_conn(mock_channel_factory):
    """
    Override the root ``mock_conn`` with a fresh-channel-per-call connection.

    pytest resolves the nearest fixture definition, so the root autouse
    ``before_each`` -- which patches ``aio_pika.connect_robust`` to return
    ``mock_conn`` -- picks this up for this module alone; the root fixture is
    untouched for every other test file.
    """
    conn = MagicMock()
    conn.is_closed = False
    conn.channel = AsyncMock(side_effect=mock_channel_factory)
    conn.close = AsyncMock()
    return conn


@pytest.fixture
def small_pools(mocker):
    """
    Shrink both pools to a size a test can exhaust cheaply.

    Both pools resolve their limits late, from ``common_settings``, via
    ``ChannelPoolConfig`` / ``DurableChannelPoolConfig``, so patching these
    attributes is the supported knob for sizing them. ``min_size`` is the one
    limit read once, at ``initialize()``, so this must be in place before
    ``connect()``.
    """
    mocker.patch.object(
        common_settings, "rabbitmq_channel_pool_size", NON_DURABLE_POOL_SIZE
    )
    mocker.patch.object(
        common_settings, "rabbitmq_channel_pool_min_size", NON_DURABLE_MIN_SIZE
    )
    mocker.patch.object(
        common_settings, "rabbitmq_durable_channel_pool_size", DURABLE_POOL_SIZE
    )
    mocker.patch.object(
        common_settings, "rabbitmq_durable_channel_pool_min_size", DURABLE_MIN_SIZE
    )


@pytest.fixture
def non_default_non_durable_client_limit(mocker, small_pools):
    """
    Pin a non-durable client limit distinct from both 1 and the shipped 8.

    Its channels are SHARED (limit > 1), which is the configuration under which
    an un-reset durable prefetch would actually be inherited. The value is
    deliberately neither 1 nor the file's shipped default of 8, so the test it
    feeds proves isolation at an arbitrary packing density rather than only at
    whatever ships.
    """
    mocker.patch.object(
        common_settings, "rabbitmq_channel_client_limit", NON_DEFAULT_CLIENT_LIMIT
    )


@pytest_asyncio.fixture
async def connected_connector(small_pools):
    """The live connector singleton, with both real pools initialized."""
    await RabbitMQBase.connector.connect()
    return RabbitMQBase.connector


@pytest.fixture
def real_pool_capacity_guard(connected_connector):
    """
    A ``CapacityGuard`` reading the two REAL pools of ``connected_connector``.

    Everywhere else in the suite the guard is fed a synthetic stats dict, so
    its verdict is only ever as true as the dict. Built over the connector the
    production singleton is also built over, this is the one place a capacity
    verdict is taken from what two real ``ChannelPool`` instances report.
    """
    return CapacityGuard(connector=connected_connector)


@pytest_asyncio.fixture
async def live_durable_checkout(connected_connector, mock_queue_id):
    """
    A durable-pool checkout held open, taken through the production path.

    Drives the real ``DurableQueueConsumer.consume_with_acks()``, which checks
    a channel out of the durable pool and applies ``DURABLE_PREFETCH`` to it.
    The returned generator is never iterated, so the checkout stays held for
    the whole test -- the state a mid-stream durable consumer is in.

    Yields the checked-out channel.
    """
    generator, _ack_tracker = await DurableQueueConsumer.consume_with_acks(
        mock_queue_id
    )

    yield _only_durable_channel(connected_connector)

    await generator.aclose()


def _only_durable_channel(connector):
    """
    The single channel the durable pool has opened.

    Reaches into ``_all_channels`` because channel *ownership* is not
    observable through any public surface -- ``pool_stats()`` reports counts,
    not identities -- and identity is the whole property under test. The
    durable pool starts at zero channels (``min_size`` 0, and this directory's
    autouse ``no_op_dead_letter_topology`` stubs the connect-time DLQ
    declare), so exactly one channel exists once a single checkout has been
    taken.
    """
    channels = list(connector._durable_channel_pool._all_channels)
    assert len(channels) == 1, (
        f"expected the durable pool to own exactly 1 channel, found "
        f"{len(channels)} -- a count of 0 means the durable path checked out "
        f"of the non-durable pool"
    )
    return channels[0]


async def _fill_non_durable_to_the_soft_threshold(connector):
    """
    Check out non-durable clients until the pool sits at its SOFT THRESHOLD.

    ``max_client_capacity`` is the packing product (``pool_size`` x
    ``channel_client_limit``), and on an overflow-enabled pool that number is a
    packing target rather than a wall. Filling to it opens every channel the
    pool is allowed to open and loads each one to ``client_limit``, which is
    the state every isolation assertion below wants: the non-durable pool has
    handed out every channel it owns, so anything a durable path borrowed from
    the wrong pool would have to show up here.

    Sizes the fill off the pool's own remaining headroom -- reported by the
    public ``pool_stats()``, which covers the non-durable pool alone -- rather
    than off its nominal maximum, so a test that already holds a non-durable
    checkout takes only the rest.

    This does NOT leave the pool unable to serve another checkout. With the
    SR-1.5 overflow band enabled (``rabbitmq_channel_overflow_enabled``, which
    ships True and which the non-durable pool takes no override of), the next
    checkout packs onto the least-loaded open channel and returns immediately
    -- see ``test_a_checkout_past_the_soft_threshold_packs_onto_an_open_channel``.
    Only the DURABLE pool, whose ``DurableChannelPoolConfig`` hard-overrides
    overflow to off, still blocks for ``rabbitmq_channel_wait_ms`` and then
    raises once it is full; that is what ``_drain_durable`` below produces.

    :returns: The list of channels handed out, one entry per checkout, so a
        shared channel appears once per client using it.
    """
    stats = connector.pool_stats()
    remaining = stats["max_client_capacity"] - stats["total_client_count"]
    return [await connector.get_channel_direct() for _ in range(remaining)]


async def _drain_durable(connector):
    """
    Check out every client slot the DURABLE pool has left to hand out.

    The mirror of ``_fill_non_durable_to_the_soft_threshold``, sized off the
    durable pool's own entry in ``pool_stats_by_pool()`` -- ``pool_stats()``
    covers the non-durable pool alone and would size the drain wrongly. At the
    durable client limit of 1 this is one checkout per channel, so a drained
    durable pool holds ``DURABLE_POOL_SIZE`` checkouts.

    Unlike its non-durable mirror this really does drain: the durable pool's
    ``max_client_capacity`` is a wall, not a packing target, because
    ``DurableChannelPoolConfig`` overrides ``overflow_enabled`` to False
    unconditionally. A further durable checkout blocks for
    ``rabbitmq_durable_channel_wait_ms`` and then raises.

    :returns: The list of channels handed out, one entry per checkout.
    """
    stats = connector.pool_stats_by_pool()[RabbitMQConnector.DURABLE_POOL_NAME]
    remaining = stats["max_client_capacity"] - stats["total_client_count"]
    return [
        await connector.get_channel_direct(durable_pool=True) for _ in range(remaining)
    ]


def _pool_is_at_its_client_capacity(connector, pool_name):
    """
    Whether ``pool_name`` has handed out every client slot it is sized for.

    ``max_client_capacity`` means two different things depending on the pool,
    which is why this reads "at its client capacity" rather than "full":

    - NON-DURABLE: a SOFT threshold. It is the packing product the pool is
      provisioned for and the number the autoscaler reacts to, but the pool
      keeps serving past it in the SR-1.5 overflow band, and the guard keeps
      admitting past it (PRD FR-11). At capacity here is not refusing.
    - DURABLE: a hard wall. Overflow is off by construction, so at capacity
      here IS refusing.

    Used as a non-vacuity check: it proves a test really reached the state it
    claims to be asserting about.
    """
    stats = connector.pool_stats_by_pool()[pool_name]
    return stats["total_client_count"] == stats["max_client_capacity"]


def _prefetch_calls(channel, prefetch_count):
    """Every ``set_qos`` call on ``channel`` carrying ``prefetch_count``."""
    return [
        call
        for call in channel.set_qos.call_args_list
        if call.kwargs.get("prefetch_count") == prefetch_count
    ]


def _channels_carrying(channels, prefetch_count):
    """The subset of ``channels`` that ever saw ``prefetch_count``."""
    return [channel for channel in channels if _prefetch_calls(channel, prefetch_count)]


def test_named_prefetch_constants_match_the_landed_settings():
    """
    Pin the module constants to the settings they stand in for.

    Every isolation assertion below is only meaningful while the three
    prefetch values stay distinct; if a settings change collided one of them
    with the pool-init value, those assertions would quietly stop proving
    anything. This fails loudly instead.
    """
    assert (
        DURABLE_PREFETCH,
        DLQ_BROWSE_PREFETCH,
        POOL_INIT_PREFETCH,
    ) == (
        _settings_module.settings.durable_consumer_prefetch_count,
        _settings_module.settings.dead_letter_browse_prefetch_count,
        common_settings.rabbitmq_channel_prefetch_count,
    )
    # The three prefetch values must stay pairwise distinct in substance, or the
    # _channels_carrying(...) assertions stop distinguishing anything.
    assert len({DURABLE_PREFETCH, DLQ_BROWSE_PREFETCH, POOL_INIT_PREFETCH}) == 3


def test_named_capacity_constants_match_the_landed_settings():
    """
    Pin the module's client-limit arithmetic to the landed settings.

    The prefetch half of this guard fired loudly for the flip; the client-limit
    half went unguarded, which is why a moved default could silently invert the
    overflow band the cap test drives through. Assert the soft threshold really
    is the packing product, and that the binding cap sits strictly ABOVE it so
    the band cannot invert again unnoticed. Bookkeeping on this module's own
    constants -- it asserts nothing about isolation behavior.
    """
    assert (
        NON_DURABLE_SOFT_THRESHOLD
        == NON_DURABLE_POOL_SIZE * common_settings.rabbitmq_channel_client_limit
    )
    assert BINDING_STREAM_CAP > NON_DURABLE_SOFT_THRESHOLD


@pytest.mark.asyncio
async def test_mocked_connection_hands_out_a_fresh_channel_per_call(mock_conn):
    """
    Guard the module-level ``mock_conn`` override.

    If this module ever picked up the root single-channel ``mock_conn``
    instead, both pools would alias one channel object and every disjointness
    assertion in this file would be vacuous.
    """
    first = await mock_conn.channel()
    second = await mock_conn.channel()

    assert first is not second


@pytest.mark.asyncio
async def test_durable_and_non_durable_pools_hand_out_disjoint_channels(
    connected_connector, live_durable_checkout
):
    """A durable checkout is never among the channels the other pool hands out."""
    drained = await _fill_non_durable_to_the_soft_threshold(connected_connector)

    assert live_durable_checkout not in drained


@pytest.mark.asyncio
async def test_durable_prefetch_never_reaches_a_non_durable_channel(
    connected_connector, live_durable_checkout
):
    """
    The Epic acceptance criterion (SDD SR-2.1 / SR-2.2).

    ``DurableQueueConsumer`` lowers its channel's prefetch to
    ``DURABLE_PREFETCH`` and never restores it, so isolation cannot rest on
    the durable path cleaning up after itself -- it rests on the durable
    channel not being in the non-durable pool at all. Driven through the real
    consume path, with two real pools, so the assertion is about what the
    non-durable pool actually handed out.
    """
    # Non-vacuity: the production path really did apply the durable prefetch,
    # so an empty result below means isolation, not a missing set_qos call.
    assert _prefetch_calls(live_durable_checkout, DURABLE_PREFETCH)

    drained = await _fill_non_durable_to_the_soft_threshold(connected_connector)

    assert _channels_carrying(drained, DURABLE_PREFETCH) == []
    # Absence from the drained list proves less than it appears to: at the
    # shipped packed default a channel handed out to one borrower is still
    # handed out to the next, so a durable path that borrowed from the wrong
    # pool could show up in the drain -- or not, depending on packing order --
    # while the property that actually has to hold is OWNERSHIP: the durable
    # checkout must not be a channel the non-durable pool owns at all. Assert
    # that directly. (See _only_durable_channel on reading _all_channels.)
    assert live_durable_checkout not in connected_connector._channel_pool._all_channels


@pytest.mark.asyncio
async def test_returned_durable_channel_is_never_handed_out_by_the_non_durable_pool(
    connected_connector, live_durable_checkout
):
    """
    Isolation must hold AFTER the durable channel goes back to its pool.

    This is the actual shape of the leak being fixed: ``return_channel()``
    does not reset prefetch, so a returned durable channel is still carrying
    ``DURABLE_PREFETCH`` when it becomes available again. It must become
    available to the durable pool only.
    """
    await connected_connector.return_channel(live_durable_checkout)

    drained = await _fill_non_durable_to_the_soft_threshold(connected_connector)

    assert live_durable_checkout not in drained


@pytest.mark.asyncio
async def test_isolation_holds_when_the_non_durable_client_limit_is_above_one(
    non_default_non_durable_client_limit, connected_connector, live_durable_checkout
):
    """
    Isolation at a shared, non-default packing density.

    At ``client_limit`` 1 the un-reset durable prefetch is a latent bug: one
    borrower per channel means nobody inherits it. Sharing a channel across
    borrowers is what makes an inherited prefetch observable by other streams.
    The shipped default is now above one (8), so this test's distinct value is
    that it proves the property at a density (3) that is neither 1 nor the
    shipped default -- an arbitrary shared density, not just whatever ships.
    """
    assert _prefetch_calls(live_durable_checkout, DURABLE_PREFETCH)

    drained = await _fill_non_durable_to_the_soft_threshold(connected_connector)

    # Non-vacuity: the raised limit really did pack several borrowers onto one
    # channel. Without this the test would silently degrade into a duplicate of
    # the client_limit-1 case above.
    assert len(drained) > len(set(drained))
    assert _channels_carrying(drained, DURABLE_PREFETCH) == []


@pytest.mark.asyncio
async def test_dlq_browse_prefetch_never_reaches_a_non_durable_channel(
    connected_connector,
):
    """
    The DLQ browse prefetch is isolated the same way the durable one is.

    ``browse`` lowers its channel's prefetch to ``DLQ_BROWSE_PREFETCH`` and,
    like the durable consume path, never restores it. The browse checkout is
    released before the drain, which is where a wrong-pool checkout would
    actually bite: the released channel would go back on the non-durable
    available queue still carrying the browse prefetch.
    """
    generator, _ack_tracker, browse_channel = await DeadLetterConsumer.browse()

    assert _prefetch_calls(browse_channel, DLQ_BROWSE_PREFETCH)

    await generator.aclose()
    # The generator was never started, so its on-cleanup return never ran;
    # release the browse checkout explicitly rather than leaking it.
    await connected_connector.return_channel(browse_channel)

    drained = await _fill_non_durable_to_the_soft_threshold(connected_connector)

    assert _channels_carrying(drained, DLQ_BROWSE_PREFETCH) == []


@pytest.mark.asyncio
async def test_non_durable_acquisition_never_returns_a_durable_pool_channel(
    connected_connector,
):
    """
    Negative control, asserted as positive ownership.

    Seeds the durable pool with an idle channel -- the case most at risk,
    since an idle channel is the one a pool is free to hand out -- then takes
    every non-durable checkout the pool allows and asserts each one is a
    channel the non-durable pool owns.
    """
    durable_channel = await connected_connector.get_channel_direct(durable_pool=True)
    await connected_connector.return_channel(durable_channel)

    drained = await _fill_non_durable_to_the_soft_threshold(connected_connector)

    # Ownership is only observable through the pool's own channel map; see
    # _only_durable_channel for why.
    non_durable_owned = connected_connector._channel_pool._all_channels
    assert [c for c in drained if c not in non_durable_owned] == []


@pytest.mark.asyncio
async def test_durable_checkout_is_absent_from_non_durable_pool_stats(
    connected_connector, live_durable_checkout
):
    """
    ``pool_stats()`` reports the non-durable pool alone.

    Per-pool admission is built on this: durable usage must not consume the
    non-durable capacity budget that surface reports.
    """
    assert connected_connector.pool_stats()["in_use_channel_count"] == 0


@pytest.mark.asyncio
async def test_durable_checkout_is_counted_in_the_durable_pool_entry(
    connected_connector, live_durable_checkout
):
    """
    The other half of the split: the durable entry is where the usage lands.

    Every capacity surface now reads a single pool's entry out of
    ``pool_stats_by_pool()``, and each of those surfaces is tested against a
    synthetic stats dict -- which can only prove the surface reads the entry it
    is handed, never that the entry is populated. This is the one place proving
    the connector itself puts a REAL durable checkout, taken through the
    production path, into the durable pool's OWN entry.
    """
    by_pool = connected_connector.pool_stats_by_pool()

    assert by_pool[RabbitMQConnector.DURABLE_POOL_NAME]["in_use_channel_count"] == 1


@pytest.mark.asyncio
async def test_durable_checkout_succeeds_while_the_non_durable_pool_is_drained(
    connected_connector, mock_queue_id
):
    """
    The starvation half of SR-2.1 (PRD G-6 / FR-3).

    A saturated non-durable pool must not be able to starve a durable
    consumer. Asserted at the POOL layer: the durable pool serves the checkout
    no matter how full the other pool is. Every durable entry point is still
    fronted by ``CapacityGuard.raise_if_at_capacity(durable_pool=True)``, which
    now measures the durable pool's own budget, so the same scenario driven
    through a guarded gRPC/REST entry point is admitted as well --
    ``test_drained_non_durable_pool_still_admits_a_durable_stream`` below
    proves that half against these same real pools. The guard stays in front of
    those entry points: its job is to shed a request that the pool could only
    answer by blocking for its wait window.

    A pool that had to wait would raise ``ChannelNotAvailableError`` after
    ``rabbitmq_channel_wait_ms``, so a successful checkout is itself proof
    that nothing blocked.
    """
    await _fill_non_durable_to_the_soft_threshold(connected_connector)

    generator, _ack_tracker = await DurableQueueConsumer.consume_with_acks(
        mock_queue_id
    )
    try:
        durable_stats = connected_connector.pool_stats_by_pool()[
            RabbitMQConnector.DURABLE_POOL_NAME
        ]
        assert durable_stats["in_use_channel_count"] == 1
    finally:
        await generator.aclose()


@pytest.mark.asyncio
async def test_a_checkout_past_the_soft_threshold_packs_onto_an_open_channel(
    connected_connector,
):
    """
    The pool fact the two admission tests below rest on (SDD SR-1.5).

    ``_fill_non_durable_to_the_soft_threshold`` no longer produces a pool that
    cannot serve another checkout, and that is the premise of its docstring and
    of the admission test that follows. Prove it directly: at the soft
    threshold the pool holds ``NON_DURABLE_POOL_SIZE`` channels and cannot
    grow, so the only way a further checkout can succeed is by packing onto a
    channel the pool ALREADY owns -- which is what the overflow tier does.

    This also fails loudly if the overflow work is missing from the library
    under test or ships defaulted off: without it this checkout blocks for
    ``rabbitmq_channel_wait_ms`` and raises ``ChannelNotAvailableError``.

    Deliberately small. Exhaustive least-loaded-selection coverage is the
    library's own lane; what this file needs is only that the band is
    reachable.
    """
    await _fill_non_durable_to_the_soft_threshold(connected_connector)
    owned_before = set(connected_connector._channel_pool._all_channels)

    # Non-vacuity: the pool really is at the soft threshold, so this checkout
    # cannot be served by an idle channel or by growing the pool.
    assert _pool_is_at_its_client_capacity(
        connected_connector, RabbitMQConnector.NON_DURABLE_POOL_NAME
    )

    overflow_channel = await connected_connector.get_channel_direct()

    # Ownership is only observable through the pool's own channel map; see
    # _only_durable_channel for why.
    assert overflow_channel in owned_before


@pytest.mark.asyncio
async def test_non_durable_pool_at_its_soft_threshold_admits_a_new_stream(
    connected_connector, real_pool_capacity_guard
):
    """
    A pod at its soft threshold admits a new non-durable stream (PRD FR-11).

    DECIDED BEHAVIOR CHANGE (Epic 7, Task 2). This test previously asserted the
    mirror image -- that a non-durable pool which had handed out every stream
    slot it was sized for shed the next request. Under two-tier admission the
    pool's ``max_client_capacity`` is a SOFT threshold: the density the pool is
    provisioned to pack at and the signal the autoscaler reacts to, reported
    rather than enforced. Refusing there would strand a pinned connection -- a
    consumer stream lives on the pod it landed on, so adding pods cannot rescue
    it and the refusal is a hard client failure rather than a rebalance. The
    stream is admitted into the overflow band instead, and only the hard bound
    below refuses it.

    The refusing tiers are exercised by
    ``test_non_durable_pool_at_the_stream_cap_rejects_a_new_stream`` below;
    this test deliberately runs under the root autouse
    ``non_binding_stream_memory_cap``, which parks both of them out of reach so
    the soft threshold is the only line in play.
    """
    await _fill_non_durable_to_the_soft_threshold(connected_connector)

    # Non-vacuity: the fill really did reach the soft threshold, which is the
    # line this test asserts is NOT a refusal. Without this the test would pass
    # on an idle pool.
    assert _pool_is_at_its_client_capacity(
        connected_connector, RabbitMQConnector.NON_DURABLE_POOL_NAME
    )

    # Admission is the absence of ChannelNotAvailableError; a raise here fails
    # the test, which is exactly the FR-11 regression this Epic exists to
    # prevent.
    real_pool_capacity_guard.raise_if_at_capacity()


@pytest.mark.asyncio
async def test_non_durable_pool_at_the_stream_cap_rejects_a_new_stream(
    mocker, connected_connector, real_pool_capacity_guard
):
    """
    The shedding half, retained against real pool stats (SDD SR-3.5).

    Moving the pool bound from wall to packing target must not leave the
    non-durable path unbounded, and this file is the only place a refusal is
    decided from what two real ``ChannelPool`` instances report rather than
    from a synthetic stats dict. So drive the pool THROUGH its overflow band --
    every checkout past the soft threshold packs, per the pool test above --
    up to a binding memory-safe stream cap, and require the guard to refuse
    there.

    Patching ``max_non_durable_streams_per_pod`` is the opt-in the root autouse
    ``non_binding_stream_memory_cap`` documents (pattern (a)): the later patch
    wins, so this test alone gets a cap that can bind.

    The refusal is asserted for a PINNED stream -- ``raise_if_at_capacity()``
    defaults to ``re_routable=False`` -- because the hard bound is the one tier
    that refuses every class. A cap below the soft threshold would prove less:
    it would never reach the band at all.
    """
    mocker.patch.object(
        _settings_module.settings,
        "max_non_durable_streams_per_pod",
        BINDING_STREAM_CAP,
    )

    await _fill_non_durable_to_the_soft_threshold(connected_connector)
    # The subtrahend is a STREAM count (the soft threshold the fill just
    # reached), not the channel count NON_DURABLE_POOL_SIZE; the two coincide
    # only at client_limit 1. 34 - 32 = 2 checkouts into the overflow band.
    overflow_checkouts = BINDING_STREAM_CAP - NON_DURABLE_SOFT_THRESHOLD
    for _ in range(overflow_checkouts):
        await connected_connector.get_channel_direct()

    # Non-vacuity: the pod really is AT the cap, and got there by overflowing
    # past the soft threshold rather than by the pool refusing en route.
    assert connected_connector.pool_stats()["total_client_count"] == (
        BINDING_STREAM_CAP
    )

    with pytest.raises(ChannelNotAvailableError):
        real_pool_capacity_guard.raise_if_at_capacity()


@pytest.mark.asyncio
async def test_drained_non_durable_pool_still_admits_a_durable_stream(
    connected_connector, real_pool_capacity_guard
):
    """
    The headline criterion: non-durable saturation must not shed durable work.

    This is the production incident, decided at the layer that actually
    rejected it. The pool-layer counterpart above shows the durable pool can
    serve the checkout; this shows the guard in front of the durable entry
    points agrees, because it reads the durable pool's own budget rather than
    a view the non-durable drain has exhausted.
    """
    await _fill_non_durable_to_the_soft_threshold(connected_connector)

    # Non-vacuity: the drain really did exhaust the non-durable budget, which
    # is the budget a combined or non-durable-keyed check would have consulted.
    assert _pool_is_at_its_client_capacity(
        connected_connector, RabbitMQConnector.NON_DURABLE_POOL_NAME
    )

    # Admission is the absence of ChannelNotAvailableError; a raise here fails
    # the test, which is the regression this Task exists to prevent.
    real_pool_capacity_guard.raise_if_at_capacity(durable_pool=True)


@pytest.mark.asyncio
async def test_drained_durable_pool_rejects_a_new_durable_stream(
    connected_connector, real_pool_capacity_guard
):
    """
    The durable side sheds on its OWN exhaustion.

    Per-pool admission is not a licence to admit durable traffic
    unconditionally: when the durable pool is genuinely full, the request has
    to be rejected up front rather than blocked in the pool's wait window.
    """
    await _drain_durable(connected_connector)

    with pytest.raises(ChannelNotAvailableError):
        real_pool_capacity_guard.raise_if_at_capacity(durable_pool=True)


@pytest.mark.asyncio
async def test_drained_durable_pool_still_admits_a_non_durable_stream(
    connected_connector, real_pool_capacity_guard
):
    """
    Isolation runs both ways: durable saturation must not shed streaming work.

    The symmetric case of the incident. Durable consumers filling their pool
    is a normal steady state, and it must not cost the streaming surface a
    single admission slot.
    """
    await _drain_durable(connected_connector)

    # Non-vacuity: the durable drain really did exhaust the durable budget.
    assert _pool_is_at_its_client_capacity(
        connected_connector, RabbitMQConnector.DURABLE_POOL_NAME
    )

    real_pool_capacity_guard.raise_if_at_capacity()


@pytest.mark.asyncio
async def test_freshly_connected_durable_pool_admits_with_no_channels_open(
    connected_connector, real_pool_capacity_guard
):
    """
    A healthy, never-used durable pool admits -- the ``min_size`` 0 trap.

    The durable pool opens no channels until something needs one, so on a
    freshly connected service it is initialized with zero channels total and
    therefore zero AVAILABLE. A durable check derived from
    ``available_channel_count`` or ``total_channel_count`` would read that as
    exhaustion and reject every durable request on a perfectly healthy
    service. Only a real pool reports this state naturally, which is why the
    check belongs here rather than in a synthetic-stats test.
    """
    durable_stats = connected_connector.pool_stats_by_pool()[
        RabbitMQConnector.DURABLE_POOL_NAME
    ]

    # Non-vacuity: this really is the trap state, not a warmed-up pool.
    assert (
        durable_stats["initialized"],
        durable_stats["total_channel_count"],
        durable_stats["available_channel_count"],
    ) == (True, 0, 0)

    real_pool_capacity_guard.raise_if_at_capacity(durable_pool=True)


# --- AC-8 under packing (t3.tj2.xn.ax.mh) --------------------------------
#
# Everything below extends the isolation coverage above to AC-8's full form:
# "Durable-consumer behavior (prefetch, delivery, queue CRUD) is byte-for-byte
# unaffected by non-durable load in tests exercising both paths concurrently."
# Epic 1 deferred it here because it only becomes testable at client_limit > 1,
# which the packing flip now ships (8). The helpers and fixture here drive the
# non-durable pool into a genuinely PACKED state -- several borrowers per
# channel, each open channel loaded to the shipped client_limit, and a couple
# of checkouts into the SR-1.5 overflow band -- and then prove the durable path
# behaves identically to the way it does against an empty non-durable pool.


def _durable_channels(connector):
    """The channels the durable pool currently owns, as a list."""
    return list(connector._durable_channel_pool._all_channels)


async def _drain_durable_consume(connector, queue_id):
    """
    Drive ``DurableQueueConsumer.consume_with_acks`` to completion, once.

    Iterates exactly ``DELIVERED_MESSAGE_COUNT`` events -- everything the
    delivering fake queue feeds -- then closes the generator so its pool
    checkout is released. Each ``__anext__`` is wrapped in ``asyncio.wait_for``
    so a mis-wired fake fails fast at ``messages_queue.get()`` rather than
    hanging.

    :returns: The delivered sequence as ``(message dict, delivery_tag)`` tuples,
        the observable shape ``EventMessage`` carries (it defines no ``__eq__``,
        so the two fields the durable path actually sets are compared directly).
    """
    generator, _ack_tracker = await DurableQueueConsumer.consume_with_acks(queue_id)
    delivered = []
    try:
        for _unused_event in range(DELIVERED_MESSAGE_COUNT):
            message = await asyncio.wait_for(
                anext(generator), timeout=DURABLE_DRAIN_TIMEOUT_S
            )
            delivered.append((message.message, message.delivery_tag))
    finally:
        await generator.aclose()
    return delivered


def _assert_non_durable_is_packed(connector, drained):
    """
    Fail unless the non-durable pool is genuinely packed, not merely busy.

    Three independent signals, so removing the packing setup in a scratch edit
    turns this red: a channel handed to more than one borrower, the pool at or
    past its soft threshold, and at least one open channel loaded to the shipped
    client_limit (8). Together they rule out the client_limit-1 configuration in
    which AC-8's inheritance risk cannot even arise.
    """
    per_channel = Counter(drained)
    assert len(drained) > len(set(drained))
    assert connector.pool_stats()["total_client_count"] >= NON_DURABLE_SOFT_THRESHOLD
    assert max(per_channel.values()) >= common_settings.rabbitmq_channel_client_limit


@pytest_asyncio.fixture
async def packed_non_durable_pool(connected_connector):
    """
    Pack the non-durable pool to its soft threshold and a little past it.

    Fills every channel the pool is sized for to the shipped ``client_limit``
    (that is ``NON_DURABLE_SOFT_THRESHOLD`` = 32 stream slots across
    ``NON_DURABLE_POOL_SIZE`` = 4 channels), then takes two more checkouts so
    the pool sits inside the SR-1.5 overflow band rather than merely at its
    edge. Asserts the packing held before yielding, so any test that depends on
    this fixture starts from a provably packed pool.

    :returns: The list of channels handed out, one entry per borrower, so a
        shared channel appears once per client using it.
    """
    drained = await _fill_non_durable_to_the_soft_threshold(connected_connector)
    # Two checkouts past the soft threshold: each packs onto the least-loaded
    # open channel (the overflow tier), so the pool is provably in the band.
    drained.append(await connected_connector.get_channel_direct())
    drained.append(await connected_connector.get_channel_direct())

    _assert_non_durable_is_packed(connected_connector, drained)
    return drained


class TestDurablePathUnderPackedNonDurableLoad:
    """
    AC-8 under packing: durable behavior is unaffected by packed non-durable load.

    Every arm here drives the REAL durable consume / CRUD paths through the two
    real pools of ``connected_connector`` while the non-durable pool is packed
    to (and past) its soft threshold, and asserts the durable side behaves
    exactly as it does against an empty non-durable pool. The three AC-8 clauses
    -- prefetch, delivery and queue CRUD -- get one arm each, plus a concurrency
    arm that runs a durable operation and the non-durable pack together.
    """

    @pytest.mark.asyncio
    async def test_durable_prefetch_is_byte_for_byte_unaffected_by_packed_load(
        self, connected_connector
    ):
        """
        AC-8 clause "prefetch ... byte-for-byte unaffected".

        Differential, not a spot check: capture the durable consumer's whole
        ``set_qos`` call list on a checkout taken against an EMPTY non-durable
        pool, then pack the non-durable pool and capture the same list on a
        fresh durable checkout. The two must be equal call-for-call. Because the
        durable pool has ``client_limit`` 1 and ``min_size`` 0, each checkout
        opens its own channel, so the two lists come off two distinct channels
        and the equality is a genuine before/after comparison.
        """
        baseline_gen, _ = await DurableQueueConsumer.consume_with_acks("q-baseline")
        baseline_channel = _only_durable_channel(connected_connector)
        baseline_qos = list(baseline_channel.set_qos.call_args_list)

        drained = await _fill_non_durable_to_the_soft_threshold(connected_connector)
        _assert_non_durable_is_packed(connected_connector, drained)

        under_load_gen, _ = await DurableQueueConsumer.consume_with_acks("q-under-load")
        under_load_channel = next(
            channel
            for channel in _durable_channels(connected_connector)
            if channel is not baseline_channel
        )
        under_load_qos = list(under_load_channel.set_qos.call_args_list)

        # Non-vacuity: the durable prefetch really was applied on the under-load
        # channel, so an equal-lists pass cannot be two empty lists.
        assert _prefetch_calls(under_load_channel, DURABLE_PREFETCH)

        assert under_load_qos == baseline_qos

        await baseline_gen.aclose()
        await under_load_gen.aclose()

    @pytest.mark.asyncio
    async def test_durable_delivery_sequence_is_identical_under_packed_load(
        self, connected_connector
    ):
        """
        AC-8 clause "delivery ... byte-for-byte unaffected".

        Drive a durable consume to completion against an empty non-durable pool,
        then again with the non-durable pool packed, and assert the delivered
        ``(message, delivery_tag)`` sequence is identical. The drain is bounded
        by ``asyncio.wait_for`` so a mis-wired fake fails fast rather than
        hanging at ``messages_queue.get()``.
        """
        baseline = await _drain_durable_consume(connected_connector, "q-baseline")

        drained = await _fill_non_durable_to_the_soft_threshold(connected_connector)
        _assert_non_durable_is_packed(connected_connector, drained)

        under_load = await _drain_durable_consume(connected_connector, "q-under-load")

        # Non-vacuity: the run really delivered the full message set, so an
        # equal-sequences pass cannot be two empty lists.
        assert len(under_load) == DELIVERED_MESSAGE_COUNT
        assert under_load == baseline

    @pytest.mark.asyncio
    async def test_queue_crud_succeeds_from_the_durable_pool_under_packed_load(
        self, connected_connector, packed_non_durable_pool
    ):
        """
        AC-8 clause "queue CRUD ... byte-for-byte unaffected".

        ``create_queue`` / ``update_queue`` / ``delete_queue`` each run while the
        non-durable pool is packed, and each must (a) complete without raising,
        (b) draw its channel from the DURABLE pool, and (c) leave the non-durable
        pool's stream count untouched -- a durable operation must never consume a
        non-durable admission slot. The non-durable count is captured before the
        operations and re-checked after each one.
        """
        non_durable_streams_before = connected_connector.pool_stats()[
            "total_client_count"
        ]
        groups = [filter_group()]
        new_groups = [filter_group()]

        queue_id, created = await DurableQueueConsumer.create_queue(groups)
        assert created is True
        assert queue_id
        assert _durable_channels(connected_connector)
        assert (
            connected_connector.pool_stats()["total_client_count"]
            == non_durable_streams_before
        )

        await DurableQueueConsumer.update_queue(queue_id, groups, new_groups)
        assert _durable_channels(connected_connector)
        assert (
            connected_connector.pool_stats()["total_client_count"]
            == non_durable_streams_before
        )

        await DurableQueueConsumer.delete_queue(queue_id)
        assert _durable_channels(connected_connector)
        assert (
            connected_connector.pool_stats()["total_client_count"]
            == non_durable_streams_before
        )

    @pytest.mark.asyncio
    async def test_durable_create_and_non_durable_pack_run_concurrently(
        self, connected_connector
    ):
        """
        AC-8 clause "in tests exercising both paths concurrently".

        Run a durable ``create_queue`` and the non-durable pack together under a
        single ``asyncio.gather`` rather than one after the other, and assert
        BOTH sides landed: the durable create returned a fresh queue id, and the
        non-durable pool ended up genuinely packed. Asserting both is what keeps
        neither side from being a silent no-op.
        """
        (queue_id, created), drained = await asyncio.gather(
            DurableQueueConsumer.create_queue([filter_group()]),
            _fill_non_durable_to_the_soft_threshold(connected_connector),
        )

        assert created is True
        assert queue_id
        assert _durable_channels(connected_connector)
        _assert_non_durable_is_packed(connected_connector, drained)
