# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Pins the contract the bidi RPCs rely on from ``merge`` (C1-C6 in its
docstring), one class per behavior. The tests are written to fail against the
designs that would quietly break that contract: a fan-in that drains sources
into a queue, a task group, strict source-order priority, and a cleanup that
does not cancel the outstanding reads first.
"""

import asyncio
import contextvars
import gc
from contextlib import aclosing

import pytest

from app.lib.streams import merge

# Bounds a wait that a broken implementation would leave hanging, so the test
# fails with a timeout instead of stalling the run. A correct implementation
# settles within a few event-loop iterations.
HANG_GUARD_S = 1.0

# How many items a test takes before it samples or stops.
ITEMS_TAKEN = 3

# Long enough that a design which drains a source ahead of the consumer is
# visibly ahead, and finite so that such a design ends instead of spinning: a
# source that never suspends would otherwise starve the event loop forever.
FAST_SOURCE_LENGTH = 10

_observed_by_reads = contextvars.ContextVar("observed_by_reads", default="unset")


class SourceError(Exception):
    """Raised by a source, standing in for a dead channel."""


class SecondSourceError(Exception):
    """Raised by a second source that fails in the same step as the first."""


class BodyError(Exception):
    """Raised by the consumer's loop body, as ``update_queue`` can raise."""


class CloseError(Exception):
    """Raised from a source's ``finally`` while the merge closes it."""


async def _forever():
    await asyncio.get_running_loop().create_future()


async def _forever_source():
    await _forever()
    yield  # pragma: no cover


async def _failing_source(error):
    raise error
    yield  # pragma: no cover


async def _items(*items):
    for item in items:
        yield item


async def _items_then_wait(name, closed, *items):
    try:
        for item in items:
            yield item
        await _forever()
    finally:
        closed.add(name)


async def _collect(sources):
    async with aclosing(merge(*sources)) as items:
        return [item async for item in items]


def _background(coroutine):
    """
    Run a consumer as its own task, marking its outcome as retrieved.

    A broken merge can fail a consumer that the test only cancels and then
    inspects for side effects. Without the retrieval, asyncio would log that
    failure as "never retrieved" when the task is collected, which can land
    in a later test's ``caplog`` and fail it for the wrong reason.
    """
    task = asyncio.ensure_future(coroutine)
    task.add_done_callback(_retrieve_outcome)
    return task


def _retrieve_outcome(task):
    if not task.cancelled():
        task.exception()


async def _take(count, *sources):
    taken = []
    async with aclosing(merge(*sources)) as items:
        async for item in items:
            taken.append(item)
            if len(taken) == count:
                break
    return taken


class _PullCounter:
    """A source that never suspends and counts how often it has been read."""

    def __init__(self):
        self.pulls = 0

    async def source(self):
        for _ in range(FAST_SOURCE_LENGTH):
            self.pulls += 1
            yield self.pulls


class _ExhaustAfterOneIterator:
    """A class-based iterator that records when the merge closes it."""

    def __init__(self, item):
        self._items = [item]
        self.closed = asyncio.Event()

    def __aiter__(self):
        return self

    async def __anext__(self):
        if not self._items:
            raise StopAsyncIteration
        return self._items.pop()

    async def aclose(self):
        self.closed.set()


class _NoAcloseIterator:
    """A class-based async iterator with ``__anext__`` but no ``aclose``."""

    def __init__(self, *items):
        self._items = list(items)

    def __aiter__(self):
        return self

    async def __anext__(self):
        if not self._items:
            raise StopAsyncIteration
        return self._items.pop(0)


class _IterableOnly:
    """An object that is only ``AsyncIterable``: it has ``__aiter__`` alone."""

    def __init__(self, *items):
        self._items = items

    def __aiter__(self):
        return _items(*self._items)


