# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from opentelemetry import metrics
from opentelemetry.metrics import Counter, Histogram, UpDownCounter

from notification_common.config.common_settings import common_settings
from notification_common.config.metadata import project_name


class CacheWriterMetrics:
    """
    Metrics for the auth-cache write-through batcher.

    There is one module-level singleton; each writer identifies itself
    via the ``queue_ref`` argument so OTel attribute aggregation produces
    one independent time-series per queue.

    That argument is a REFERENCE to the queue, not its id. These writers
    exist only for non-durable queues, whose id is the stream's reconnect
    token -- not a credential, but the queue's identifier, so an authorized
    principal that obtains it could attach to the same stream and take a share
    of its events; it is kept out of telemetry as defense in depth -- so the
    caller redacts it (see ``app.lib.queue_names.queue_ref_for_name``) before
    it becomes an exported attribute. The reference is stable, so the
    time-series is still one series per queue.

    It is also the bare ``sha256-<12 hex>`` reference, byte-identical to
    the ``queue_ref`` span attribute the same stream carries, so a series
    here joins that trace by exact match rather than by substring.

    One queue can have more than one live writer: the cache is namespaced by
    queue AND presenting principal, so two principals attached to one
    non-durable queue run one writer each. They deliberately report under the
    SAME ``queue_ref`` -- a principal component in an exported attribute
    would break that exact-match join, and answers no operator question.
    What must NOT be shared between them is the depth baseline, which is why
    ``record_queue_depth`` and ``forget_queue`` take a separate
    ``baseline_key`` (the writer's cache-scope key) for that bookkeeping.

    That makes the baseline per SCOPE, not per writer -- the distinction
    matters only for a same-scope replacement, where a principal's new
    writer is registered before the replaced one has finished draining.
    A re-attach by that principal reaches this, and so does a second
    concurrent client of it sharing its token on this pod: both resolve to
    one scope key. Those two writers share one baseline for that
    overlap, so the replaced writer's final zero sample subtracts the
    successor's depth as well as its own and the series reads low. It
    self-corrects on the successor's next sample: that sample is a delta
    against the zero its predecessor left behind, so it re-adds the
    successor's full depth.
    """

    def __init__(self):
        if not common_settings.metrics_enabled:
            return

        self.meter = metrics.get_meter_provider().get_meter(project_name())

        # queue_depth is gauge-shaped (current asyncio.Queue size,
        # sampled by each writer's drainer). UpDownCounter is used
        # here rather than an observable gauge to match the codebase
        # convention (see SseMetrics.active_connections) and because
        # we already have a natural sampling site inside the drainer
        # loop — a callback-driven observable gauge would require us
        # to expose a registry of live writers to a periodic poller.
        # Tradeoff: an UpDownCounter only accepts deltas, so we
        # convert each absolute sample into a delta against the last
        # reported value. The previous-value bookkeeping is keyed by
        # the caller's `baseline_key` -- one per cache scope -- so that
        # the concurrent writers of one queue's two principals don't
        # share a baseline. It is NOT keyed by queue_ref: those two
        # writers share that attribute.
        self.queue_depth: UpDownCounter = self.meter.create_up_down_counter(
            "cache_writer.queue_depth",
            unit="{op}",
            description="Current number of Ops buffered in the CacheWriter",
        )
        self._last_queue_depth: dict[str, int] = {}

        self.flush_latency: Histogram = self.meter.create_histogram(
            "cache_writer.flush_latency",
            unit="ms",
            description="Wall-time per CacheWriter flush",
        )

        self.flush_ops: Counter = self.meter.create_counter(
            "cache_writer.flush_ops",
            unit="{op}",
            description="Number of Ops successfully flushed by the CacheWriter",
        )

        self.drops: Counter = self.meter.create_counter(
            "cache_writer.drops",
            unit="{op}",
            description="Number of Ops dropped on CacheWriter buffer overflow",
        )

    def record_flush(self, queue_ref: str, ops_count: int, latency_ms: float) -> None:
        if not common_settings.metrics_enabled:
            return

        attrs = {"queue_ref": queue_ref}
        self.flush_ops.add(ops_count, attrs)
        self.flush_latency.record(latency_ms, attrs)

    def record_drop(self, queue_ref: str, count: int = 1) -> None:
        if not common_settings.metrics_enabled:
            return

        self.drops.add(count, {"queue_ref": queue_ref})

    def record_queue_depth(self, queue_ref: str, depth: int, baseline_key: str) -> None:
        """
        Report a writer's current buffer depth as a delta against the last
        sample reported for its cache scope.

        :param queue_ref: The exported attribute; one series per queue.
        :param baseline_key: Identifies the writer's cache SCOPE for the
            delta bookkeeping. Required, and distinct from ``queue_ref``,
            because two scopes can report under one ``queue_ref`` -- sharing
            the baseline would make each one's sample cancel the other's.
            Writers within one scope (a same-scope replacement) do share it;
            see the class docstring.
        """
        if not common_settings.metrics_enabled:
            return

        previous = self._last_queue_depth.get(baseline_key, 0)
        delta = depth - previous
        if delta != 0:
            self.queue_depth.add(delta, {"queue_ref": queue_ref})
            self._last_queue_depth[baseline_key] = depth

    def forget_queue(self, baseline_key: str) -> None:
        """
        Drop the per-scope baseline state for a writer that has stopped.

        Should be called once after that writer's final
        ``record_queue_depth`` (which will typically be 0), with the same
        ``baseline_key``. Without this, the dict grows unboundedly across
        reconnects.
        """
        if not common_settings.metrics_enabled:
            return

        self._last_queue_depth.pop(baseline_key, None)


cache_writer_metrics = CacheWriterMetrics()
