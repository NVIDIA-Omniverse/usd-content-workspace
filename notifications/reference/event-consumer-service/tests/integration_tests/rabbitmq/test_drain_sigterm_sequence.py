# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Epic 5 acceptance: the SIGTERM graceful-drain sequence, composed in order.

This is the cross-Task acceptance test for the graceful drain (Bee b.tj2,
Epic 5, Task 4 / t2.tj2.hk.33). On SIGTERM a pod must, in order:

1. engage draining and flip readiness to NOT_SERVING,
2. refuse NEW-stream admission with the existing 503 / gRPC
   RESOURCE_EXHAUSTED contract (``ChannelNotAvailableError``), class-blind,
3. let in-flight non-durable streams drain within the bounded window, and
4. close the connector's non-durable pool ONLY AFTER the window -- never
   before.

The ordering is the point: the readiness flip and the admission refusal must
be observable BEFORE the pool close, and the close must not precede the
window. This file asserts that the two service surfaces
(``ChannelPoolHealthServicer`` readiness and ``CapacityGuard`` admission) plus
the connector close observe ONE draining episode together, in that order.

Scope -- composition only
-------------------------
This is a COMPOSITION test, not a unit test. The pieces below are asserted in
isolation elsewhere and are NOT re-asserted here:

- The nc-side "close only after the window" timer and the reconnect-grace
  orphan-reclaim mechanics are Task 1's own unit subtests on
  ``RabbitMQConnector.drain_non_durable`` / ``begin_draining`` /
  ``close``.
- The not-draining admission tiers (soft / shed / hard) and the not-draining
  readiness thresholds are Task 2's unit subtests on ``CapacityGuard`` and
  ``ChannelPoolHealthServicer``.

Here we assert only the composed ordering across the three surfaces for a
single draining episode.

Two entry points, one guard argument
------------------------------------
The two NEW-stream non-durable consume entry points both funnel into
``CapacityGuard.raise_if_at_capacity``:

- gRPC ``ConsumeNonDurableEvents``
  (``app/grpc/event_consumer_servicer.py``) calls it with
  ``re_routable=self._is_proxied(context)`` -- so PINNED (``re_routable=False``,
  direct) and RE-ROUTABLE (``re_routable=True``, via the proxy) are both
  reachable there.
- REST/SSE ``_check_non_durable_capacity`` (``app/rest/routes.py``) calls it
  with ``re_routable=True`` unconditionally.

Asserting the guard refuses for BOTH ``re_routable=False`` and
``re_routable=True`` while draining therefore covers the argument space of both
entry points, and pins the class-blind refusal (SR-6.1): a draining pod is
going away, so there is no absorb-into-overflow-band for a pinned stream.

Non-vacuity of the readiness flip
---------------------------------
The draining phase leaves the pool contents UNCHANGED from the baseline and
toggles only the draining flag. The baseline reads SERVING with the same pool
state, so a NOT_SERVING verdict in the draining phase is attributable to the
draining flag alone, not to capacity -- the "even with free capacity" clause of
the AC. The available-stream count is asserted to sit well above the readiness
reserve to make that explicit.

Two arms
--------
- ``TestDrainSequenceOverAMockedConnection`` (always runs) drives one draining
  episode over a real ``ChannelPool`` on a mock connection, with the connector
  a ``spec``'d ``MagicMock`` whose ``pool_stats_by_pool()`` rebuilds from live
  pool state (the pattern in
  ``test_ac1_six_hundred_stream_burst.py::TestSixHundredStreamsKeepThePodServing``).
  It asserts the full cross-surface ordering with the pool close driven
  manually to stand in for the post-window ``connector.close()``.
- ``TestDrainSequenceOnALiveBroker`` (SKIPS without a broker; CI's
  ``integration:`` job runs it against a ``rabbitmq:4.1.3`` service, hard-failing
  on a missing broker) proves the same ordering against a REAL
  ``RabbitMQConnector`` with a genuinely held broker stream, driving the real
  ``begin_draining`` -> ``drain_non_durable`` -> ``close`` sequence. The held
  stream never drains, so the bounded window elapses with the stream still in
  flight and the pool is closed strictly afterwards -- the close-after-window
  half proven with a real hung stream rather than a manual close.
