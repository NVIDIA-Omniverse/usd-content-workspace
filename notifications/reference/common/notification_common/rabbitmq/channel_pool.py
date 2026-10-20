# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
from typing import Dict

from aio_pika.abc import AbstractRobustChannel, AbstractRobustConnection

from notification_common.config.common_settings import common_settings
from notification_common.errors.errors import ChannelNotAvailableError

logger = logging.getLogger(__name__)


class ChannelPoolConfig:
    """
    The per-pool channel limits for a single ChannelPool.

    Each value may be set explicitly to give one pool limits that differ from
    every other pool in the process. Any value left as None instead follows the
    process-global common_settings value, and that fallback is resolved on each
    read rather than snapshotted at construction, so a pool constructed before
    the settings are adjusted still sees the adjustment.

    An explicit value always wins over the global, including an explicit 0.

    One value is an exception to all of the above: overflow_enabled is
    global-only BY DESIGN (SDD SR-1.5 wiring decision 2). It takes no
    constructor parameter and has no per-pool backing field, and that absence
    is deliberate rather than an oversight to be "fixed" later. Giving the
    non-durable pool its own config object is what the decision rules out,
    because the memory-safe stream cap's coupling contract (see the service's
    app/lib/rabbitmq/stream_memory_cap.py) is valid only while that pool
    carries no per-pool overrides. The durable pool opts out by overriding the
    property, not by being constructed differently.

    :param pool_size: The maximum number of channels the pool may open.
        Defaults to common_settings.rabbitmq_channel_pool_size.
    :param min_size: The number of channels created when the pool is
        initialized. Defaults to
        common_settings.rabbitmq_channel_pool_min_size.
    :param client_limit: The maximum number of concurrent clients that may
        share a single channel. Defaults to
        common_settings.rabbitmq_channel_client_limit. This is a hard
        per-channel bound only while overflow is disabled; on an
        overflow-enabled pool it is a packing target that admission may exceed
        in the SR-1.5 overflow band.
    :param prefetch_count: The QoS prefetch count applied to each channel in
        the pool. Defaults to
        common_settings.rabbitmq_channel_prefetch_count.
    :param wait_ms: How long, in milliseconds, a caller waits for a channel
        before the pool raises ChannelNotAvailableError. Defaults to
        common_settings.rabbitmq_channel_wait_ms.

    Not a constructor parameter:

    :ivar overflow_enabled: Whether the pool admits a checkout past
        client_limit onto the least-loaded open channel once every channel is
        open and full. Always follows
        common_settings.rabbitmq_channel_overflow_enabled; it cannot be
        overridden per pool.
    """

    def __init__(
        self,
        pool_size: int | None = None,
        min_size: int | None = None,
        client_limit: int | None = None,
        prefetch_count: int | None = None,
        wait_ms: int | None = None,
    ):
        self._pool_size = pool_size
        self._min_size = min_size
        self._client_limit = client_limit
        self._prefetch_count = prefetch_count
        self._wait_ms = wait_ms

    @property
    def pool_size(self) -> int:
        """The maximum number of channels the pool may open."""
        if self._pool_size is not None:
            return self._pool_size
        return common_settings.rabbitmq_channel_pool_size

    @property
    def min_size(self) -> int:
        """The number of channels created when the pool is initialized."""
        if self._min_size is not None:
            return self._min_size
        return common_settings.rabbitmq_channel_pool_min_size

    @property
    def client_limit(self) -> int:
        """
        The maximum number of concurrent clients sharing one channel.

        A hard per-channel bound only while overflow is disabled; on an
        overflow-enabled pool it is a packing target that admission may exceed
        in the SR-1.5 overflow band.
        """
        if self._client_limit is not None:
            return self._client_limit
        return common_settings.rabbitmq_channel_client_limit

    @property
    def prefetch_count(self) -> int:
        """The QoS prefetch count applied to each channel in the pool."""
        if self._prefetch_count is not None:
            return self._prefetch_count
        return common_settings.rabbitmq_channel_prefetch_count

    @property
    def wait_ms(self) -> int:
        """How long, in milliseconds, a caller waits for a channel."""
        if self._wait_ms is not None:
            return self._wait_ms
        return common_settings.rabbitmq_channel_wait_ms

    @property
    def overflow_enabled(self) -> bool:
        """
        Whether the pool admits a checkout past client_limit when full.

        Global-only by design (SDD SR-1.5 wiring decision 2): there is no
        constructor parameter and no per-pool backing field, so this always
        follows common_settings and is resolved on each read.
        """
        return common_settings.rabbitmq_channel_overflow_enabled


