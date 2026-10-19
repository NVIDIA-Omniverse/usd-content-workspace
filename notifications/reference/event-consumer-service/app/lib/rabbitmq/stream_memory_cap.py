# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Memory-safe cap on concurrent non-durable streams (SDD SR-3.5).

A non-durable stream's worst case is its whole prefetch window sitting
delivered-but-unacked on the broker, so the number of streams a pod may admit
has to track the configured prefetch rather than being a constant. This module
owns that arithmetic; the capacity guard consumes it, and this cap is the
guard's sole hard admission threshold for non-durable streams.

Measurement provenance
----------------------
Both constants below were measured on 2026-08-13 against the rc cluster -- a
prod-sized RabbitMQ 4.1.3 broker: 3-node quorum, 4Gi ``mem_limit`` and a 0.6
relative high watermark per node, ~600 B JSON event payloads. The raw numbers,
the harness that produced them, and worked RAM-sizing examples are in the
source repository's internal design record for non-durable channel capacity,
which is not published.

- Per held (delivered-but-unacked) message: a measured 1.5-1.8 kB band, roughly
  1.0-1.2 kB of broker metadata over the 600 B payload. That is about twice the
  RabbitMQ docs' ">= 720 B" classic-metadata floor, so sizing from the
  documented floor would under-provision by ~2x.
- Per-stream channel share: ~5 kB, measured at ~8 streams per channel.

Scope of the channel-share term
-------------------------------
``BYTES_PER_STREAM_CHANNEL_SHARE`` is the measured share at ~8 streams/channel.
It is genuinely negligible at the default prefetch (5 kB against ~883 kB of
prefetch-held messages), but it is a flat per-stream term and therefore
UNDER-counts at low packing density: at ``channel_client_limit=1`` a stream owns
a whole channel, whose measured cost at scale is ~31-43 kB including its
connection share -- about 8x this term. The flat form is what SR-3.5 specifies,
so it is used as specified rather than being claimed universally negligible.
The under-count is bounded (tens of kB per stream) and is dominated by the
prefetch term at any prefetch worth capping.

Reconnect-grace orphan backlog (SR-3.5, amended 2026-08-17)
-----------------------------------------------------------
This cap counts LIVE streams, so there is one broker-memory term it structurally
cannot see: the queues left behind by dropped non-durable streams during their
reconnect grace. An orphaned queue holds no stream, so it contributes nothing to
the count the cap divides the budget across.

The mechanism: ``rabbitmq_base`` declares a non-durable queue with only
``{"x-expires": non_durable_queue_ttl_seconds * 1000}`` (default 20 s). There is
no ``x-message-ttl`` and no ``x-max-length`` -- only the DLQ carries a length
bound. So for the whole grace window the queue stays bound to the topic
exchange and keeps accumulating every matching event, with no consumer draining
it and nothing bounding its depth.

Under mass churn (a reconnect storm, a rolling restart) the resulting term is::

    orphaned queues x matching event rate x grace seconds x per-message cost

Unlike the prefetch-held term above, this memory is disk-pageable -- the broker
can page these bodies out under pressure, since nothing is holding them in an
unacked delivery window -- and it is time-bounded: each orphan self-deletes at
the ``x-expires`` deadline.

It is deliberately NOT folded into ``per_stream_worst_case_bytes``. That
function answers "what does one live stream hold?", and this term is neither
per-stream (it scales with churn and event rate, not with admitted streams) nor
prefetch-shaped. Inflating the per-stream figure to absorb it would tax every
steady-state stream for a transient, pageable cost. It belongs instead in
``broker_stream_memory_budget_mib`` and in broker sizing: the budget must be set
low enough relative to real broker memory to leave headroom for this backlog.

No ``x-max-length`` is set on non-durable queues, and that is a settled decision
(recorded 2026-08-18 on the SR-3.5 amendment), not an open question: the term is
handled purely here in the sizing model, and the reconnect-token "no missed
events" guarantee stays fully intact. The rationale is that ``x-max-length``
applies to a queue at ALL times, not only while it is orphaned, so a live-but-
slow consumer's backlog would be silently drop-headed too -- an unacceptable
weakening of the delivery semantics in exchange for bounding a memory term that
is already disk-pageable and grace-TTL-bounded.

