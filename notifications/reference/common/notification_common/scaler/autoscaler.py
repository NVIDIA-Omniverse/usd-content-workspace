# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Autoscaler for services with channel pool limits.

This module provides the main autoscaling loop that:
1. Polls pods for channel pool metrics
2. Calculates total available capacity across all pods
3. Makes scaling decisions against a minimum available capacity threshold and
   a minimum replica floor. Scale-up fires on either of two conditions: the
   instantaneous available capacity is below the threshold, or the deployment
   sits below the replica floor. Scale-down is deliberately not symmetric with
   either: it requires sustained excess capacity -- the minimum observed over a
   trailing window -- and is additionally suppressed for a stabilization period
   after any scale-up that actually executed, so the fleet cannot hand back a
   pod it just added
4. Scales the deployment via Kubernetes API
"""

import asyncio
import logging
import signal
import time
from collections import deque

from notification_common.scaler.k8s_client import K8sClient, K8sClientError, PodInfo
from notification_common.scaler.metrics_client import MetricsClient, ChannelPoolMetrics
from notification_common.scaler.scaler_settings import scaler_settings

logger = logging.getLogger(__name__)

# Annotation key for pod deletion cost (Kubernetes built-in feature)
_POD_DELETION_COST_ANNOTATION = "controller.kubernetes.io/pod-deletion-cost"


class Autoscaler:
    """
    Autoscaler for services with channel pool capacity limits.

    Monitors total available capacity across all pods and scales the
    deployment up or down to maintain a minimum available capacity buffer.
    """

    def __init__(self):
        self._k8s_client = K8sClient()
        self._metrics_client = MetricsClient()

        # Anchor for every time-based guard that protects capacity. A scaler
        # pod that restarts mid-incident (node drain, rollout, OOM, eviction)
        # has no memory of what the previous instance did, so any such guard
        # that starts at the epoch reads as "long expired" and fires on the
        # very first iteration after the restart. Any state added later that
        # slows scale-down (minimum-over-a-window tracking, post-scale-up
        # stabilization) must initialize from this field for the same reason:
        # a restart must never be a way to bypass it.
        self._process_start_time: float = time.time()

        # Deliberate asymmetry: only the scale-DOWN cooldown is armed from
        # process start. Amnesia on the down path is fail-dangerous (a restart
        # removes capacity during an incident); on the up path it is fail-safe
        # (at worst one extra pod, and the starting_pods gate in _try_scale_up
        # closes the loop as soon as that pod is visible). Anchoring the
        # scale-up cooldown would instead delay emergency scale-up by a full
        # cooldown after every restart, which is exactly the reaction time we
        # need to preserve.
        self._last_scale_up_time: float = 0
        self._last_scale_down_time: float = self._process_start_time

        self._last_available_capacity: int = 0

        # Trailing (timestamp, total_available) samples feeding the scale-down
        # decision, which uses their minimum rather than the latest reading.
        # The window is anchored at _process_start_time, never at the first
        # sample: a scaler that just restarted has no memory of what the
        # previous instance saw, and must observe a full window of sustained
        # excess capacity itself before it may give any capacity back.
        self._capacity_window: deque[tuple[float, int]] = deque()

        # Post-scale-up stabilization: scale-down stays suppressed until this
        # anchor is older than the configured stabilization window. This is a
        # separate field from _last_scale_up_time on purpose. That one stays 0
        # so a restarted scaler can still scale up immediately (and so
        # _get_poll_interval does not read a fresh process as "recently scaled
        # up"); this one is armed from process start so a restart cannot be a
        # way to bypass the window. Post-restart the process-start-anchored
        # scale-down cooldown dominates it anyway, so the window is inert
        # there -- it can only ever be the more conservative of the two.
        self._scale_up_stabilization_anchor: float = self._process_start_time

        self._shutdown_event = asyncio.Event()

    async def run(self) -> None:
        """
        Run the autoscaler main loop.

        This method runs indefinitely, polling pods and making scaling
        decisions until a shutdown signal is received.
        """
        self._log_startup_config()
        self._setup_signal_handlers()

        try:
            await self._main_loop()
        finally:
            await self._cleanup()

    @staticmethod
    def _log_startup_config() -> None:
        """Log the autoscaler configuration on startup."""
        logger.info(
            f"Starting autoscaler for deployment "
            f"'{scaler_settings.scaler_target_deployment}' "
            f"in namespace '{scaler_settings.scaler_target_namespace}'"
        )
        min_cap = scaler_settings.scaler_min_available_capacity
        down_cooldown = scaler_settings.scaler_scale_down_cooldown_seconds
        down_window = scaler_settings.scaler_scale_down_window_seconds
        stabilization = scaler_settings.scaler_post_scale_up_stabilization_seconds
        logger.info(
            f"Config: min_available_capacity={min_cap}, "
            f"min_replicas={scaler_settings.scaler_min_replicas}, "
            f"max_replicas={scaler_settings.scaler_max_replicas}, "
            f"scale_down_cooldown={down_cooldown}s "
            f"(armed_from=process_start), "
            f"scale_down_window={down_window}s, "
            f"post_scale_up_stabilization={stabilization}s"
        )

        # A floor above the ceiling is reported once here rather than rejected
        # by a cross-field validator, for the reason scaler_settings.py records
        # for the window/stabilization pair: a cross-field failure raises at
        # import time and crashes the scaler instead of degrading it. The
        # decision path clamps only the UPWARD enforcement to the ceiling, so
        # the misconfiguration costs no capacity -- but it does mean the two
        # directions stop agreeing on one number, and where the fleet settles
        # depends on which side of the pair it started on. An operator has to
        # be told exactly once, in those terms.
        if scaler_settings.scaler_min_replicas > scaler_settings.scaler_max_replicas:
            logger.warning(
                f"min_replicas ({scaler_settings.scaler_min_replicas}) is above "
                f"max_replicas ({scaler_settings.scaler_max_replicas}); the two "
                f"directions are enforced at different values: upward floor "
                f"enforcement is capped at max_replicas "
                f"({scaler_settings.scaler_max_replicas}), while scale-down "
                f"still refuses to shrink below the configured min_replicas "
                f"({scaler_settings.scaler_min_replicas}), so the fleet holds "
                f"wherever it already sits within "
                f"[{scaler_settings.scaler_max_replicas}, "
                f"{scaler_settings.scaler_min_replicas}]"
            )

    def _setup_signal_handlers(self) -> None:
        """Set up signal handlers for graceful shutdown."""
        loop = asyncio.get_running_loop()

        for sig in (signal.SIGINT, signal.SIGTERM):
            # noinspection PyTypeChecker
            loop.add_signal_handler(sig, self._handle_shutdown_signal)

    def _handle_shutdown_signal(self) -> None:
        """Handle shutdown signals."""
        logger.info("Received shutdown signal, stopping autoscaler...")
        self._shutdown_event.set()

    async def _main_loop(self) -> None:
        """The main autoscaling loop."""
        while not self._shutdown_event.is_set():
            try:
                await self._scaling_iteration()
            except K8sClientError as e:
                logger.error(f"Kubernetes API error: {e}")
            except Exception as e:
                logger.exception(f"Unexpected error in scaling iteration: {e}")

            # Determine the poll interval based on current state
            poll_interval = self._get_poll_interval()

            # Wait for the poll interval, but allow early exit on shutdown
            try:
                await asyncio.wait_for(
                    self._shutdown_event.wait(), timeout=poll_interval
                )
            except asyncio.TimeoutError:
                pass  # Normal timeout, continue the loop

    async def _scaling_iteration(self) -> None:
        """Perform a single scaling iteration."""
        pods = await self._k8s_client.list_pods(
            namespace=scaler_settings.scaler_target_namespace,
            label_selector=scaler_settings.label_selector,
        )

        # Drop terminating pods (deletionTimestamp set) up front, before any
        # capacity accounting. A pod on SIGTERM stays phase=Running with a
        # podIP and keeps answering /metrics/channel-pool with its full
        # channels_total for the whole preStop + drain window while refusing
        # every new stream, so its reported slots are phantom capacity. Filter
        # here, where pod identity is still in scope, so a terminating pod is
        # never scraped, never in the capacity sum, not counted as a starting
        # pod, and gets no deletion-cost patch. This is orthogonal to readiness:
        # _scaling_iteration deliberately keeps unready Running pods (an unready
        # pod may be saturated and must still be counted).
        live_pods = [p for p in pods if not p.terminating]

        running_pods = [p for p in live_pods if p.phase == "Running"]
        non_running_pods = [p for p in live_pods if p.phase != "Running"]

        if not running_pods:
            logger.warning("No running pods found")
            return

        # Attempt to collect metrics from ALL running pods, not
        # just ready ones. The metrics response itself tells us
        # whether an unready pod is saturated (responded) or
        # still starting up (did not respond).
        metrics = await self._collect_metrics(running_pods)

        if not metrics:
            logger.warning("Could not get metrics from any pod")
            return

        # Pods that returned metrics are either ready or saturated
        # (capacity-exhausted). Pods that are Running but did NOT
        # return metrics are genuinely still starting up. Non-Running
        # pods (e.g. Pending) are also starting.
        pods_with_metrics = {m.pod_name for m in metrics}
        starting_pods = non_running_pods + [
            p for p in running_pods if p.name not in pods_with_metrics
        ]

        await self._update_pod_deletion_costs(running_pods, metrics)

        total_available, pod_capacity, max_available = self._calculate_capacity(metrics)

        current_replicas = await self._k8s_client.get_deployment_replicas(
            namespace=scaler_settings.scaler_target_namespace,
            deployment_name=(scaler_settings.scaler_target_deployment),
        )

        total_capacity = sum(m.channels_total for m in metrics)
        total_used = total_capacity - total_available

        # One timestamp for the whole iteration, shared by the window sample
        # and by every time-based guard the decision consults, so the sample
        # and the cooldowns can never disagree about when "now" was.
        now = time.time()

        # Only iterations that actually produced metrics record a sample. The
        # early returns above (no running pods, no metrics from any pod)
        # deliberately do not: the absence of an observation is not an
        # observation of low load, and recording one would let a metrics
        # outage age into permission to scale down.
        windowed_available = self._record_available_capacity(now, total_available)

        window_min = "n/a" if windowed_available is None else windowed_available
        logger.info(
            f"Pods: {len(metrics)} reporting, "
            f"{len(starting_pods)} starting | "
            f"Capacity: {total_used}/{total_capacity} used, "
            f"{total_available} available "
            f"(min: {scaler_settings.scaler_min_available_capacity}), "
            f"window min {window_min}"
            f" | Replicas: {current_replicas}"
        )

        # Adaptive polling stays on the instantaneous reading. Hot polling is
        # meant to react to a real low reading as it happens, and feeding it
        # the window minimum would only hold polling hot long after the load
        # that justified it went away.
        self._last_available_capacity = total_available

        await self._make_scaling_decision(
            now,
            total_available,
            windowed_available,
            pod_capacity,
            max_available,
            current_replicas,
            len(starting_pods),
        )

    def _record_available_capacity(
        self, now: float, total_available: int
    ) -> int | None:
        """
        Record an availability sample and return the trailing window minimum.

        The minimum is used rather than an average or an EWMA because it is
        strictly conservative: it is never above the instantaneous reading, so
        smoothing can only ever suppress a scale-down the unsmoothed logic
        would have taken, never manufacture one. A timestamped window is also
        invariant to the poll cadence, which switches between the hot and cold
        intervals and would otherwise need a per-sample correction.

        Samples are recorded verbatim, including negative values. Availability
        goes negative whenever channels in use exceed the reported capacity --
        routinely under the overflow admission band, and also when a pod
        reports zero capacity while still holding channels. A negative sample
        only holds the minimum down, which suppresses scale-down, so there is
        nothing to clamp.

        :param now: The timestamp of this observation
        :param total_available: Total available slots across all pods
        :return: The minimum available capacity over the trailing window, or
            None when the window is not yet backed by observation
        """
        window = scaler_settings.scaler_scale_down_window_seconds
        cutoff = now - window

        self._capacity_window.append((now, total_available))

        # Drop a sample only once the one behind it has itself aged past the
        # cutoff. That retains the newest sample at or before the cutoff, so
        # the window stays covered edge to edge instead of starting at
        # whichever sample happens to be the oldest survivor.
        while len(self._capacity_window) >= 2 and self._capacity_window[1][0] <= cutoff:
            self._capacity_window.popleft()

        # No amnesia across a restart: until the process itself has been up for
        # a full window, it cannot have watched one, whatever its samples say.
        if now - self._process_start_time < window:
            return None

        # A startup blackout: the process is old enough, but metrics only began
        # arriving recently, so nothing observed reaches back to the cutoff.
        # This can only fire before the first straddling sample is retained --
        # once one exists the prune rule always keeps one there.
        if self._capacity_window[0][0] > cutoff:
            return None

        return min(v for _, v in self._capacity_window)

    async def _collect_metrics(self, pods: list[PodInfo]) -> list[ChannelPoolMetrics]:
        """
        Collect metrics from all pods concurrently.

        :param pods: List of pods to collect metrics from
        :return: List of successfully collected metrics
        """
        tasks = [self._metrics_client.get_pod_metrics(pod.name, pod.ip) for pod in pods]

        results = await asyncio.gather(*tasks, return_exceptions=True)

        metrics = []
        for result in results:
            if isinstance(result, ChannelPoolMetrics):
                metrics.append(result)
            elif isinstance(result, Exception):
                logger.warning(f"Error collecting metrics: {result}")

        return metrics

    @staticmethod
    def _calculate_capacity(
        metrics: list[ChannelPoolMetrics],
    ) -> tuple[int, int, int]:
        """
        Calculate the total available capacity across all pods.

        :param metrics: List of metrics from pods
        :return: Tuple of (total_available_slots, largest capacity reported by
                 any healthy reporter, max_available_on_single_pod)
        """
        if not metrics:
            return 0, 0, 0

        total_available = 0
        max_available = 0
        pod_capacity = 0
        zero_capacity_reporters = 0
        for m in metrics:
            available = m.channels_total - m.channels_in_use
            total_available += available
            max_available = max(max_available, available)
            if m.channels_total == 0:
                zero_capacity_reporters += 1
            # Take the largest capacity any pod reports rather than an
            # arbitrary reporter's. A pod that is restarting or has not built
            # its channel pool yet reports channels_total=0, and
            # such a reporter must not collapse the scale-down deadband
            # (total_available >= min_capacity + pod_capacity) down to no
            # deadband at all. During a rollout that raises per-pod capacity
            # the fleet reports a mix of old and new values; the widest pod
            # bounds what a single deletion can cost, so the max keeps the
            # deadband at least as wide as the capacity actually lost.
            pod_capacity = max(pod_capacity, m.channels_total)

        # Give the common deadband-protection event a Loki signature. A
        # still-starting or restarting pod reports channels_total=0 and is
        # discarded from pod_capacity by the max rule above; without this line
        # an operator diagnosing "why didn't it scale down during that rollout"
        # has no signal that a zero-capacity reporter was distorting capacity
        # (the permanently-unreachable pod_capacity<=0 case already WARNs in
        # _make_scaling_decision). One summary count per poll, not per pod, so
        # this is a low-frequency line with no hot-path cost.
        if zero_capacity_reporters:
            logger.info(
                f"Discarding {zero_capacity_reporters} zero-capacity "
                f"reporter(s) (channels_total=0) from pod_capacity; "
                f"pod_capacity={pod_capacity}"
            )

        return total_available, pod_capacity, max_available

    async def _update_pod_deletion_costs(
        self,
        monitored_pods: list[PodInfo],
        metrics: list[ChannelPoolMetrics],
    ) -> None:
        """
        Update pod deletion cost annotations based on available capacity.

        Pods with more available capacity get a lower (more negative) deletion
        cost, making them preferred for deletion during scale-down. Pods with
        less available capacity (more active clients) get a higher deletion
        cost, protecting them from deletion.

        Pods that failed to return metrics are given a high deletion cost to
        protect them from deletion (we can't know their actual utilization).

        :param monitored_pods: All pods being monitored for metrics
        :param metrics: Metrics from pods that responded successfully
        """
        namespace = scaler_settings.scaler_target_namespace

        pods_with_metrics = {m.pod_name for m in metrics}

        # Update deletion costs sequentially. With typically < 10 pods, this
        # adds negligible latency and avoids connection/timeout issues that
        # can occur with concurrent requests to the K8s API.
        for m in metrics:
            # Use negative available capacity as deletion cost.
            # Pods with more available capacity have a lower (more negative)
            # cost and are preferred for deletion. This ensures the pod with
            # the most available capacity is deleted first during scale-down.
            available_capacity = m.channels_total - m.channels_in_use
            deletion_cost = str(-available_capacity)
            await self._patch_pod_deletion_cost(namespace, m.pod_name, deletion_cost)

        # For pods that didn't return metrics, set a high deletion cost to
        # protect them from deletion. We don't know their utilization, so we
        # assume they may have active connections.
        for pod in monitored_pods:
            if pod.name not in pods_with_metrics:
                # Use a high positive value to protect the pod from deletion
                logger.warning(
                    f"Pod {pod.name} did not return metrics, "
                    "setting high deletion cost to protect it"
                )
                await self._patch_pod_deletion_cost(namespace, pod.name, "1000000")

    async def _patch_pod_deletion_cost(
        self, namespace: str, pod_name: str, deletion_cost: str
    ) -> None:
        """
        Patch the deletion cost annotation on a single pod.

        :param namespace: The Kubernetes namespace
        :param pod_name: The name of the pod
        :param deletion_cost: The deletion cost value to set
        """
        try:
            await self._k8s_client.patch_pod_annotation(
                namespace=namespace,
                pod_name=pod_name,
                annotation_key=_POD_DELETION_COST_ANNOTATION,
                annotation_value=deletion_cost,
            )
        except K8sClientError as e:
            logger.warning(f"Failed to update deletion cost for pod {pod_name}: {e}")

    async def _make_scaling_decision(
        self,
        now: float,
        total_available: int,
        windowed_available: int | None,
        pod_capacity: int,
        max_available: int,
        current_replicas: int,
        starting_pods: int,
    ) -> None:
        """
        Make and execute a scaling decision based on available capacity.

        Scale-up: When the instantaneous available capacity falls below the
        minimum threshold, or when the deployment is below the replica floor.
        Both take the same action, so capacity pressure is tested first and the
        reason logged is the one that actually justifies the scale-up.
        Scale-down: When the windowed minimum available capacity exceeds the
        minimum + one pod's worth of capacity.

        The floor keys on the deployment's spec.replicas -- what
        K8sClient.get_deployment_replicas returns -- not on how many pods are
        reporting metrics. That is what makes it reach the case it exists for: a
        chart that omits spec.replicas so a helm upgrade cannot fight the
        autoscaler leaves Kubernetes defaulting a fresh Deployment to 1, and a
        fleet that never sees load has no capacity pressure to climb from. It is
        evaluated statelessly from the live settings and the current replica
        count, so a scaler that has just restarted enforces the floor on its
        first observed iteration with no anchored state to rebuild -- amnesia on
        the up path is fail-safe, as it already is for the scale-up cooldown.

        Enforcement is limited to iterations that reach a decision at all: the
        two early returns in _scaling_iteration (no running pods, no metrics
        from any pod) still take no action and record no sample, because the
        absence of an observation is not an observation.

        :param now: Timestamp shared with the window sample for this iteration
        :param total_available: Total available slots across all pods, as read
            this iteration
        :param windowed_available: Minimum available slots over the trailing
            scale-down window; None when the window is not yet backed by
            observation, which forbids scale-down
        :param pod_capacity: Largest capacity reported by any healthy
            reporter; 0 means the capacity is unknown and scale-down is
            suppressed
        :param max_available: Max available capacity on any single pod
            (will be deleted first during scale-down)
        :param current_replicas: Current number of replicas
        :param starting_pods: Number of pods still starting up (did
            not respond to the metrics endpoint)
        """
        min_capacity = scaler_settings.scaler_min_available_capacity
        scale_down_threshold = min_capacity + pod_capacity

        # The floor actually enforced. Clamped to the ceiling so a min > max
        # misconfiguration cannot ask for a scale-up that _try_scale_up's max
        # check refuses, which would log "Already at max replicas" at WARNING on
        # every poll for the life of the process. _log_startup_config names that
        # misconfiguration once at startup instead.
        effective_floor = min(
            scaler_settings.scaler_min_replicas, scaler_settings.scaler_max_replicas
        )

        # Deliberately asymmetric: the capacity-pressure scale-UP branch reads
        # the instantaneous capacity, never the windowed minimum. Running out of
        # capacity is an emergency and must not be damped -- waiting a window to
        # confirm it would be waiting while clients are refused. Only the
        # scale-DOWN side demands sustained evidence. (Kubernetes draws the same
        # line: its HPA defaults the scale-up stabilization window to 0 and the
        # scale-down one to 300 seconds.)
        if total_available < min_capacity:
            await self._try_scale_up(
                now,
                current_replicas,
                starting_pods,
                reason=f"available {total_available} < min {min_capacity}",
            )

        # The replica minimum is a floor in both directions, not merely a lower
        # bound on scale-down: a fleet found below it is raised to it whatever
        # the load. Subordinate to capacity pressure above only because the two
        # branches take the same action and the pressing reason is the one worth
        # logging. Sitting above the scale-down chain costs nothing -- a fleet
        # below the floor is one the min-replicas clamp in _try_scale_down would
        # refuse to shrink anyway.
        elif current_replicas < effective_floor:
            await self._try_scale_up(
                now,
                current_replicas,
                starting_pods,
                reason=(
                    f"below min replicas {effective_floor} "
                    f"(available {total_available})"
                ),
            )

        # A pod_capacity of 0 means no reporter published a usable capacity, so
        # the deadband below cannot be evaluated: treat the capacity as unknown
        # and refuse to scale down. This is defense in depth and is not
        # reachable through _scaling_iteration — with every reporter at 0,
        # total_available is -sum(channels_in_use) <= 0, which is always below
        # the (validator-enforced >= 1) min_capacity, and the capacity-pressure
        # branch above tests that same instantaneous value, so such a fleet
        # always scales up instead. Smoothing does not change this: it replaces
        # only the scale-down input. Only a direct call to this method with a
        # hand-built (pod_capacity=0, total_available >= min_capacity) pair and
        # current_replicas >= effective_floor exercises the warning — below the
        # floor, the branch above claims the iteration first.
        elif pod_capacity <= 0:
            logger.warning(
                "Suppressing scale-down: no reporter published a usable pod "
                f"capacity (pod_capacity={pod_capacity}, "
                f"available {total_available} >= min {min_capacity})"
            )

        # Only scale down if we'd still have min_capacity after losing a pod,
        # judged on the windowed minimum so that a momentary trough in load
        # cannot buy a pod deletion. The deadband itself is unchanged, and
        # pod_capacity stays the current snapshot's: what a deletion would cost
        # is a property of the fleet as it is now, not of its history.
        elif (
            windowed_available is not None
            and windowed_available >= scale_down_threshold
        ):
            await self._try_scale_down(
                now,
                current_replicas,
                windowed_available,
                total_available,
                pod_capacity,
                max_available,
                starting_pods,
            )

        # Smoothing changed the outcome: this iteration's reading on its own
        # would have permitted the scale-down and the window refused it. At
        # INFO because the suppression messages inside _try_scale_down are
        # DEBUG and therefore invisible at production log levels, and gated on
        # the outcome actually changing so it costs at most one line per poll,
        # only while the fleet is sitting on the threshold.
        elif total_available >= scale_down_threshold:
            window = scaler_settings.scaler_scale_down_window_seconds
            window_min = "n/a" if windowed_available is None else windowed_available
            logger.info(
                "Suppressing scale-down: excess capacity not sustained "
                f"(available now {total_available}, window min {window_min} "
                f"over {window}s, threshold {scale_down_threshold})"
            )

    async def _try_scale_up(
        self,
        now: float,
        current_replicas: int,
        starting_pods: int,
        *,
        reason: str,
    ) -> None:
        """
        Attempt to scale up the deployment.

        :param now: Current timestamp
        :param current_replicas: Current number of replicas
        :param starting_pods: Number of pods still starting up
        :param reason: Why the scale-up was asked for, rendered verbatim in the
            SCALE UP line. Required rather than defaulted because more than one
            condition reaches here, and the line must not assert a capacity
            shortfall for a scale-up the replica floor asked for.
        """
        # Don't scale up if there are already pods starting.
        # Wait for them to become ready first.
        if starting_pods > 0:
            logger.info(f"Skipping scale-up: {starting_pods} pod(s) " f"still starting")
            return

        # Check cooldown
        time_since_last = now - self._last_scale_up_time
        if time_since_last < scaler_settings.scaler_scale_up_cooldown_seconds:
            logger.info(
                f"In scale-up cooldown "
                f"({time_since_last:.0f}s/"
                f"{scaler_settings.scaler_scale_up_cooldown_seconds}s)"
            )
            return

        # Check max replicas
        if current_replicas >= scaler_settings.scaler_max_replicas:
            logger.warning(
                f"Already at max replicas ({scaler_settings.scaler_max_replicas})"
            )
            return

        # Scale up by 1
        new_replicas = current_replicas + 1
        logger.info(f"SCALE UP: {current_replicas} -> {new_replicas} ({reason})")

        await self._k8s_client.scale_deployment(
            namespace=scaler_settings.scaler_target_namespace,
            deployment_name=scaler_settings.scaler_target_deployment,
            replicas=new_replicas,
        )

        self._last_scale_up_time = now

        # Both stamps happen only here, after scale_deployment actually
        # succeeded. A scale-up that was gated (pods still starting, at max
        # replicas) or cooled down added no capacity, so it must not hold off
        # scale-down.
        self._scale_up_stabilization_anchor = now

    async def _try_scale_down(
        self,
        now: float,
        current_replicas: int,
        windowed_available: int,
        total_available: int,
        pod_capacity: int,
        max_available: int,
        starting_pods: int,
    ) -> None:
        """
        Attempt to scale down the deployment.

        :param now: Current timestamp
        :param current_replicas: Current number of replicas
        :param windowed_available: Minimum available capacity over the trailing
            scale-down window -- the value the decision was made on
        :param total_available: Total available capacity across all pods as
            read this iteration, carried purely so the log line can report the
            fleet's current state alongside the windowed value it acted on
        :param pod_capacity: Largest capacity reported by any healthy
            reporter, which bounds what a single deletion can cost
        :param max_available: Max available capacity on any single pod (will be
            deleted first)
        :param starting_pods: Number of pods still starting up. Scale-down waits
            for the fleet to settle: the moment a starting pod begins reporting,
            total_available jumps by a full pod-step, which can cross the
            scale-down deadband and immediately undo the scale-up that just
            completed.
        """
        min_capacity = scaler_settings.scaler_min_available_capacity

        # Don't scale down while pods are starting, symmetric with the scale-up
        # gate. Checked first so the cooldown timestamp is never consulted or
        # stamped for a decision that was going to be refused anyway.
        #
        # Known limitation: K8sClient._parse_pods only emits PodInfo for pods
        # that already have a podIP, so a pod created seconds ago is invisible
        # here. This closes the large window (pod exists, booting, not yet
        # reporting) but not the brief pre-IP sliver; the scale-down cooldown
        # remains the backstop for that.
        if starting_pods > 0:
            logger.info(f"Skipping scale-down: {starting_pods} pod(s) still starting")
            return

        # Don't scale down for a stabilization period after a scale-up actually
        # executed. Sits directly after the starting-pods gate so the two "the
        # fleet just grew / is growing" refusals stay together, and before the
        # cooldown for the same reason that gate is first: never consult or
        # stamp the cooldown for a decision already refused.
        #
        # At INFO, like the other scale-down suppression lines: this is the "why
        # did it not scale down" answer an incident timeline needs, and it is
        # invisible at production log levels as DEBUG. The volume is bounded --
        # _try_scale_down is only reached once the windowed comparison has
        # already passed. The scaling-decision suppression lines in this
        # function (starting-pods, this stabilization gate, cooldown, and
        # min-replicas) are all INFO so the refusal reason is reconstructable
        # from production logs regardless of which gate wins.
        stabilization = scaler_settings.scaler_post_scale_up_stabilization_seconds
        time_since_scale_up = now - self._scale_up_stabilization_anchor
        if time_since_scale_up < stabilization:
            logger.info(
                f"In post-scale-up stabilization window "
                f"({time_since_scale_up:.0f}s/{stabilization}s)"
            )
            return

        # Check cooldown
        time_since_last = now - self._last_scale_down_time
        if time_since_last < scaler_settings.scaler_scale_down_cooldown_seconds:
            logger.info(
                f"In scale-down cooldown "
                f"({time_since_last:.0f}s/"
                f"{scaler_settings.scaler_scale_down_cooldown_seconds}s)"
            )
            return

        # Check min replicas
        if current_replicas <= scaler_settings.scaler_min_replicas:
            logger.info(
                f"Already at min replicas ({scaler_settings.scaler_min_replicas})"
            )
            return

        # Scale down by 1. The pod with the most available capacity will be
        # deleted first (due to deletion cost annotation).
        new_replicas = current_replicas - 1
        # Both terms come from the same snapshot. Subtracting a single pod's
        # current availability from the windowed minimum would mix a historical
        # quantity with a current one and describe a fleet that never existed.
        remaining_available = total_available - max_available
        logger.info(
            f"SCALE DOWN: {current_replicas} -> {new_replicas} "
            f"(window min {windowed_available} >= min {min_capacity} + "
            f"pod_capacity {pod_capacity}, "
            f"available now {total_available}, "
            f"remaining after scale: ~{remaining_available})"
        )

        await self._k8s_client.scale_deployment(
            namespace=scaler_settings.scaler_target_namespace,
            deployment_name=scaler_settings.scaler_target_deployment,
            replicas=new_replicas,
        )

        self._last_scale_down_time = now

    def _get_poll_interval(self) -> float:
        """
        Get the appropriate poll interval based on current capacity.

        Uses adaptive polling: faster when capacity is low, or we recently
        scaled, slower when capacity is high and stable.

        :return: Poll interval in seconds
        """
        now = time.time()
        min_capacity = scaler_settings.scaler_min_available_capacity

        # Use fast polling if:
        # 1. We recently scaled up (still stabilizing)
        # 2. Available capacity is below 2x the minimum (getting tight)
        recent_scale_up = (
            now - self._last_scale_up_time
            < scaler_settings.scaler_scale_up_cooldown_seconds * 2
        )
        capacity_is_low = self._last_available_capacity < min_capacity * 2

        if recent_scale_up or capacity_is_low:
            return scaler_settings.scaler_poll_interval_hot_seconds
        else:
            return scaler_settings.scaler_poll_interval_cold_seconds

    async def _cleanup(self) -> None:
        """Clean up resources."""
        logger.info("Cleaning up autoscaler resources...")
        await self._k8s_client.close()
        await self._metrics_client.close()
        logger.info("Autoscaler shutdown complete")


async def run_autoscaler() -> None:
    """
    Entry point for running the autoscaler.

    This function creates an Autoscaler instance and runs it.
    """
    # Configure logging exactly like the main service does in logging_config.py
    log_fmt = "%(asctime)s - " + logging.BASIC_FORMAT
    logging.basicConfig(
        level=getattr(logging, scaler_settings.scaler_log_level.upper()),
        format=log_fmt,
    )

    # Validate settings
    if not scaler_settings.scaler_target_deployment:
        raise ValueError("OMNI_EVENTS_SCALER_TARGET_DEPLOYMENT must be set")

    if scaler_settings.scaler_min_available_capacity < 1:
        raise ValueError("OMNI_EVENTS_SCALER_MIN_AVAILABLE_CAPACITY must be at least 1")

    logger.info("Autoscaler starting...")
    autoscaler = Autoscaler()
    await autoscaler.run()
