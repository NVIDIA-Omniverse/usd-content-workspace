# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import asyncio
import logging
import time
from contextvars import ContextVar, Token
from typing import Any

from notification_common.redis import redis_client

from app.config import settings
from app.lib.observability.cache_writer_metrics import cache_writer_metrics
from app.lib.permissions.storage_permissions.cache_scope import CacheScope

logger = logging.getLogger(__name__)

# Hardcoded per the SDD: only the buffer cap is operator-tunable.
# Debounce window and per-batch op cap are fixed.
_DEBOUNCE_SECONDS = 0.050  # 50 ms
_BATCH_OP_CAP = 100

# When Redis is unreachable for an extended window the drainer can fail
# every iteration. Suppress the exception traceback after this many
# consecutive failures to avoid log flooding; the metric still emits
# every flush failure.
_REDIS_FAILURE_LOG_INTERVAL = 50


class CacheWriter:
    """
    Per-scope write-through batcher for auth-cache mutations.

    One writer per (queue, presenting principal) pair -- see
    :class:`~app.lib.permissions.storage_permissions.cache_scope.CacheScope`.
    Two principals attached to the same non-durable queue therefore each get
    their own writer and their own Redis namespace; two streams from the
    same principal share one, as they always have.

    The hot auth-check path calls the synchronous :meth:`enqueue` to push
    an Op onto an :class:`asyncio.Queue`. A long-lived background drainer
    task batches Ops with a debounce-or-cap policy and flushes them to
    Redis via a single pipelined call per batch.

    Lifecycle:
        - ``__init__`` creates the buffer; it does NOT start the drainer.
        - ``start()`` creates and stores the drainer task. Callers must
          call ``start()`` before any mutations enqueue Ops.
        - ``drain_and_stop(timeout)`` flushes pending Ops within the
          timeout, then cancels the drainer. Idempotent.

    Failure model:
        - ``enqueue`` never raises. On overflow the OLDEST Op is dropped
          and ``cache_writer.drops`` is incremented.
        - The drainer never raises out of its loop. Redis exceptions are
          logged (rate-limited); the loop continues.
    """

    def __init__(self, scope: CacheScope) -> None:
        # These writers exist only for non-durable queues, so the scope's
        # queue name carries the stream's reconnect token. The scope key is
        # what the Redis keys are built from and must stay exact, but
        # nothing observable may repeat it: every log line, task name, and
        # metric attribute below uses this reference instead. It is the bare
        # `sha256-...` form, identical to the `queue_ref` the same stream's
        # spans carry, so a metric series joins its trace by exact match.
        # It is queue-only, so the two writers of one queue's two principals
        # report under the same reference -- deliberately; the depth
        # baseline, which those two must NOT share, is keyed on the scope
        # key instead.
        self._queue_ref = scope.queue_ref
        self._buffer: asyncio.Queue[Any] = asyncio.Queue(
            maxsize=settings.settings.cache_writer_buffer_cap
        )
        # Public read-only handle so callers (DirectoryContents mutators)
        # can build Ops without reaching into private state.
        self.scope_key = scope.key
        self._drainer_task: asyncio.Task | None = None
        self._stopped = False
        self._current_pipeline_task: asyncio.Task | None = None
        self._consecutive_redis_failures = 0
        # True between the drainer's first Op-pull and the end of the
        # corresponding flush. Closes the shutdown race where the buffer
        # is empty AND no pipeline task exists yet, but the drainer is
        # holding ops in a local batch.
        self._batch_in_flight = False

    @property
    def queue_ref(self) -> str:
        """
        The reference to this writer's queue that is safe to write down.

        Public because the writer's lifecycle is driven from outside it --
        the non-durable consumer and ``register_writer`` both log about a
        writer they hold -- and those lines must name the same stream the
        same way this class's own lines do.
        """
        return self._queue_ref

    def start(self) -> None:
        """
        Create and store the long-lived drainer task. Idempotent — a
        second call is a no-op.

        If Redis is not configured (``redis_client.redis_client is None``)
        the drainer is NOT created. Subsequent ``enqueue`` calls fill the
        buffer to its cap and then drop-oldest fires from ``enqueue`` —
        keeping drop accounting at the buffer boundary instead of
        silently losing batches downstream.
        """
        if self._drainer_task is not None:
            return
        if redis_client.redis_client is None:
            logger.info(
                "CacheWriter drainer not started: Redis is not configured "
                "(queue=%s). Ops will accumulate in the buffer until "
                "the cap is reached, then drop-oldest fires.",
                self._queue_ref,
            )
            return
        self._drainer_task = asyncio.create_task(
            self._drain_loop(), name=f"cache-writer-drainer:{self._queue_ref}"
        )

    def enqueue(self, op: Any) -> None:
        """
        Push an Op onto the buffer.

        Synchronous from the caller's perspective; never awaits and
        never raises. On overflow the OLDEST queued Op is dropped (popped
        from the front) and ``cache_writer.drops`` is incremented.
        """
        try:
            self._buffer.put_nowait(op)
        except asyncio.QueueFull:
            try:
                self._buffer.get_nowait()
            except asyncio.QueueEmpty:
                # Cap is 0 or another consumer drained it between the
                # put_nowait failure and now. Either way, dropping this
                # op is the correct outcome.
                pass
            try:
                self._buffer.put_nowait(op)
            except asyncio.QueueFull:
                # Defensive: another producer raced us. The new op is
                # dropped instead of an older one; still record the drop.
                pass
            cache_writer_metrics.record_drop(self._queue_ref)

    async def drain_and_stop(self, timeout: float) -> None:
        """
        Flush any pending Ops then stop the drainer task.

        Bounded — returns within ``timeout`` even if Redis hangs. Cancels
        any in-flight pipeline if the timeout fires. Idempotent: calling
        twice after the writer has stopped does not raise.
        """
        if self._stopped:
            return
        self._stopped = True

        task = self._drainer_task
        if task is None or task.done():
            self._drainer_task = None
            return

        try:
            await asyncio.wait_for(self._wait_for_empty(), timeout=timeout)
        except asyncio.TimeoutError:
            # Force-cancel an in-flight pipeline so the drainer can exit.
            in_flight = self._current_pipeline_task
            if in_flight is not None and not in_flight.done():
                in_flight.cancel()

        task.cancel()
        try:
            await task
        except asyncio.CancelledError:
            pass
        except Exception:
            logger.exception(
                "CacheWriter drainer raised on shutdown (queue=%s)",
                self._queue_ref,
            )
        self._drainer_task = None

        # Final depth sample to bring the gauge series to 0, then drop this
        # scope's baseline so it does not leak across reconnects. The
        # baseline is keyed on the scope, not the reported reference: the
        # two writers of one queue's two principals report under one
        # `queue_ref` and would otherwise share -- and corrupt -- each
        # other's last-sample state. Writers of the SAME scope (a re-attach
        # that replaced this writer) do share it: the zero sample below
        # subtracts the successor's depth along with this writer's, and the
        # successor's next sample re-adds it.
        cache_writer_metrics.record_queue_depth(self._queue_ref, 0, self.scope_key)
        cache_writer_metrics.forget_queue(self.scope_key)

    async def _wait_for_empty(self) -> None:
        """
        Poll until the buffer is empty AND no pipeline is in flight AND
        the drainer is not holding a batch.

        ``_batch_in_flight`` covers the window between the drainer's
        first ``buffer.get()`` and the moment the corresponding flush
        either schedules its pipeline task or returns — without it,
        ``drain_and_stop`` can return mid-batch and the in-hand Ops are
        silently dropped on shutdown.
        """
        while True:
            in_flight = self._current_pipeline_task
            pipeline_done = in_flight is None or in_flight.done()
            if self._buffer.empty() and pipeline_done and not self._batch_in_flight:
                return
            await asyncio.sleep(0.005)

    async def _drain_loop(self) -> None:
        """
        Long-lived drainer. Waits for the first Op, batches more under a
        debounce-or-cap policy, then flushes the batch via a single
        Redis pipeline per iteration.
        """
        try:
            while True:
                first = await self._buffer.get()
                self._batch_in_flight = True
                try:
                    batch = [first]
                    deadline = time.monotonic() + _DEBOUNCE_SECONDS

                    while len(batch) < _BATCH_OP_CAP:
                        remaining = deadline - time.monotonic()
                        if remaining <= 0:
                            break
                        try:
                            op = await asyncio.wait_for(
                                self._buffer.get(), timeout=remaining
                            )
                        except asyncio.TimeoutError:
                            break
                        batch.append(op)

                    cache_writer_metrics.record_queue_depth(
                        self._queue_ref, self._buffer.qsize(), self.scope_key
                    )

                    await self._flush(batch)
                finally:
                    self._batch_in_flight = False
        except asyncio.CancelledError:
            raise
        except Exception:
            # The loop must never raise upward. A bug here would silently
            # stop the drainer; loud-log it.
            logger.exception(
                "CacheWriter drainer crashed unexpectedly "
                "(queue=%s); drainer is exiting",
                self._queue_ref,
            )

    async def _flush(self, batch: list[Any]) -> None:
        """
        Stage every Op onto a single Redis pipeline and execute.

        Failures are logged (rate-limited) and never raised. The in-flight
        pipeline coroutine is tracked as a task so ``drain_and_stop`` can
        cancel it if the shutdown timeout fires.

        ``start()`` refuses to spawn the drainer when ``redis_client`` is
        None, so this method is only reached when a real client exists.
        """
        start = time.monotonic()
        pipeline = redis_client.redis_client.pipeline()
        for op in batch:
            try:
                # NOTE: relies on Epic 1's Op contract that ``apply_to``
                # is all-or-nothing on staging. If a future Op violates
                # this, the partial commands staged before the raise
                # will still execute below.
                op.apply_to(pipeline)
            except Exception:
                logger.exception(
                    "CacheWriter Op.apply_to raised (queue=%s); skipping op",
                    self._queue_ref,
                )

        execute_task = asyncio.create_task(pipeline.execute())
        self._current_pipeline_task = execute_task
        try:
            await execute_task
            latency_ms = (time.monotonic() - start) * 1000.0
            cache_writer_metrics.record_flush(self._queue_ref, len(batch), latency_ms)
            self._consecutive_redis_failures = 0
        except asyncio.CancelledError:
            raise
        except Exception:
            self._consecutive_redis_failures += 1
            should_log = (
                self._consecutive_redis_failures == 1
                or self._consecutive_redis_failures % _REDIS_FAILURE_LOG_INTERVAL == 0
            )
            if should_log:
                logger.exception(
                    "CacheWriter Redis flush failed "
                    "(queue=%s, consecutive_failures=%d, batch_size=%d)",
                    self._queue_ref,
                    self._consecutive_redis_failures,
                    len(batch),
                )
        finally:
            self._current_pipeline_task = None