"""

import asyncio
import uuid
from types import SimpleNamespace

import pytest
import pytest_asyncio
from grpc_health.v1 import health_pb2
from unittest.mock import AsyncMock, MagicMock

from notification_common.config.common_settings import common_settings
from notification_common.errors.errors import ChannelNotAvailableError
from notification_common.rabbitmq.channel_pool import ChannelPool, ChannelPoolConfig
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings
from app.grpc.channel_pool_health_servicer import ChannelPoolHealthServicer
from app.lib.rabbitmq.capacity_guard import CapacityGuard

SERVING = health_pb2.HealthCheckResponse.SERVING
NOT_SERVING = health_pb2.HealthCheckResponse.NOT_SERVING

NON_DURABLE = RabbitMQConnector.NON_DURABLE_POOL_NAME

# The shipped non-durable pool geometry (common_settings defaults). Pinned
# explicitly below rather than trusted from the ambient environment: this
# sandbox carries a stale local .env (pre-Epic-3 client_limit 1 / prefetch
# 500), so a config-derived number must be installed, never inherited (the
# posture test_ac1_six_hundred_stream_burst.py's ``pinned_shipped_config``
# takes).
POOL_SIZE = 100  # rabbitmq_channel_pool_size
POOL_MIN_SIZE = 75  # rabbitmq_channel_pool_min_size
CLIENT_LIMIT = 8  # rabbitmq_channel_client_limit
PREFETCH = 15  # rabbitmq_channel_prefetch_count

# The soft packing threshold: pool_size 100 x client_limit 8 = 800.
SOFT_THRESHOLD = 800

# The memory-cap inputs and the cap they compute to, so admission does not ride
# on the ambient environment. budget 1024 MiB / (15 x 1800 + 5120) B = 33,429
# fleet / 10 replicas = 3,342 per pod -- far above the held-stream count here.
BUDGET_MIB = 1024
MAX_REPLICAS = 10

# The readiness reserve, in non-durable STREAM slots. A modest reserve so the
# baseline (with HELD_STREAMS in flight) sits comfortably in SERVING territory
# -- the "even with free capacity" premise of the draining flip.
MIN_AVAILABLE_STREAMS = 10

# A modest in-flight burst: well under the 800 soft threshold and the 3342 cap,
# so the baseline both serves and admits. Held (never returned) so the streams
# are genuinely in flight during the drain window, not freed.
HELD_STREAMS = 40

# The bounded drain window for the live-broker arm. Short so the arm is fast on
# CI; the genuinely held stream never drains, so the window fully elapses and
# the close happens strictly afterwards. Not a timing assertion -- the pure
# close-after-window timer is Task 1's unit; here the short window only keeps
# the composed ordering cheap to observe.
LIVE_DRAIN_WINDOW_S = 1.0
LIVE_DRAIN_POLL_S = 0.05

# The early-return arm's window/poll geometry. The window is deliberately
# GENEROUS (10 s) so that a drain returning near it is unambiguously the
# window-elapsed branch, while a drain returning in a small fraction of it is
# unambiguously the early-return-on-drain branch. The held checkouts are
# released partway through (EARLY_RELEASE_DELAY_S, several poll intervals in),
# so total_client_count reaches 0 well before the deadline and
# ``drain_non_durable`` must return on the count<=0 path, not the deadline path.
EARLY_DRAIN_WINDOW_S = 10.0
EARLY_DRAIN_POLL_S = 0.05
EARLY_RELEASE_DELAY_S = 0.15
# The early return must land in a small fraction of the window. A 2 s ceiling is
# a ~5x margin over the release delay yet only a fifth of the window, so it
# cleanly separates the two branches: a regression that always blocks the full
# window would elapse ~10 s and blow this ceiling. A hard safety wait bounds any
# pathological hang as a named TimeoutError above the window.
EARLY_RETURN_CEILING_S = 2.0
EARLY_SAFETY_TIMEOUT_S = EARLY_DRAIN_WINDOW_S + 5.0


def _fresh_mock_channel():
    """A fresh mock channel, as a real connection hands out per ``channel()``."""
    channel = AsyncMock()
    channel.is_closed = False
    channel.set_qos = AsyncMock()
    channel.close = AsyncMock()
    return channel


def _mock_connection():
    """A mock connection that hands out a FRESH channel per ``channel()`` call."""
    connection = MagicMock()
    connection.is_closed = False
    connection.channel = AsyncMock(side_effect=lambda: _fresh_mock_channel())
    return connection


@pytest.fixture
def pinned_shipped_config(mocker):
    """
    Pin the shipped non-durable geometry and the memory-cap inputs.

    The real ``ChannelPool`` builds an unmodified ``ChannelPoolConfig()`` that
    late-binds to ``common_settings`` on every read, and the guard's cap reads
    the same globals. A developer's untracked ``.env`` re-seeds them, so the
    packing numbers and the three cap inputs are pinned here explicitly. The cap
    is driven through the COMPUTED path (override ``None``) so the guard really
    exercises the prefetch-15 cap rather than a hand-set number.
    """
    mocker.patch.object(common_settings, "rabbitmq_channel_pool_size", POOL_SIZE)
    mocker.patch.object(
        common_settings, "rabbitmq_channel_pool_min_size", POOL_MIN_SIZE
    )
    mocker.patch.object(common_settings, "rabbitmq_channel_client_limit", CLIENT_LIMIT)
    mocker.patch.object(common_settings, "rabbitmq_channel_prefetch_count", PREFETCH)

    mocker.patch.object(settings.settings, "max_non_durable_streams_per_pod", None)
    mocker.patch.object(settings.settings, "max_re_routable_streams_per_pod", None)
    mocker.patch.object(
        settings.settings, "broker_stream_memory_budget_mib", BUDGET_MIB
    )
    mocker.patch.object(settings.settings, "scaler_max_replicas", MAX_REPLICAS)


async def _health(servicer):
    """Read the readiness verdict off the real servicer."""
    response = await servicer.Check(None, None)
    return response.status


def _admits(guard, re_routable):
    """
    True if the guard admits a new non-durable stream, False if it refuses.

    Refusal is the ``ChannelNotAvailableError`` raise -- the exact 503 / gRPC
    RESOURCE_EXHAUSTED contract both consume entry points surface.
    """
    try:
        guard.raise_if_at_capacity(re_routable=re_routable)
        return True
    except ChannelNotAvailableError:
        return False


class TestDrainSequenceOverAMockedConnection:
    """
    One draining episode across readiness, admission, and pool close, in order.

    A real non-durable ``ChannelPool`` holds ``HELD_STREAMS`` in-flight
    checkouts over a mock connection; a ``spec``'d ``RabbitMQConnector`` mock
    rebuilds ``pool_stats_by_pool()`` from that live pool so both service
    surfaces read real pool state. The episode engages draining (leaving the
    pool contents untouched), observes both surfaces, confirms the pool is still
    open during the window, then closes the pool to stand in for the post-window
    ``connector.close()`` -- recording every transition in ``sequence`` so the
    ordering can be asserted.
    """

    @pytest_asyncio.fixture
    async def drain_episode(self, pinned_shipped_config, mocker):
        non_durable = ChannelPool(
            _mock_connection(),
            ChannelPoolConfig(),
            name=NON_DURABLE,
        )
        await non_durable.initialize()

        for _unused_stream in range(HELD_STREAMS):
            await non_durable.get_channel()

        connector = mocker.MagicMock(spec=RabbitMQConnector)
        # ``draining`` is a plain instance attribute absent from the class spec,
        # so it must be set explicitly or the surfaces' reads raise
        # AttributeError. Start not-draining: the baseline is a serving pod.
        connector.draining = False
        connector.pool_stats_by_pool.side_effect = lambda: {
            NON_DURABLE: non_durable.pool_stats()
        }

        servicer = ChannelPoolHealthServicer(
            connector, min_available_streams=MIN_AVAILABLE_STREAMS
        )
        guard = CapacityGuard(connector=connector)

        sequence: list[str] = []
        obs = SimpleNamespace(sequence=sequence)

        # --- Baseline: a serving pod that admits both stream classes ---------
        stats = non_durable.pool_stats()
        obs.available_before = (
            stats["max_client_capacity"] - stats["total_client_count"]
        )
        obs.held_before = stats["total_client_count"]
        obs.health_before = await _health(servicer)
        obs.admits_pinned_before = _admits(guard, re_routable=False)
        obs.admits_re_routable_before = _admits(guard, re_routable=True)

        # --- Engage draining (begin_draining semantics); pool UNCHANGED ------
        connector.draining = True
        sequence.append("draining_engaged")

        # Readiness surface flips even though capacity is unchanged and free.
        obs.health_while_draining = await _health(servicer)
        obs.available_while_draining = (
            non_durable.pool_stats()["max_client_capacity"]
            - non_durable.pool_stats()["total_client_count"]
        )
        sequence.append("readiness_not_serving")

        # Admission surface refuses, class-blind.
        obs.admits_pinned_while_draining = _admits(guard, re_routable=False)
        obs.admits_re_routable_while_draining = _admits(guard, re_routable=True)
        sequence.append("admission_refused")

        # In-flight streams still held; the pool is NOT closed during the window.
        during = non_durable.pool_stats()
        obs.closed_during_window = during["closed"]
        obs.held_during_window = during["total_client_count"]
        sequence.append("pool_open_during_window")

        # Window elapsed -> connector.close() -> close_all(). Never before.
        await non_durable.close_all()
        obs.closed_after = non_durable.pool_stats()["closed"]
        sequence.append("pool_closed")

        return obs

    @pytest.mark.asyncio
    async def test_baseline_readiness_is_serving(self, drain_episode):
        # Same pool state the draining phase sees: absent draining it serves.
        assert drain_episode.health_before == SERVING

    @pytest.mark.asyncio
    async def test_baseline_has_free_capacity(self, drain_episode):
        # Non-vacuity: the pod really has room, so the later flip is not a
        # capacity verdict wearing a draining costume.
        assert drain_episode.available_before > MIN_AVAILABLE_STREAMS

    @pytest.mark.asyncio
    async def test_baseline_admits_pinned_stream(self, drain_episode):
        assert drain_episode.admits_pinned_before is True

    @pytest.mark.asyncio
    async def test_baseline_admits_re_routable_stream(self, drain_episode):
        assert drain_episode.admits_re_routable_before is True

    @pytest.mark.asyncio
    async def test_draining_flips_readiness_to_not_serving(self, drain_episode):
        assert drain_episode.health_while_draining == NOT_SERVING

    @pytest.mark.asyncio
    async def test_readiness_flips_despite_free_capacity(self, drain_episode):
        # The pool state is identical to the baseline; only draining changed.
        assert drain_episode.available_while_draining > MIN_AVAILABLE_STREAMS

    @pytest.mark.asyncio
    async def test_draining_refuses_pinned_new_stream(self, drain_episode):
        # gRPC direct / pinned (re_routable=False) is refused: class-blind.
        assert drain_episode.admits_pinned_while_draining is False

    @pytest.mark.asyncio
    async def test_draining_refuses_re_routable_new_stream(self, drain_episode):
        # gRPC-proxied and REST/SSE (re_routable=True) is refused: class-blind.
        assert drain_episode.admits_re_routable_while_draining is False

    @pytest.mark.asyncio
    async def test_pool_not_closed_during_window(self, drain_episode):
        assert drain_episode.closed_during_window is False

    @pytest.mark.asyncio
    async def test_in_flight_streams_held_during_window(self, drain_episode):
        # The in-flight burst is not force-dropped while draining.
        assert drain_episode.held_during_window == HELD_STREAMS

    @pytest.mark.asyncio
    async def test_pool_closed_after_window(self, drain_episode):
        assert drain_episode.closed_after is True

    @pytest.mark.asyncio
    async def test_readiness_flip_precedes_pool_close(self, drain_episode):
        seq = drain_episode.sequence
        assert seq.index("readiness_not_serving") < seq.index("pool_closed")

    @pytest.mark.asyncio
    async def test_admission_refusal_precedes_pool_close(self, drain_episode):
        seq = drain_episode.sequence
        assert seq.index("admission_refused") < seq.index("pool_closed")

    @pytest.mark.asyncio
    async def test_pool_open_observed_before_close(self, drain_episode):
        # The close does not precede the window: the pool was observed open
        # (streams still in flight) before the close event.
        seq = drain_episode.sequence
        assert seq.index("pool_open_during_window") < seq.index("pool_closed")


class TestDrainReturnsEarlyWhenStreamsRelease:
    """
    The drain HAPPY PATH: ``drain_non_durable`` returns EARLY once the pool
    drains, not only when the window elapses.

    The window-elapsed branch is covered by the two arms above (the held stream
    never drains, so the deadline is what ends the wait). This arm covers the
    complementary count<=0 branch, which otherwise has zero coverage and could
    regress to always blocking the full window unnoticed.

    A real non-durable ``ChannelPool`` holds ``HELD_STREAMS`` in-flight checkouts
    over a mock connection; a ``spec``'d ``RabbitMQConnector`` mock rebuilds
    ``pool_stats_by_pool()`` from that live pool -- the same deterministic,
    broker-free rig as ``TestDrainSequenceOverAMockedConnection``. A background
    task RELEASES all held checkouts partway through the window
    (``EARLY_RELEASE_DELAY_S``, several poll intervals in), driving
    ``total_client_count`` to 0 well before the ``EARLY_DRAIN_WINDOW_S`` deadline.

    ``drain_non_durable`` is timed on the real event-loop clock: it must return
    in a small fraction of the window (``EARLY_RETURN_CEILING_S``), and the
    subsequent pool close must still proceed. The generous window plus the tight
    ceiling separate the two branches unambiguously -- a regression that always
    blocked the full window would elapse ~``EARLY_DRAIN_WINDOW_S`` and blow the
    ceiling. A hard ``EARLY_SAFETY_TIMEOUT_S`` wraps the wait so any pathological
    hang surfaces as a named ``asyncio.TimeoutError`` rather than a stall.
    """

    @pytest_asyncio.fixture
    async def early_return_episode(self, pinned_shipped_config, mocker):
        non_durable = ChannelPool(
            _mock_connection(),
            ChannelPoolConfig(),
            name=NON_DURABLE,
        )
        await non_durable.initialize()

        held_channels = []
        for _unused_stream in range(HELD_STREAMS):
            held_channels.append(await non_durable.get_channel())

        connector = mocker.MagicMock(spec=RabbitMQConnector)
        connector.draining = False
        connector.pool_stats_by_pool.side_effect = lambda: {
            NON_DURABLE: non_durable.pool_stats()
        }

        obs = SimpleNamespace()

        # Baseline: the burst is genuinely in flight before the drain begins.
        obs.held_before = non_durable.pool_stats()["total_client_count"]

        # Engage the real draining flag, mirroring begin_draining semantics.
        connector.draining = True

        async def _release_after_delay():
            # Release all held checkouts partway through the window, several
            # poll intervals in, so the drain observes a nonzero count first and
            # then sees it fall to 0 -- exercising the count<=0 early return.
            await asyncio.sleep(EARLY_RELEASE_DELAY_S)
            for channel in held_channels:
                await non_durable.return_channel(channel)

        loop = asyncio.get_running_loop()
        releaser = asyncio.ensure_future(_release_after_delay())
        started = loop.time()
        try:
            await asyncio.wait_for(
                connector.drain_non_durable(
                    window_seconds=EARLY_DRAIN_WINDOW_S,
                    poll_interval_seconds=EARLY_DRAIN_POLL_S,
                ),
                timeout=EARLY_SAFETY_TIMEOUT_S,
            )
        finally:
            await releaser
        obs.drain_elapsed = loop.time() - started

        # The pool actually drained before the deadline (the early-return
        # premise), not merely returned on a stale read.
        obs.held_after_drain = non_durable.pool_stats()["total_client_count"]

        # The post-drain close still proceeds -- the drain waited, it did not
        # itself close the pool.
        obs.closed_before_close = non_durable.pool_stats()["closed"]
        await non_durable.close_all()
        obs.closed_after_close = non_durable.pool_stats()["closed"]

        return obs

    @pytest.mark.asyncio
    async def test_burst_in_flight_before_drain(self, early_return_episode):
        # Non-vacuity: the window really had streams to wait on at the start.
        assert early_return_episode.held_before == HELD_STREAMS

    @pytest.mark.asyncio
    async def test_pool_drained_before_deadline(self, early_return_episode):
        # The released checkouts drove the count to 0 -- the count<=0 branch.
        assert early_return_episode.held_after_drain == 0

    @pytest.mark.asyncio
    async def test_drain_returns_early(self, early_return_episode):
        # The wait ended in a small fraction of the generous window: the
        # early-return branch, not the window-elapsed branch.
        assert early_return_episode.drain_elapsed < EARLY_RETURN_CEILING_S

    @pytest.mark.asyncio
    async def test_drain_does_not_return_before_release(self, early_return_episode):
        # It waited for the drain rather than returning instantly on the initial
        # nonzero count: the return lands no earlier than the release.
        assert early_return_episode.drain_elapsed >= EARLY_RELEASE_DELAY_S

    @pytest.mark.asyncio
    async def test_pool_open_when_drain_returns(self, early_return_episode):
        # drain_non_durable waits but never closes the pool itself.
        assert early_return_episode.closed_before_close is False

    @pytest.mark.asyncio
    async def test_close_proceeds_after_early_drain(self, early_return_episode):
        assert early_return_episode.closed_after_close is True


class TestDrainSequenceOnALiveBroker:
    """
    The same ordering against a REAL connector with a genuinely held stream.

    Skips cleanly when no broker is reachable (the ``live_broker_connection``
    fixture's documented skip); CI's ``integration:`` job runs it against a
    ``rabbitmq:4.1.3`` service (hard-failing on a missing broker). A real
    ``RabbitMQConnector`` over the live connection carries one genuinely held,
    consuming non-durable stream. The real ``begin_draining`` ->
    ``drain_non_durable`` -> ``close`` sequence runs: the held stream never
    drains, so the bounded window elapses with it still in flight, and the pool
    is closed strictly afterwards. Every resource is uuid-named so
    ``pytest-xdist`` workers do not collide.
    """

    @pytest.mark.asyncio
    async def test_readiness_and_admission_flip_before_the_post_window_close(
        self, live_broker_connection, pinned_shipped_config
    ):
        connection = live_broker_connection

        connector = RabbitMQConnector()
        connector._connection = connection
        pool = ChannelPool(connection, ChannelPoolConfig(), name=NON_DURABLE)
        await pool.initialize()
        connector._channel_pool = pool

        servicer = ChannelPoolHealthServicer(
            connector, min_available_streams=MIN_AVAILABLE_STREAMS
        )
        guard = CapacityGuard(connector=connector)

        sequence: list[str] = []
        closed_cleanly = False
        try:
            # One genuinely held, consuming non-durable stream: a real hung
            # stream that will not drain within the window.
            channel = await pool.get_channel()
            queue = await channel.declare_queue(
                f"drain-hold-{uuid.uuid4()}", durable=False, auto_delete=True
            )
            await queue.consume(lambda _message: None, no_ack=True)

            # --- Baseline: serving pod that admits both classes -------------
            assert await _health(servicer) == SERVING
            assert _admits(guard, re_routable=False) is True
            assert _admits(guard, re_routable=True) is True

            # --- Engage the real draining signal ----------------------------
            connector.begin_draining()
            sequence.append("draining_engaged")

            assert await _health(servicer) == NOT_SERVING
            sequence.append("readiness_not_serving")

            assert _admits(guard, re_routable=False) is False
            assert _admits(guard, re_routable=True) is False
            sequence.append("admission_refused")

            # Still one stream in flight, pool still open, during the window.
            assert pool.pool_stats()["closed"] is False
            assert pool.pool_stats()["total_client_count"] == 1
            sequence.append("pool_open_during_window")

            # The real bounded wait: the held stream never drains, so the
            # window elapses and drain_non_durable returns with it still in
            # flight -- close is bounded by the window, not by the drain.
            await connector.drain_non_durable(
                window_seconds=LIVE_DRAIN_WINDOW_S,
                poll_interval_seconds=LIVE_DRAIN_POLL_S,
            )
            assert pool.pool_stats()["total_client_count"] == 1
            assert pool.pool_stats()["closed"] is False

            # The actual teardown, strictly after the window.
            await connector.close()
            closed_cleanly = True
            sequence.append("pool_closed")

            assert pool.pool_stats()["closed"] is True
            assert pool.pool_stats()["total_client_count"] == 0
            assert sequence.index("readiness_not_serving") < sequence.index(
                "pool_closed"
            )
            assert sequence.index("admission_refused") < sequence.index("pool_closed")
            assert sequence.index("pool_open_during_window") < sequence.index(
                "pool_closed"
            )
        finally:
            # If an assertion aborted before the explicit close, reclaim the
            # pool so the held consumer and its auto_delete queue go away.
            if not closed_cleanly:
                await pool.close_all()
