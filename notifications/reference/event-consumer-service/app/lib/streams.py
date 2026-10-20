# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Async stream composition for the bidirectional RPCs.

Each bidi RPC fans an event-producing source and a client-request listener
into one loop with ``merge``. The callers depend on the exact contract stated
on ``merge``, so read it before changing either side.
"""

import asyncio
import contextlib
from typing import AsyncGenerator, AsyncIterable, AsyncIterator


async def merge[T](*sources: AsyncIterable[T]) -> AsyncGenerator[T, None]:
    """
    Yield items from every source as they become available.

    Use it as ``async with contextlib.aclosing(merge(a, b)) as items:`` so the
    cleanup below runs when the ``async with`` exits rather than whenever the
    garbage collector finalizes the generator.

    The contract, which the callers rely on:

    * **C1 One read ahead per source.** Each source has at most one
      outstanding read. A source's next read is scheduled only after the
      consumer resumes the merge, that is, after the ``yield`` that delivered
      its previous item returns. So no source is read more than one item
      ahead of the consumer: a filter update is applied before the next
      client request is read.
    * **C2 Errors unwrapped.** A source's exception leaves the merge as the
      same object, never wrapped in an ``ExceptionGroup``. The servicer's
      ``_dead_channel_mapped`` catches a dead-channel error with a plain
      ``except`` and depends on this.
    * **C3 Cleanup on every exit.** On exhaustion, a source error, the
      consumer's exception or ``aclose()``, and cancellation of the consuming
      task, the merge (1) cancels every outstanding read, (2) waits for those
      reads with ``asyncio.wait``, which never raises a read's outcome, so a
      further cancellation of the consuming task still propagates, and (3)
      ``aclose()``s every open source in reverse source order, attempting
      every one even if one raises. A source with an outstanding read is
      therefore torn down by cancelling that read, inside the merge, before
      the error reaches any wrapper above it. A source whose cancelled read
      is still running (only possible when the wait in (2) was itself
      cancelled) is left to that cancellation, because ``aclose()`` on a
      running async generator raises ``RuntimeError``. A source with no
      ``aclose`` is skipped.
    * **C4 Exhaustion.** An exhausted source is closed at once and the others
      continue. The merge ends when every source is exhausted.
    * **C5 Each read in its own task.** Each read is a task created from the
      consumer's context when it is scheduled, so a context variable the
      consumer set (the RPC's tracing span among them) is visible in the
      read, and one a source sets during a read does not persist to that
      source's next read. There is no long-lived per-source task and no
      shared queue.
    * **C6 Tie-break by schedule order.** Among reads that are done when the
      merge wakes, the one scheduled earliest is delivered. On the first
      round that is source order; after it, a source with an item ready is
      passed over for at most one item per other ready source, so a source
      whose reads complete without suspending cannot starve the others.
      There is no other ordering between sources.

    Every read's outcome is retrieved by a done-callback, so an outcome the
    merge never consumes is discarded rather than logged as "Task exception
    was never retrieved". A read that was already done when the merge
    cancelled it needs no callback, because the cancel itself clears asyncio's
    log flag. The callback is what covers a read that fails while it is being
    cancelled, such as one whose source's cleanup raises after the merge
    cancelled the read.

    Do not replace this with an ``asyncio.TaskGroup`` or an anyio task group
    feeding a queue: a task group raises ``ExceptionGroup``, which breaks C2
    and lets a dead channel bypass ``_dead_channel_mapped``, and a ``yield``
    inside a task group inside an async generator lets cancellation hit the
    wrong task (PEP 789). Do not replace it with one task per source draining
    into a queue either: that reads ahead without bound, which breaks C1, and
    loses a source's exception unless extra plumbing carries it.

    :param sources: The async iterables to merge. Each is turned into an
        iterator with ``aiter`` once, up front.
    :return: An async generator over the sources' items.
    """
    iterators = [aiter(source) for source in sources]
    is_open = [True] * len(iterators)
    # Outstanding reads, keyed by task, valued by the index of their source.
    # Insertion order is schedule order, which is what C6 reads.
    reads: dict[asyncio.Future[T], int] = {}

    def schedule(index: int) -> None:
        read = asyncio.ensure_future(anext(iterators[index]))
        read.add_done_callback(_retrieve)
        reads[read] = index

    try:
        for index in range(len(iterators)):
            schedule(index)

        while reads:
            done, _ = await asyncio.wait(reads, return_when=asyncio.FIRST_COMPLETED)
            read = next(r for r in reads if r in done)
            index = reads.pop(read)
            try:
                item = read.result()
            except StopAsyncIteration:
                is_open[index] = False
                await _aclose(iterators[index])
                continue
            yield item
            schedule(index)
    finally:
        for read in reads:
            read.cancel()
        try:
            if reads:
                await asyncio.wait(reads)
        finally:
            running = {index for read, index in reads.items() if not read.done()}
            # Callbacks run last-in first-out, so pushing in source order
            # closes in reverse source order.
            async with contextlib.AsyncExitStack() as stack:
                for index, iterator in enumerate(iterators):
                    if is_open[index] and index not in running:
                        stack.push_async_callback(_aclose, iterator)


def _retrieve(read: asyncio.Future) -> None:
    """Mark a read's outcome as retrieved so asyncio never logs it as lost."""
    if not read.cancelled():
        read.exception()


async def _aclose(iterator: AsyncIterator) -> None:
    """Close an iterator that has an ``aclose``; skip one that does not."""
    aclose = getattr(iterator, "aclose", None)
    if aclose is not None:
        await aclose()
