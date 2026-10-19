# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import logging

from notification_common.errors.errors import (
    ChannelNotAvailableError,
)
from notification_common.rabbitmq.rabbitmq_connector import (
    RabbitMQConnector,
)

from app.config import settings
from app.lib.observability.admission_metrics import admission_metrics
from app.lib.rabbitmq.stream_memory_cap import memory_safe_stream_cap

logger = logging.getLogger(__name__)


def re_routable_shed_threshold(soft_threshold: int, hard_bound: int) -> int:
    """
    Return the stream count at which re-routable streams are shed.

    Re-routable streams -- ones a client retry can land on another pod --
    are refused inside the overflow band so the band above the shed
    threshold stays reserved for pinned streams, which no amount of
    scale-out can rescue (PRD FR-12, SDD SR-3.7).

    The operator override is clamped into ``[soft_threshold, hard_bound]``
    because neither end of the band is knowable at settings-validation
    time. Setting it at or above the hard bound is a supported posture --
    it disables the shed tier for fleets with nowhere to re-route to.
    When it is unset the threshold is the floor-division midpoint of the
    live band, ``soft + (hard - soft) // 2``: on a narrow band the
    re-routable half is the smaller one, and on a width-1 band it equals
    the soft threshold, shedding re-routable traffic at the packing product.

    Read per admission decision rather than cached, for the same reason
    ``memory_safe_stream_cap()`` is: caching would freeze the value
    against a later settings change.

    :param soft_threshold: The non-durable pool's own stream capacity,
        the report-only packing and scaling target.
    :param hard_bound: The stream count at which every class refuses --
        the memory-safe stream cap, or the pool bound under the
        incomputable-cap fallback.
    :return: The shed threshold, always within ``[soft_threshold,
        hard_bound]`` when the band is non-empty, and ``hard_bound``
        otherwise.
    """
    if hard_bound <= soft_threshold:
        # The band is empty or inverted -- degenerate under the
        # incomputable-cap fallback (where the pool bound IS the soft
        # threshold), and inverted whenever a generous packing product
        # exceeds the cap. At shipped defaults the band is NOT inverted
        # (soft 800 against a computed cap of ~3342); it inverts only under
        # operator misconfiguration -- e.g. raising prefetch back to 500 at
        # the shipped client_limit 8 collapses the cap to 118 against the
        # same 800 soft threshold. There is nothing to
        # reserve, so the shed tier collapses onto the hard bound and
        # every class refuses there. Without this short-circuit the
        # midpoint arithmetic takes a negative delta and lands ABOVE the
        # hard bound.
        return hard_bound

    override = settings.settings.max_re_routable_streams_per_pod

    if override is None:
        # Floor division so the midpoint never rounds up past the hard bound.
        return soft_threshold + (hard_bound - soft_threshold) // 2

    return min(max(override, soft_threshold), hard_bound)


