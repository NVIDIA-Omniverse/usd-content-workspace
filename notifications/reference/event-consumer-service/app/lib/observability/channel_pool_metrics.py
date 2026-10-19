# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging
from typing import Callable, Iterable

from opentelemetry import metrics
from opentelemetry.metrics import CallbackOptions, Observation

from notification_common.config.common_settings import common_settings
from notification_common.config.metadata import project_name

logger = logging.getLogger(__name__)


class ChannelPoolMetrics:
    """
    Observable-gauge view of the RabbitMQ channel pools.

    Every gauge here is an OTel OBSERVABLE gauge whose callback reads the
    already-maintained ``RabbitMQConnector.pool_stats_by_pool()`` snapshot at
    SCRAPE time and yields one ``Observation`` per pool (attribute
    ``pool`` in ``{non_durable, durable}``). Nothing runs on the per-stream or
    per-message consume path: the pool already tracks these counts as part of
    normal checkout/return bookkeeping, so the gauges add zero hot-path work and
    only cost a single O(channels) stats read per collection cycle, at the
    exporter's (infrequent) cadence.

    This closes the observability gap behind the 2026-08-11 channel-pool
    exhaustion incident: pool utilization, the active gRPC consume-stream count
    (``total_client_count``), and the packing density
    (``total_client_count`` / ``total_channel_count``, a dashboard expression)
    become first-class metrics instead of living only in the ``/metrics/
    channel-pool`` pull endpoint and the autoscaler's aggregate log line.
    """

    def __init__(self):
        if not common_settings.metrics_enabled:
            return

        self.meter = metrics.get_meter_provider().get_meter(project_name())

        self.meter.create_observable_gauge(
            "channel_pool.total_channel_count",
            callbacks=[self._make_callback("total_channel_count")],
            unit="{channel}",
            description="Channels currently open in the pool",
        )
        self.meter.create_observable_gauge(
            "channel_pool.in_use_channel_count",
            callbacks=[self._make_callback("in_use_channel_count")],
            unit="{channel}",
            description="Channels carrying at least one client",
        )
        self.meter.create_observable_gauge(
            "channel_pool.available_channel_count",
            callbacks=[self._make_callback("available_channel_count")],
            unit="{channel}",
            description="Channels available for checkout",
        )
        self.meter.create_observable_gauge(
            "channel_pool.total_client_count",
            callbacks=[self._make_callback("total_client_count")],
            unit="{stream}",
            description="Active consume streams (clients) across the pool's channels",
        )
        self.meter.create_observable_gauge(
            "channel_pool.max_pool_size",
            callbacks=[self._make_callback("max_pool_size")],
            unit="{channel}",
            description="Maximum channels the pool will grow to before packing",
        )
        self.meter.create_observable_gauge(
            "channel_pool.channel_client_limit",
            callbacks=[self._make_callback("channel_client_limit")],
            unit="{stream}",
            description="Soft per-channel client (stream) packing limit",
        )
        self.meter.create_observable_gauge(
            "channel_pool.max_client_capacity",
            callbacks=[self._make_callback("max_client_capacity")],
            unit="{stream}",
            description="Soft stream capacity (max_pool_size * channel_client_limit)",
        )

    @staticmethod
    def _make_callback(
        stat_key: str,
    ) -> Callable[[CallbackOptions], Iterable[Observation]]:
        """
        Build an observable-gauge callback that reports ``stat_key`` per pool.

        The callback reads the connector's per-pool stats at collection time and
        yields one Observation per pool. It is deliberately defensive:
        ``pool_stats_by_pool()`` returns ``{}`` before ``connect()`` and omits a
        pool that does not exist yet, and a given stats dict may not carry the
        key, so every lookup degrades to "emit nothing" rather than raising --
        a raising callback would drop the whole scrape. As a backstop the whole
        read-and-yield is wrapped so any unexpected error (e.g. the stats read
        racing a concurrent channel checkout/return on the event-loop thread)
        also yields nothing rather than propagating out and dropping the scrape.
        """

        def _observe(options: CallbackOptions) -> Iterable[Observation]:
            # Imported lazily so this module does not import the rabbitmq
            # package at load time (the guard/connector import graph pulls in
            # settings); the connector singleton is the same one every capacity
            # surface reads.
            from app.lib.rabbitmq.rabbitmq_base import rabbitmq_connector

            try:
                by_pool = rabbitmq_connector.pool_stats_by_pool() or {}
                for pool_name, stats in by_pool.items():
                    value = stats.get(stat_key)
                    if value is not None:
                        yield Observation(value, {"pool": pool_name})
            except Exception:
                # This callback runs on the exporter's collection thread; a
                # raising callback drops the entire scrape. Swallow any error so
                # one stray scrape is skipped instead. Logged at debug only --
                # this is off the hot path but should stay cheap.
                logger.debug(
                    "channel_pool.%s gauge scrape skipped after error",
                    stat_key,
                    exc_info=True,
                )

        return _observe


channel_pool_metrics = ChannelPoolMetrics()