# ---------------------------------------------------------------------------
# Context plumbing: per-scope registry + ContextVar
# ---------------------------------------------------------------------------
#
# DirectoryContents mutators run on the auth-check hot path with no direct
# reference to the owning stream's CacheWriter. To enqueue per-mutation Ops
# without changing every mutator's signature, callers (the consumer
# lifecycle) bind the active cache-scope key to a ContextVar before mutations
# run, and mutators call ``active_writer()`` to look up the writer via the
# module-level registry.
#
# The registry and the ContextVar are keyed on a CacheScope's ``key`` -- the
# queue name plus the presenting principal's reference -- so a second
# principal attaching to the same non-durable queue registers its OWN writer
# instead of displacing the first's. The functions below keep the
# ``queue_id`` spelling in their names for now; the value they carry is a
# cache-scope key.
#
# Contracts:
#   - ``register_writer`` is defensive: a second registration for the same
#     scope key drains-and-stops the prior writer to avoid leaking its
#     drainer task.
#   - ``active_writer()`` NEVER raises. It returns ``None`` when the
#     ContextVar is unset, when no writer is registered for the bound
#     scope key, or when ``settings.redis_enabled`` is False.

_writer_registry: dict[str, CacheWriter] = {}

# Anchors drain-and-stop tasks scheduled by `register_writer` for replaced
# writers. Without a strong reference, the event loop may drop the task
# (engineering best-practices §`asyncio.create_task` orphan rule). Tasks
# discard themselves on completion via add_done_callback so the set stays
# bounded by the number of in-flight replace-drains (typically zero).
_pending_drain_tasks: set[asyncio.Task] = set()

