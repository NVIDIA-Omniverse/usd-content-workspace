# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import asyncio
import logging
from typing import AsyncGenerator

from aio_pika.abc import AbstractQueue
from notification_common.auth import principal_id
from notification_common.errors.errors import BadRequestError, ServerError

from app.config import settings
from app.lib.models.event_message import EventMessage
from app.lib.models.filter_group import FilterGroup
from app.lib.queue_names import (
    NON_DURABLE_SUFFIX,
    redact_principal_id,
    redact_reconnect_token,
)
from app.lib.permissions import permissions
from app.lib.permissions.storage_permissions import (
    storage_permissions,
    storage_permissions_cache,
)
from app.lib.permissions.storage_permissions.cache_scope import CacheScope
from app.lib.permissions.storage_permissions.storage_cache import StorageCache
from app.lib.permissions.storage_permissions.cache_writer import (
    CacheWriter,
    register_writer,
    reset_active_queue_id,
    set_active_queue_id,
    unregister_writer,
)
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase

logger = logging.getLogger(__name__)

# Hardcoded per the SDD: bounded shutdown timeout for the per-scope
# CacheWriter drain. Must stay short enough that a hung Redis cannot
# block channel release.
_CACHE_WRITER_DRAIN_TIMEOUT_SEC = 1.0

# ContextVar.reset can raise LookupError or ValueError when the token
# was not produced by the current context. Bound to a tuple constant so the
# formatter has no `except (A, B):` parens to strip: Black 25 strips them,
# and the stripped form is a SyntaxError under 3.13-grammar tooling
# (see b.8qg).
_RESET_TOKEN_EXC = (LookupError, ValueError)

# RuntimeError is the reachable leg of the __del__ safety net below:
# `asyncio.get_event_loop()` raises it when no loop is current, the ordinary
# case for a consumer garbage-collected after its loop closed. AttributeError
# has no known trigger in that body; it is kept as a cheap belt-and-braces
# catch, and it is what the original clause intended to cover -- `b.8qg` is
# about that clause being silently flattened. At true interpreter shutdown the
# body raises ImportError instead ("sys.meta_path is None, Python is likely
# shutting down"), which is deliberately not caught: the process is exiting and
# the channel needs no return, and the interpreter reports it as "Exception
# ignored" on stderr.
# Bound to a tuple constant for the same reason as _RESET_TOKEN_EXC above -- so
# the formatter has no `except (A, B):` parens to strip, the stripped form
# being a SyntaxError under 3.13-grammar tooling (see b.8qg).
_DEL_CLEANUP_EXC = (RuntimeError, AttributeError)