class CapacityGuard:
    """
    Guards against admitting a new request when the selected channel
    pool can no longer safely carry another stream.

    Admission is measured in STREAMS (clients), not channels, and
    against one selected pool: the non-durable pool by default,
    the durable pool when the caller passes durable_pool=True.
    Each entry point gates on the resource it actually consumes,
    so a durable checkout no longer occupies a non-durable
    admission slot. Counting streams keeps the check truthful at
    any packing density -- once a channel carries more than one
    client, in-use channel counts understate the real load.

    Non-durable admission is THREE-TIER, and only the outer two
    tiers refuse:

    - The SOFT threshold is the non-durable pool's own stream
      capacity (max_client_capacity, the pool_size x
      channel_client_limit packing product). It refuses no one --
      it is the density the pool is provisioned to pack at and the
      signal the autoscaler reacts to, so it is reported, not
      enforced. Streams admitted above it land in the pool's
      overflow band (SR-1.5). Refusing there would strand a pinned
      connection: a consumer stream is bound to the pod it landed
      on for its whole life, so adding pods cannot rescue it and
      the refusal is a hard client failure rather than a rebalance
      (PRD FR-11).
    - The SHED threshold sits inside that overflow band and
      refuses RE-ROUTABLE streams only -- ones whose client retry
      can land on another pod. That reserves the rest of the band
      for pinned streams, for which scale-out is no remedy (PRD
      FR-12, SDD SR-3.7). See ``re_routable_shed_threshold()`` for
      where the threshold lands and how the operator override is
      clamped.
    - The HARD bound refuses EVERY class. The memory-safe stream
      cap is that bound: it sheds new streams with the normal
      rejection contract before an under-provisioned broker crosses
      its memory high watermark and blocks publishers, which would
      stall ingestion for every producer and consumer rather than
      just the marginal client.

    The class is supplied by the caller and defaults to pinned,
    the conservative side: misclassifying a re-routable stream as
    pinned costs only band fairness, while misclassifying a pinned
    stream as re-routable rejects traffic nothing can rescue --
    the exact failure FR-11 exists to prevent.

    The pool bound refuses on the non-durable path in exactly one
    case. When the cap is incomputable -- a prefetch of 0 or less
    leaves the per-stream broker footprint undefined -- the pool
    bound becomes the hard bound. That keeps admission bounded by a
    real number instead of running unbounded on a pod whose memory
    cost per stream is unknown, so the always-on invariant of
    SR-3.5 holds by falling back fail-safe rather than by falling
    through. That fallback refusal is CLASS-BLIND, the documented
    exception to FR-5: the pool bound and the soft threshold are
    the same number there, so the band is degenerate, the shed
    threshold collapses onto it, and both classes refuse at that
    one value.

    Durable admission is unchanged and single-tier: strict, gated
    on the durable pool's own stream budget, with no overflow band
    and no cap. The handful of long-lived durable queues are not
    the fan-out memory risk the cap exists to contain.

    Where this guard does read a pool's capacity -- the durable
    bound, and the non-durable incomputable-cap fallback -- it
    reads the real pool capacity, not the virtual/reserve threshold
    used by the readiness probe and autoscaler. The readiness probe
    reserve exists to give the autoscaler headroom and to prevent
    probe oscillation -- neither concern applies here because the
    guard is a per-request check, not a binary on/off signal. Using
    full pool capacity means Envoy traffic can use every stream
    slot the pool really has while internal traffic has already
    been redirected by the readiness probe.
    """

    def __init__(self, connector: RabbitMQConnector):
        self._connector = connector

    @staticmethod
    def _has_non_durable_capacity(stats: dict, re_routable: bool) -> bool:
        """
        Return True if the non-durable pool may admit another stream.

        The memory-safe stream cap is the hard bound and refuses every class.
        The pool's own stream capacity is a soft packing target that refuses
        no one, so a stream admitted above it lands in the pool's overflow
        band instead of being rejected. Inside that band the shed threshold
        refuses re-routable streams only, reserving the rest of the band for
        pinned ones. Only when the cap is incomputable does the pool bound
        take over as the hard bound, which keeps admission bounded rather
        than unbounded; the band is degenerate there, so that refusal is
        class-blind.

        The hard-bound check runs BEFORE the shed check so that at or above
        the hard bound operators see the cap line rather than the shed line,
        and so the fallback refusal stays class-blind.

        The cap and the shed threshold are read per admission decision rather
        than cached, so a settings change takes effect on the next request
        instead of being frozen at the guard singleton's import-time
        construction.

        :param stats: The non-durable pool's stats, already read by the caller.
        :param re_routable: True if a client retry of this stream can land on
            another pod, False if the stream is pinned to this pod.
        :return: True to keep admitting, False to shed this stream.
        """
        cap = memory_safe_stream_cap()
        soft_threshold = stats["max_client_capacity"]
        hard_bound = cap if cap is not None else soft_threshold
        streams = stats["total_client_count"]

        if streams >= hard_bound:
            if cap is not None:
                logger.warning(
                    f"Capacity check: memory-safe stream cap reached "
                    f"(streams: {streams}, cap: {cap})"
                )
                admission_metrics.record_shed(reason="memory_cap")
            else:
                pool_name = RabbitMQConnector.NON_DURABLE_POOL_NAME
                logger.info(
                    f"Capacity check: {pool_name} pool exhausted "
                    f"(streams in use: {streams}, max streams: {hard_bound})"
                )
                admission_metrics.record_shed(reason="pool_exhausted")

            return False

        if re_routable:
            shed = re_routable_shed_threshold(soft_threshold, hard_bound)

            if streams >= shed:
                logger.info(
                    f"Capacity check: shedding re-routable stream at the shed "
                    f"threshold (streams: {streams}, shed threshold: {shed}, "
                    f"soft threshold: {soft_threshold}, "
                    f"hard bound: {hard_bound})"
                )
                admission_metrics.record_shed(
                    reason="shed_threshold", stream_class="re_routable"
                )
                return False

        admission_metrics.record_admitted(re_routable)
        return True

    def has_capacity(
        self, durable_pool: bool = False, re_routable: bool = False
    ) -> bool:
        """
        Return True if the selected pool can accept new clients.

        Checks these conditions against that pool's own stats:
        - The pool has been initialized
        - The pool is not closed
        - On the non-durable path only, the connector is not draining --
          a draining pod refuses every new non-durable stream, class-blind,
          ahead of the tiers below (SR-6.1)
        - The pool's stream count is below that path's hard bound --
          the durable pool's own stream capacity on the durable path,
          and the memory-safe stream cap on the non-durable path,
          which falls back to the non-durable pool's stream capacity
          only when the cap is incomputable
        - On the non-durable path only, and for a re-routable stream
          only, the pool's stream count is also below the shed
          threshold

        :param durable_pool: True to check the durable channel pool,
            False (the default) to check the non-durable one.
        :param re_routable: True if a client retry of this stream can
            land on another pod, False (the default) if the stream is
            pinned to this pod. Ignored entirely on the durable path,
            which has no shed tier.
        """
        pool_name = (
            RabbitMQConnector.DURABLE_POOL_NAME
            if durable_pool
            else RabbitMQConnector.NON_DURABLE_POOL_NAME
        )
        by_pool = self._connector.pool_stats_by_pool() or {}
        stats = by_pool.get(pool_name) or {}

        if not stats or not stats.get("initialized"):
            logger.debug(
                f"Capacity check: no capacity ({pool_name} pool not initialized)"
            )
            return False

        if stats.get("closed"):
            logger.debug(f"Capacity check: no capacity ({pool_name} pool closed)")
            return False

        if not durable_pool:
            # A draining pod refuses EVERY new non-durable stream, ahead of
            # the soft/shed/hard tiers and irrespective of `re_routable`
            # (SR-6.1). This app-side refusal -- not the readiness flip -- is
            # the load-bearing stop-new-arrivals mechanism: streaming traffic
            # reaches pods via the -envoy Service with
            # publishNotReadyAddresses: true, so Kubernetes forces
            # conditions.ready = true for every address and Contour's
            # EndpointSlice translator
            # (internal/xdscache/v3/endpointslicetranslator.go,
            # RecalculateEndpoints) keeps routing to a not-ready/terminating
            # endpoint (it gates on the `ready` condition alone and never
            # inspects `serving`/`terminating`). The refusal is CLASS-BLIND:
            # the pod is going away, so there is no absorb-into-overflow-band
            # for a pinned stream here -- both pinned and re-routable new
            # streams are refused. Riding the existing refusal keeps the
            # contract shape unchanged (503 / gRPC RESOURCE_EXHAUSTED). The
            # durable path is deliberately left untouched.
            if self._connector.draining:
                logger.info(
                    "Capacity check: refusing new non-durable stream "
                    "(connector draining)"
                )
                admission_metrics.record_shed(reason="draining")
                return False

            return self._has_non_durable_capacity(stats, re_routable)

        available = stats["max_client_capacity"] - stats["total_client_count"]

        if available <= 0:
            logger.info(
                f"Capacity check: {pool_name} pool exhausted "
                f"(streams in use: {stats['total_client_count']}, "
                f"max streams: {stats['max_client_capacity']})"
            )
            return False

        return True

    def raise_if_at_capacity(
        self, durable_pool: bool = False, re_routable: bool = False
    ) -> None:
        """
        Raise ChannelNotAvailableError if the selected pool is at
        capacity. This is intended to be called at the start of
        request handlers so that new requests are rejected before
        attempting to acquire a channel from the pool.

        :param durable_pool: True to check the durable channel pool,
            False (the default) to check the non-durable one.
        :param re_routable: True if a client retry of this stream can
            land on another pod, False (the default) if the stream is
            pinned to this pod. Ignored entirely on the durable path,
            which has no shed tier.
        """
        if not self.has_capacity(durable_pool=durable_pool, re_routable=re_routable):
            raise ChannelNotAvailableError()


def _build_capacity_guard() -> CapacityGuard:
    """Build the module-level CapacityGuard singleton."""
    from app.lib.rabbitmq.rabbitmq_base import (
        rabbitmq_connector,
    )

    return CapacityGuard(connector=rabbitmq_connector)


capacity_guard = _build_capacity_guard()