_active_queue_id_var: ContextVar[str | None] = ContextVar(
    "cache_writer_active_queue_id", default=None
)


def register_writer(writer: CacheWriter) -> None:
    """
    Register ``writer`` under its own cache-scope key.

    Takes the writer rather than a key and a writer because the registry
    is keyed on the writer's cache-scope key, so the key is the writer's
    to supply: passing both makes a key/writer mismatch representable, and
    a mismatched pair would register a writer where no lookup can reach it
    while shadowing the one that belongs there. The call site already
    holds the writer, so passing it alone costs nothing there, and it is
    symmetric with :func:`unregister_writer`.

    If a writer is already registered for this scope key, drain-and-stop
    the prior writer (best-effort, bounded) and replace it. This handles
    the corner case where a consumer's cleanup path raced ahead of a
    reconnect attempt — the old drainer task would otherwise leak. A
    re-attach by the same principal reaches that replacement, and so does a
    second CONCURRENT client of that principal sharing its token on this
    pod: both resolve to one scope key. A different principal on the same
    queue has a different scope key and registers alongside.

    For two concurrent same-principal streams that is a live limitation: the
    second registration drains and stops the first stream's writer, the
    surviving first stream's mutators then enqueue onto the successor's
    writer, and if the successor ends first the survivor's write-through is
    silently lost (redesign filed on b.uze).

    The registry is keyed on the raw scope key (which embeds a non-durable
    broker name, and so the stream's reconnect token) because lookups must
    match the value mutators bind to the ContextVar. That value is never
    written down: the task name and the log line below both use the
    writer's redacted ``queue_ref`` instead.
    """
    scope_key = writer.scope_key
    existing = _writer_registry.get(scope_key)
    if existing is not None and existing is not writer:
        queue_ref = writer.queue_ref
        # Bound before the call so the failure path below can dispose of
        # it: the coroutine object exists as soon as the argument is
        # evaluated, i.e. before `create_task` runs.
        coro = existing.drain_and_stop(timeout=1.0)
        try:
            task = asyncio.create_task(
                coro, name=f"cache-writer-replace-drain:{queue_ref}"
            )
        except RuntimeError:
            # No running loop. Close the coroutine that will now never be
            # scheduled, else it warns "was never awaited" at GC time.
            coro.close()
            # Nothing we can do here; the prior writer is orphaned but the
            # new one supersedes it in the registry.
            logger.warning(
                "register_writer could not schedule drain of replaced "
                "writer (queue=%s): no running event loop",
                queue_ref,
            )
        else:
            # Anchor the task so the loop doesn't GC it mid-drain; the
            # callback removes it on completion to keep the set bounded.
            _pending_drain_tasks.add(task)
            task.add_done_callback(_pending_drain_tasks.discard)
    _writer_registry[scope_key] = writer


