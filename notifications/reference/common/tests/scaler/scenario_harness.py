# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Deterministic scenario driver for load-shape tests against a real Autoscaler.

This module is a test helper, not a test module: it is deliberately not
``test_``-prefixed so pytest does not collect it. ``tests/scaler/`` has an
``__init__.py``, so consumers import it as
``from tests.scaler.scenario_harness import ...`` under pytest's default
prepend import mode.

**What it does.** It runs many consecutive ``Autoscaler._scaling_iteration``
calls against a simulated fleet whose pod count changes as the scaler's own
decisions take effect, on a simulated clock. Driving ``_scaling_iteration``
(rather than the decision helpers) is deliberate: the real starting-pods
derivation, the real capacity math, the real gate order and the real
cooldown/window/stabilization state all run. Only the Kubernetes and metrics
client boundary is mocked. ``_main_loop`` is never used -- it awaits a real
``asyncio.wait_for`` timeout and would put wall-clock sleeps in the suite.

**Two time idioms coexist in this package on purpose.** ``test_autoscaler.py``
uses the ``freezer`` fixture; this harness patches ``time.time`` directly with
a callable clock. freezegun also patches ``time.monotonic``, which is the
asyncio event loop's clock, and a scenario runs hundreds of ``await``s -- there
is no reason to take that on when the code under test only ever reads
``time.time()``. Do not "harmonize" the two.

**All scenario-facing times are relative to the scenario start (t0).**
``LoadPoint.at``, ``ScenarioDriver.run_until``'s ``t_end``, ``TickRecord.t``
and ``ScaleEvent.t`` are all seconds since t0. The fake clock is installed
before the ``Autoscaler`` is constructed, so the instance's
``_process_start_time`` is exactly t0 -- which is what makes the restart
regression and the no-amnesia behavior reachable from a scenario.

**Load model fidelity, and the direction it errs.** Real stream checkouts are
sticky to the pod that accepted them. This harness redistributes the whole
demand across the reporting pods on every tick, so a new pod instantly
relieves saturation and a deleted pod's load instantly reappears elsewhere.
Both biases flatter the scaler: the model recovers from a scale-up faster than
production does, and it never strands load on a pod that is going away. A
scenario that still shows a structural failure here shows it a fortiori in
production.

**The model cannot express the overflow regime.** Per-pod
``channels_in_use`` is clamped at that pod's ``channels_total``, so the
two-tier admission band (``channels_in_use > channels_total``, and with it
negative per-pod availability from routine overflow rather than from a
zero-capacity reporter) is not reachable here. That limitation is accepted for
this Epic's structural tests; the retuning before-picture is Epic 6's
(``t1.tj2.wi``) concern.

**The tick predicate flags are a non-vacuity discriminator, not an oracle.**
``TickRecord.below_min_capacity`` and
``TickRecord.unsmoothed_scale_down_eligible`` are the pre-Epic-4 branch
predicates evaluated at the LANDED ``pod_capacity`` (the max any reporter
published). They exist so a scenario can prove it actually drove the system
into the dangerous state rather than passing vacuously. They are NOT a
faithful reconstruction of the old logic: with a zero-capacity reporter in the
fleet the old code read ``metrics[0].channels_total``, so the flag understates
what the old predicate would have said. A scenario about the zero-capacity
reporter must compute that predicate by hand from ``TickRecord.metrics``.

**Fixtures.** ``fake_clock``, ``scenario_settings`` and ``scenario`` live here.
A scenario file must import ALL THREE, one per line so flake8's per-line
``noqa`` applies::

    from tests.scaler.scenario_harness import fake_clock  # noqa: F401
    from tests.scaler.scenario_harness import scenario  # noqa: F401
    from tests.scaler.scenario_harness import scenario_settings  # noqa: F401

