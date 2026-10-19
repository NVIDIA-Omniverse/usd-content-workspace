# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

from grpc_health.v1 import health_pb2
from grpc_health.v1.health import aio as health

from notification_common.rabbitmq.rabbitmq_connector import (
    RabbitMQConnector,
)

logger = logging.getLogger(__name__)


class ChannelPoolHealthServicer(health.HealthServicer):
    """
    A gRPC health servicer that reports readiness based on the
    non-durable channel pool's available STREAM slots.

    Availability is measured in streams (clients), not channels,
    so the verdict stays truthful at any packing density -- once
    a channel carries more than one client, in-use channel counts
    understate how much room is left.

    Readiness keys on the non-durable pool alone. The durable
    pool has its own capacity budget and initializes with zero
    channels, so it must never gate readiness.

    Uses the readiness reserve threshold so that the pod is
    removed from the primary Kubernetes Service (and therefore
    from kube-proxy routing for internal clients) before the
    pool is fully exhausted. Readiness is a scaling and steering
    signal: it keys on the soft threshold -- the non-durable
    pool's own stream capacity -- less the reserve, so it trips
    earlier than any refusal does. This is intentionally a
    different threshold from the CapacityGuard, whose hard
    threshold on the non-durable path is the memory-safe stream
    cap. Re-routable streams are refused earlier than that, at
    the shed threshold inside the overflow band, and the strict
    pool bound survives only as the incomputable-cap fallback.
    A not-ready pod therefore keeps admitting: the -envoy
    Service (publishNotReadyAddresses: true) keeps the pod in
    Envoy's EDS cluster, which is what lets a pinned connection
    burst past the soft threshold after internal traffic has
    been redirected away by the readiness probe.

    NOT_SERVING when:
    - The connector is draining (graceful shutdown), regardless of
      available capacity
    - The non-durable channel pool does not exist yet
    - The non-durable channel pool has not been initialized yet
    - The non-durable channel pool is closed
    - Available stream slots drop below the configured reserve
    """

    def __init__(
        self,
        connector: RabbitMQConnector,
        min_available_streams: int,
    ):
        super().__init__()
        self._connector = connector
        self._min_available_streams = min_available_streams

    async def Check(self, request, context):
        status = self._determine_status()
        return health_pb2.HealthCheckResponse(status=status)

    def _determine_status(self):
        # A draining pod flips readiness to NOT_SERVING ahead of every
        # capacity check: it is going away, so it must stop being a target
        # even while it still has free stream slots (SR-6.1). The flip is
        # necessary but NOT sufficient to de-route streaming traffic.
        # Streaming reaches pods via the -envoy Service, which sets
        # publishNotReadyAddresses: true; Kubernetes then forces
        # conditions.ready = true for every address on that Service, and
        # Contour's EndpointSlice translator
        # (internal/xdscache/v3/endpointslicetranslator.go,
        # RecalculateEndpoints) gates endpoint selection solely on the
        # EndpointSlice `ready` condition -- it never inspects
        # `serving`/`terminating`. So a not-ready/terminating endpoint is
        # still routed to, and this readiness flip alone does NOT de-route a
        # draining pod. The load-bearing stop-new-arrivals mechanism is
        # app-side: CapacityGuard refuses new non-durable streams while
        # draining (see app/lib/rabbitmq/capacity_guard.py). This citation
        # lives here so the terminating-endpoint question is not
        # re-litigated.
        if self._connector.draining:
            logger.debug("Readiness: NOT_SERVING (connector draining)")
            return health_pb2.HealthCheckResponse.NOT_SERVING

        by_pool = self._connector.pool_stats_by_pool() or {}
        stats = by_pool.get(RabbitMQConnector.NON_DURABLE_POOL_NAME) or {}

        if not stats or not stats.get("initialized"):
            logger.debug("Readiness: NOT_SERVING (non-durable pool not initialized)")
            return health_pb2.HealthCheckResponse.NOT_SERVING

        if stats.get("closed"):
            logger.debug("Readiness: NOT_SERVING (non-durable pool closed)")
            return health_pb2.HealthCheckResponse.NOT_SERVING

        available = stats["max_client_capacity"] - stats["total_client_count"]

        if available < self._min_available_streams:
            logger.info(
                "Readiness: NOT_SERVING "
                f"(available streams: {available}, "
                f"threshold: {self._min_available_streams})"
            )
            return health_pb2.HealthCheckResponse.NOT_SERVING

        return health_pb2.HealthCheckResponse.SERVING