class TestMergeReadsOneItemAheadPerSource:
    """
    C1: a source is read again only after the consumer resumes the merge.

    A design that drains sources into a queue fails both tests. One that
    schedules a source's next read before yielding its current item passes
    the first and fails the second, because the read runs at the body's
    first await.
    """

    @pytest.mark.asyncio
    async def test_source_is_pulled_exactly_k_times_when_its_kth_item_arrives(self):
        counter = _PullCounter()
        pulls_on_entry = []

        async with asyncio.timeout(HANG_GUARD_S):
            async with aclosing(merge(counter.source(), _forever_source())) as items:
                async for item in items:
                    pulls_on_entry.append(counter.pulls)
                    if item == ITEMS_TAKEN:
                        break

        assert pulls_on_entry == list(range(1, ITEMS_TAKEN + 1))

    @pytest.mark.asyncio
    async def test_an_await_in_the_body_does_not_pull_the_source_again(self):
        counter = _PullCounter()
        pulls_after_await = []

        async with asyncio.timeout(HANG_GUARD_S):
            async with aclosing(merge(counter.source(), _forever_source())) as items:
                async for item in items:
                    await asyncio.sleep(0)
                    await asyncio.sleep(0)
                    pulls_after_await.append(counter.pulls)
                    if item == ITEMS_TAKEN:
                        break

        assert pulls_after_await == list(range(1, ITEMS_TAKEN + 1))


class TestMergeSourceErrorsEscapeUnwrapped:
    """
    C2: ``_dead_channel_mapped`` catches a source error with a plain except.

    ``pytest.raises(SourceError)`` does not match an ``ExceptionGroup`` that
    wraps one, so the test also rules out the wrap a task group would add.
    """

    @pytest.mark.asyncio
    async def test_source_exception_escapes_as_the_same_object(self):
        error = SourceError("channel closed")

        with pytest.raises(SourceError) as exc_info:
            async with asyncio.timeout(HANG_GUARD_S):
                await _collect([_failing_source(error), _forever_source()])

        assert exc_info.value is error


class TestMergeCleansUpOnEveryExit:
    """
    C3: every source's ``finally`` has run by the time the ``async with``
    exits, whichever way it exits.

    Most tests here merge two sources. While the consumer is in its loop body,
    source ``idle`` has delivered its item and sits at its ``yield``, and
    source ``pending`` has an outstanding read that never completes. The
    merge must cancel the pending read, because ``aclose`` on a generator
    mid-read raises ``RuntimeError``, and ``aclose`` the idle one. Where the
    consumer runs in the test's own task, the ``closed`` assertion follows the
    ``async with`` with no await between, so nothing else can have run first.
    """

    @staticmethod
    def _sources(closed):
        return (
            _items_then_wait("idle", closed, "item"),
            _items_then_wait("pending", closed),
        )

    @pytest.mark.asyncio
    async def test_consumer_break_closes_every_source(self):
        closed = set()

        async with asyncio.timeout(HANG_GUARD_S):
            async with aclosing(merge(*self._sources(closed))) as items:
                async for _ in items:
                    break

        assert closed == {"idle", "pending"}

    @pytest.mark.asyncio
    async def test_body_exception_closes_every_source(self):
        closed = set()

        with pytest.raises(BodyError):
            async with asyncio.timeout(HANG_GUARD_S):
                async with aclosing(merge(*self._sources(closed))) as items:
                    async for _ in items:
                        raise BodyError

        assert closed == {"idle", "pending"}

    @pytest.mark.asyncio
    async def test_source_error_closes_the_other_sources(self):
        closed = set()
        sources = [_failing_source(SourceError()), _items_then_wait("pending", closed)]

        with pytest.raises(SourceError):
            async with asyncio.timeout(HANG_GUARD_S):
                await _collect(sources)

        assert closed == {"pending"}

    @classmethod
    async def _cancel_consumer(cls, closed, *, in_body):
        """
        Cancel a consumer after its first item and return the task, plus the
        set of closed sources sampled the instant its ``async with`` exited.

        With ``in_body`` the consumer is awaiting inside its loop body when it
        is cancelled; without it, the consumer has resumed the merge, which is
        waiting on both sources' reads.
        """
        delivered = asyncio.Event()
        closed_at_exit = []

        async def consume():
            try:
                async with aclosing(merge(*cls._sources(closed))) as items:
                    async for _ in items:
                        delivered.set()
                        if in_body:
                            await _forever()
            finally:
                closed_at_exit.append(set(closed))

        consumer = _background(consume())
        await asyncio.wait_for(delivered.wait(), timeout=HANG_GUARD_S)
        await asyncio.sleep(0)
        consumer.cancel()
        await asyncio.wait({consumer}, timeout=HANG_GUARD_S)
        return consumer, closed_at_exit

    @pytest.mark.asyncio
    @pytest.mark.parametrize("in_body", [True, False], ids=["in_body", "in_merge"])
    async def test_cancellation_of_the_consuming_task_propagates(self, in_body):
        consumer, _ = await self._cancel_consumer(set(), in_body=in_body)

        with pytest.raises(asyncio.CancelledError):
            consumer.result()

    @pytest.mark.asyncio
    @pytest.mark.parametrize("in_body", [True, False], ids=["in_body", "in_merge"])
    async def test_cancellation_of_the_consuming_task_closes_every_source(
        self, in_body
    ):
        closed = set()

        _, closed_at_exit = await self._cancel_consumer(closed, in_body=in_body)

        assert closed_at_exit == [{"idle", "pending"}]

    @staticmethod
    def _ready_together(close_order, raising_on_close=None):
        """
        Two sources whose first reads complete in the same step. A consumer
        that stops on the first item leaves both sitting at their ``yield``,
        so the merge closes both with ``aclose``.
        """

        async def source(name):
            try:
                yield name
                await _forever()
            finally:
                close_order.append(name)
                if name == raising_on_close:
                    raise CloseError

        return source("first"), source("second")

    @pytest.mark.asyncio
    async def test_open_sources_are_closed_in_reverse_source_order(self):
        close_order = []

        await _take(1, *self._ready_together(close_order))

        assert close_order == ["second", "first"]

    @pytest.mark.asyncio
    async def test_a_source_whose_close_raises_does_not_stop_the_others_closing(
        self,
    ):
        close_order = []

        with pytest.raises(CloseError):
            await _take(1, *self._ready_together(close_order, "second"))

        assert close_order == ["second", "first"]