class NonDurableQueueConsumer(RabbitMQBase):
    """
    Class for consuming from a non-durable queue in rabbitmq.
    """

    def __init__(
        self,
        queue: AbstractQueue,
        queue_id: str,
        channel,
    ):
        super().__init__()

        self.queue = queue
        # This IS the stream's reconnect token: it is handed to the client
        # as one and accepted back as one. The token is not a credential:
        # presenting it requires a valid bearer token and every delivered
        # event is authorized per principal by the permissions policy (see
        # `consume` below). It is the queue's identifier, so an authorized
        # principal that obtains it could attach to the same stream and take
        # a share of its events; it is kept out of telemetry as defense in
        # depth. Never log or export it -- use `queue_ref` below, which is
        # what every observability site here writes down.
        self.queue_id = queue_id
        self.channel = channel

        # What this stream's storage-permissions cache is namespaced by:
        # the queue plus the principal that presented the request. Set by
        # ``get_instance`` on every path, Redis or not, because it is also
        # what ``update_queue`` re-initializes the cache under mid-stream.
        # Declared here, after ``self.channel``, so that ``__del__``'s
        # ``hasattr(self, "channel")`` guard keeps meaning what its comment
        # says it means -- and it is deliberately NOT read inside
        # ``__del__``.
        self._cache_scope: CacheScope | None = None

        # This stream's storage-permissions cache, captured by
        # ``get_instance`` once the cache is initialized, so ``consume``'s
        # teardown can report the events dropped against unauthorized
        # entries still in it. Held here rather than read from the
        # ContextVar because that teardown may run in another context (an
        # async generator closed by the event loop's finalizer, for one).
        # None when storage permissions are disabled. Declared after
        # ``self.channel`` for the same reason as ``_cache_scope``.
        self._storage_cache: StorageCache | None = None

        # What this connect did with the filter set the opening request
        # named. This is a fact about the request the service received, not
        # about the bindings the queue ends up holding: the service keeps
        # no filter state and never reads a binding set back from the
        # broker. Three values, all three load-bearing:
        #
        # * ``True`` -- the connect took the request's ``filter_groups`` as
        #   a set to establish. Either a fresh connect, which creates the
        #   queue and binds that set, or a connect-time update whose two
        #   sets differed as the client sent them, for which any bind and
        #   unbind calls that difference required have returned. That
        #   difference can be empty -- ``update_queue`` compares the two
        #   as lists, so a set re-sent reordered differs and issues no
        #   call -- and ``True`` therefore does not promise a call.
        # * ``False`` -- a connect-time update found the request's
        #   ``filter_groups`` equal to its ``previous_filter_groups``, so
        #   ``update_queue`` issued no bind or unbind call.
        # * ``None`` -- a reconnect that named no filter set at all,
        #   because it carried no ``previous_filter_groups``. Such a
        #   request carries no ``filter_groups`` either, because a token
        #   plus a set without its predecessor is rejected by
        #   ``get_instance`` as an incomplete filter update.
        #
        # Written once, by ``get_instance``, and never again: the mid-stream
        # updates that follow do not touch it, which is why it is named for
        # the connect and not for the last update. The gRPC servicer reads
        # it to decide whether the stream's first response can acknowledge a
        # set -- only the two non-``None`` values come from a request that
        # named one. Declared here, after ``self.channel``, for the same
        # reason as ``_cache_scope`` above.
        self.connect_time_update_changed: bool | None = None

        # Per-scope write-through batcher and the ContextVar token used to
        # restore the prior `_active_queue_id_var` value on cleanup. Both
        # are populated by ``get_instance`` when ``settings.redis_enabled``
        # and remain ``None`` otherwise (Epic 65's CacheWriter would refuse
        # to start anyway, but skipping construction keeps things clean).
        self._cache_writer: CacheWriter | None = None
        self._active_queue_id_token = None

    @property
    def queue_ref(self) -> str:
        """
        A reference to this stream's queue that is safe to write down.

        ``queue_id`` is the reconnect token, so it cannot go into a log
        line, a span attribute, or a metric attribute. This is the redacted
        stand-in every one of those uses: stable across pods and restarts,
        so one stream is still followable end to end, and non-reversible, so
        following it does not hand a reader the token itself.
        """
        return redact_reconnect_token(self.queue_id)

    async def consume(self) -> AsyncGenerator[EventMessage, None]:
        """
        Uses the consume method but also does authorization checks on the messages.

        Every delivered event is authorized against the consuming principal's
        own permissions, so holding this stream's reconnect token is not by
        itself enough to receive anything.
        """
        consumer = self._consume_with_delayed_acks(
            self.queue,
            permissions.can_consume_non_durable_queue_event,
        )

        try:
            async for message in consumer:
                yield message
        finally:
            try:
                await consumer.aclose()
            except RuntimeError:
                pass

            self._report_dropped_storage_events()

            await asyncio.shield(self.cleanup_channel())

    def _report_dropped_storage_events(self) -> None:
        """
        Report the storage events dropped against unauthorized entries still
        in this stream's storage-permissions cache. Never raises, so the
        channel cleanup that follows it in ``consume`` always runs.
        """
        cache = self._storage_cache
        if cache is None:
            return
        try:
            storage_permissions_cache.report_dropped_events_at_stream_end(cache)
        except Exception as exc:
            logger.debug(
                "could not report dropped storage events at stream end (%s) "
                "for queue %s",
                type(exc).__name__,
                self.queue_ref,
            )

    @classmethod
    async def get_instance(
        cls,
        queue_id: str | None = None,
        filter_groups: list[FilterGroup] | None = None,
        previous_filter_groups: list[FilterGroup] | None = None,
    ) -> NonDurableQueueConsumer:
        """
        We will either have a queue_id or a list of
        filter_groups.

        In the case of a queue_id it means that it's a
        reconnection attempt, and we need to look up the queue
        by id.

        If we don't have a queue_id, then it's an initial
        request, and we need to create a queue with the filter
        groups.

        Whichever path runs, the storage-permissions cache is
        initialized for filter_groups exactly once before the
        consumer is returned, so no events flow against an
        unhydrated cache and the Storage API is not asked for
        the same listing twice. It is initialized under this
        stream's own cache scope -- the queue plus the principal
        making this request -- so two principals holding the same
        reconnect token do not share authorization state.

        :param queue_id: Optional queue identifier for
            reconnection
        :param filter_groups: Optional list of filter groups
            for initial queue creation or for updating queue
            filters.
        :param previous_filter_groups: Optional list of filter
            groups included when updating queue filters.
        :return: A NonDurableQueueConsumer instance
        :raises BadRequestError: If args are invalid
        :raises AuthenticationError: If the request's token is no
            longer usable when the principal is resolved
        """
        if not filter_groups and not queue_id:
            raise BadRequestError("Must provide either filter_groups or " "queue_id")

        if previous_filter_groups and not queue_id:
            raise BadRequestError(
                "Filters cannot be updated before " "consuming with initial filters"
            )

        if previous_filter_groups and not filter_groups:
            raise BadRequestError(
                "Must provide new filter_groups when " "updating filters"
            )

        if queue_id and filter_groups and not previous_filter_groups:
            raise BadRequestError(
                "Must provide previous_filter_groups " "when updating filters"
            )

        filter_groups = filter_groups or []

        # The presenting principal is needed here as a *value*: it becomes
        # half of the `CacheScope` built below, which is the per-principal
        # isolation of this stream's authorization cache and writer registry.
        # It is resolved BEFORE anything with a side effect, mirroring
        # DurableQueueConsumer.create_queue, so that a resolution failure
        # costs no channel checkout and leaves no queue behind.
        #
        # It is a cached read on the ordinary path -- the transport resolved
        # the token before the handler ran, and `authenticate` re-checked it
        # at the entry point. It can still raise, on the terms
        # `app.lib.auth.authenticate`'s docstring states for every in-request
        # resolution.
        #
        # The value is an identity and is never written down -- it is
        # redacted below and only ever reaches a Redis key and the writer
        # registry.
        principal = await principal_id.principal_id()

        # One pool checkout now equals one stream on every capacity
        # surface. A direct (pinned) gRPC connection's streams are no
        # longer multiplexed onto a single shared checkout; at
        # client_limit=1 each stream takes a real checkout, so it is
        # counted, absorbed by the overflow band, and bounded by the
        # memory-safe cap like everyone else's (Epic 7 / SDD SR-1.5,
        # SR-3.7). This two-tier semantic is the intent, not a
        # regression. Mirrors DurableQueueConsumer's explicit
        # durable_pool=True on the other side.
        channel = await cls.connector.get_channel_direct()

        consumer: NonDurableQueueConsumer | None = None
        try:
            # Recorded before the branch reassigns queue_id, because that is
            # the only place the shape of this connect is still readable.
            fresh_connect = not queue_id
            if fresh_connect:
                queue_id, queue = await cls._create_queue(channel, filter_groups, False)
            else:
                queue = await cls.get_queue(channel, queue_id)

            consumer = cls(queue, queue_id, channel)

            if fresh_connect:
                # A fresh connect names filter_groups and creates the queue
                # with a binding per key derived from that set, so it took
                # the request's set as one to establish -- the same verdict
                # update_queue reports with True below. Recorded here, where
                # the branch that made those calls runs, rather than left
                # for a caller to re-derive from the request's shape.
                consumer.connect_time_update_changed = True

            # The storage-permissions cache is namespaced by the queue AND
            # the principal presenting the request, so a second principal
            # attaching with this stream's reconnect token builds its own
            # authorization state instead of inheriting this one's, and
            # registers its own writer instead of draining this one's.
            # Derived here, once, before the writer bind, because every
            # mutation below is addressed by it.
            scope = CacheScope(queue.name, redact_principal_id(principal))
            consumer._cache_scope = scope

            # Bind the active scope to the current context BEFORE any
            # storage-permissions mutation runs, so write-through ops emit
            # against the right scope. Both branches below sit after this
            # bind, and exactly one of them mutates the cache:
            # update_queue re-initializes when a filter update actually
            # changes the filters, and the initialize call covers every
            # other path (fresh connect, plain reconnect, and a filter
            # update whose filters turned out to be unchanged).
            if settings.settings.redis_enabled:
                writer = CacheWriter(scope)
                writer.start()
                register_writer(writer)
                consumer._cache_writer = writer
                consumer._active_queue_id_token = set_active_queue_id(scope.key)

            cache_initialized = False
            if previous_filter_groups:
                cache_initialized = await consumer.update_queue(
                    filter_groups, previous_filter_groups
                )
                # Recorded, not recomputed: the "filters are unchanged"
                # verdict stays where update_queue's docstring says it
                # lives, and the servicer reads it back rather than
                # deriving the equality a second time. This never overwrites
                # the fresh-connect True above: a request carrying
                # previous_filter_groups without a queue_id is rejected by
                # the guards, so the two branches are mutually exclusive.
                consumer.connect_time_update_changed = cache_initialized

            if not cache_initialized:
                await storage_permissions.initialize(scope, filter_groups)

            # Read after the initialization above, which created the cache
            # in this context if storage permissions are enabled. It is the
            # same instance the stream's authorization checks use.
            consumer._storage_cache = StorageCache.get_current_instance()

            return consumer
        except BaseException:
            # BaseException catches asyncio.CancelledError too; gRPC
            # stream cancellation between connector.get_channel_direct()
            # and the return must still release the pool checkout via
            # connector.return_channel(channel) below, otherwise
            # the channel leaks with client_count=1 forever (observed
            # in dev cluster as readiness probe failures).
            #
            # Tear down the writer side first so the channel release is
            # the last thing we do regardless of how drain_and_stop fares.
            if consumer is not None and consumer._cache_writer is not None:
                writer = consumer._cache_writer
                token = consumer._active_queue_id_token
                consumer._cache_writer = None
                consumer._active_queue_id_token = None
                unregister_writer(writer)
                try:
                    await writer.drain_and_stop(timeout=_CACHE_WRITER_DRAIN_TIMEOUT_SEC)
                except Exception:
                    logger.exception(
                        "CacheWriter drain failed during get_instance "
                        "error path (queue=%s)",
                        writer.queue_ref,
                    )
                if token is not None:
                    try:
                        reset_active_queue_id(token)
                    except _RESET_TOKEN_EXC:
                        # Token created in a different context; ignore.
                        pass
            await cls.connector.return_channel(channel)
            raise

    async def update_queue(
        self,
        new_filter_groups: list[FilterGroup],
        previous_filter_groups: list[FilterGroup],
    ) -> bool:
        """
        Updates the bindings on a non-durable queue,
        and initializes new storage API permissions cache.

        The "filters are unchanged, do nothing" predicate lives
        here and only here; callers decide what to do next from
        the returned value rather than re-deriving it.

        :return: True if the storage-permissions cache was
            re-initialized for new_filter_groups, False if the
            filters were unchanged and nothing was done.
        """
        if new_filter_groups == previous_filter_groups:
            return False

        if not previous_filter_groups or not new_filter_groups:
            raise BadRequestError(
                "Must provide new_filter_groups and "
                "previous_filter_groups when updating filters"
            )

        # The scope is set by ``get_instance`` on every path; a consumer built
        # outside it would otherwise reach ``StorageCache.get_instance(None)``,
        # which proceeds silently under whatever scope the context happens to
        # hold rather than this stream's.
        if self._cache_scope is None:
            raise ServerError(
                "Cache scope is not set; NonDurableQueueConsumer must be "
                "built via get_instance"
            )

        # use the filter groups to re-initialize any
        # necessary storage permissions, under this stream's own scope --
        # the gRPC mid-stream filter update reaches this method from the
        # servicer, which holds no scope of its own.
        await storage_permissions.initialize(self._cache_scope, new_filter_groups)

        # update the bindings on the queue to start getting the
        # updated set of events
        await self._update_bindings(
            self.channel, self.queue, previous_filter_groups, new_filter_groups
        )

        return True

    def __del__(self):
        """
        Safety net to ensure a channel is released when the
        instance is garbage collected without proper cleanup.
        """
        if hasattr(self, "channel") and self.channel is not None:
            try:
                import asyncio

                loop = asyncio.get_event_loop()
                if loop.is_running():
                    asyncio.create_task(self.cleanup_channel())
            except _DEL_CLEANUP_EXC:
                # `getattr` rather than the attribute: an AttributeError raised
                # inside `__del__` is swallowed by the interpreter -- taking the
                # warning with it -- so the id is read defensively regardless of
                # how the handler was entered. The id is in fact always set once
                # this handler is reachable: the `hasattr(self, "channel")` guard
                # above only passes after `__init__` has assigned `self.channel`,
                # which it does after `self.queue_id`. Two facts about
                # `RabbitMQBase` guarantee that: it defines no `__init__` that
                # could run in between, and it declares no class-level `channel`
                # attribute, so there is no inherited value the guard could see
                # on an instance whose `__init__` has not reached that line.
                #
                # A missing id is reported as `unknown` rather than redacted:
                # redacting `""` yields the digest of the empty string, one
                # constant reference that every such warning would share and
                # that reads exactly like a real stream's -- inventing a queue
                # nobody can find and aggregating unrelated failures under it.
                # Same reasoning as the empty-value passthrough in
                # `app.lib.observability.access_log_redaction`.
                queue_id = getattr(self, "queue_id", None)
                queue_ref = redact_reconnect_token(queue_id) if queue_id else "unknown"
                logger.warning(
                    "Could not return channel in __del__ for queue %s",
                    queue_ref,
                )

    async def cleanup_channel(self):
        """
        Drain the per-scope CacheWriter, then return the
        channel to the pool it came from.
        """
        writer = self._cache_writer
        token = self._active_queue_id_token
        self._cache_writer = None
        self._active_queue_id_token = None

        try:
            if writer is not None:
                try:
                    await writer.drain_and_stop(timeout=_CACHE_WRITER_DRAIN_TIMEOUT_SEC)
                except Exception:
                    logger.exception(
                        "CacheWriter drain failed during cleanup_channel (queue=%s)",
                        writer.queue_ref,
                    )
                finally:
                    unregister_writer(writer)
            if token is not None:
                try:
                    reset_active_queue_id(token)
                except _RESET_TOKEN_EXC:
                    # Token created in a different context; ignore.
                    pass
        finally:
            channel = self.channel
            self.channel = None
            if channel is not None:
                # noinspection PyBroadException
                try:
                    await self.connector.return_channel(channel)
                except Exception:
                    logger.exception(
                        "Failed to return channel in cleanup "
                        f"for queue {self.queue_ref}"
                    )

    @staticmethod
    async def _queue_name(queue_id) -> str:
        """Override to append the non-durable suffix to the base name."""
        base_name = await RabbitMQBase._queue_name(queue_id)
        return f"{base_name}{NON_DURABLE_SUFFIX}"