Only ``scenario`` is requested directly, but pytest resolves a fixture's own
dependencies against the REQUESTING module's namespace, not against the module
the fixture was defined in, so the other two have to be visible there too.
"""

import inspect
import time
from dataclasses import dataclass
from unittest.mock import AsyncMock

import pytest
import pytest_asyncio

from notification_common.scaler.autoscaler import Autoscaler
from notification_common.scaler.k8s_client import PodInfo
from notification_common.scaler.metrics_client import ChannelPoolMetrics
from notification_common.scaler.scaler_settings import ScalerSettings, scaler_settings
from tests.scaler.test_autoscaler import make_metrics, make_pod_info

# Per-pod ``channels_total`` as the fleet reports it today: the channel pool
# size minus the reserve the service keeps back for its own use. This is the
# reported metric, NOT the raw pool size.
CHANNEL_UNIT_POD_CAPACITY = 80

# Per-pod ``channels_total`` after the Epic-3 change of accounting unit, where
# one unit is a client stream rather than a broker channel: 800 minus the same
# 80-unit reserve. Recorded as a hard input on the Epic.
STREAM_UNIT_POD_CAPACITY = 720

# How long after ``scale_deployment`` a new pod first appears in ``list_pods``.
# K8sClient._parse_pods only emits a PodInfo for a pod that already has a
# podIP, so a pod created seconds ago is invisible to the scaler entirely. The
# 2026-08-11 incident logged "0 starting" throughout a 5->8 scale-up that
# completed in roughly 27 seconds, which is only consistent with a new pod
# being invisible for most of its boot.
POD_VISIBLE_AFTER_SECONDS = 25.0

# How long after ``scale_deployment`` the new pod starts answering
# /metrics/channel-pool. Same derivation: the scale-up completed in ~27s, and
# the pod only counts as "reporting" once its metrics endpoint answers.
POD_READY_AFTER_SECONDS = 27.0

# A realistic wall-clock epoch for t0, deliberately not 0: a fresh Autoscaler
# leaves _last_scale_up_time at 0, and _get_poll_interval reads
# now - _last_scale_up_time, so a near-epoch t0 would make every fresh
# instance look like it had just scaled up and pin polling to the hot interval.
SCENARIO_T0 = 1_800_000_000.0

# Default ceiling on iterations per run_until call, sized as
# ceil(scenario duration / hot poll interval) with headroom: the ~80-unit
# incident replay measures at roughly 1600-1700 ticks, so 2500 leaves room
# without letting a runaway scenario hang the suite.
DEFAULT_MAX_TICKS = 2500

# The four damping knobs whose SHIPPED defaults a replay must run with, plus
# the thresholds and cadences the decision reads. Patched from the declared
# model defaults rather than from literals so the fixture is hermetic against a
# stray OMNI_EVENTS_* in the environment while still meaning "the values we
# actually ship".
_SHIPPED_DEFAULT_FIELDS = (
    "scaler_min_replicas",
    "scaler_max_replicas",
    "scaler_min_available_capacity",
    "scaler_scale_up_cooldown_seconds",
    "scaler_scale_down_cooldown_seconds",
    "scaler_scale_down_window_seconds",
    "scaler_post_scale_up_stabilization_seconds",
    "scaler_poll_interval_hot_seconds",
    "scaler_poll_interval_cold_seconds",
)

# Bound by name, never by position, so a later signature change surfaces as a
# clear KeyError here instead of silently shifting every recorded value.
_DECISION_SIGNATURE = inspect.signature(Autoscaler._make_scaling_decision)


def shipped_default(field_name: str):
    """
    Read a setting's declared default off the model.

    :param field_name: The ScalerSettings field name
    :return: The default declared in ScalerSettings for that field
    """
    return ScalerSettings.model_fields[field_name].default


class FakeClock:
    """
    A callable clock the driver owns, substituted for ``time.time``.

    Held as a callable rather than a value so the same object can be installed
    once, before the Autoscaler is constructed, and moved afterwards.
    """

    def __init__(self, now: float = SCENARIO_T0):
        self.now = now

    def __call__(self) -> float:
        return self.now

    def set(self, now: float) -> None:
        """
        Move the clock to an absolute time.

        :param now: The absolute timestamp the clock should read
        """
        self.now = now


@dataclass(frozen=True)
class LoadPoint:
    """
    A step in the demand timeline, holding until the next point.

    Demand is expressed as total concurrent stream checkouts across the whole
    fleet, NOT as per-pod availability, precisely so the same timeline can be
    replayed at two different per-pod capacities.
    """

    at: float
    streams: int


@dataclass(frozen=True)
class ScaleEvent:
    """A scale_deployment call the scaler actually made."""

    t: float
    direction: str
    from_replicas: int
    to_replicas: int


@dataclass(frozen=True)
class TickRecord:
    """
    What one ``_scaling_iteration`` saw and what the fleet looked like.

    Every capacity field comes from the real code: on an iteration that
    reached the decision they are the arguments ``_make_scaling_decision`` was
    called with, and on the two early-return paths (no running pods, no
    metrics from any pod) they come from calling the real
    ``Autoscaler._calculate_capacity`` on the metrics the harness served. No
    field re-implements the scaler's own formula.
    """

    t: float
    replicas: int
    reporting_pods: int
    starting_pods: int
    pod_capacity_reported: int
    total_available: int
    windowed_available: int | None
    demand: int
    total_in_use: int
    unplaceable_streams: int
    below_min_capacity: bool
    unsmoothed_scale_down_eligible: bool
    decision_reached: bool
    metrics: tuple[ChannelPoolMetrics, ...] = ()


@dataclass(eq=False)
class _SimPod:
    """
    One simulated pod. ``index`` is never reused, so ``pod-N`` is stable.

    Compared by identity (``eq=False``) so membership and removal against the
    ordered fleet list can never match the wrong pod.
    """

    index: int
    created_at: float
    terminating: bool = False
    pinned_in_use: int | None = None
    pinned_capacity: int | None = None
    in_use: int = 0


class ScenarioDriver:
    """
    Drives real scaling iterations against a simulated, mutating fleet.

    Fleet order is deterministic: ``list_pods`` returns the visible pods in
    fleet-list order, ``_collect_metrics`` preserves ``asyncio.gather`` order,
    so the metrics list index IS the fleet index. ``mark_terminating`` accepts
    ``first=True`` to move a pod to fleet position 0, which is how a scenario
    puts a zero-capacity reporter at ``metrics[0]``.

    Per-pod capacity is a constructor parameter, never a constant, so Epic 6
    (``t1.tj2.wi``) can re-drive every scenario at stream-unit values without
    restructuring anything.
    """

    def __init__(
        self,
        autoscaler: Autoscaler,
        mocker,
        *,
        pod_capacity: int,
        load: list[LoadPoint],
        initial_replicas: int,
        clock: FakeClock,
        visible_after: float = POD_VISIBLE_AFTER_SECONDS,
        ready_after: float = POD_READY_AFTER_SECONDS,
    ):
        """
        :param autoscaler: An Autoscaler constructed under ``clock``
        :param mocker: The pytest-mock fixture, used for every patch
        :param pod_capacity: The ``channels_total`` every healthy pod reports
        :param load: The fleet-wide demand timeline
        :param initial_replicas: Pods present, and reporting, at t0
        :param clock: The fake clock already installed over ``time.time``
        :param visible_after: Seconds from creation until a pod appears in
            ``list_pods``
        :param ready_after: Seconds from creation until a pod answers the
            metrics endpoint
        """
        if ready_after < visible_after:
            raise ValueError("ready_after must not precede visible_after")

        self._autoscaler = autoscaler
        self._clock = clock
        self._pod_capacity = pod_capacity
        self._load = sorted(load, key=lambda point: point.at)
        self._replicas = initial_replicas
        self._visible_after = visible_after
        self._ready_after = ready_after

        self._t0 = clock()
        self._now = 0.0
        self._next_index = 1
        self._fleet: list[_SimPod] = []
        self._ticks: list[TickRecord] = []
        self._scale_events: list[ScaleEvent] = []
        self._served: list[ChannelPoolMetrics] = []
        self._serving: dict[str, ChannelPoolMetrics] = {}
        self._unplaceable = 0

        # Pods present at scenario start report from t0.
        for _ in range(initial_replicas):
            self._add_pod(self._t0 - ready_after)

        # A spy, not a mock: the real decision still runs. It is the only way
        # to record the starting-pods count, the replica count and the
        # capacity figures exactly as the real derivation produced them,
        # without the harness deriving any of them itself.
        self._decision_spy = mocker.spy(autoscaler, "_make_scaling_decision")

        self.patch_pod_annotation = AsyncMock()
        self._patch_clients(mocker)

    @property
    def autoscaler(self) -> Autoscaler:
        """The instance under test."""
        return self._autoscaler

    @property
    def clock(self) -> FakeClock:
        """The clock installed over ``time.time``."""
        return self._clock

    @property
    def pod_capacity(self) -> int:
        """The ``channels_total`` every healthy pod reports."""
        return self._pod_capacity

    @property
    def now(self) -> float:
        """The scenario time the next tick will run at, in seconds since t0."""
        return self._now

    @property
    def replicas(self) -> int:
        """The deployment's current desired replica count."""
        return self._replicas

    @property
    def ticks(self) -> tuple[TickRecord, ...]:
        """Every iteration run so far, oldest first."""
        return tuple(self._ticks)

    @property
    def scale_events(self) -> tuple[ScaleEvent, ...]:
        """Every scale_deployment call the scaler made, oldest first."""
        return tuple(self._scale_events)

    def direction_changes(self) -> int:
        """
        Count sign flips of the replica delta across the scale events.

        This is the crisp anti-oscillation observable: a fleet that grows and
        then gives the pod back scores 1, a fleet that only grows scores 0.

        :return: How many times the direction of scaling reversed
        """
        changes = 0
        previous: str | None = None
        for event in self._scale_events:
            if previous is not None and event.direction != previous:
                changes += 1
            previous = event.direction
        return changes

    async def run_until(
        self, t_end: float, *, max_ticks: int = DEFAULT_MAX_TICKS
    ) -> None:
        """
        Run iterations until the scenario clock reaches ``t_end``.

        The clock advances by the scaler's OWN ``_get_poll_interval``, never by
        a fixed step: sample density inside the trailing window depends on the
        adaptive cadence, so a fixed tick would misrepresent the behavior under
        test.

        :param t_end: Scenario time to run to, in seconds since t0
        :param max_ticks: Iteration ceiling for THIS call, so a runaway
            scenario fails fast instead of hanging
        :raises AssertionError: If ``max_ticks`` is exceeded
        """
        ticks_this_call = 0
        while self._now < t_end:
            if ticks_this_call >= max_ticks:
                raise AssertionError(
                    f"Scenario exceeded max_ticks={max_ticks} at t={self._now:.1f}s "
                    f"before reaching t_end={t_end:.1f}s"
                )
            await self._tick()
            ticks_this_call += 1

    def pin_pod_load(self, pod_index: int, channels_in_use: int) -> None:
        """
        Hold one pod's in-use count fixed, excluding it from the demand split.

        A pinned pod is how a scenario builds a deliberately uneven fleet --
        the every-tick even split otherwise makes every pod a tie for the
        most-available-first deletion rule.

        :param pod_index: The pod's never-reused index (pod-N)
        :param channels_in_use: The in-use count that pod reports from now on
        """
        self._pod(pod_index).pinned_in_use = channels_in_use

    def pin_pod_capacity(self, pod_index: int, channels_total: int) -> None:
        """
        Hold one pod's reported ``channels_total`` at a value of its own.

        This is the mid-rollout fleet: a pod still running the previous image
        publishes the previous, narrower capacity while the rest of the fleet
        publishes the new one. It is the shape that discriminates the landed
        widest-healthy-reporter rule from the old ``metrics[0]`` one without a
        zero-capacity reporter, which the "capacity unknown" suppression guard
        would catch on its own.

        The even demand split assumes every free pod reports the same
        capacity, so a pod given a capacity of its own should normally also be
        pinned with ``pin_pod_load``; otherwise its share of the split clamps
        at the narrower value and the difference surfaces as unplaceable
        demand.

        :param pod_index: The pod's never-reused index (pod-N)
        :param channels_total: The capacity that pod reports from now on
        """
        self._pod(pod_index).pinned_capacity = channels_total

    def mark_terminating(
        self,
        pod_index: int | None = None,
        *,
        channels_in_use: int = 0,
        first: bool = False,
    ) -> int:
        """
        Turn a pod into a zero-capacity reporter.

        It stays Running and keeps reporting, but publishes
        ``channels_total=0`` -- the shape ``MetricsClient.get_pod_metrics``
        produces from its ``data.get(..., 0)`` defaults when a pod is
        terminating, restarting, or has not built its channel pool yet.

        :param pod_index: The pod to mark, defaulting to the first reporting
            pod in fleet order
        :param channels_in_use: What that pod still reports as in use
        :param first: Move the pod to fleet position 0, so it lands at
            ``metrics[0]``
        :return: The index of the pod that was marked
        """
        if pod_index is None:
            pod = self._reporting_pods()[0]
        else:
            pod = self._pod(pod_index)

        pod.terminating = True
        pod.pinned_in_use = channels_in_use

        if first:
            self._fleet.remove(pod)
            self._fleet.insert(0, pod)

        return pod.index

    async def aclose(self) -> None:
        """Release the autoscaler's clients, matching the suite's convention."""
        # noinspection PyProtectedMember
        await self._autoscaler._cleanup()

    def _patch_clients(self, mocker) -> None:
        """
        Mock only the Kubernetes and metrics client boundary.

        ``_collect_metrics``, ``_calculate_capacity``,
        ``_update_pod_deletion_costs``, ``_make_scaling_decision``,
        ``_try_scale_up`` and ``_try_scale_down`` all run for real.

        :param mocker: The pytest-mock fixture
        """
        k8s = self._autoscaler._k8s_client
        metrics = self._autoscaler._metrics_client
        mocker.patch.object(k8s, "list_pods", AsyncMock(side_effect=self._list_pods))
        mocker.patch.object(
            k8s,
            "get_deployment_replicas",
            AsyncMock(side_effect=self._get_deployment_replicas),
        )
        mocker.patch.object(
            k8s, "scale_deployment", AsyncMock(side_effect=self._scale_deployment)
        )
        mocker.patch.object(k8s, "patch_pod_annotation", self.patch_pod_annotation)
        mocker.patch.object(
            metrics, "get_pod_metrics", AsyncMock(side_effect=self._get_pod_metrics)
        )

    async def _tick(self) -> None:
        """Run one iteration at the current scenario time and record it."""
        self._clock.set(self._t0 + self._now)
        self._place_load()

        decisions_before = self._decision_spy.call_count
        # noinspection PyProtectedMember
        await self._autoscaler._scaling_iteration()
        decided = self._decision_spy.call_count > decisions_before

        self._ticks.append(self._build_tick(decided))

        # noinspection PyProtectedMember
        self._now += self._autoscaler._get_poll_interval()

    def _build_tick(self, decided: bool) -> TickRecord:
        """
        Capture what the iteration just run observed.

        :param decided: Whether the iteration reached the scaling decision
        :return: The record for this tick
        """
        if decided:
            decision = self._decision_arguments()
            total_available = decision["total_available"]
            pod_capacity_reported = decision["pod_capacity"]
            starting_pods = decision["starting_pods"]
            replicas = decision["current_replicas"]
            windowed_available = decision["windowed_available"]
        else:
            # The early-return paths never reach the decision, so the real
            # capacity call is made here on the very metrics that were served.
            total_available, pod_capacity_reported, _ = Autoscaler._calculate_capacity(
                self._served
            )
            starting_pods = len(self._visible_pods()) - len(self._served)
            replicas = self._replicas
            windowed_available = None

        min_capacity = scaler_settings.scaler_min_available_capacity
        return TickRecord(
            t=self._now,
            replicas=replicas,
            reporting_pods=len(self._served),
            starting_pods=starting_pods,
            pod_capacity_reported=pod_capacity_reported,
            total_available=total_available,
            windowed_available=windowed_available,
            demand=self._demand_at(self._now),
            total_in_use=sum(m.channels_in_use for m in self._served),
            unplaceable_streams=self._unplaceable,
            below_min_capacity=total_available < min_capacity,
            unsmoothed_scale_down_eligible=(
                total_available >= min_capacity + pod_capacity_reported
            ),
            decision_reached=decided,
            metrics=tuple(self._served),
        )

    def _decision_arguments(self) -> dict:
        """
        Read the last decision call's arguments by name.

        :return: The arguments ``_make_scaling_decision`` was last called with
        """
        call = self._decision_spy.call_args
        bound = _DECISION_SIGNATURE.bind(None, *call.args, **call.kwargs)
        bound.apply_defaults()
        return bound.arguments

    def _place_load(self) -> None:
        """
        Distribute the current demand across the reporting pods.

        The demand splits evenly across reporting pods that are neither
        terminating nor pinned (remainder to the lowest-indexed ones), clamped
        per pod at that pod's ``channels_total``. Pinned and terminating pods
        carry the in-use count the scenario gave them, on top of the demand.
        Invisible and starting pods carry nothing. Demand that does not fit is
        recorded as unplaceable -- that is the incident's exhaustion state, and
        it is what makes a channel-unit run and a stream-unit run diverge.
        """
        demand = self._demand_at(self._now)
        reporting = self._reporting_pods()
        free = [p for p in reporting if not p.terminating and p.pinned_in_use is None]

        for pod in reporting:
            if pod not in free:
                pod.in_use = 0 if pod.pinned_in_use is None else pod.pinned_in_use

        placed = 0
        if free:
            base, remainder = divmod(demand, len(free))
            for position, pod in enumerate(free):
                share = base + (1 if position < remainder else 0)
                # Free pods normally all report the same capacity, so an even
                # split can only clamp when the whole fleet is saturated. A
                # pod given a narrower capacity of its own (pin_pod_capacity)
                # clamps earlier, which is why such a pod should also be
                # pinned out of the split.
                pod.in_use = min(share, self._capacity_of(pod))
                placed += pod.in_use

        self._unplaceable = demand - placed
        self._served = [
            make_metrics(pod.index, pod.in_use, self._capacity_of(pod))
            for pod in reporting
        ]
        self._serving = {m.pod_name: m for m in self._served}

    def _demand_at(self, t: float) -> int:
        """
        The fleet-wide demand in force at a scenario time.

        :param t: Scenario time, in seconds since t0
        :return: Total concurrent stream checkouts demanded across the fleet
        """
        demand = 0
        for point in self._load:
            if point.at <= t:
                demand = point.streams
        return demand

    def _add_pod(self, created_at: float) -> _SimPod:
        """
        Add a pod to the back of the fleet.

        :param created_at: The absolute time the pod was created
        :return: The new pod
        """
        pod = _SimPod(index=self._next_index, created_at=created_at)
        self._next_index += 1
        self._fleet.append(pod)
        return pod

    def _pod(self, pod_index: int) -> _SimPod:
        """
        Look up a pod by its never-reused index.

        :param pod_index: The index (pod-N)
        :return: The pod
        """
        for pod in self._fleet:
            if pod.index == pod_index:
                return pod
        raise AssertionError(f"No pod-{pod_index} in the fleet")

    def _capacity_of(self, pod: _SimPod) -> int:
        """
        The ``channels_total`` a pod reports.

        :param pod: The pod
        :return: 0 while terminating, its pinned capacity if it has one, and
            the scenario's per-pod capacity otherwise
        """
        if pod.terminating:
            return 0
        if pod.pinned_capacity is not None:
            return pod.pinned_capacity
        return self._pod_capacity

    def _visible_pods(self) -> list[_SimPod]:
        """
        Pods old enough for K8s to have given them a podIP, in fleet order.

        :return: The pods ``list_pods`` would return right now
        """
        now = self._clock()
        return [
            pod for pod in self._fleet if now - pod.created_at >= self._visible_after
        ]

    def _reporting_pods(self) -> list[_SimPod]:
        """
        Pods whose metrics endpoint answers, in fleet order.

        :return: The pods that will produce metrics right now
        """
        now = self._clock()
        return [pod for pod in self._fleet if now - pod.created_at >= self._ready_after]

    async def _list_pods(self, **_kwargs) -> list[PodInfo]:
        """
        The fleet as K8s would report it: visible pods only, all Running.

        :return: PodInfo for every visible pod, in fleet order
        """
        reporting = self._reporting_pods()
        return [
            make_pod_info(pod.index, ready=pod in reporting)
            for pod in self._visible_pods()
        ]

    async def _get_deployment_replicas(self, **_kwargs) -> int:
        """
        :return: The deployment's current desired replica count
        """
        return self._replicas

    async def _get_pod_metrics(
        self, pod_name: str, pod_ip: str | None = None
    ) -> ChannelPoolMetrics | None:
        """
        Serve one pod's metrics, or nothing while it is still starting.

        :param pod_name: The pod whose metrics were requested
        :param pod_ip: Unused; the harness keys off the name
        :return: The metrics served this tick, or None
        """
        return self._serving.get(pod_name)

    async def _scale_deployment(
        self, *, namespace: str, deployment_name: str, replicas: int
    ) -> None:
        """
        Record the scale action and mutate the fleet to match.

        Scale-up stamps a new pod with the current time, so it goes through
        the full invisible -> starting -> reporting lifecycle. Scale-down
        removes the reporting pod with the most available capacity, mirroring
        the pod-deletion-cost annotation the real code writes; ties break on
        the lowest fleet index.

        :param namespace: Unused
        :param deployment_name: Unused
        :param replicas: The replica count the scaler asked for
        """
        current = self._replicas
        direction = "up" if replicas > current else "down"
        self._scale_events.append(ScaleEvent(self._now, direction, current, replicas))
        self._replicas = replicas

        if direction == "up":
            for _ in range(replicas - current):
                self._add_pod(self._clock())
        else:
            for _ in range(current - replicas):
                self._remove_most_available_pod()

    def _remove_most_available_pod(self) -> None:
        """Delete the reporting pod with the most available capacity."""
        reporting = self._reporting_pods()
        if not reporting:
            raise AssertionError("Scale-down with no reporting pod to remove")

        target = max(
            reporting,
            key=lambda pod: (self._capacity_of(pod) - pod.in_use, -pod.index),
        )
        self._fleet.remove(target)