class DurableChannelPoolConfig(ChannelPoolConfig):
    """
    The per-pool channel limits for the durable / DLQ-browse ChannelPool.

    Identical to ChannelPoolConfig except that any value left as None follows
    the durable rabbitmq_durable_channel_* settings instead of the non-durable
    rabbitmq_channel_* ones. As with ChannelPoolConfig, that fallback is
    late-bound: it is resolved from common_settings on each read rather than
    snapshotted at construction, so a pool constructed before the durable
    settings are adjusted still sees the adjustment.

    An explicit value always wins over the global, including an explicit 0.

    The durable pool stays strict by construction: overflow_enabled is
    overridden to False unconditionally, so the SR-1.5 overflow band never
    applies here no matter how the process-global flag is set, and there is
    deliberately no rabbitmq_durable_channel_overflow_enabled setting to make
    it configurable. The resulting contract -- client_limit of 1 keeping a
    durable checkout exclusive on its channel, and a hard refusal with
    ChannelNotAvailableError once pool_size channels are all checked out -- is
    what Epic 7's Tasks 2 and 5 assert against; relaxing it would silently
    break those assertions.

    :param pool_size: The maximum number of channels the pool may open.
        Defaults to common_settings.rabbitmq_durable_channel_pool_size.
    :param min_size: The number of channels created when the pool is
        initialized. Defaults to
        common_settings.rabbitmq_durable_channel_pool_min_size, which is 0 so
        the durable pool opens no channels until a durable checkout needs one.
    :param client_limit: The maximum number of concurrent clients that may
        share a single channel. Defaults to
        common_settings.rabbitmq_durable_channel_client_limit, which is 1 so
        durable checkouts stay exclusive.
    :param prefetch_count: The QoS prefetch count applied to each channel in
        the pool. Defaults to
        common_settings.rabbitmq_durable_channel_prefetch_count.
    :param wait_ms: How long, in milliseconds, a caller waits for a channel
        before the pool raises ChannelNotAvailableError. Defaults to
        common_settings.rabbitmq_durable_channel_wait_ms.
    """

    @property
    def pool_size(self) -> int:
        """The maximum number of channels the pool may open."""
        if self._pool_size is not None:
            return self._pool_size
        return common_settings.rabbitmq_durable_channel_pool_size

    @property
    def min_size(self) -> int:
        """The number of channels created when the pool is initialized."""
        if self._min_size is not None:
            return self._min_size
        return common_settings.rabbitmq_durable_channel_pool_min_size

    @property
    def client_limit(self) -> int:
        """The maximum number of concurrent clients sharing one channel."""
        if self._client_limit is not None:
            return self._client_limit
        return common_settings.rabbitmq_durable_channel_client_limit

    @property
    def prefetch_count(self) -> int:
        """The QoS prefetch count applied to each channel in the pool."""
        if self._prefetch_count is not None:
            return self._prefetch_count
        return common_settings.rabbitmq_durable_channel_prefetch_count

    @property
    def wait_ms(self) -> int:
        """How long, in milliseconds, a caller waits for a channel."""
        if self._wait_ms is not None:
            return self._wait_ms
        return common_settings.rabbitmq_durable_channel_wait_ms

    @property
    def overflow_enabled(self) -> bool:
        """
        Always False: the durable pool never admits past client_limit.

        Ignores common_settings.rabbitmq_channel_overflow_enabled, which
        governs the non-durable pool only.
        """
        return False