def unregister_writer(writer: CacheWriter) -> None:
    """
    Remove ``writer``'s registry entry, if ``writer`` still owns it.

    Takes the writer rather than a scope key because the entry belongs to
    a writer and not to the key. A same-scope re-attach replaces the entry
    (see :func:`register_writer`), and the replaced writer's own cleanup
    path runs afterwards; popping by key there would remove the LIVE
    successor, after which ``active_writer()`` returns ``None`` in the
    successor's context and its write-through is silently lost. Both call
    sites already hold the writer object, so passing it costs nothing at
    the call site and makes a key/writer mismatch unrepresentable.

    Idempotent — a missing entry, or an entry owned by a successor writer,
    is left alone.
    """
    scope_key = writer.scope_key
    if _writer_registry.get(scope_key) is writer:
        del _writer_registry[scope_key]


def get_writer_for_queue(queue_id: str) -> CacheWriter | None:
    """
    Return the registered writer for the cache-scope key ``queue_id``, or
    ``None``.
    """
    return _writer_registry.get(queue_id)


def set_active_queue_id(queue_id: str) -> Token:
    """
    Bind the cache-scope key ``queue_id`` to the current async context and
    return a Token so the caller can restore the prior value via
    ``reset_active_queue_id``.
    """
    return _active_queue_id_var.set(queue_id)


def reset_active_queue_id(token: Token) -> None:
    """Restore the prior ContextVar value captured by ``set_active_queue_id``."""
    _active_queue_id_var.reset(token)


def active_writer() -> CacheWriter | None:
    """
    Return the CacheWriter for the cache-scope key bound to the current
    context, or ``None`` if any of the following holds:

    - ``settings.redis_enabled`` is False (no write-through configured).
    - The ContextVar has not been set in this context.
    - No writer is registered for the bound scope key.

    Never raises. Mutators on the auth hot path call this and silently
    skip enqueueing when it returns ``None``.
    """
    if not settings.settings.redis_enabled:
        return None
    queue_id = _active_queue_id_var.get()
    if queue_id is None:
        return None
    return _writer_registry.get(queue_id)