Coupling contract
-----------------
``memory_safe_stream_cap`` reads the PROCESS-GLOBAL
``common_settings.rabbitmq_channel_prefetch_count``. That is correct only while
the non-durable pool carries NO explicit ``ChannelPoolConfig.prefetch_count``
override: the connector builds that pool with no per-pool config, so the pool
late-binds to this same global on every read and applies it verbatim via
``set_qos`` (per-consumer, ``global_=False``, which is what makes the
prefetch-window term per-stream rather than per-channel). Any change that gives
the non-durable pool its own prefetch MUST re-point this formula at that pool's
effective prefetch in the same change, or the cap silently computes from a
number the pool no longer uses.
"""

from __future__ import annotations

import logging
from functools import cache

from notification_common.config.common_settings import common_settings

from app.config import settings

logger = logging.getLogger(__name__)

# Top of the measured 1.5-1.8 kB band, chosen deliberately so the cap errs
# conservative: over-estimating the per-message cost yields a smaller, safer
# stream budget.
BYTES_PER_HELD_MESSAGE = 1800

# The ~5 kB per-stream channel share measured at ~8 streams/channel. See the
# module docstring for where this term under-counts.
BYTES_PER_STREAM_CHANNEL_SHARE = 5120

_BYTES_PER_MIB = 1024 * 1024


def per_stream_worst_case_bytes(prefetch_count: int) -> int:
    """
    Return the worst-case broker memory one non-durable stream can hold.

    The worst case is a fully saturated prefetch window -- every delivered
    message unacked -- plus the stream's share of its channel.

    :param prefetch_count: The QoS prefetch applied to the stream's consumer.
    :return: Worst-case bytes held on the broker by a single stream.
    """
    return prefetch_count * BYTES_PER_HELD_MESSAGE + BYTES_PER_STREAM_CHANNEL_SHARE


@cache
def _warn_unbounded_prefetch_once(prefetch_count: int) -> None:
    """
    Log the unbounded-prefetch warning once per distinct prefetch value.

    Cached because the caller sits on the admission path; without this the
    warning would be emitted once per request. Tests may call
    ``_warn_unbounded_prefetch_once.cache_clear()``.

    :param prefetch_count: The offending configured prefetch, for the message.
    """
    logger.warning(
        f"rabbitmq_channel_prefetch_count is {prefetch_count}, which means "
        "unlimited prefetch, so the per-stream broker memory footprint is "
        "undefined and no memory-safe stream cap can be computed. Set "
        "max_non_durable_streams_per_pod to cap non-durable streams "
        "explicitly, or configure a positive prefetch."
    )


def memory_safe_stream_cap() -> int | None:
    """
    Return the per-pod cap on concurrent non-durable streams.

    The operator override wins unconditionally. Otherwise the fleet-wide
    memory budget is divided by the worst-case per-stream footprint at the
    configured prefetch and then apportioned across the maximum fleet size.

    Deliberately not memoized: it is a handful of integer operations, and
    caching would freeze the result against a later settings change, which is
    the import-time-freeze problem this module exists to avoid.

    :return: The per-pod stream cap, or None when no memory cap applies.
    """
    override = settings.settings.max_non_durable_streams_per_pod
    if override is not None:
        return override

    prefetch = common_settings.rabbitmq_channel_prefetch_count
    if prefetch <= 0:
        # 0 means "unlimited" to set_qos and the pool still accepts it, so the
        # per-stream footprint is undefined rather than small. Computing from
        # 0 would yield an absurdly permissive cap -- the exact silent failure
        # this cap exists to prevent -- while clamping to a 1-stream cap would
        # turn an accepted config into a total outage. Report "no cap" loudly
        # instead and let the operator choose.
        _warn_unbounded_prefetch_once(prefetch)
        return None

    budget_bytes = settings.settings.broker_stream_memory_budget_mib * _BYTES_PER_MIB
    fleet_cap = budget_bytes // per_stream_worst_case_bytes(prefetch)
    per_pod = fleet_cap // settings.settings.scaler_max_replicas

    # Floor division throughout: the cap must never round UP past the budget.
    # The max(1, ...) floor keeps a pathologically small budget from wedging
    # the pod into rejecting 100% of traffic. It is a deliberate, documented
    # EXCEPTION to the fleet-budget invariant
    # (cap x scaler_max_replicas x per-stream bytes <= budget): in a
    # configuration that is already pathologically wrong, availability is
    # preferred over budget conformance, and the shed WARNING keeps the
    # misconfiguration visible rather than silent.
    return max(1, per_pod)