class TestMergeSecondCancellationDuringCleanup:
    """
    C3: a cancellation that lands while the merge waits for its cancelled
    reads still propagates. The source whose read is still running must be
    left to that cancellation: ``aclose`` on it raises ``RuntimeError``,
    which would replace the ``CancelledError``.
    """

    @pytest.mark.asyncio
    async def test_second_cancellation_propagates_as_cancelled_error(self):
        reading = asyncio.Event()
        cleanup_started = asyncio.Event()
        finish_cleanup = asyncio.Event()
        read_tasks = []

        async def slow_to_clean_up():
            try:
                reading.set()
                await _forever()
                yield  # pragma: no cover
            finally:
                read_tasks.append(asyncio.current_task())
                cleanup_started.set()
                await finish_cleanup.wait()

        consumer = _background(_collect([slow_to_clean_up()]))
        try:
            await asyncio.wait_for(reading.wait(), timeout=HANG_GUARD_S)
            consumer.cancel()
            await asyncio.wait_for(cleanup_started.wait(), timeout=HANG_GUARD_S)
            consumer.cancel()
            await asyncio.wait({consumer}, timeout=HANG_GUARD_S)
        finally:
            # Released on every path, so a failing run cannot leave the read
            # parked in its cleanup and hang the event loop's teardown.
            finish_cleanup.set()
            if read_tasks:
                await asyncio.wait(read_tasks, timeout=HANG_GUARD_S)

        with pytest.raises(asyncio.CancelledError):
            consumer.result()


class TestMergeExhaustion:
    """C4: an exhausted source is closed at once and the others continue."""

    @pytest.mark.asyncio
    async def test_exhausted_source_is_closed_while_its_peer_is_still_open(self):
        exhausted = _ExhaustAfterOneIterator("a1")
        release_peer = asyncio.Event()

        async def peer():
            await release_peer.wait()
            yield "b1"

        consumer = _background(_collect([exhausted, peer()]))
        try:
            await asyncio.wait_for(exhausted.closed.wait(), timeout=HANG_GUARD_S)
            merge_still_running = not consumer.done()
        finally:
            release_peer.set()
            await asyncio.wait_for(consumer, timeout=HANG_GUARD_S)

        assert merge_still_running

    @pytest.mark.asyncio
    async def test_peer_items_after_an_exhaustion_are_all_delivered(self):
        released = asyncio.Event()

        async def release_after_first_item():
            yield "a1"
            released.set()

        async def peer():
            await released.wait()
            yield "b1"
            yield "b2"

        items = await asyncio.wait_for(
            _collect([release_after_first_item(), peer()]), timeout=HANG_GUARD_S
        )

        assert items == ["a1", "b1", "b2"]

    @pytest.mark.asyncio
    async def test_merge_ends_when_every_source_is_exhausted(self):
        items = await asyncio.wait_for(
            _collect([_items("a1"), _items("b1", "b2"), _items()]),
            timeout=HANG_GUARD_S,
        )

        assert sorted(items) == ["a1", "b1", "b2"]

    @pytest.mark.asyncio
    async def test_merge_of_no_sources_yields_nothing(self):
        assert await _collect([]) == []