class ChannelInfo:
    """Information about a channel in the pool."""

    def __init__(
        self, channel: AbstractRobustChannel, config: ChannelPoolConfig | None = None
    ):
        self.channel = channel
        self.client_count = 0
        self.is_available = True
        self._config = config or ChannelPoolConfig()

    def increment_clients(self) -> None:
        """Increment the client count and update availability."""
        self.client_count += 1
        if self.client_count >= self._config.client_limit:
            self.is_available = False

    def decrement_clients(self) -> bool:
        """
        Decrement the client count and update availability.

        Returns True if the decrement was successful, False if the client count
        was already 0 (indicating a double-return or other bug).
        """
        if self.client_count <= 0:
            return False
        self.client_count -= 1
        if self.client_count < self._config.client_limit:
            self.is_available = True
        return True

    def is_closed(self) -> bool:
        """Check if the underlying channel is closed."""
        return self.channel.is_closed


class ChannelPool:
    """
    A pool of RabbitMQ channels that can be shared across multiple clients.

    Manages channel allocation, reuse, and cleanup with configurable limits
    on pool size and clients per channel.

    Acquisition is two-tier on an overflow-enabled pool (SDD SR-1.5). Up to
    the point where every channel is open and carrying client_limit clients,
    client_limit is honored as written and the pool packs each checkout onto
    the least-loaded channel under it, growing the pool toward pool_size
    before it packs any channel past the limit. Once the pool is full and no
    channel is under the limit, client_limit becomes a packing target rather
    than a refusal: the checkout is admitted anyway onto the least-loaded open
    channel, and that channel's client_count rises past client_limit. The
    reason is that a pinned (sticky) gRPC stream cannot be rescued by
    autoscaling -- a burst arriving on connections already bound to this
    process has nowhere else to go, so the pool absorbs it instead of failing
    the streams outright.

    The overflow band therefore has no upper bound of its own. Bounding it is
    the consuming service's job, via the memory-safe stream cap that refuses
    admission before the process runs out of memory; the pool deliberately
    does not duplicate that decision. Overflow is non-durable-only:
    DurableChannelPoolConfig forces overflow_enabled to False, so a durable
    pool keeps the strict one-tier behavior and still raises
    ChannelNotAvailableError once it is full.

    :param connection: The robust connection the pool opens its channels on.
    :param config: The limits governing this pool. Defaults to a plain
        ChannelPoolConfig, which follows the process-global non-durable
        rabbitmq_channel_* settings.
    :param name: An optional label identifying this pool in its log lines, so
        a process running more than one pool stays reconstructable from an
        incident timeline. When given, every log line the pool emits is
        prefixed with "[<name>] ". When omitted, the pool logs exactly the
        unprefixed messages it always has.
    """

    def __init__(
        self,
        connection: AbstractRobustConnection,
        config: ChannelPoolConfig | None = None,
        name: str | None = None,
    ):
        self._connection = connection
        self._config = config or ChannelPoolConfig()
        self._name = name
        # Precomputed once because it is interpolated into every log line. An
        # unnamed pool gets an empty prefix so its messages are byte-identical
        # to the single-pool ones operators already grep for.
        self._log_prefix = f"[{name}] " if name else ""
        self._lock = asyncio.Lock()
        self._available_channels: asyncio.Queue[ChannelInfo] = asyncio.Queue()
        self._all_channels: Dict[AbstractRobustChannel, ChannelInfo] = {}
        self._initialized = False
        self._closed = False

    @property
    def config(self) -> ChannelPoolConfig:
        """The limits governing this pool."""
        return self._config

    @property
    def name(self) -> str | None:
        """The label identifying this pool in its log lines, if it has one."""
        return self._name

    async def initialize(self) -> None:
        """Initialize the pool with minimum number of channels."""
        if self._initialized:
            return

        async with self._lock:
            if self._initialized:
                return

            logger.info(
                f"{self._log_prefix}Initializing channel pool with "
                f"{self._config.min_size} channels"
            )

            for _ in range(self._config.min_size):
                channel_info = await self._create_channel_info()
                await self._available_channels.put(channel_info)

            self._initialized = True
            logger.info(f"{self._log_prefix}Channel pool initialized")

    async def get_channel(self) -> AbstractRobustChannel:
        """
        Get a channel from the pool.

        Returns the channel with the lowest client count to distribute the load evenly.
        Will create new channels up to max_pool_size if needed.

        With overflow disabled, a pool that is full and has no channel under
        client_limit raises ChannelNotAvailableError once wait_ms elapses. With
        overflow enabled that same state packs instead: the checkout is
        admitted onto the least-loaded open channel past client_limit, so
        ChannelNotAvailableError remains reachable only for a closed pool, for
        the wait_ms timeout, and for the degenerate states where the pool holds
        no open channel to pack onto at all (pool_size of 0, or every tracked
        channel closed).

        :return: A channel from the pool
        :raises ChannelNotAvailableError: When no channel is available within timeout
        """
        if self._closed:
            raise ChannelNotAvailableError()

        timeout_seconds = self._config.wait_ms / 1000.0

        try:
            # Try to get an available channel with timeout
            # Note: _get_best_available_channel increments the client count atomically
            # under the lock to prevent race conditions
            channel_info = await asyncio.wait_for(
                self._get_best_available_channel(), timeout=timeout_seconds
            )

            return channel_info.channel

        except asyncio.TimeoutError:
            raise ChannelNotAvailableError()

    async def return_channel(self, channel: AbstractRobustChannel) -> None:
        """
        Return a channel to the pool.

        Decrement the client count and make the channel available again
        if it's under the client limit.

        :param channel: The channel to return to the pool
        """
        if self._closed or channel not in self._all_channels:
            return

        async with self._lock:
            channel_info = self._all_channels[channel]

            if channel_info.is_closed():
                # Remove a closed channel from pool
                await self._remove_channel_info(channel_info)
                return

            if not channel_info.decrement_clients():
                # Channel was already at 0 clients - this is a double-return bug.
                # Log and ignore to prevent negative counts.
                logger.warning(
                    f"{self._log_prefix}Attempted to return a channel that has "
                    "no clients. This may indicate a double-return bug in the "
                    "calling code."
                )
                return

            # Put the channel back in the "available" queue if no clients are using it.
            # This MUST happen inside the lock to prevent a race condition where:
            # 1. We decrement to 0 and release lock
            # 2. Another coroutine finds this channel via _all_channels scan
            # 3. That coroutine increments to 1
            # 4. We then put a channel with count=1 into the queue
            # By keeping put() inside the lock, we ensure the channel truly has
            # count=0 when added to the queue.
            if channel_info.is_available and channel_info.client_count == 0:
                await self._available_channels.put(channel_info)

    async def close_all(self) -> None:
        """Close all channels and clean up the pool."""
        self._closed = True

        async with self._lock:
            logger.info(f"{self._log_prefix}Closing {len(self._all_channels)} channels")

            # Close all channels
            for channel_info in list(self._all_channels.values()):
                await self._close_channel_info(channel_info)

            # Clear the queue
            while not self._available_channels.empty():
                try:
                    self._available_channels.get_nowait()
                except asyncio.QueueEmpty:
                    break

            self._all_channels.clear()
            self._initialized = False

        logger.info(f"{self._log_prefix}All channels closed")

    def pool_stats(self) -> dict:
        """
        Get statistics about the current pool state.

        :return: Dictionary with pool statistics
        """
        # Snapshot the channel entries into a local before the Python-level
        # loop so the read is atomic under the GIL. pool_stats() is called from
        # cross-thread readers (the OTel metrics-scrape daemon thread and the
        # /metrics/channel-pool reader) without holding self._lock, while
        # _all_channels is mutated on the event-loop thread; iterating the live
        # dict could raise "dictionary changed size during iteration". The
        # snapshot is a consistent-enough point-in-time view for these stats.
        channel_infos = list(self._all_channels.values())
        total_count = len(channel_infos)
        available_count = self._available_channels.qsize()

        # Count channels that have active clients and total client connections
        in_use_count = 0
        total_client_count = 0
        for channel_info in channel_infos:
            total_client_count += channel_info.client_count
            if channel_info.client_count > 0:
                in_use_count += 1

        # Calculate max capacity (channels * clients per channel). On an
        # overflow-enabled pool this is the soft packing threshold rather than
        # a ceiling, so total_client_count may legitimately exceed it; the
        # readiness servicer and the autoscaler feed both key on it as the
        # point where the pool stops growing and starts packing.
        max_client_capacity = self._config.pool_size * self._config.client_limit

        return {
            "min_pool_size": self._config.min_size,
            "max_pool_size": self._config.pool_size,
            "channel_client_limit": self._config.client_limit,
            "total_channel_count": total_count,
            "available_channel_count": available_count,
            "in_use_channel_count": in_use_count,
            "total_client_count": total_client_count,
            "max_client_capacity": max_client_capacity,
            "overflow_enabled": self._config.overflow_enabled,
            "initialized": self._initialized,
            "closed": self._closed,
        }

    def _least_loaded_channel(self, limit: int | None) -> ChannelInfo | None:
        """
        Find the open channel carrying the fewest clients.

        Closed channels are skipped. When limit is given, only channels
        strictly under it are considered; passing None considers every open
        channel regardless of client count, which is what the overflow tier
        needs. Ties go to the first channel encountered at the lowest count.

        NOTE: This method reads _all_channels and must only be called while
        holding self._lock. It does not acquire the lock itself.

        :param limit: The client count a channel must be under to qualify, or
            None to consider every open channel.
        :return: The least-loaded qualifying channel, or None if there is none
        """
        best_channel = None
        lowest_client_count = float("inf")

        for channel_info in self._all_channels.values():
            if channel_info.is_closed():
                continue
            if limit is not None and channel_info.client_count >= limit:
                continue
            if channel_info.client_count < lowest_client_count:
                best_channel = channel_info
                lowest_client_count = channel_info.client_count

        return best_channel

    async def _get_best_available_channel(self) -> ChannelInfo:
        """
        Get the best available channel,
        preferring channels with the lowest client count to distribute the load evenly.
        Creates new channels if needed and possible.

        Selection walks these tiers in order:

        1. Drain the queue of channels sitting at zero clients.
        2. Scan for the least-loaded open channel under client_limit.
        3. Grow the pool with a new channel if it holds fewer than pool_size.
        4. Overflow, on an overflow-enabled pool only: pack onto the
           least-loaded open channel even though it is already at or past
           client_limit.
        5. Last resort: block on the queue until a channel is returned, then
           retry from the top.

        Tier 4 sits after the growth tier, under the same lock, and that
        ordering is what guarantees overflow never engages while the pool can
        still grow -- reaching it is itself the proof that the pool is full,
        so no separate growth re-check is needed. It also means that with
        overflow enabled, tier 5 is reached only in the degenerate states
        where there is no open channel to pack onto: pool_size of 0, or an
        _all_channels that is empty or holds only closed entries.

        This method atomically increments the client count under the lock to prevent
        race conditions where multiple coroutines could select the same channel.

        :return: The best available channel info (with client count already incremented)
        """
        # First priority: channels with 0 clients (from the queue)
        while True:
            try:
                channel_info = self._available_channels.get_nowait()
                # noinspection PyUnresolvedReferences
                if channel_info.is_closed():
                    # Remove a closed channel from the pool
                    async with self._lock:
                        await self._remove_channel_info(channel_info)
                    continue

                # Validate and increment under lock. Between get_nowait() and here,
                # another coroutine could have found this channel via the second
                # priority path (scanning _all_channels) and incremented it.
                async with self._lock:
                    # noinspection PyUnresolvedReferences
                    if channel_info.client_count < self._config.client_limit:
                        channel_info.increment_clients()
                        return channel_info
                    # Channel was claimed by another coroutine, try the next one
                    continue

            except asyncio.QueueEmpty:
                break

        # Second priority: find the existing channel with the lowest client count
        async with self._lock:
            best_channel = self._least_loaded_channel(self._config.client_limit)

            if best_channel:
                # Increment client count atomically under lock before returning
                best_channel.increment_clients()
                return best_channel

            # Third priority: create a new channel if the pool is not full
            if len(self._all_channels) < self._config.pool_size:
                channel_info = await self._create_channel_info()
                channel_info.increment_clients()
                return channel_info

            # Fourth priority: overflow. Every channel is open and at or past
            # client_limit, and reaching this point already proves the pool
            # cannot grow, so admit the client anyway onto the least-loaded
            # channel rather than failing a stream that autoscaling cannot
            # move elsewhere. The service's memory-safe stream cap, not the
            # pool, bounds how far this band runs.
            if self._config.overflow_enabled:
                best_channel = self._least_loaded_channel(None)
                if best_channel:
                    best_channel.increment_clients()
                    logger.debug(
                        f"{self._log_prefix}Pool full, admitting client past the "
                        f"channel client limit: channel now has "
                        f"{best_channel.client_count} clients, limit is "
                        f"{self._config.client_limit}"
                    )
                    return best_channel

        # Last resort: wait for a channel to become available
        channel_info = await self._available_channels.get()

        if channel_info.is_closed():
            # Remove a closed channel from the pool and try again
            async with self._lock:
                await self._remove_channel_info(channel_info)
            return await self._get_best_available_channel()

        # Validate and increment under lock:
        # the same race condition as the first priority
        async with self._lock:
            if channel_info.client_count < self._config.client_limit:
                channel_info.increment_clients()
                return channel_info

        # The channel was claimed by another coroutine while we waited, try again
        return await self._get_best_available_channel()

    async def _create_channel_info(self) -> ChannelInfo:
        """
        Create a new channel and wrap it in ChannelInfo.

        NOTE: This method modifies _all_channels and must only be called while
        holding self._lock to prevent race conditions.

        :return: New ChannelInfo instance
        """
        channel = await self._connection.channel()
        await channel.set_qos(prefetch_count=self._config.prefetch_count)

        channel_info = ChannelInfo(channel, self._config)
        self._all_channels[channel] = channel_info

        logger.debug(
            f"{self._log_prefix}Created new channel, "
            f"pool size: {len(self._all_channels)}"
        )
        return channel_info

    async def _remove_channel_info(self, channel_info: ChannelInfo) -> None:
        """
        Remove a channel info from the pool and close the channel.

        :param channel_info: The channel info to remove
        """
        if channel_info.channel in self._all_channels:
            del self._all_channels[channel_info.channel]

        await self._close_channel_info(channel_info)

    async def _close_channel_info(self, channel_info: ChannelInfo) -> None:
        """
        Close a channel if it's not already closed.

        An instance method rather than a static one so the failure path can
        name the pool the channel belonged to.

        :param channel_info: The channel info to close
        """
        if not channel_info.is_closed():
            try:
                await channel_info.channel.close()
            except Exception as e:
                logger.warning(f"{self._log_prefix}Error closing channel: {e}")
