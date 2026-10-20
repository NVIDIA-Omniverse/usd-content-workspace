# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Unit tests for the per-scope auth-cache write-through batcher.

Timing is driven via two test affordances rather than the real wall
clock so the suite stays deterministic under ``pytest-xdist -n6`` and
``pytest-random-order``:

* ``fake_clock`` patches ``time.monotonic`` inside the cache_writer
  module so the drainer's debounce deadline can be advanced explicitly.
* Yields to the drainer use ``asyncio.sleep(0)`` (or short, bounded
  polls), never real-clock sleeps timed against a 50 ms window.

Pipelines are mocked via ``AsyncMock`` and patched onto
``notification_common.redis.redis_client.redis_client``.
"""

from __future__ import annotations

import asyncio
import inspect
import logging

import pytest
import pytest_asyncio
from notification_common.redis import redis_client as redis_client_module

from app.config import settings as _settings_module
from app.lib.queue_names import (
    NON_DURABLE_SUFFIX,
    QUEUE_NAME_PREFIX,
    redact_reconnect_token,
)
from app.lib.permissions.storage_permissions import cache_writer as cache_writer_module
from app.lib.permissions.storage_permissions.cache_scope import CacheScope
from app.lib.permissions.storage_permissions.cache_writer import (
    _BATCH_OP_CAP,
    _DEBOUNCE_SECONDS,
    _REDIS_FAILURE_LOG_INTERVAL,
    CacheWriter,
    active_writer,
    get_writer_for_queue,
    register_writer,
    reset_active_queue_id,
    set_active_queue_id,
    unregister_writer,
)
from tests.fixtures.cache_scope import (
    CACHE_SCOPE,
    SECOND_CACHE_SCOPE,
    TEST_PRINCIPAL_REF,
)
from tests.fixtures.reconnect_token import NON_DURABLE_QUEUE_NAME, RECONNECT_TOKEN

#: What a writer is really constructed with: the cache scope of a
#: non-durable stream, whose queue name embeds the stream's reconnect
#: token. These writers exist for no other kind of queue, so an id-shaped
#: stand-in here would let every telemetry assertion below pass whether or
#: not the writer redacts anything.
SCOPE = CACHE_SCOPE

#: The same queue as seen by a SECOND principal. Its writer is a peer of
#: ``SCOPE``'s, not a replacement for it.
OTHER_SCOPE = SECOND_CACHE_SCOPE

#: What addresses that scope's storage -- the registry key, the ContextVar
#: value, and the metrics depth baseline. It carries the raw queue name, so
#: it embeds the reconnect token and is never exported.
SCOPE_KEY = SCOPE.key

#: The redacted reference the writer exports in place of that name. Metric
#: attributes, log lines, and the drainer task name all carry this.
#:
#: It is derived from the TOKEN, not from the name: the same bare
#: ``sha256-<12 hex>`` value the stream's spans carry as ``queue_ref``, which
#: is what lets a metric series join its trace by exact match. Spelling it
#: as ``queue_ref_for_name(NON_DURABLE_QUEUE_NAME)`` would let the writer and
#: the helper agree on a wrapped form and still miss that join.
QUEUE_REF = redact_reconnect_token(RECONNECT_TOKEN)
# Generous margin above the writer's drain timeout to bound assertions
# without making real-clock 50 ms windows the timing oracle.
SHUTDOWN_MARGIN_SEC = 2.0


def scope_for(queue_id: str) -> CacheScope:
    """
    A scope on its own distinct non-durable queue, for the registry and
    ContextVar tests that only need several unrelated keys.

    Built as a real non-durable broker name rather than a bare label so
    ``queue_ref`` renders the way production's does; the principal half is
    held constant, because varying the QUEUE is what these tests are about.
    """
    return CacheScope(
        f"{QUEUE_NAME_PREFIX}:{queue_id}{NON_DURABLE_SUFFIX}", TEST_PRINCIPAL_REF
    )


@pytest.fixture
def fake_clock(mocker):
    """
    Substitute the ``time`` module reference inside the cache_writer
    module so the debounce deadline is driven by the test rather than
    the wall clock — without affecting the asyncio loop's monotonic
    clock (which would deadlock ``await asyncio.sleep`` and similar).

    Returns a tiny driver: ``clock.now`` reads, ``clock.advance(s)``
    moves forward by ``s`` seconds.
    """

    class _Clock:
        def __init__(self):
            self.now = 1000.0

        def advance(self, seconds):
            self.now += seconds

    clock = _Clock()

    class _FakeTimeModule:
        @staticmethod
        def monotonic():
            return clock.now

    mocker.patch.object(cache_writer_module, "time", _FakeTimeModule)
    return clock


@pytest.fixture
def mock_pipeline(mocker):
    """A fresh pipeline mock per test, with execute as an AsyncMock."""
    pipeline = mocker.MagicMock(name="pipeline")
    pipeline.execute = mocker.AsyncMock(name="pipeline.execute")
    return pipeline


@pytest.fixture
def mock_redis_client(mocker, mock_pipeline):
    client = mocker.MagicMock(name="redis_client")
    client.pipeline = mocker.MagicMock(return_value=mock_pipeline)
    mocker.patch.object(redis_client_module, "redis_client", client)
    return client


@pytest.fixture
def make_op(mocker):
    """
    Op factory. The Op type is a presumed Epic 1 contract not yet
    implemented; tests stub it as a Mock with an ``apply_to`` method.
    """

    def _factory(name="op"):
        op = mocker.Mock(name=name)
        op.apply_to = mocker.Mock(name=f"{name}.apply_to")
        return op

    return _factory


@pytest.fixture
def metrics(mocker):
    """
    Replace the singleton on the cache_writer module so metric calls
    can be asserted without touching OTel.
    """
    fake = mocker.MagicMock(name="cache_writer_metrics")
    mocker.patch.object(cache_writer_module, "cache_writer_metrics", fake)
    return fake


@pytest.fixture
def small_buffer(mocker):
    """Override the buffer cap on settings for overflow tests."""
    mocker.patch.object(_settings_module.settings, "cache_writer_buffer_cap", 2)


@pytest_asyncio.fixture
async def writer(mock_redis_client, fake_clock, metrics):
    """
    A started writer with a clean shutdown via drain_and_stop().
    """
    w = CacheWriter(SCOPE)
    w.start()
    try:
        yield w
    finally:
        await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)


async def _wait_until(predicate, timeout=1.0, step=0.001):
    """
    Poll ``predicate`` on the event loop until it returns truthy or
    the timeout fires. Used instead of fixed sleeps so tests don't
    encode wall-clock assumptions.
    """
    deadline = asyncio.get_event_loop().time() + timeout
    while asyncio.get_event_loop().time() < deadline:
        if predicate():
            return True
        await asyncio.sleep(step)
    return False


# ---------------------------------------------------------------------------
# Behaviors
# ---------------------------------------------------------------------------


class TestEnqueueSignature:

    def test_enqueue_is_synchronous_def_not_async(self):
        assert not inspect.iscoroutinefunction(CacheWriter.enqueue)


class TestEnqueue:

    pytestmark = pytest.mark.asyncio

    async def test_enqueue_returns_none_synchronously(
        self, writer, make_op, fake_clock
    ):
        result = writer.enqueue(make_op())

        assert result is None

    async def test_enqueue_does_not_await(self, writer, make_op):
        # If enqueue were a coroutine or returned an awaitable we'd
        # see it here; assert directly.
        ret = writer.enqueue(make_op())
        assert not inspect.isawaitable(ret)

    async def test_enqueue_succeeds_when_drainer_is_paused(
        self, mock_redis_client, fake_clock, metrics, make_op
    ):
        # Construct without starting; drainer never runs.
        w = CacheWriter(SCOPE)
        try:
            for _ in range(5):
                w.enqueue(make_op())
        finally:
            await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)


class TestDebounceFlush:

    pytestmark = pytest.mark.asyncio

    async def test_does_not_flush_before_debounce_elapses(
        self, writer, mock_pipeline, make_op, fake_clock
    ):
        for _ in range(3):
            writer.enqueue(make_op())

        # Yield to the drainer so it can pick up the first op and enter
        # the debounce wait. Time has not advanced; the deadline is
        # fixed at clock.now + 0.050 and we have NOT advanced.
        for _ in range(20):
            await asyncio.sleep(0)

        mock_pipeline.execute.assert_not_called()

    async def test_flushes_after_debounce_elapses(
        self, writer, mock_pipeline, make_op, fake_clock
    ):
        for _ in range(3):
            writer.enqueue(make_op())

        # Let the drainer pick up the first op.
        for _ in range(5):
            await asyncio.sleep(0)
        # Advance past the debounce deadline.
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)

        flushed = await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)

        assert flushed
        assert mock_pipeline.execute.await_count == 1


class TestCapFlush:

    pytestmark = pytest.mark.asyncio

    async def test_flushes_immediately_when_op_cap_reached(
        self, writer, mock_pipeline, make_op, fake_clock
    ):
        # Enqueue exactly cap ops without advancing the clock at all.
        ops = [make_op(f"op-{i}") for i in range(_BATCH_OP_CAP)]
        for op in ops:
            writer.enqueue(op)

        flushed = await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)

        assert flushed

    async def test_pipeline_contains_exactly_cap_ops_on_cap_flush(
        self, writer, mock_pipeline, make_op, fake_clock
    ):
        ops = [make_op(f"op-{i}") for i in range(_BATCH_OP_CAP)]
        for op in ops:
            writer.enqueue(op)

        await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)

        applied = sum(op.apply_to.call_count for op in ops)
        assert applied == _BATCH_OP_CAP


class TestPipelineFlush:

    pytestmark = pytest.mark.asyncio

    async def test_uses_single_pipeline_per_flush(
        self, writer, mock_redis_client, mock_pipeline, make_op, fake_clock
    ):
        for _ in range(5):
            writer.enqueue(make_op())
        for _ in range(5):
            await asyncio.sleep(0)
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)

        await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)

        assert mock_redis_client.pipeline.call_count == 1

    async def test_executes_pipeline_once_per_flush(
        self, writer, mock_pipeline, make_op, fake_clock
    ):
        for _ in range(5):
            writer.enqueue(make_op())
        for _ in range(5):
            await asyncio.sleep(0)
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)

        await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)

        assert mock_pipeline.execute.await_count == 1

    async def test_each_op_apply_to_invoked_exactly_once(
        self, writer, mock_pipeline, make_op, fake_clock
    ):
        ops = [make_op(f"op-{i}") for i in range(7)]
        for op in ops:
            writer.enqueue(op)
        for _ in range(5):
            await asyncio.sleep(0)
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)

        await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)

        for op in ops:
            op.apply_to.assert_called_once_with(mock_pipeline)


class TestDropOldestOnOverflow:

    pytestmark = pytest.mark.asyncio

    async def test_dropping_oldest_on_overflow_records_drop_metric(
        self, mock_redis_client, fake_clock, metrics, make_op, small_buffer
    ):
        w = CacheWriter(SCOPE)  # not started: drainer cannot drain
        try:
            w.enqueue(make_op("a"))
            w.enqueue(make_op("b"))
            w.enqueue(make_op("c"))  # overflow

            metrics.record_drop.assert_called_once_with(QUEUE_REF)
        finally:
            await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

    async def test_overflow_drops_oldest_op_from_pipeline(
        self,
        mock_redis_client,
        mock_pipeline,
        fake_clock,
        metrics,
        make_op,
        small_buffer,
    ):
        w = CacheWriter(SCOPE)  # buffer up before drainer can run
        op_a = make_op("a")
        op_b = make_op("b")
        op_c = make_op("c")
        w.enqueue(op_a)
        w.enqueue(op_b)
        w.enqueue(op_c)  # drops op_a

        w.start()
        for _ in range(5):
            await asyncio.sleep(0)
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)
        await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)
        await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

        op_a.apply_to.assert_not_called()
        op_b.apply_to.assert_called_once()
        op_c.apply_to.assert_called_once()

    async def test_no_drop_metric_on_happy_path(
        self, writer, metrics, make_op, fake_clock, mock_pipeline
    ):
        for _ in range(3):
            writer.enqueue(make_op())
        for _ in range(5):
            await asyncio.sleep(0)
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)
        await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)

        metrics.record_drop.assert_not_called()


class TestDrainAndStop:

    pytestmark = pytest.mark.asyncio

    async def test_pending_ops_are_flushed_before_returning(
        self, mock_redis_client, mock_pipeline, fake_clock, metrics, make_op
    ):
        w = CacheWriter(SCOPE)
        w.start()
        ops = [make_op(f"op-{i}") for i in range(4)]
        for op in ops:
            w.enqueue(op)

        # No pre-coordination: drain_and_stop must wait on its own
        # for the in-hand batch to flush before cancelling the drainer.
        await asyncio.wait_for(
            w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC),
            timeout=SHUTDOWN_MARGIN_SEC + 1.0,
        )

        assert mock_pipeline.execute.await_count >= 1
        for op in ops:
            op.apply_to.assert_called_once_with(mock_pipeline)

    async def test_drain_does_not_lose_in_flight_batch(
        self, mock_redis_client, mock_pipeline, fake_clock, metrics, make_op
    ):
        # Bug-shut: with a sub-cap, sub-debounce batch sitting between
        # the drainer's first buffer.get() and pipeline.execute, the
        # earlier _wait_for_empty() condition could see an empty buffer
        # + no in-flight pipeline and let drain_and_stop cancel the
        # drainer mid-batch. The _batch_in_flight flag added in the fix
        # closes that window. This test calls drain_and_stop with no
        # intervening yields, no clock advance, and no flush coordination
        # — every op must reach the pipeline before drain returns.
        n = 7
        ops = [make_op(f"op-{i}") for i in range(n)]
        w = CacheWriter(SCOPE)
        w.start()
        for op in ops:
            w.enqueue(op)

        await asyncio.wait_for(
            w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC),
            timeout=SHUTDOWN_MARGIN_SEC + 1.0,
        )

        assert mock_pipeline.execute.await_count == 1
        for op in ops:
            op.apply_to.assert_called_once_with(mock_pipeline)

    async def test_drainer_task_is_not_running_after_stop(
        self, mock_redis_client, fake_clock, metrics, make_op
    ):
        w = CacheWriter(SCOPE)
        w.start()
        task = w._drainer_task
        assert task is not None

        await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

        assert task.done()


class TestDrainAndStopTimeout:

    pytestmark = pytest.mark.asyncio

    async def test_returns_within_timeout_when_pipeline_hangs(
        self,
        mock_redis_client,
        mock_pipeline,
        fake_clock,
        metrics,
        make_op,
        mocker,
    ):
        # Replace execute with one that never resolves until cancelled.
        hang = asyncio.Event()
        cancelled = asyncio.Event()

        async def _hang():
            try:
                await hang.wait()
            except asyncio.CancelledError:
                cancelled.set()
                raise

        mock_pipeline.execute = mocker.AsyncMock(side_effect=_hang)

        w = CacheWriter(SCOPE)
        w.start()
        w.enqueue(make_op())
        # Let drainer pick op and start the (hung) pipeline.
        for _ in range(5):
            await asyncio.sleep(0)
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)
        await _wait_until(
            lambda: w._current_pipeline_task is not None
            and not w._current_pipeline_task.done()
        )

        # Bound the action with wait_for so a regression hangs the
        # test instead of producing a misleading pass.
        await asyncio.wait_for(w.drain_and_stop(timeout=0.05), timeout=2.0)

        assert cancelled.is_set()

    async def test_returns_without_raising_when_pipeline_hangs(
        self,
        mock_redis_client,
        mock_pipeline,
        fake_clock,
        metrics,
        make_op,
        mocker,
    ):
        hang = asyncio.Event()

        async def _hang():
            await hang.wait()

        mock_pipeline.execute = mocker.AsyncMock(side_effect=_hang)

        w = CacheWriter(SCOPE)
        w.start()
        w.enqueue(make_op())
        for _ in range(5):
            await asyncio.sleep(0)
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)

        # The action under test must not raise. wait_for guards the test.
        await asyncio.wait_for(w.drain_and_stop(timeout=0.05), timeout=2.0)


class TestDrainAndStopIdempotent:

    pytestmark = pytest.mark.asyncio

    async def test_second_call_is_no_op(
        self, mock_redis_client, fake_clock, metrics, make_op
    ):
        w = CacheWriter(SCOPE)
        w.start()
        await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

        # Second call should return cleanly without raising.
        await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

    async def test_second_call_does_not_flush_again(
        self, mock_redis_client, mock_pipeline, fake_clock, metrics, make_op
    ):
        w = CacheWriter(SCOPE)
        w.start()
        w.enqueue(make_op())
        for _ in range(5):
            await asyncio.sleep(0)
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)
        await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)
        first_flush_count = mock_pipeline.execute.await_count

        await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

        assert mock_pipeline.execute.await_count == first_flush_count


class TestRedisUnavailable:

    pytestmark = pytest.mark.asyncio

    async def test_drainer_does_not_raise_when_pipeline_execute_fails(
        self,
        mock_redis_client,
        mock_pipeline,
        fake_clock,
        metrics,
        make_op,
        mocker,
    ):
        mock_pipeline.execute = mocker.AsyncMock(
            side_effect=ConnectionError("redis down")
        )

        w = CacheWriter(SCOPE)
        w.start()
        w.enqueue(make_op())
        for _ in range(5):
            await asyncio.sleep(0)
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)
        await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)

        # Drainer must still be alive after a Redis failure.
        assert w._drainer_task is not None
        assert not w._drainer_task.done()

        await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

    async def test_continues_processing_after_recovery(
        self,
        mock_redis_client,
        mock_pipeline,
        fake_clock,
        metrics,
        make_op,
        mocker,
    ):
        # First attempt fails; subsequent succeed.
        outcomes = [ConnectionError("down"), None, None]

        async def _execute():
            r = outcomes.pop(0)
            if isinstance(r, Exception):
                raise r
            return r

        mock_pipeline.execute = mocker.AsyncMock(side_effect=_execute)

        w = CacheWriter(SCOPE)
        w.start()
        try:
            w.enqueue(make_op())
            for _ in range(5):
                await asyncio.sleep(0)
            fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)
            await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)

            # Second batch should succeed.
            w.enqueue(make_op())
            for _ in range(5):
                await asyncio.sleep(0)
            fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)

            ok = await _wait_until(lambda: mock_pipeline.execute.await_count >= 2)
            assert ok
        finally:
            await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

    async def test_logs_first_failure_and_periodically_after(
        self,
        mock_redis_client,
        mock_pipeline,
        fake_clock,
        metrics,
        make_op,
        mocker,
        caplog,
    ):
        mock_pipeline.execute = mocker.AsyncMock(side_effect=ConnectionError("down"))

        w = CacheWriter(SCOPE)
        w.start()
        try:
            with caplog.at_level(logging.ERROR, logger=cache_writer_module.__name__):
                # Drive 3 successive failed flushes; only the 1st should log.
                for _ in range(3):
                    w.enqueue(make_op())
                    for _ in range(5):
                        await asyncio.sleep(0)
                    fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)
                    prior = mock_pipeline.execute.await_count
                    await _wait_until(lambda: mock_pipeline.execute.await_count > prior)
        finally:
            await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

        flush_failed_records = [
            r for r in caplog.records if "Redis flush failed" in r.message
        ]
        # First failure logs; failures 2-3 are below the 50-failure
        # log interval and should be suppressed.
        assert len(flush_failed_records) == 1


class TestEnqueueDuringRedisHang:

    pytestmark = pytest.mark.asyncio

    async def test_enqueue_remains_unblocked_while_pipeline_hangs(
        self,
        mock_redis_client,
        mock_pipeline,
        fake_clock,
        metrics,
        make_op,
        mocker,
        small_buffer,
    ):
        hang = asyncio.Event()

        async def _hang():
            await hang.wait()

        mock_pipeline.execute = mocker.AsyncMock(side_effect=_hang)

        w = CacheWriter(SCOPE)
        w.start()
        try:
            # Force a flush attempt so the pipeline is in flight.
            w.enqueue(make_op("first"))
            for _ in range(5):
                await asyncio.sleep(0)
            fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)
            await _wait_until(
                lambda: w._current_pipeline_task is not None
                and not w._current_pipeline_task.done()
            )

            # With Redis hung, the drainer is parked on execute. Many
            # producer enqueues must still complete synchronously and
            # the buffer must start dropping the oldest op once full.
            for i in range(10):
                w.enqueue(make_op(f"hot-{i}"))

            assert metrics.record_drop.call_count >= 1
        finally:
            hang.set()
            await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)


class TestCancelledError:

    pytestmark = pytest.mark.asyncio

    async def test_drainer_task_exits_cleanly_on_cancellation(
        self, mock_redis_client, fake_clock, metrics, make_op
    ):
        w = CacheWriter(SCOPE)
        w.start()
        task = w._drainer_task
        assert task is not None

        task.cancel()
        try:
            await task
        except asyncio.CancelledError:
            pass

        assert task.done()
        # Cleanup so the writer's bookkeeping is consistent.
        await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

    async def test_no_pending_tasks_after_cancellation(
        self, mock_redis_client, fake_clock, metrics, make_op
    ):
        w = CacheWriter(SCOPE)
        w.start()

        await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

        leaked = [
            t for t in asyncio.all_tasks() if "cache-writer-drainer" in (t.get_name())
        ]
        assert leaked == []


class TestTelemetry:

    pytestmark = pytest.mark.asyncio

    async def test_queue_depth_recorded_by_drainer(
        self, writer, mock_pipeline, metrics, make_op, fake_clock
    ):
        writer.enqueue(make_op())
        for _ in range(5):
            await asyncio.sleep(0)
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)
        await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)

        # Drainer samples queue depth right before each flush.
        metrics.record_queue_depth.assert_called()

    async def test_queue_depth_never_recorded_from_enqueue(
        self, mock_redis_client, fake_clock, metrics, make_op
    ):
        # Construct without start so only enqueue runs (no drainer).
        w = CacheWriter(SCOPE)
        try:
            for _ in range(5):
                w.enqueue(make_op())

            metrics.record_queue_depth.assert_not_called()
        finally:
            await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

    async def test_flush_latency_recorded_on_each_successful_flush(
        self, writer, mock_pipeline, metrics, make_op, fake_clock
    ):
        for _ in range(3):
            writer.enqueue(make_op())
        for _ in range(5):
            await asyncio.sleep(0)
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)
        await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)

        # record_flush takes (queue_ref, ops_count, latency_ms); the
        # latency arg is a float histogram value, recorded once per flush.
        assert metrics.record_flush.call_count == 1
        args, _ = metrics.record_flush.call_args
        assert args[0] == QUEUE_REF
        assert args[1] == 3
        assert isinstance(args[2], float)

    async def test_flush_ops_counter_increments_by_batch_size(
        self, writer, mock_pipeline, metrics, make_op, fake_clock
    ):
        for _ in range(7):
            writer.enqueue(make_op())
        for _ in range(5):
            await asyncio.sleep(0)
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)
        await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)

        metrics.record_flush.assert_called_once()
        args, _ = metrics.record_flush.call_args
        assert args[1] == 7

    async def test_drops_counter_incremented_on_overflow(
        self, mock_redis_client, fake_clock, metrics, make_op, small_buffer
    ):
        w = CacheWriter(SCOPE)
        try:
            w.enqueue(make_op("a"))
            w.enqueue(make_op("b"))
            w.enqueue(make_op("c"))  # overflow -> 1 drop

            metrics.record_drop.assert_called_once_with(QUEUE_REF)
        finally:
            await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

    async def test_drops_counter_zero_on_happy_path(
        self, writer, mock_pipeline, metrics, make_op, fake_clock
    ):
        for _ in range(3):
            writer.enqueue(make_op())
        for _ in range(5):
            await asyncio.sleep(0)
        fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)
        await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)

        metrics.record_drop.assert_not_called()


# Sanity check: the module-level constants the tests pivot on are the
# ones the SDD calls out. If they change, the assertions about cap
# behavior / debounce semantics need to be revisited too.


class TestModuleConstants:

    def test_debounce_seconds_is_50ms(self):
        assert _DEBOUNCE_SECONDS == 0.050

    def test_batch_op_cap_is_100(self):
        assert _BATCH_OP_CAP == 100

    def test_redis_failure_log_interval_is_50(self):
        assert _REDIS_FAILURE_LOG_INTERVAL == 50


class TestShutdownReleasesPerScopeState:
    """
    On shutdown the writer must settle the gauge series to zero AND
    release the per-SCOPE baseline state held by the metrics singleton, so
    reconnects under the same scope don't see a stale baseline and so the
    singleton's _last_queue_depth dict doesn't grow unboundedly.

    The baseline is keyed on the scope key rather than on the exported
    ``queue_ref`` because two principals on one queue share that reference
    and would otherwise corrupt each other's last-sample state.
    """

    pytestmark = pytest.mark.asyncio

    async def test_drain_and_stop_records_final_queue_depth_zero(
        self, mock_redis_client, fake_clock, metrics, make_op
    ):
        w = CacheWriter(SCOPE)
        w.start()

        await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

        metrics.record_queue_depth.assert_any_call(QUEUE_REF, 0, SCOPE_KEY)

    async def test_drain_and_stop_calls_forget_queue_with_the_scope_key(
        self, mock_redis_client, fake_clock, metrics, make_op
    ):
        w = CacheWriter(SCOPE)
        w.start()

        await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

        metrics.forget_queue.assert_called_once_with(SCOPE_KEY)

    async def test_drain_and_stop_emits_depth_zero_before_forget_queue(
        self, mock_redis_client, fake_clock, metrics, make_op
    ):
        # forget_queue clears the per-scope baseline that
        # record_queue_depth's delta math relies on. If forget_queue
        # were called first, the depth=0 emission would be calculated
        # against a freshly-zeroed baseline and the gauge series would
        # be wrong on the next register/emit cycle for this scope.
        w = CacheWriter(SCOPE)
        w.start()

        await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

        depth_zero_idx = next(
            i
            for i, c in enumerate(metrics.mock_calls)
            if c[0] == "record_queue_depth" and c.args == (QUEUE_REF, 0, SCOPE_KEY)
        )
        forget_idx = next(
            i for i, c in enumerate(metrics.mock_calls) if c[0] == "forget_queue"
        )
        assert depth_zero_idx < forget_idx

    async def test_two_scopes_on_one_queue_release_separate_baselines(
        self, mock_redis_client, fake_clock, metrics, make_op
    ):
        # Each writer must release only its OWN baseline; releasing by
        # queue_ref would drop the other principal's live one.
        first = CacheWriter(SCOPE)
        second = CacheWriter(OTHER_SCOPE)
        first.start()
        second.start()

        await first.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)
        await second.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

        forgotten = [c.args[0] for c in metrics.forget_queue.call_args_list]

        assert forgotten == [SCOPE_KEY, OTHER_SCOPE.key]


# ---------------------------------------------------------------------------
# Public contract & context plumbing
# ---------------------------------------------------------------------------


class TestCacheWriterPublicContract:
    """
    DirectoryContents mutators rely on ``writer.scope_key`` being a
    public attribute derived from the constructor's scope so they can
    build Op instances without reaching into private state.
    """

    def test_scope_key_attribute_derived_from_constructor_scope(self):
        assert CacheWriter(SCOPE).scope_key == SCOPE.key

    def test_two_principals_on_one_queue_get_distinct_scope_keys(self):
        assert CacheWriter(SCOPE).scope_key != CacheWriter(OTHER_SCOPE).scope_key

    def test_two_principals_on_one_queue_share_the_exported_reference(self):
        # Deliberate: the exported attribute stays queue-only so both
        # writers' series join the same stream's spans by exact match.
        assert CacheWriter(SCOPE).queue_ref == CacheWriter(OTHER_SCOPE).queue_ref


class TestCacheWriterSecretHygiene:
    """
    A writer's queue name embeds the stream's reconnect token.

    Token hygiene: the token is the queue's identifier, kept out of
    telemetry as defense in depth. The name has to stay exact where Redis
    keys and the writer registry are built from it, and must never appear
    where it is exported: metric attributes, log lines, and the drainer's
    task name (which surfaces in ``asyncio`` diagnostics and in any task
    dump).
    """

    pytestmark = pytest.mark.asyncio

    async def test_the_scope_key_keeps_the_exact_broker_name(self):
        # The Redis keys and the registry are built from this; redacting it
        # would point the cache at a queue that does not exist.
        assert CacheWriter(SCOPE).scope_key.startswith(NON_DURABLE_QUEUE_NAME)

    async def test_the_exported_reference_is_the_bare_token_reference(self):
        # Bare, not wrapped back into `event-queue:...:nd`: this value has
        # to be byte-identical to the `queue_ref` span attribute the same
        # stream carries, or the metric series joins its trace by nothing.
        assert CacheWriter(SCOPE)._queue_ref == QUEUE_REF

    async def test_the_exported_reference_does_not_contain_the_token(self):
        assert RECONNECT_TOKEN not in CacheWriter(SCOPE)._queue_ref

    async def test_the_public_queue_ref_property_is_the_bare_reference(self):
        # `register_writer` and the non-durable consumer both log about a
        # writer they hold, through this property; it must name the stream
        # the same way the writer's own lines do.
        assert CacheWriter(SCOPE).queue_ref == QUEUE_REF

    async def test_the_public_queue_ref_property_does_not_contain_the_token(self):
        assert RECONNECT_TOKEN not in CacheWriter(SCOPE).queue_ref

    @pytest_asyncio.fixture
    async def exercised_metrics(
        self, mock_redis_client, fake_clock, metrics, make_op, small_buffer
    ):
        """
        Drive every metric call site the writer has -- a drop, a depth
        sample, and the shutdown pair -- and hand back the recorder.
        """
        w = CacheWriter(SCOPE)
        w.start()
        try:
            for _ in range(3):  # enough to overflow the 2-op buffer
                w.enqueue(make_op())
        finally:
            await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

        return metrics

    async def test_no_exported_metric_reference_carries_the_token(
        self, exercised_metrics
    ):
        # Every recorder takes the value it EXPORTS as an OTel attribute
        # first; that is the one that must be redacted. (The depth
        # baseline the writer passes alongside it is not exported -- see
        # the two tests below.)
        exported = [
            c.args[0]
            for c in exercised_metrics.mock_calls
            if c[0] in {"record_flush", "record_drop", "record_queue_depth"}
        ]

        assert [ref for ref in exported if RECONNECT_TOKEN in ref] == []

    async def test_every_exported_metric_reference_is_the_bare_reference(
        self, exercised_metrics
    ):
        exported = {
            c.args[0]
            for c in exercised_metrics.mock_calls
            if c[0] in {"record_flush", "record_drop", "record_queue_depth"}
        }

        assert exported == {QUEUE_REF}

    async def test_the_depth_baseline_is_the_scope_key(self, exercised_metrics):
        # The one argument that legitimately carries the raw name. It is
        # internal bookkeeping: `CacheWriterMetrics` uses it only as a
        # dict key and never as an attribute, which
        # ``tests/app/lib/observability/test_cache_writer_metrics.py``
        # pins from the other side.
        baselines = {
            c.args[2]
            for c in exercised_metrics.mock_calls
            if c[0] == "record_queue_depth"
        }

        assert baselines == {SCOPE_KEY}

    @pytest.fixture
    def redis_not_configured_records(self, mocker, caplog):
        """
        The INFO line ``start()`` emits when Redis is not configured.

        The only line this class writes on a HEALTHY deployment that has
        simply left Redis off -- so it is the one an operator is most likely
        to see, and the one most likely to be read by someone who is not
        looking for a leak.
        """

        def _run():
            mocker.patch.object(redis_client_module, "redis_client", None)

            with caplog.at_level(logging.INFO, logger=cache_writer_module.__name__):
                CacheWriter(SCOPE).start()

            return caplog.records

        return _run

    async def test_the_redis_not_configured_line_is_emitted(
        self, redis_not_configured_records
    ):
        # Guards the two guards below: an empty capture would make them
        # vacuous.
        assert redis_not_configured_records() != []

    async def test_the_redis_not_configured_line_names_the_reference(
        self, redis_not_configured_records
    ):
        records = redis_not_configured_records()

        assert any(QUEUE_REF in record.getMessage() for record in records)

    async def test_the_redis_not_configured_line_does_not_carry_the_token(
        self, redis_not_configured_records
    ):
        records = redis_not_configured_records()

        assert [r for r in records if RECONNECT_TOKEN in str(r.__dict__)] == []

    async def test_the_drainer_task_name_does_not_carry_the_token(
        self, mock_redis_client, fake_clock, metrics
    ):
        w = CacheWriter(SCOPE)
        w.start()
        try:
            assert RECONNECT_TOKEN not in w._drainer_task.get_name()
        finally:
            await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)

    @pytest.fixture
    def failed_flush_records(
        self,
        mock_redis_client,
        mock_pipeline,
        fake_clock,
        metrics,
        make_op,
        mocker,
        caplog,
    ):
        """
        Drive a failed flush and hand back what it logged.

        The Redis-failure path is chosen because that is where this module
        logs; a guard driven over the healthy path would capture nothing
        and pass no matter what the code did.
        """

        async def _run():
            mock_pipeline.execute = mocker.AsyncMock(
                side_effect=ConnectionError("down")
            )

            w = CacheWriter(SCOPE)
            w.start()
            try:
                with caplog.at_level(
                    logging.DEBUG, logger=cache_writer_module.__name__
                ):
                    w.enqueue(make_op())
                    for _ in range(5):
                        await asyncio.sleep(0)
                    fake_clock.advance(_DEBOUNCE_SECONDS + 0.01)
                    await _wait_until(lambda: mock_pipeline.execute.await_count >= 1)
            finally:
                await w.drain_and_stop(timeout=SHUTDOWN_MARGIN_SEC)
            return caplog.records

        return _run

    async def test_the_failure_path_logs_something_to_inspect(
        self, failed_flush_records
    ):
        # Guards the guard below: an empty capture would make it vacuous.
        assert await failed_flush_records() != []

    async def test_no_log_record_names_the_token(self, failed_flush_records):
        records = await failed_flush_records()

        assert [r for r in records if RECONNECT_TOKEN in str(r.__dict__)] == []


@pytest.fixture
def isolated_registry():
    """
    Snapshot and restore the module-level ``_writer_registry`` around
    each test. Without this, tests that register writers leak state into
    each other (and across xdist workers, since the registry is a
    plain module-level dict).
    """
    original = dict(cache_writer_module._writer_registry)
    cache_writer_module._writer_registry.clear()
    try:
        yield cache_writer_module._writer_registry
    finally:
        cache_writer_module._writer_registry.clear()
        cache_writer_module._writer_registry.update(original)


class TestRegisterWriterSecretHygiene:
    """
    Token hygiene: the reconnect token embedded in that name is the queue's
    identifier, kept out of telemetry as defense in depth.

    The registry is keyed on the raw scope key, which carries the
    non-durable broker name -- lookups have to match the value mutators bind
    to the ContextVar -- so the two things ``register_writer`` itself writes
    down are the ones that matter: the replace-drain task's name, which
    surfaces in any ``asyncio`` task dump, and the warning it logs when there
    is no loop to schedule that drain on. Both name the writer by its
    redacted reference.
    """

    @pytest.fixture
    def replacement(self, isolated_registry):
        """
        Register one writer for the scope and hand back a second, distinct
        writer for the SAME scope -- the replace path's precondition. Only a
        re-attach by the same principal can reach it now.
        """
        register_writer(CacheWriter(SCOPE))
        return CacheWriter(SCOPE)

    @pytest_asyncio.fixture
    async def replace_drain_task(self, replacement):
        """
        Drive the replace path and hand back the drain task it scheduled.

        Taken as a set difference because ``_pending_drain_tasks`` is a
        module-level anchor set, and awaited on teardown so the task's own
        done-callback gets to remove itself from it.
        """
        before = set(cache_writer_module._pending_drain_tasks)

        register_writer(replacement)

        (task,) = set(cache_writer_module._pending_drain_tasks) - before
        yield task
        await task

    @pytest.mark.asyncio
    async def test_the_replace_drain_task_is_named_by_the_reference(
        self, replace_drain_task
    ):
        expected = f"cache-writer-replace-drain:{QUEUE_REF}"

        assert replace_drain_task.get_name() == expected

    @pytest.mark.asyncio
    async def test_the_replace_drain_task_name_does_not_carry_the_token(
        self, replace_drain_task
    ):
        assert RECONNECT_TOKEN not in replace_drain_task.get_name()

    def test_the_no_event_loop_warning_names_the_reference(self, replacement, caplog):
        # Deliberately a SYNC test: with no running loop the replace path's
        # `asyncio.create_task` raises and the warning below is what runs
        # instead. It is the one line here that is only ever emitted when
        # something has already gone wrong, which is exactly when a leaked
        # token would go unnoticed.
        with caplog.at_level(logging.WARNING, logger=cache_writer_module.__name__):
            register_writer(replacement)

        assert QUEUE_REF in caplog.text

    def test_the_no_event_loop_warning_does_not_carry_the_token(
        self, replacement, caplog
    ):
        with caplog.at_level(logging.WARNING, logger=cache_writer_module.__name__):
            register_writer(replacement)

        assert RECONNECT_TOKEN not in caplog.text


class TestRegisterWriterDisposesAnUnschedulableDrain:
    """
    The drain coroutine that no loop will ever run is closed, not dropped.

    ``register_writer`` binds ``existing.drain_and_stop(timeout=1.0)``
    BEFORE ``create_task``, because the coroutine object exists as soon as
    the argument is evaluated. When there is no running loop and
    ``create_task`` raises, that coroutine has been created and will never
    be awaited -- so the interpreter emits a "coroutine ... was never
    awaited" RuntimeWarning at GC time, from wherever the collection
    happens to run. That is noise attached to an already-degraded path,
    and under ``-W error`` it is an unrelated test's failure.
    """

    @pytest.fixture
    def drain_coroutine(self, mocker, isolated_registry):
        """
        Drive the no-loop replace path and hand back the coroutine it
        created for the writer it could not drain.

        The prior writer's ``drain_and_stop`` is replaced with a plain
        async function (via ``new=``, not ``side_effect=``, so the call
        returns a real coroutine rather than an AsyncMock's) and every
        coroutine it hands out is captured.
        """
        created = []

        async def _drain(timeout=None):
            return None

        def _make_drain(*args, **kwargs):
            coro = _drain(*args, **kwargs)
            created.append(coro)
            return coro

        existing = CacheWriter(SCOPE)
        mocker.patch.object(existing, "drain_and_stop", new=_make_drain)
        register_writer(existing)

        # Deliberately a SYNC context: with no running loop the replace
        # path's ``asyncio.create_task`` raises, which is the branch under
        # test.
        register_writer(CacheWriter(SCOPE))

        (coro,) = created
        return coro

    def test_the_unschedulable_coroutine_is_closed(self, drain_coroutine):
        assert inspect.getcoroutinestate(drain_coroutine) == inspect.CORO_CLOSED

    def test_the_replacement_still_takes_over_the_registry(self, drain_coroutine):
        # The prior writer is orphaned, but the new one must supersede it
        # regardless -- closing the coroutine is cleanup, not a bail-out.
        assert get_writer_for_queue(SCOPE_KEY) is not None


class TestUnregisterWriterIdentityGuard:
    """
    ``unregister_writer`` takes the WRITER, and removes the entry only if
    that writer still owns it.

    The entry belongs to a writer, not to the key. A same-scope re-attach
    replaces the entry and the replaced writer's own cleanup path runs
    afterwards; popping by key there would remove the LIVE successor,
    after which ``active_writer()`` returns ``None`` in the successor's
    context and its write-through is silently lost.
    """

    pytestmark = pytest.mark.asyncio

    @pytest.fixture
    def first_writer(self, mock_redis_client, isolated_registry):
        return CacheWriter(SCOPE)

    @pytest.fixture
    def successor(self, mock_redis_client, isolated_registry):
        return CacheWriter(SCOPE)

    async def test_unregistering_the_replaced_writer_leaves_the_successor(
        self, first_writer, successor
    ):
        register_writer(first_writer)
        register_writer(successor)

        unregister_writer(first_writer)

        assert get_writer_for_queue(SCOPE_KEY) is successor

    async def test_unregistering_the_successor_removes_the_entry(
        self, first_writer, successor
    ):
        register_writer(first_writer)
        register_writer(successor)

        unregister_writer(successor)

        assert get_writer_for_queue(SCOPE_KEY) is None

    async def test_unregistering_a_never_registered_writer_is_a_no_op(
        self, first_writer, successor
    ):
        register_writer(first_writer)

        unregister_writer(successor)

        assert get_writer_for_queue(SCOPE_KEY) is first_writer

    async def test_unregistering_with_an_empty_registry_does_not_raise(
        self, first_writer
    ):
        unregister_writer(first_writer)

        assert get_writer_for_queue(SCOPE_KEY) is None

    async def test_unregistering_one_principals_writer_leaves_the_others(
        self, mock_redis_client, isolated_registry
    ):
        # The two writers of one queue's two principals live under
        # different keys, so one stream's cleanup must not disturb the
        # other's.
        mine = CacheWriter(SCOPE)
        theirs = CacheWriter(OTHER_SCOPE)
        register_writer(mine)
        register_writer(theirs)

        unregister_writer(mine)

        assert get_writer_for_queue(OTHER_SCOPE.key) is theirs


class TestCacheWriterContextPlumbing:
    """
    Direct unit coverage for the registry + ContextVar helpers. These
    are exercised transitively by the consumer-lifecycle tests, but the
    defensive ``register_writer`` replace path and ``active_writer``
    short-circuits need lockdown tests so a future refactor cannot
    silently regress them.

    The registry and the ContextVar are keyed on a scope key -- the queue
    name plus the presenting principal's reference -- so each writer below
    is built from a real scope rather than a bare label.
    """

    pytestmark = pytest.mark.asyncio

    @pytest.fixture(autouse=True)
    def redis_enabled(self, mocker):
        # ``active_writer()`` short-circuits to None when redis is
        # disabled. Most of these tests need it enabled.
        mocker.patch.object(_settings_module.settings, "redis_enabled", True)

    @pytest.fixture
    def scope_q1(self):
        return scope_for("q1")

    @pytest.fixture
    def scope_qa(self):
        return scope_for("q-a")

    @pytest.fixture
    def scope_qb(self):
        return scope_for("q-b")

    async def test_register_then_get_returns_same_writer(
        self, mock_redis_client, isolated_registry, scope_q1
    ):
        w = CacheWriter(scope_q1)

        register_writer(w)

        assert get_writer_for_queue(scope_q1.key) is w

    async def test_register_writer_keys_the_entry_on_the_writers_scope_key(
        self, mock_redis_client, isolated_registry, scope_q1
    ):
        # The key is the writer's to supply: passing one alongside it
        # would make a key/writer mismatch representable, and a mismatched
        # pair registers a writer where no lookup can reach it.
        w = CacheWriter(scope_q1)

        register_writer(w)

        assert isolated_registry == {w.scope_key: w}

    async def test_get_writer_for_queue_returns_none_when_unregistered(
        self, isolated_registry
    ):
        assert get_writer_for_queue("never-registered") is None

    async def test_unregister_removes_writer(
        self, mock_redis_client, isolated_registry, scope_q1
    ):
        w = CacheWriter(scope_q1)
        register_writer(w)

        unregister_writer(w)

        assert get_writer_for_queue(scope_q1.key) is None

    async def test_register_writer_replaces_and_drains_prior(
        self, mock_redis_client, isolated_registry, mocker, scope_q1
    ):
        """
        Lockdown for the most subtle invariant: re-registering for the
        same SCOPE must drain-and-stop the prior writer so its drainer
        task does not leak. Only a re-attach by the same principal can
        reach this path -- a different principal has a different scope
        key and registers alongside.
        """
        w1 = CacheWriter(scope_q1)
        # Spy on drain_and_stop instead of the bound coroutine; the
        # registry schedules an asyncio.create_task on the result, so
        # the spy must return an awaitable.
        drain_spy = mocker.AsyncMock(name="w1.drain_and_stop")
        w1.drain_and_stop = drain_spy

        w2 = CacheWriter(scope_q1)

        register_writer(w1)
        register_writer(w2)

        # The new writer is now installed.
        assert get_writer_for_queue(scope_q1.key) is w2

        # The drain task fires on the next loop iteration.
        for _ in range(5):
            await asyncio.sleep(0)

        drain_spy.assert_awaited_once()
        # The bounded shutdown timeout must be passed (SDD value).
        assert drain_spy.await_args.kwargs.get("timeout") == 1.0

    async def test_register_same_writer_twice_does_not_drain_itself(
        self, mock_redis_client, isolated_registry, mocker, scope_q1
    ):
        """
        Re-registering the SAME writer (identity-equal) must not drain
        it — the registry only drains when replacing with a different
        writer instance.
        """
        w = CacheWriter(scope_q1)
        drain_spy = mocker.AsyncMock(name="w.drain_and_stop")
        w.drain_and_stop = drain_spy

        register_writer(w)
        register_writer(w)

        for _ in range(5):
            await asyncio.sleep(0)

        drain_spy.assert_not_awaited()

    async def test_set_active_queue_id_returns_token(self, isolated_registry):
        token = set_active_queue_id("q-active")
        try:
            # Token is opaque; just assert non-None and use-shape.
            assert token is not None
        finally:
            reset_active_queue_id(token)

    async def test_reset_active_queue_id_restores_prior_value(
        self, mock_redis_client, isolated_registry, scope_qa, scope_qb
    ):
        # Establish a baseline value, then assert reset takes us back.
        outer_token = set_active_queue_id(scope_qa.key)
        try:
            inner_token = set_active_queue_id(scope_qb.key)

            # Use active_writer as the observable: register a writer for
            # the inner scope and confirm it is selected.
            w_inner = CacheWriter(scope_qb)
            register_writer(w_inner)
            assert active_writer() is w_inner

            reset_active_queue_id(inner_token)

            # After reset, active_writer should look up against the
            # outer scope key, which has no registered writer.
            assert active_writer() is None
        finally:
            reset_active_queue_id(outer_token)

    async def test_active_writer_returns_none_when_redis_disabled(
        self, isolated_registry, mocker, scope_q1
    ):
        mocker.patch.object(_settings_module.settings, "redis_enabled", False)
        # Even with a registered writer + bound scope key, redis-disabled
        # short-circuits to None.
        w = CacheWriter(scope_q1)
        register_writer(w)
        token = set_active_queue_id(scope_q1.key)
        try:
            assert active_writer() is None
        finally:
            reset_active_queue_id(token)

    async def test_active_writer_returns_none_when_context_var_unset(
        self, mock_redis_client, isolated_registry, scope_q1
    ):
        # Registering a writer is not enough — the ContextVar must be
        # bound for the auth-check path to find it.
        w = CacheWriter(scope_q1)
        register_writer(w)

        # No set_active_queue_id call → ContextVar default is None.
        assert active_writer() is None

    async def test_active_writer_returns_none_when_scope_key_unregistered(
        self, mock_redis_client, isolated_registry
    ):
        # ContextVar bound but no writer in the registry for that scope.
        token = set_active_queue_id("not-in-registry")
        try:
            assert active_writer() is None
        finally:
            reset_active_queue_id(token)

    async def test_active_writer_does_not_raise_under_any_short_circuit(
        self, isolated_registry, mocker
    ):
        # Belt-and-suspenders: per the docstring contract, active_writer
        # NEVER raises. Drive each short-circuit in turn.
        mocker.patch.object(_settings_module.settings, "redis_enabled", False)
        active_writer()  # redis disabled

        mocker.patch.object(_settings_module.settings, "redis_enabled", True)
        active_writer()  # ContextVar unset

        token = set_active_queue_id("missing")
        try:
            active_writer()  # registry miss
        finally:
            reset_active_queue_id(token)

    async def test_active_writer_finds_registered_writer_for_bound_scope_key(
        self, mock_redis_client, isolated_registry, scope_q1
    ):
        w = CacheWriter(scope_q1)
        register_writer(w)
        token = set_active_queue_id(scope_q1.key)
        try:
            assert active_writer() is w
        finally:
            reset_active_queue_id(token)

    async def test_active_writer_selects_by_principal_on_one_queue(
        self, mock_redis_client, isolated_registry
    ):
        # The isolation the scope exists for, at the lookup layer: two
        # principals on ONE queue must each reach their own writer.
        mine = CacheWriter(SCOPE)
        theirs = CacheWriter(OTHER_SCOPE)
        register_writer(mine)
        register_writer(theirs)

        token = set_active_queue_id(OTHER_SCOPE.key)
        try:
            assert active_writer() is theirs
        finally:
            reset_active_queue_id(token)

    async def test_context_var_isolation_across_concurrent_tasks(
        self, mock_redis_client, isolated_registry, scope_qa, scope_qb
    ):
        """
        Two concurrent asyncio tasks each binding a different scope key
        must not see each other's binding via active_writer(). Lockdown
        for the ContextVar contract: per-context, not global.
        """
        w_a = CacheWriter(scope_qa)
        w_b = CacheWriter(scope_qb)
        register_writer(w_a)
        register_writer(w_b)

        observed: dict[str, CacheWriter | None] = {}
        ready = asyncio.Event()
        proceed = asyncio.Event()

        async def task_a():
            token = set_active_queue_id(scope_qa.key)
            try:
                ready.set()
                await proceed.wait()
                observed["a"] = active_writer()
            finally:
                reset_active_queue_id(token)

        async def task_b():
            token = set_active_queue_id(scope_qb.key)
            try:
                await ready.wait()
                proceed.set()
                observed["b"] = active_writer()
            finally:
                reset_active_queue_id(token)

        await asyncio.gather(task_a(), task_b())

        assert observed["a"] is w_a
        assert observed["b"] is w_b
