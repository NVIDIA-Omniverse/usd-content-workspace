# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
from contextlib import asynccontextmanager
from typing import AsyncGenerator

import aio_pika
from aio_pika.abc import AbstractExchange, AbstractRobustChannel

from notification_common.config.common_settings import common_settings
from notification_common.rabbitmq.channel_pool import (
    ChannelPool,
    DurableChannelPoolConfig,
)

logger = logging.getLogger(__name__)


class RabbitMQConnector:
    """
    Class for rabbitmq connection and channel pools.

    This module creates an instance, and it is that instance that should be
    imported in other modules.
    """

    EXCHANGE_NAME = "events"

    # Pool names, used both as the keys of pool_stats_by_pool() and as the
    # label each ChannelPool prefixes its log lines with, so a stats key and an
    # incident-timeline log line name the same pool the same way. Consumers
    # should reference these constants rather than repeating the bare literals.
    NON_DURABLE_POOL_NAME = "non_durable"
    DURABLE_POOL_NAME = "durable"

    def __init__(self):
        self.closing = False
        # Distinct from `closing`: `draining` marks the bounded graceful-drain
        # window opened by begin_draining() on SIGTERM. It is a plain attribute
        # so readiness probes and the capacity guard can read connector.draining
        # synchronously per request, and it stays independent of `closing` —
        # begin_draining() never sets `closing` and never tears anything down.
        self.draining = False

        self._lock = asyncio.Lock()
        self._connection: aio_pika.RobustConnection | None = None
        # Both pools share the one connection above. _channel_pool is the
        # non-durable pool; it keeps its original name because callers outside
        # this library reach into it by that name.
        self._channel_pool: ChannelPool | None = None
        self._durable_channel_pool: ChannelPool | None = None

    def _pools(self) -> list[ChannelPool]:
        """Every pool this connector owns, non-durable first."""
        return [p for p in (self._channel_pool, self._durable_channel_pool) if p]

    async def connection(self):
        if self._connection.is_closed:
            await self.connect()

        return self._connection

    @asynccontextmanager
    async def get_channel(
        self, durable_pool: bool = False
    ) -> AsyncGenerator[AbstractRobustChannel, None]:
        """
        Get a channel from a pool using the context manager pattern.

        Automatically returns the channel to the pool it came from when done.
        The return dispatches by ownership, so the caller never restates which
        pool it borrowed from.

        :param durable_pool: True to acquire from the durable channel pool,
            False (the default) to acquire from the non-durable one.
        """
        channel = await self.get_channel_direct(durable_pool=durable_pool)
        try:
            yield channel
        finally:
            await self.return_channel(channel)

    async def get_channel_direct(
        self, durable_pool: bool = False
    ) -> AbstractRobustChannel:
        """
        Get a channel from a pool directly without using a context manager.

        The caller is responsible for returning the channel to the pool using
        return_channel() when done.

        :param durable_pool: True to acquire from the durable channel pool,
            False (the default) to acquire from the non-durable one.
        :return: Channel from the selected pool
        """
        if self._connection is None or self._connection.is_closed:
            await self.connect()

        pool = self._durable_channel_pool if durable_pool else self._channel_pool
        return await pool.get_channel()

    @asynccontextmanager
    async def get_temporary_channel(
        self,
    ) -> AsyncGenerator[AbstractRobustChannel, None]:
        """
        Create a temporary channel directly from the connection, bypassing the pool.

        This is useful for operations that would be affected by cached info in
        the channel. E.g., if we are trying to see if the queue exists, we
        shouldn't reuse a channel pool channel.

        The channel is automatically closed when the context manager exits.
        """
        if self._connection is None or self._connection.is_closed:
            await self.connect()

        channel = await self._connection.channel()
        try:
            yield channel
        finally:
            # Always call close(), even when is_closed is True at this
            # moment. RobustChannel auto-reopens after a broker-side
            # close (e.g., NOT_FOUND on a passive declare), so
            # is_closed can be True transiently while a new underlying
            # AMQP channel is being opened by _on_close.restore().
            # close() also clears the restore signal, preventing the
            # auto-reopen, and is a no-op if the channel is genuinely
            # closed. Wrap in try/except so a benign double-close
            # doesn't surface as a leak.
            try:
                await channel.close()
            except Exception:
                logger.debug(
                    "Error closing temporary channel; ignoring",
                    exc_info=True,
                )

    async def return_channel(self, channel: AbstractRobustChannel) -> None:
        """
        Return a channel to the pool for reuse.

        The channel is offered to every pool this connector owns, and lands in
        whichever one owns it. A pool that did not hand out the channel ignores
        it, so a channel from a pool that is no longer owned here, or one that
        never came from a pool at all, is silently dropped rather than
        mis-credited.

        :param channel: Channel to return to its pool
        """
        for pool in self._pools():
            await pool.return_channel(channel)

    async def connect(self):
        """Create the channel connection."""

        async with self._lock:
            if not self._connection or self._connection.is_closed:
                logger.info("Connecting to rabbitmq...")

                self._connection = await aio_pika.connect_robust(
                    common_settings.rabbitmq_uri
                )

            if not self._channel_pool:
                logger.info("Creating rabbitmq channel pool...")
                self._channel_pool = ChannelPool(
                    self._connection, name=self.NON_DURABLE_POOL_NAME
                )
                await self._channel_pool.initialize()

            # Guarded separately from the non-durable pool above so a connector
            # that was left holding only one of the two converges on a
            # reconnect instead of keeping the missing pool missing.
            if not self._durable_channel_pool:
                logger.info("Creating rabbitmq durable channel pool...")
                self._durable_channel_pool = ChannelPool(
                    self._connection,
                    DurableChannelPoolConfig(),
                    name=self.DURABLE_POOL_NAME,
                )
                await self._durable_channel_pool.initialize()

            await self.declare_exchange()

            logger.info("Connected to rabbitmq")

    async def close(self):
        logger.info("Closing rabbitmq connection...")

        for pool in self._pools():
            await pool.close_all()

        self._channel_pool = None
        self._durable_channel_pool = None

        if self._connection and not self._connection.is_closed:
            await self._connection.close()

        logger.info("Closed rabbitmq connection")

    def prepare_to_shutdown(self):
        """
        Mark the client as ready to shut down.
        ALl processing should stop ASAP because the connection is about to close.
        """
        logger.info("Stopping rabbitmq processing to allow graceful shutdown...")
        self.closing = True

    def begin_draining(self) -> None:
        """
        Enter the bounded graceful-drain state.

        Sets `self.draining = True` so synchronous readers (the readiness
        servicer, the capacity guard) can observe that new work should be
        refused and existing non-durable streams allowed to wind down. This is
        deliberately independent of `closing`: it does NOT set `self.closing`
        and does NOT close or tear down either channel pool or the connection —
        the actual reclaim happens later on the close() path. Idempotent: a
        second call is a no-op.
        """
        if self.draining:
            return
        logger.info("Beginning graceful drain of non-durable streams...")
        self.draining = True

    async def drain_non_durable(
        self,
        window_seconds: float | None = None,
        poll_interval_seconds: float | None = None,
    ) -> None:
        """
        Wait for the non-durable pool's in-flight checkouts to drain.

        Polls the non-durable pool's active checkout count
        (pool_stats_by_pool()[NON_DURABLE_POOL_NAME]["total_client_count"]) every
        `poll_interval_seconds` and returns as soon as that count reaches 0 or
        the `window_seconds` deadline elapses, whichever comes first. The wait is
        keyed on the NON-DURABLE pool only; the durable pool and its checkouts
        are not part of this accounting. When the non-durable pool is
        absent/empty (count already 0) it returns near-immediately.

        This method waits but never itself closes the pool: any checkouts still
        held at window expiry are reclaimed by the existing close() ->
        close_all() path invoked by the shutdown sequence.

        :param window_seconds: Bounded drain window; falls back to
            common_settings.shutdown_drain_window_seconds when None.
        :param poll_interval_seconds: Re-read interval; falls back to
            common_settings.shutdown_drain_poll_interval_seconds when None.
        """
        # Late-bound reads from the settings singleton (consistent with the rest
        # of the module) so tests can override via the settings singleton.
        if window_seconds is None:
            window_seconds = common_settings.shutdown_drain_window_seconds
        if poll_interval_seconds is None:
            poll_interval_seconds = common_settings.shutdown_drain_poll_interval_seconds

        def _non_durable_count() -> int:
            return (
                self.pool_stats_by_pool()
                .get(self.NON_DURABLE_POOL_NAME, {})
                .get("total_client_count", 0)
            )

        loop = asyncio.get_running_loop()
        deadline = loop.time() + window_seconds
        while True:
            count = _non_durable_count()
            if count <= 0:
                logger.info("Non-durable pool drained; no streams in flight")
                return
            remaining = deadline - loop.time()
            if remaining <= 0:
                logger.info(
                    "Drain window elapsed with %d non-durable stream(s) still "
                    "in flight; proceeding to close",
                    count,
                )
                return
            # Cap the sleep at the remaining window so a large poll interval
            # cannot push the return well past the deadline.
            await asyncio.sleep(min(poll_interval_seconds, remaining))

    async def declare_exchange(self):
        """
        Ensure an 'events' topic exchange exists and is durable.

        Borrows a channel for the declaration and returns it immediately. The
        declaration is a one-shot operation that any channel can carry, so the
        non-durable pool is chosen deliberately rather than incidentally: it
        keeps startup identical to the single-pool behavior. The base
        connector's exchange declaration itself takes no durable checkout; a
        subclass that declares further topology on this same connect() path may,
        and this service's EventConsumerConnector does (its
        _declare_dead_letter_topology() takes a durable-pool checkout), so the
        durable pool is not guaranteed to stay at zero open channels.
        """
        if self._channel_pool:
            logger.info(f"Declaring topic exchange with name '{self.EXCHANGE_NAME}'")
            channel = await self._channel_pool.get_channel()
            try:
                await channel.declare_exchange(
                    self.EXCHANGE_NAME, type="topic", durable=True
                )
            finally:
                await self._channel_pool.return_channel(channel)

    async def get_exchange(self, channel: aio_pika.Channel) -> AbstractExchange:
        """
        Get the events exchange using the provided channel.

        :param channel: Channel to use for accessing the exchange
        :return: The events exchange
        """
        return await channel.get_exchange(self.EXCHANGE_NAME)

    def pool_stats(self) -> dict:
        """
        Get statistics about the current non-durable channel pool state.

        This reports the non-durable pool alone and never aggregates the two
        pools, so its shape is unchanged from the single-pool version and the
        capacity surfaces that read it by key keep seeing non-durable capacity.
        For per-pool statistics, use pool_stats_by_pool().

        :return: Dictionary with pool statistics, or empty dict if pool not initialized
        """
        if self._channel_pool:
            return self._channel_pool.pool_stats()
        return {}

    def pool_stats_by_pool(self) -> dict[str, dict]:
        """
        Get statistics for every pool this connector owns, keyed by pool name.

        Values are each pool's own ChannelPool.pool_stats() dict, reported
        separately and never aggregated: capacity surfaces key on the
        non-durable pool alone.

        :return: {NON_DURABLE_POOL_NAME: {...}, DURABLE_POOL_NAME: {...}},
                 omitting any pool that does not exist yet (empty before
                 connect()).
        """
        stats: dict[str, dict] = {}
        if self._channel_pool:
            stats[self.NON_DURABLE_POOL_NAME] = self._channel_pool.pool_stats()
        if self._durable_channel_pool:
            stats[self.DURABLE_POOL_NAME] = self._durable_channel_pool.pool_stats()
        return stats