class TestMergeRunsEachReadInTheConsumersContext:
    """
    C5: each read is a fresh task copying the consumer's context when it is
    scheduled, which is how the RPC's tracing span reaches the reads. A
    long-lived task per source fails both tests.
    """

    @pytest.mark.asyncio
    async def test_each_read_sees_the_value_the_consumer_set_before_it(self):
        seen = []

        async def recording():
            for read in range(1, ITEMS_TAKEN + 1):
                seen.append(_observed_by_reads.get())
                yield read

        token = _observed_by_reads.set("before-1")
        try:
            async with aclosing(merge(recording())) as items:
                async for item in items:
                    _observed_by_reads.set(f"before-{item + 1}")
        finally:
            _observed_by_reads.reset(token)

        assert seen == [f"before-{read}" for read in range(1, ITEMS_TAKEN + 1)]

    @pytest.mark.asyncio
    async def test_a_value_a_source_sets_does_not_persist_to_its_next_read(self):
        seen = []

        async def setting():
            for read in range(1, ITEMS_TAKEN + 1):
                seen.append(_observed_by_reads.get())
                _observed_by_reads.set(f"set-by-read-{read}")
                yield read

        await _collect([setting()])

        assert seen == ["unset"] * ITEMS_TAKEN


class TestMergeTieBreak:
    """C6: ready reads are served in schedule order, never by fixed priority."""

    @pytest.mark.asyncio
    async def test_reads_ready_on_the_first_round_arrive_in_source_order(self):
        items = await _collect([_items("a"), _items("b"), _items("c")])

        assert items == ["a", "b", "c"]

    @pytest.mark.asyncio
    async def test_an_always_ready_source_does_not_starve_a_peer(self):
        async def always_ready():
            for count in range(1, FAST_SOURCE_LENGTH + 1):
                yield f"a{count}"

        async def ready_after_one_turn():
            await asyncio.sleep(0)
            yield "b"
            await _forever()

        items = await asyncio.wait_for(
            _take(ITEMS_TAKEN, always_ready(), ready_after_one_turn()),
            timeout=HANG_GUARD_S,
        )

        assert items == ["a1", "b", "a2"]


class TestMergeRetrievesEveryReadOutcome:
    """
    An outcome the merge never consumes must not log as "Task exception was
    never retrieved".

    A read that was already done when the merge cancelled it is silenced by
    the cancel itself, which clears asyncio's log flag even on a finished
    task. A read that fails while it is being cancelled sets that flag again
    afterwards, and only the merge's done-callback retrieves it.
    """

    @pytest.mark.asyncio
    async def test_two_sources_failing_together_log_no_unretrieved_exception(
        self, caplog
    ):
        sources = [_failing_source(SourceError()), _failing_source(SecondSourceError())]

        with pytest.raises(SourceError):
            await _collect(sources)
        gc.collect()

        assert "never retrieved" not in caplog.text

    @pytest.mark.asyncio
    async def test_a_read_failing_while_cancelled_logs_no_unretrieved_exception(
        self, caplog
    ):
        async def fails_when_cancelled():
            try:
                await _forever()
                yield  # pragma: no cover
            finally:
                raise CloseError

        sources = [_failing_source(SourceError()), fails_when_cancelled()]

        with pytest.raises(SourceError):
            async with asyncio.timeout(HANG_GUARD_S):
                await _collect(sources)
        gc.collect()

        assert "never retrieved" not in caplog.text


class TestMergeAcceptsAnyAsyncIterable:
    """``merge`` takes async iterables, not only async generators."""

    @pytest.mark.asyncio
    async def test_iterator_without_aclose_is_merged_to_exhaustion(self):
        items = await _collect([_NoAcloseIterator("x1", "x2"), _items("g1")])

        assert sorted(items) == ["g1", "x1", "x2"]

    @pytest.mark.asyncio
    async def test_iterator_without_aclose_is_skipped_on_early_exit(self):
        items = await _take(1, _NoAcloseIterator("x1", "x2"), _items("g1"))

        assert items == ["x1"]

    @pytest.mark.asyncio
    async def test_object_that_is_only_async_iterable_is_merged(self):
        items = await _collect([_IterableOnly("i1", "i2")])

        assert items == ["i1", "i2"]