@pytest.fixture
def fake_clock(mocker) -> FakeClock:
    """
    Install a callable fake clock over ``time.time`` for the whole test.

    Patched on the stdlib ``time`` module object that ``autoscaler.py``
    imports, so every read the code under test makes -- construction,
    iteration, guards, poll interval -- comes from this clock.
    """
    clock = FakeClock(SCENARIO_T0)
    mocker.patch.object(time, "time", clock)
    return clock


@pytest.fixture
def scenario_settings(mocker) -> None:
    """
    Pin the scaler settings a replay must run with.

    The target identifiers are test values; every behavioral knob is patched
    to the default DECLARED on ScalerSettings rather than to a literal. That
    keeps the fixture hermetic against a stray OMNI_EVENTS_* in the
    environment while still meaning "the values we actually ship" -- which is
    what an incident replay has to run with. Epic 6 (``t1.tj2.wi``) owns any
    retune of these, so a failure here after a retune is an intentional update
    to make, not a regression.

    This fixture neither modifies nor depends on ``test_autoscaler.py``'s
    ``mock_settings``, which deliberately pins the OLD defaults.
    """
    mocker.patch.object(scaler_settings, "scaler_target_deployment", "scenario-target")
    mocker.patch.object(scaler_settings, "scaler_target_namespace", "scenario-ns")
    mocker.patch.object(scaler_settings, "scaler_target_label_selector", "app=scenario")

    for field_name in _SHIPPED_DEFAULT_FIELDS:
        mocker.patch.object(scaler_settings, field_name, shipped_default(field_name))


@pytest_asyncio.fixture
async def scenario(mocker, fake_clock, scenario_settings):
    """
    Factory building an Autoscaler under the fake clock and driving a fleet.

    Construction happens inside the factory, after ``fake_clock`` is active,
    so the instance's ``_process_start_time`` is the scenario's t0. Every
    driver handed out is cleaned up on teardown.

    :return: A callable taking the ScenarioDriver keyword arguments minus
        ``autoscaler``, ``mocker`` and ``clock``, and returning the driver
    """
    drivers: list[ScenarioDriver] = []

    def _scenario(
        *,
        pod_capacity: int,
        load: list[LoadPoint],
        initial_replicas: int,
        visible_after: float = POD_VISIBLE_AFTER_SECONDS,
        ready_after: float = POD_READY_AFTER_SECONDS,
    ) -> ScenarioDriver:
        driver = ScenarioDriver(
            Autoscaler(),
            mocker,
            pod_capacity=pod_capacity,
            load=load,
            initial_replicas=initial_replicas,
            clock=fake_clock,
            visible_after=visible_after,
            ready_after=ready_after,
        )
        drivers.append(driver)
        return driver

    yield _scenario

    for driver in drivers:
        await driver.aclose()
