# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import datetime
import logging
import time
from unittest.mock import AsyncMock

import pytest
import pytest_asyncio

from notification_common.scaler import autoscaler as autoscaler_module
from notification_common.scaler.autoscaler import Autoscaler, run_autoscaler
from notification_common.scaler.k8s_client import PodInfo
from notification_common.scaler.metrics_client import ChannelPoolMetrics
from notification_common.scaler.scaler_settings import scaler_settings

# Default capacity for test pods
DEFAULT_POD_CAPACITY = 100

# A fleet wide enough to model any availability these tests need without a pod
# reporting more channels in use than it has.
FLEET_POD_COUNT = 5

# Replica count for tests that do not care about the exact number: it sits
# strictly between scaler_min_replicas and scaler_max_replicas as mock_settings
# patches them, so neither replica guard can decide their outcome.
FLEET_REPLICAS = 5

# Availability on the single pod that would be deleted first, as the direct
# _try_scale_down callers report it.
MAX_AVAILABLE_PER_POD = 80

# Frozen-clock spacing between polls in the iteration-driven window tests.
POLL_STEP_SECONDS = 5

# Trailing window for the iteration-driven tests, patched down from the
# mock_settings value purely so a test can poll across it in a few ticks. The
# landed window is time-pruned in SECONDS, not bounded by a sample count, so
# shortening it changes nothing but how much frozen time a test has to cover.
SHORT_WINDOW_SECONDS = 30

# A scale-down cooldown short enough to elapse well inside the stabilization
# window, so a test can leave the stabilization window as the only guard still
# holding a freshly constructed instance back.
SHORT_COOLDOWN_SECONDS = 10

# The replica floor the floor tests enforce, patched UP from the 1 that
# mock_settings pins so a fleet has somewhere to sit below it. Kept well under
# the patched max_replicas (10) so the ceiling is never the reason.
FLOOR_REPLICAS = 3

# A floor ABOVE the ceiling: the misconfiguration the effective-floor clamp and
# the startup warning both exist for. Upward enforcement stops at the ceiling.
INVERTED_FLOOR = 5
INVERTED_CEILING = 3


def make_pod_info(pod_num: int, ready: bool = True, phase: str = "Running") -> PodInfo:
    """Create a PodInfo for testing with consistent naming convention."""
    return PodInfo(f"pod-{pod_num}", f"10.0.0.{pod_num}", ready, phase)


def make_metrics(
    pod_num: int, channels_in_use: int, channels_total: int = DEFAULT_POD_CAPACITY
) -> ChannelPoolMetrics:
    """Create ChannelPoolMetrics for testing with consistent naming convention."""
    utilization = int((channels_in_use / channels_total) * 100) if channels_total else 0
    return ChannelPoolMetrics(
        f"pod-{pod_num}",
        f"10.0.0.{pod_num}",
        utilization,
        channels_in_use,
        channels_total,
    )


def scale_down_threshold(pod_capacity: int = DEFAULT_POD_CAPACITY) -> int:
    """The deadband a decision has to clear before a pod may be given back."""
    return scaler_settings.scaler_min_available_capacity + pod_capacity


def starved_available() -> int:
    """Availability low enough to demand a scale-up."""
    return scaler_settings.scaler_min_available_capacity - 1


def busy_available() -> int:
    """
    Availability while load is high.

    Above the scale-up trigger and far below the scale-down deadband, so
    neither branch of the decision acts on it.
    """
    return scaler_settings.scaler_min_available_capacity + 1


def idle_available(pod_capacity: int = DEFAULT_POD_CAPACITY) -> int:
    """Availability once the load has gone away: comfortably scale-down eligible."""
    return scale_down_threshold(pod_capacity) + pod_capacity


def make_fleet_metrics(
    total_available: int,
    pod_count: int = FLEET_POD_COUNT,
    capacity: int = DEFAULT_POD_CAPACITY,
) -> list[ChannelPoolMetrics]:
    """
    Build a fleet whose per-pod availabilities sum to ``total_available``.

    Every pod reports the same ``channels_total``, so ``pod_capacity`` -- and
    with it the scale-down deadband -- stays put across a whole scenario while
    only the availability moves.

    :param total_available: The total availability the fleet should report
    :param pod_count: How many pods report
    :param capacity: Channels each pod reports as its total
    :return: Metrics for the whole fleet
    """
    in_use = pod_count * capacity - total_available
    base, remainder = divmod(in_use, pod_count)
    return [
        make_metrics(index + 1, base + (1 if index < remainder else 0), capacity)
        for index in range(pod_count)
    ]


def feed_window(
    autoscaler: Autoscaler, samples: list[tuple[int, int]], now: float
) -> int | None:
    """
    Record samples through the real recording seam and return the window
    minimum reported for the last of them.

    ``_record_available_capacity`` takes the observation timestamp as an
    argument, so handing it historical timestamps drives the real pruning
    logic. The deque behind it is never touched directly, which is what keeps
    these tests off the implementation's internals.

    :param autoscaler: The instance whose window should be fed
    :param samples: ``(age_in_seconds, total_available)`` pairs, oldest first
    :param now: The timestamp the ages are measured back from
    :return: The trailing window minimum after the last sample, or None when
        the window is not yet backed by observation
    """
    windowed = None
    for age, total_available in samples:
        # noinspection PyProtectedMember
        windowed = autoscaler._record_available_capacity(now - age, total_available)
    return windowed


def capacity_reason(total_available: int | None = None) -> str:
    """
    The reason the capacity-pressure branch hands ``_try_scale_up``.

    ``_try_scale_up`` renders whatever reason it is given verbatim and checks
    none of it, so a direct caller has to supply one. This is the string the
    landed capacity branch supplies, kept in one place so the direct callers
    below stay honest about which condition they are standing in for.

    :param total_available: The availability the caller observed, defaulting to
        one low enough to have tripped the branch
    :return: The reason string
    """
    available = starved_available() if total_available is None else total_available
    return (
        f"available {available} < min {scaler_settings.scaler_min_available_capacity}"
    )


async def complete_a_scale_up(autoscaler: Autoscaler, up_time: float) -> None:
    """
    Drive a scale-up that actually executes, at ``up_time``.

    Only a completed scale-up arms the post-scale-up stabilization window, so
    every earlier guard is elapsed first. Callers must have mocked
    ``scale_deployment`` before calling this.

    :param autoscaler: The instance to scale up
    :param up_time: The timestamp of the scale-up
    """
    expire_cooldowns(autoscaler, up_time)
    # noinspection PyProtectedMember
    await autoscaler._try_scale_up(
        up_time,
        current_replicas=FLEET_REPLICAS,
        starting_pods=0,
        reason=capacity_reason(),
    )


async def make_a_decision(
    autoscaler: Autoscaler,
    now: float,
    *,
    current_replicas: int,
    total_available: int,
    starting_pods: int = 0,
) -> None:
    """
    Run the real decision on a fleet described by two numbers.

    The windowed reading is the instantaneous one, so smoothing is neither the
    reason a branch fired nor the reason one did not, and ``pod_capacity`` is
    the fleet-wide constant every other test in this file uses, which fixes the
    scale-down deadband at ``min_capacity + DEFAULT_POD_CAPACITY``.

    :param autoscaler: The instance to decide on
    :param now: The timestamp the guards are evaluated against
    :param current_replicas: The deployment's spec.replicas, as
        ``get_deployment_replicas`` would report it
    :param total_available: The availability the fleet reports this iteration
    :param starting_pods: Pods still starting up
    """
    # noinspection PyProtectedMember
    await autoscaler._make_scaling_decision(
        now=now,
        total_available=total_available,
        windowed_available=total_available,
        pod_capacity=DEFAULT_POD_CAPACITY,
        max_available=MAX_AVAILABLE_PER_POD,
        current_replicas=current_replicas,
        starting_pods=starting_pods,
    )


async def attempt_scale_down(
    autoscaler: Autoscaler,
    now: float,
    available: int | None = None,
    starting_pods: int = 0,
    current_replicas: int = FLEET_REPLICAS,
) -> None:
    """
    Attempt a scale-down whose only possible refusals are the guards inside
    ``_try_scale_down``.

    The windowed and instantaneous readings are the same scale-down-eligible
    value, and the replica count is above the minimum, so whether
    ``scale_deployment`` is called is decided purely by the guards.

    :param autoscaler: The instance to scale down
    :param now: The timestamp the guards are evaluated against
    :param available: The availability to report, defaulting to a comfortably
        scale-down-eligible one
    :param starting_pods: Pods still starting up
    :param current_replicas: The replica count to scale down from
    """
    available = idle_available() if available is None else available
    # noinspection PyProtectedMember
    await autoscaler._try_scale_down(
        now,
        current_replicas=current_replicas,
        windowed_available=available,
        total_available=available,
        pod_capacity=DEFAULT_POD_CAPACITY,
        max_available=MAX_AVAILABLE_PER_POD,
        starting_pods=starting_pods,
    )


def expire_cooldowns(autoscaler: Autoscaler, now: float | None = None) -> None:
    """
    Back-date every time-based scale-down guard so it reads as long elapsed.

    A freshly constructed Autoscaler arms its scale-down cooldown AND its
    post-scale-up stabilization anchor from process start, so tests that
    exercise a later guard have to move past both first, otherwise they pass
    (or fail) because of a guard they never meant to test. Every offset is
    derived from the currently configured durations rather than from literals
    so they survive changes to the defaults or to the values ``mock_settings``
    patches in.

    :param autoscaler: The instance whose guards should read as elapsed
    :param now: The timestamp the test will pass to the guard under test;
        defaults to the current time
    """
    reference = time.time() if now is None else now
    # noinspection PyProtectedMember
    autoscaler._last_scale_up_time = (
        reference - scaler_settings.scaler_scale_up_cooldown_seconds - 1
    )
    # noinspection PyProtectedMember
    autoscaler._last_scale_down_time = (
        reference - scaler_settings.scaler_scale_down_cooldown_seconds - 1
    )
    # noinspection PyProtectedMember
    autoscaler._scale_up_stabilization_anchor = (
        reference - scaler_settings.scaler_post_scale_up_stabilization_seconds - 1
    )


@pytest.fixture
def autoscaler():
    return Autoscaler()


@pytest_asyncio.fixture
async def cleanup_autoscaler(autoscaler):
    yield
    # noinspection PyProtectedMember
    await autoscaler._cleanup()


@pytest_asyncio.fixture
async def new_autoscaler():
    """
    Factory for Autoscalers constructed inside the test body.

    Tests that pin construction-time state need the instance built after their
    time control is active, which the module-level ``autoscaler`` fixture
    cannot guarantee. Every instance handed out is cleaned up on teardown, to
    match the ``cleanup_autoscaler`` convention.
    """
    created: list[Autoscaler] = []

    def _new_autoscaler() -> Autoscaler:
        instance = Autoscaler()
        created.append(instance)
        return instance

    yield _new_autoscaler

    for instance in created:
        # noinspection PyProtectedMember
        await instance._cleanup()


@pytest.fixture
def mock_settings(mocker):
    """Fixture to mock scaler settings."""
    mocker.patch.object(scaler_settings, "scaler_target_deployment", "test-deployment")
    mocker.patch.object(scaler_settings, "scaler_target_namespace", "test-namespace")
    mocker.patch.object(scaler_settings, "scaler_min_available_capacity", 30)
    mocker.patch.object(scaler_settings, "scaler_min_replicas", 1)
    mocker.patch.object(scaler_settings, "scaler_max_replicas", 10)
    mocker.patch.object(scaler_settings, "scaler_scale_up_cooldown_seconds", 10)
    mocker.patch.object(scaler_settings, "scaler_scale_down_cooldown_seconds", 300)
    mocker.patch.object(scaler_settings, "scaler_scale_down_window_seconds", 300)
    mocker.patch.object(
        scaler_settings, "scaler_post_scale_up_stabilization_seconds", 600
    )
    mocker.patch.object(scaler_settings, "scaler_target_label_selector", "app=test")


@pytest.fixture
def floor_settings(mocker, mock_settings) -> int:
    """
    Raise the replica floor above where the floor tests place the fleet.

    ``mock_settings`` pins ``scaler_min_replicas`` to 1, and no fleet in this
    file sits below 1, so the floor branch is unreachable under it alone.
    Patched after it, so this is the value in force.

    :return: The floor now configured
    """
    mocker.patch.object(scaler_settings, "scaler_min_replicas", FLOOR_REPLICAS)
    return scaler_settings.scaler_min_replicas


@pytest.fixture
def inverted_replica_bounds(mocker, mock_settings) -> tuple[int, int]:
    """
    Configure a replica floor above the replica ceiling.

    :return: The configured ``(min_replicas, max_replicas)`` pair
    """
    mocker.patch.object(scaler_settings, "scaler_min_replicas", INVERTED_FLOOR)
    mocker.patch.object(scaler_settings, "scaler_max_replicas", INVERTED_CEILING)
    return INVERTED_FLOOR, INVERTED_CEILING


@pytest.fixture
def short_scale_down_window(mocker, mock_settings) -> int:
    """
    Shrink the trailing window so a test can poll across it in a few ticks.

    :return: The window length now in effect, in seconds
    """
    mocker.patch.object(
        scaler_settings, "scaler_scale_down_window_seconds", SHORT_WINDOW_SECONDS
    )
    return scaler_settings.scaler_scale_down_window_seconds


class _PollDriver:
    """
    Drives real ``_scaling_iteration`` polls against a mocked cluster.

    The window is only exercised end to end through ``_scaling_iteration``:
    that is where the sample is recorded and where the resulting minimum is
    handed to the decision. Tests that want the window itself under test
    therefore drive whole polls rather than calling the decision directly.
    """

    def __init__(self, autoscaler, mocker, freezer, replicas):
        self._autoscaler = autoscaler
        self._freezer = freezer
        self.scale_deployment = AsyncMock()
        self._metrics = AsyncMock(return_value=[])
        self._list_pods = AsyncMock(return_value=[])
        mocker.patch.object(autoscaler, "_collect_metrics", self._metrics)
        mocker.patch.object(autoscaler, "_update_pod_deletion_costs", AsyncMock())
        mocker.patch.object(autoscaler._k8s_client, "list_pods", self._list_pods)
        mocker.patch.object(
            autoscaler._k8s_client,
            "get_deployment_replicas",
            AsyncMock(return_value=replicas),
        )
        mocker.patch.object(
            autoscaler._k8s_client, "scale_deployment", self.scale_deployment
        )

    def advance(self, seconds: float) -> None:
        """Move the frozen clock forward."""
        self._freezer.tick(datetime.timedelta(seconds=seconds))

    async def poll(
        self, total_available: int, pod_count: int = FLEET_POD_COUNT
    ) -> None:
        """
        Run one real polling iteration observing ``total_available``.

        :param total_available: The availability the fleet reports this poll
        :param pod_count: How many pods report this poll. Defaults to the
            file-wide fleet size; a scenario that wants the reporting fleet to
            disagree with the deployment's spec.replicas -- which is what tells
            a decision keyed on spec.replicas from one keyed on reporters --
            passes its own.
        """
        metrics = make_fleet_metrics(total_available, pod_count)
        self._metrics.return_value = metrics
        self._list_pods.return_value = [
            make_pod_info(index + 1) for index in range(len(metrics))
        ]
        # noinspection PyProtectedMember
        await self._autoscaler._scaling_iteration()

    async def poll_for(
        self, total_available: int, duration: float, clear_gates: bool = False
    ) -> None:
        """
        Poll every ``POLL_STEP_SECONDS`` of frozen time for ``duration``.

        :param total_available: The availability the fleet reports throughout
        :param duration: How much frozen time to cover
        :param clear_gates: Back-date the cooldown and stabilization guards
            before every poll, leaving the trailing window as the only thing
            that can still refuse a scale-down
        """
        for _ in range(int(duration // POLL_STEP_SECONDS)):
            self.advance(POLL_STEP_SECONDS)
            if clear_gates:
                expire_cooldowns(self._autoscaler)
            await self.poll(total_available)

    async def poll_no_metrics(self) -> None:
        """
        Run one real iteration that reaches the "no metrics" early return.

        One Running pod is listed so the first early return ("no running
        pods") is cleared, but ``_collect_metrics`` yields nothing, so the
        iteration hits the second early return and -- by contract -- records
        no window sample.
        """
        self._list_pods.return_value = [make_pod_info(1)]
        self._metrics.return_value = []
        # noinspection PyProtectedMember
        await self._autoscaler._scaling_iteration()

    async def poll_no_running_pods(self) -> None:
        """
        Run one real iteration that reaches the "no running pods" early return.

        With no pods listed at all the iteration returns before it ever
        collects metrics or computes capacity, and -- by contract -- records
        no window sample.
        """
        self._list_pods.return_value = []
        # noinspection PyProtectedMember
        await self._autoscaler._scaling_iteration()


@pytest.fixture
def poll_driver(mocker):
    """
    Factory binding an Autoscaler to a mocked cluster and a frozen clock.

    Taking the instance as an argument rather than producing one keeps
    construction inside the test body, where the clock is already frozen --
    ``_process_start_time`` is captured in ``__init__`` and several tests turn
    on what it was.
    """

    def _poll_driver(autoscaler, freezer, replicas: int = FLEET_REPLICAS):
        return _PollDriver(autoscaler, mocker, freezer, replicas)

    return _poll_driver


@pytest.fixture
def autoscaler_info_logs(real_get_logger, caplog):
    """
    Make the autoscaler module's INFO records visible to caplog.

    ``real_get_logger`` puts the stdlib ``logging.getLogger`` back, which caplog
    needs both to install its handler on the real root and to restore, on
    teardown, the level ``set_level`` raises here.
    """
    caplog.set_level(logging.INFO, logger=autoscaler_module.logger.name)
    return caplog


def logged_lines(logs) -> list[tuple[str, str]]:
    """
    The autoscaler module's own records, level and message.

    Other loggers can emit during a test, so the records are filtered by
    logger name; comparing the whole list is then a statement about everything
    the module said, not just about one line being present.

    :param logs: The caplog fixture (usually via ``autoscaler_info_logs``)
    :return: ``(levelname, message)`` pairs, oldest first
    """
    return [
        (record.levelname, record.message)
        for record in logs.records
        if record.name == autoscaler_module.logger.name
    ]


class TestCalculateCapacity:
    def test_calculates_total_available_and_pod_capacity(self):
        # 3 pods, each with a capacity of 100, using 50, 80, 30 respectively
        metrics = [make_metrics(1, 50), make_metrics(2, 80), make_metrics(3, 30)]

        total_available, pod_capacity, max_available = Autoscaler._calculate_capacity(
            metrics
        )

        # Available: (100-50) + (100-80) + (100-30) = 50 + 20 + 70 = 140
        assert total_available == 140
        assert pod_capacity == DEFAULT_POD_CAPACITY
        # Pod-3 has the most available (70)
        assert max_available == 70

    def test_returns_zero_for_empty_list(self):
        total_available, pod_capacity, max_available = Autoscaler._calculate_capacity(
            []
        )

        assert total_available == 0
        assert pod_capacity == 0
        assert max_available == 0

    def test_handles_single_pod(self):
        metrics = [make_metrics(1, 75)]

        total_available, pod_capacity, max_available = Autoscaler._calculate_capacity(
            metrics
        )

        assert total_available == 25  # 100 - 75
        assert pod_capacity == DEFAULT_POD_CAPACITY
        assert max_available == 25

    def test_handles_fully_utilized_pods(self):
        metrics = [make_metrics(1, 100), make_metrics(2, 100)]

        total_available, pod_capacity, max_available = Autoscaler._calculate_capacity(
            metrics
        )

        assert total_available == 0
        assert pod_capacity == DEFAULT_POD_CAPACITY
        assert max_available == 0


class TestCalculateCapacityHealthyReporter:
    """
    Pins the landed healthy-reporter rule (SR-5.1).

    The rule is ``pod_capacity = max(m.channels_total for m in metrics)`` --
    the largest capacity ANY reporter publishes, taken over ALL reporters with
    no zero-filter and an accumulator that starts at 0. A pod that is
    terminating, restarting, or has not built its channel pool yet publishes
    ``channels_total=0``; under the previous ``metrics[0]`` rule such a
    reporter collapsed the scale-down deadband
    (``total_available >= min_capacity + pod_capacity``) to no deadband at all
    whenever it happened to sort first.
    """

    @pytest.mark.parametrize(
        "position,metrics",
        [
            (
                "first",
                [make_metrics(1, 0, 0), make_metrics(2, 50), make_metrics(3, 30)],
            ),
            (
                "middle",
                [make_metrics(1, 50), make_metrics(2, 0, 0), make_metrics(3, 30)],
            ),
            (
                "last",
                [make_metrics(1, 50), make_metrics(2, 30), make_metrics(3, 0, 0)],
            ),
        ],
    )
    def test_zero_capacity_reporter_does_not_lower_pod_capacity(
        self, position, metrics
    ):
        _, pod_capacity, _ = Autoscaler._calculate_capacity(metrics)

        assert pod_capacity == DEFAULT_POD_CAPACITY

    def test_two_zero_capacity_reporters_do_not_lower_pod_capacity(self):
        metrics = [make_metrics(1, 0, 0), make_metrics(2, 50), make_metrics(3, 0, 0)]

        _, pod_capacity, _ = Autoscaler._calculate_capacity(metrics)

        assert pod_capacity == DEFAULT_POD_CAPACITY

    def test_all_zero_capacity_reporters_return_zero_capacity(self):
        # A whole-fleet rollout, or every pod answering without the
        # channels_total key, yields an all-zero list. The max-over-all rule
        # returns 0 here; a filter-then-max rule would raise ValueError out of
        # an empty sequence, which _main_loop's broad except would swallow into
        # a silently non-scaling scaler.
        metrics = [make_metrics(1, 0, 0), make_metrics(2, 0, 0)]

        assert Autoscaler._calculate_capacity(metrics) == (0, 0, 0)

    def test_sole_zero_capacity_reporter_returns_zero_capacity(self):
        # The last pod standing, terminating: no usable capacity anywhere.
        metrics = [make_metrics(1, 0, 0)]

        assert Autoscaler._calculate_capacity(metrics) == (0, 0, 0)

    @pytest.mark.parametrize(
        "order,metrics",
        [
            ("ascending", [make_metrics(1, 0, 80), make_metrics(2, 0, 720)]),
            ("descending", [make_metrics(1, 0, 720), make_metrics(2, 0, 80)]),
        ],
    )
    def test_disagreeing_reporters_yield_the_widest_capacity(self, order, metrics):
        # Mid-rollout the fleet reports a mix of old and new per-pod capacities.
        # The widest pod bounds what a single deletion can cost, so max keeps
        # the deadband at least as wide as the capacity actually lost -- and,
        # unlike a positional rule, gives the same answer whichever order the
        # Kubernetes API happens to list the pods in.
        _, pod_capacity, _ = Autoscaler._calculate_capacity(metrics)

        assert pod_capacity == 720

    def test_zero_capacity_reporter_still_subtracts_its_in_use_channels(self):
        # A reporter excluded from pod_capacity by the max is NOT excluded from
        # the total: its channels_total - channels_in_use is negative and is
        # summed in as-is. That errs toward a smaller total_available, i.e.
        # away from scaling down, so it is pinned as landed rather than clamped.
        metrics = [make_metrics(1, 10, 0), make_metrics(2, 50)]

        total_available, _, _ = Autoscaler._calculate_capacity(metrics)

        assert total_available == 40  # (0 - 10) + (100 - 50)

    def test_zero_capacity_reporter_never_wins_max_available(self):
        # max_available accumulates from 0, so a negative per-pod availability
        # can never become the reported maximum.
        metrics = [make_metrics(1, 10, 0), make_metrics(2, 50)]

        _, _, max_available = Autoscaler._calculate_capacity(metrics)

        assert max_available == 50


class TestCalculateCapacityZeroReporterLog:
    """
    The one-per-poll INFO signature for the deadband-protection event (b.3c9).

    A terminating or still-starting pod reports ``channels_total=0`` and is
    discarded from ``pod_capacity`` by the max rule. Without a log line an
    operator diagnosing "why didn't it scale down during that rollout" has no
    signal that a zero-capacity reporter was distorting capacity. The line is
    emitted once per poll with the reporter COUNT, not once per pod, so it stays
    a low-frequency INFO signature; its level is the contract, because
    production runs above DEBUG (see the Grafana production diagnostics
    runbook).
    """

    def _discarding_records(self, logs):
        return [
            (record.levelname, record.message)
            for record in logs.records
            if record.name == autoscaler_module.logger.name
            and record.message.startswith("Discarding")
        ]

    def test_emits_one_info_line_when_a_zero_capacity_reporter_is_discarded(
        self, autoscaler_info_logs
    ):
        # One zero reporter alongside a healthy 100-capacity pod: pod_capacity
        # resolves to 100 and the line names both the count and that resolved
        # capacity.
        autoscaler_info_logs.clear()

        Autoscaler._calculate_capacity([make_metrics(1, 0, 0), make_metrics(2, 50)])

        assert self._discarding_records(autoscaler_info_logs) == [
            (
                "INFO",
                "Discarding 1 zero-capacity reporter(s) (channels_total=0) "
                "from pod_capacity; pod_capacity=100",
            )
        ]

    def test_the_line_counts_every_zero_capacity_reporter(self, autoscaler_info_logs):
        # Two zero reporters and one healthy pod: the count is 2 and the
        # resolved pod_capacity is still the healthy pod's 100.
        autoscaler_info_logs.clear()

        Autoscaler._calculate_capacity(
            [make_metrics(1, 0, 0), make_metrics(2, 50), make_metrics(3, 0, 0)]
        )

        assert self._discarding_records(autoscaler_info_logs) == [
            (
                "INFO",
                "Discarding 2 zero-capacity reporter(s) (channels_total=0) "
                "from pod_capacity; pod_capacity=100",
            )
        ]

    def test_does_not_emit_when_no_reporter_has_zero_capacity(
        self, autoscaler_info_logs
    ):
        # Every pod reports real capacity, so the deadband is never distorted
        # and the signature must stay silent -- a per-poll line that fired on
        # healthy fleets would be noise.
        autoscaler_info_logs.clear()

        Autoscaler._calculate_capacity([make_metrics(1, 50), make_metrics(2, 30)])

        assert self._discarding_records(autoscaler_info_logs) == []


@pytest.mark.asyncio
class TestUpdatePodDeletionCosts:
    async def test_updates_deletion_cost_for_all_pods(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_patch = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "patch_pod_annotation", mock_patch)

        ready_pods = [make_pod_info(1), make_pod_info(2), make_pod_info(3)]
        # Pod 1: 50 in use (50 available), Pod 2: 80 in use (20 available),
        # Pod 3: 30 in use (70 available)
        metrics = [make_metrics(1, 50), make_metrics(2, 80), make_metrics(3, 30)]

        await autoscaler._update_pod_deletion_costs(ready_pods, metrics)

        assert mock_patch.call_count == 3
        # Deletion cost is negative available capacity.
        # Pod-3 has the most available (70), so the lowest cost (-70), deleted first.
        # Pod-2 has the least available (20), so highest cost (-20), protected.
        mock_patch.assert_any_call(
            namespace="test-namespace",
            pod_name="pod-1",
            annotation_key="controller.kubernetes.io/pod-deletion-cost",
            annotation_value="-50",  # 100 - 50 = 50 available, cost = -50
        )
        mock_patch.assert_any_call(
            namespace="test-namespace",
            pod_name="pod-2",
            annotation_key="controller.kubernetes.io/pod-deletion-cost",
            annotation_value="-20",  # 100 - 80 = 20 available, cost = -20
        )
        mock_patch.assert_any_call(
            namespace="test-namespace",
            pod_name="pod-3",
            annotation_key="controller.kubernetes.io/pod-deletion-cost",
            annotation_value="-70",  # 100 - 30 = 70 available, cost = -70
        )

    async def test_continues_on_patch_failure(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        from notification_common.scaler.k8s_client import K8sClientError

        # First call fails, second succeeds
        mock_patch = AsyncMock(side_effect=[K8sClientError("Failed"), None])
        mocker.patch.object(autoscaler._k8s_client, "patch_pod_annotation", mock_patch)

        ready_pods = [make_pod_info(1), make_pod_info(2)]
        metrics = [make_metrics(1, 50), make_metrics(2, 80)]

        # Should not raise, continues to next pod
        await autoscaler._update_pod_deletion_costs(ready_pods, metrics)

        # Both pods should have been attempted
        assert mock_patch.call_count == 2

    async def test_empty_metrics_and_pods_does_nothing(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_patch = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "patch_pod_annotation", mock_patch)

        await autoscaler._update_pod_deletion_costs([], [])

        mock_patch.assert_not_called()

    async def test_sets_high_deletion_cost_for_pods_without_metrics(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_patch = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "patch_pod_annotation", mock_patch)

        # 3 ready pods, but only 2 returned metrics (pod-3 didn't respond)
        ready_pods = [make_pod_info(1), make_pod_info(2), make_pod_info(3)]
        metrics = [make_metrics(1, 50), make_metrics(2, 80)]

        await autoscaler._update_pod_deletion_costs(ready_pods, metrics)

        # 3 calls: 2 for pods with metrics + 1 for the pod without metrics
        assert mock_patch.call_count == 3

        # Pod without metrics gets high deletion cost to protect it
        mock_patch.assert_any_call(
            namespace="test-namespace",
            pod_name="pod-3",
            annotation_key="controller.kubernetes.io/pod-deletion-cost",
            annotation_value="1000000",
        )
        # Pods with metrics get normal deletion costs
        mock_patch.assert_any_call(
            namespace="test-namespace",
            pod_name="pod-1",
            annotation_key="controller.kubernetes.io/pod-deletion-cost",
            annotation_value="-50",
        )
        mock_patch.assert_any_call(
            namespace="test-namespace",
            pod_name="pod-2",
            annotation_key="controller.kubernetes.io/pod-deletion-cost",
            annotation_value="-20",
        )

    async def test_all_pods_missing_metrics_get_high_deletion_cost(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_patch = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "patch_pod_annotation", mock_patch)

        # 2 ready pods, but neither returned metrics
        ready_pods = [make_pod_info(1), make_pod_info(2)]
        metrics = []  # No metrics returned

        await autoscaler._update_pod_deletion_costs(ready_pods, metrics)

        # Both pods should get high deletion cost
        assert mock_patch.call_count == 2
        mock_patch.assert_any_call(
            namespace="test-namespace",
            pod_name="pod-1",
            annotation_key="controller.kubernetes.io/pod-deletion-cost",
            annotation_value="1000000",
        )
        mock_patch.assert_any_call(
            namespace="test-namespace",
            pod_name="pod-2",
            annotation_key="controller.kubernetes.io/pod-deletion-cost",
            annotation_value="1000000",
        )


@pytest.mark.asyncio
class TestCollectMetrics:
    async def test_collects_metrics_from_all_pods(
        self, autoscaler, mocker, cleanup_autoscaler
    ):
        pods = [make_pod_info(1), make_pod_info(2)]
        mock_metrics = [make_metrics(1, 50), make_metrics(2, 60)]

        mocker.patch.object(
            autoscaler._metrics_client,
            "get_pod_metrics",
            side_effect=mock_metrics,
        )

        result = await autoscaler._collect_metrics(pods)

        assert len(result) == 2
        assert result[0].channels_in_use == 50
        assert result[1].channels_in_use == 60

    async def test_filters_out_failed_metrics(
        self, autoscaler, mocker, cleanup_autoscaler
    ):
        pods = [make_pod_info(1), make_pod_info(2)]
        mock_metrics = [make_metrics(1, 50), None]  # Failed to get metrics from pod-2

        mocker.patch.object(
            autoscaler._metrics_client,
            "get_pod_metrics",
            side_effect=mock_metrics,
        )

        result = await autoscaler._collect_metrics(pods)

        assert len(result) == 1
        assert result[0].pod_name == "pod-1"


@pytest.mark.asyncio
class TestTryScaleUp:
    async def test_scales_up_when_below_min_capacity(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)

        # The caller reached _try_scale_up, so the branch above it already
        # judged the capacity too low; nothing here re-checks it -- not even
        # the reason, which is rendered verbatim.
        await autoscaler._try_scale_up(
            time.time(),
            current_replicas=3,
            starting_pods=0,
            reason=capacity_reason(),
        )

        mock_scale.assert_called_once_with(
            namespace="test-namespace",
            deployment_name="test-deployment",
            replicas=4,
        )

    async def test_does_not_scale_during_cooldown(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)

        autoscaler._last_scale_up_time = time.time()

        await autoscaler._try_scale_up(
            time.time(),
            current_replicas=3,
            starting_pods=0,
            reason=capacity_reason(),
        )

        mock_scale.assert_not_called()

    async def test_does_not_scale_above_max_replicas(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)

        await autoscaler._try_scale_up(
            time.time(),
            current_replicas=10,
            starting_pods=0,
            reason=capacity_reason(),
        )

        mock_scale.assert_not_called()

    async def test_does_not_scale_when_pods_starting(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_scale = AsyncMock()
        mocker.patch.object(
            autoscaler._k8s_client,
            "scale_deployment",
            mock_scale,
        )

        await autoscaler._try_scale_up(
            time.time(),
            current_replicas=3,
            starting_pods=1,
            reason=capacity_reason(),
        )

        mock_scale.assert_not_called()


@pytest.mark.asyncio
class TestTryScaleUpSuppressionLogLevels:
    """
    The scale-up suppression lines are INFO, not DEBUG (b.3c9).

    Both "why did it not scale up" answers -- pods still starting, and the
    scale-up cooldown -- have to survive production log levels for an incident
    timeline to reconstruct the refusal, so each is re-leveled to INFO. The
    ``autoscaler_info_logs`` fixture caps the module logger at INFO, so a line
    still emitted at DEBUG would be filtered out and never captured: seeing the
    message at INFO here is exactly the proof it is no longer DEBUG.
    """

    async def test_skip_starting_line_is_info(
        self,
        autoscaler,
        mocker,
        mock_settings,
        cleanup_autoscaler,
        autoscaler_info_logs,
    ):
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", AsyncMock())
        autoscaler_info_logs.clear()

        await autoscaler._try_scale_up(
            time.time(),
            current_replicas=3,
            starting_pods=2,
            reason=capacity_reason(),
        )

        assert logged_lines(autoscaler_info_logs) == [
            ("INFO", "Skipping scale-up: 2 pod(s) still starting")
        ]

    async def test_cooldown_line_is_info(
        self,
        autoscaler,
        mocker,
        mock_settings,
        cleanup_autoscaler,
        autoscaler_info_logs,
    ):
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", AsyncMock())
        now = time.time()
        autoscaler._last_scale_up_time = now
        cooldown = scaler_settings.scaler_scale_up_cooldown_seconds
        autoscaler_info_logs.clear()

        await autoscaler._try_scale_up(
            now,
            current_replicas=3,
            starting_pods=0,
            reason=capacity_reason(),
        )

        assert logged_lines(autoscaler_info_logs) == [
            ("INFO", f"In scale-up cooldown (0s/{cooldown}s)")
        ]


@pytest.mark.asyncio
class TestTryScaleDown:
    async def test_scales_down_when_excess_capacity(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        expire_cooldowns(autoscaler)

        # min_available=30, pod_capacity=100
        # Scale down when: total_available >= 30 + 100 = 130
        await autoscaler._try_scale_down(
            time.time(),
            current_replicas=5,
            windowed_available=200,
            total_available=200,
            pod_capacity=100,
            max_available=80,
            starting_pods=0,
        )

        mock_scale.assert_called_once_with(
            namespace="test-namespace",
            deployment_name="test-deployment",
            replicas=4,
        )

    async def test_does_not_scale_during_cooldown(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)

        # Elapse the earlier guards, then re-arm only the cooldown, so this
        # pins the cooldown rather than the stabilization window a freshly
        # constructed instance also starts inside.
        expire_cooldowns(autoscaler)
        autoscaler._last_scale_down_time = time.time()

        await autoscaler._try_scale_down(
            time.time(),
            current_replicas=5,
            windowed_available=200,
            total_available=200,
            pod_capacity=100,
            max_available=80,
            starting_pods=0,
        )

        mock_scale.assert_not_called()

    async def test_does_not_scale_below_min_replicas(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        # Elapse the cooldown so this pins the min-replicas guard, not the
        # cooldown.
        expire_cooldowns(autoscaler)

        await autoscaler._try_scale_down(
            time.time(),
            current_replicas=1,
            windowed_available=200,
            total_available=200,
            pod_capacity=100,
            max_available=80,
            starting_pods=0,
        )

        mock_scale.assert_not_called()

    @pytest.mark.parametrize(
        "starting_pods,expected_scaled",
        [(0, True), (1, False), (3, False)],
    )
    async def test_starting_pods_block_scale_down(
        self,
        autoscaler,
        mocker,
        mock_settings,
        cleanup_autoscaler,
        starting_pods,
        expected_scaled,
    ):
        # SR-5.2. Every argument other than starting_pods is byte-identical
        # across the cases, and the cooldowns are explicitly elapsed, so the
        # starting-pods gate is the only thing that can differ.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        expire_cooldowns(autoscaler)

        await autoscaler._try_scale_down(
            time.time(),
            current_replicas=5,
            windowed_available=200,
            total_available=200,
            pod_capacity=100,
            max_available=80,
            starting_pods=starting_pods,
        )

        assert mock_scale.called is expected_scaled

    async def test_gated_call_does_not_consume_the_scale_down_cooldown(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        # The gate is checked before the cooldown, so a refused call must not
        # stamp _last_scale_down_time. Asserted through the mock rather than by
        # reading the timestamp back, so an implementation that stamps and
        # then returns is caught.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        now = time.time()
        expire_cooldowns(autoscaler, now)

        await autoscaler._try_scale_down(
            now,
            current_replicas=5,
            windowed_available=200,
            total_available=200,
            pod_capacity=100,
            max_available=80,
            starting_pods=1,
        )
        await autoscaler._try_scale_down(
            now,
            current_replicas=5,
            windowed_available=200,
            total_available=200,
            pod_capacity=100,
            max_available=80,
            starting_pods=0,
        )

        mock_scale.assert_called_once_with(
            namespace="test-namespace",
            deployment_name="test-deployment",
            replicas=4,
        )


@pytest.mark.asyncio
class TestTryScaleDownSuppressionLogLevels:
    """
    The scale-down suppression lines are INFO, not DEBUG (b.3c9).

    The three gates re-leveled here -- pods still starting, the scale-down
    cooldown, and already-at-min-replicas -- join the stabilization line
    (already INFO) so the refusal reason is reconstructable from production logs
    whichever gate wins. Each test isolates one gate by clearing the gates ahead
    of it, and the ``autoscaler_info_logs`` fixture caps the module logger at
    INFO, so a line still at DEBUG would be filtered out rather than captured.
    """

    async def test_skip_starting_line_is_info(
        self,
        autoscaler,
        mocker,
        mock_settings,
        cleanup_autoscaler,
        autoscaler_info_logs,
    ):
        # The starting-pods gate is checked first, so no other gate has to be
        # elapsed for it to be the reason.
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", AsyncMock())
        autoscaler_info_logs.clear()

        await autoscaler._try_scale_down(
            time.time(),
            current_replicas=5,
            windowed_available=200,
            total_available=200,
            pod_capacity=100,
            max_available=80,
            starting_pods=3,
        )

        assert logged_lines(autoscaler_info_logs) == [
            ("INFO", "Skipping scale-down: 3 pod(s) still starting")
        ]

    async def test_cooldown_line_is_info(
        self,
        autoscaler,
        mocker,
        mock_settings,
        cleanup_autoscaler,
        autoscaler_info_logs,
    ):
        # Elapse the earlier guards, then re-arm only the cooldown so it is the
        # gate that fires (mirrors test_does_not_scale_during_cooldown).
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", AsyncMock())
        now = time.time()
        expire_cooldowns(autoscaler, now)
        autoscaler._last_scale_down_time = now
        cooldown = scaler_settings.scaler_scale_down_cooldown_seconds
        autoscaler_info_logs.clear()

        await autoscaler._try_scale_down(
            now,
            current_replicas=5,
            windowed_available=200,
            total_available=200,
            pod_capacity=100,
            max_available=80,
            starting_pods=0,
        )

        assert logged_lines(autoscaler_info_logs) == [
            ("INFO", f"In scale-down cooldown (0s/{cooldown}s)")
        ]

    async def test_already_at_min_line_is_info(
        self,
        autoscaler,
        mocker,
        mock_settings,
        cleanup_autoscaler,
        autoscaler_info_logs,
    ):
        # Elapse the cooldown and stabilization so the min-replicas gate is the
        # one that fires.
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", AsyncMock())
        expire_cooldowns(autoscaler)
        min_replicas = scaler_settings.scaler_min_replicas
        autoscaler_info_logs.clear()

        await autoscaler._try_scale_down(
            time.time(),
            current_replicas=min_replicas,
            windowed_available=200,
            total_available=200,
            pod_capacity=100,
            max_available=80,
            starting_pods=0,
        )

        assert logged_lines(autoscaler_info_logs) == [
            ("INFO", f"Already at min replicas ({min_replicas})")
        ]


@pytest.mark.asyncio
class TestMakeScalingDecision:
    async def test_scales_up_when_available_below_minimum(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_scale_up = AsyncMock()
        mocker.patch.object(autoscaler, "_try_scale_up", mock_scale_up)

        # total_available=29 < min_available_capacity=30
        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=29,
            windowed_available=29,
            pod_capacity=100,
            max_available=29,
            current_replicas=3,
            starting_pods=0,
        )

        mock_scale_up.assert_called_once()

    async def test_does_not_scale_up_when_above_minimum(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_scale_up = AsyncMock()
        mocker.patch.object(autoscaler, "_try_scale_up", mock_scale_up)

        # total_available=60 >= min_available_capacity=30
        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=60,
            windowed_available=60,
            pod_capacity=100,
            max_available=30,
            current_replicas=3,
            starting_pods=0,
        )

        mock_scale_up.assert_not_called()

    async def test_scales_down_when_excess_capacity(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_scale_down = AsyncMock()
        mocker.patch.object(autoscaler, "_try_scale_down", mock_scale_down)

        # total_available=200 >= min(30) + pod_capacity(100) = 130
        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=200,
            windowed_available=200,
            pod_capacity=100,
            max_available=80,
            current_replicas=5,
            starting_pods=0,
        )

        mock_scale_down.assert_called_once()

    async def test_does_not_scale_down_when_would_go_below_minimum(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_scale_down = AsyncMock()
        mocker.patch.object(autoscaler, "_try_scale_down", mock_scale_down)

        # total_available=100 < min(30) + pod_capacity(100) = 130
        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=100,
            windowed_available=100,
            pod_capacity=100,
            max_available=50,
            current_replicas=3,
            starting_pods=0,
        )

        mock_scale_down.assert_not_called()

    async def test_no_action_in_safe_zone(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_scale_up = AsyncMock()
        mock_scale_down = AsyncMock()
        mocker.patch.object(autoscaler, "_try_scale_up", mock_scale_up)
        mocker.patch.object(autoscaler, "_try_scale_down", mock_scale_down)

        # total_available=80, min=30, min+pod_capacity=130
        # 80 >= 30 (no scale up) but 80 < 130 (no scale down)
        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=80,
            windowed_available=80,
            pod_capacity=100,
            max_available=40,
            current_replicas=3,
            starting_pods=0,
        )

        mock_scale_up.assert_not_called()
        mock_scale_down.assert_not_called()

    async def test_forwards_starting_pods_to_try_scale_down(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        # The starting-pods gate lives inside _try_scale_down, so a decision
        # path that drops the argument disables the gate silently.
        mock_scale_down = AsyncMock()
        mocker.patch.object(autoscaler, "_try_scale_down", mock_scale_down)

        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=200,
            windowed_available=200,
            pod_capacity=100,
            max_available=80,
            current_replicas=5,
            starting_pods=2,
        )

        # The landed call forwards positionally:
        # (now, current_replicas, windowed_available, total_available,
        #  pod_capacity, max_available, starting_pods)
        assert mock_scale_down.call_args.args[6] == 2

    async def test_zero_capacity_at_index_zero_no_longer_reaches_scale_down(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        # Task success criterion 1, in direct-argument form. A terminating pod
        # sorts first and reports channels_total=0. Under the old metrics[0]
        # rule pod_capacity would be 0, the deadband would collapse to
        # "total_available >= 30", and this fleet (100 available) would have
        # scaled down. The max rule keeps pod_capacity at 100, so the deadband
        # is 130 and the fleet is left alone.
        mock_scale_down = AsyncMock()
        mocker.patch.object(autoscaler, "_try_scale_down", mock_scale_down)
        metrics = [make_metrics(1, 0, 0), make_metrics(2, 50), make_metrics(3, 50)]
        total_available, pod_capacity, max_available = Autoscaler._calculate_capacity(
            metrics
        )

        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=total_available,
            windowed_available=total_available,
            pod_capacity=pod_capacity,
            max_available=max_available,
            current_replicas=3,
            starting_pods=0,
        )

        mock_scale_down.assert_not_called()

    async def test_narrowest_reporter_at_index_zero_no_longer_reaches_scale_down(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        # The same success criterion where the suppression guard cannot mask
        # it: mid-rollout the narrow old-capacity pod sorts first, so the old
        # metrics[0] rule would have priced a deletion at 80 and scaled down at
        # a deadband of 110. The max rule prices it at the widest pod's 720,
        # and 280 available is nowhere near the resulting deadband of 750.
        mock_scale_down = AsyncMock()
        mocker.patch.object(autoscaler, "_try_scale_down", mock_scale_down)
        metrics = [
            make_metrics(1, 0, 80),
            make_metrics(2, 620, 720),
            make_metrics(3, 620, 720),
        ]
        total_available, pod_capacity, max_available = Autoscaler._calculate_capacity(
            metrics
        )

        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=total_available,
            windowed_available=total_available,
            pod_capacity=pod_capacity,
            max_available=max_available,
            current_replicas=3,
            starting_pods=0,
        )

        mock_scale_down.assert_not_called()

    async def test_sole_zero_capacity_reporter_does_not_reach_scale_down(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        # With every reporter at zero capacity the deadband term vanishes, so
        # what actually keeps this safe is that total_available collapses with
        # it and the scale-UP branch claims the decision first.
        mock_scale_up = AsyncMock()
        mock_scale_down = AsyncMock()
        mocker.patch.object(autoscaler, "_try_scale_up", mock_scale_up)
        mocker.patch.object(autoscaler, "_try_scale_down", mock_scale_down)
        total_available, pod_capacity, max_available = Autoscaler._calculate_capacity(
            [make_metrics(1, 0, 0)]
        )

        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=total_available,
            windowed_available=total_available,
            pod_capacity=pod_capacity,
            max_available=max_available,
            current_replicas=3,
            starting_pods=0,
        )

        mock_scale_down.assert_not_called()

    async def test_unknown_pod_capacity_suppresses_scale_down(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        # Defense in depth: _calculate_capacity cannot produce this pair (an
        # all-zero fleet also has total_available <= 0 and takes the scale-up
        # branch), so only a direct call reaches the guard.
        mock_scale_down = AsyncMock()
        mocker.patch.object(autoscaler, "_try_scale_down", mock_scale_down)

        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=200,
            windowed_available=200,
            pod_capacity=0,
            max_available=80,
            current_replicas=5,
            starting_pods=0,
        )

        mock_scale_down.assert_not_called()

    async def test_unknown_pod_capacity_warns(
        self,
        autoscaler,
        mocker,
        mock_settings,
        cleanup_autoscaler,
        real_get_logger,
        caplog,
    ):
        # The operator's only visibility into a suppressed scale-down. The
        # record is a WARNING, so the default capture level already sees it
        # once real_get_logger has put caplog's handler on the real root.
        mocker.patch.object(autoscaler, "_try_scale_down", AsyncMock())

        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=200,
            windowed_available=200,
            pod_capacity=0,
            max_available=80,
            current_replicas=5,
            starting_pods=0,
        )

        assert (
            "Suppressing scale-down: no reporter published a usable pod" in caplog.text
        )


@pytest.mark.asyncio
class TestMinReplicasFloor:
    """
    Pins the replica floor as a floor in BOTH directions (b.qwa).

    ``scaler_min_replicas`` used to be enforced only as a clamp inside
    ``_try_scale_down``, so nothing ever RAISED a fleet that was already below
    it. A deployment whose chart omits ``spec.replicas`` -- so that a helm
    upgrade cannot fight the autoscaler -- starts at the Kubernetes default of
    1, and a fleet that never sees load has no capacity pressure to climb from:
    in dev the fleet climbed 1 -> 2 under the capacity trigger and then rested
    at 2, one pod below a configured ``minReplicas: 3``, because 2 pods'
    availability cleared the scale-up threshold while still sitting inside the
    scale-down deadband.

    The landed rule is a second scale-up condition, ``current_replicas <
    min(min_replicas, max_replicas)``, tested after capacity pressure and
    taking the same action through the same ``_try_scale_up`` -- so every
    existing scale-up gate applies to it unchanged.

    These tests drive the real ``_make_scaling_decision`` and assert on the
    ``scale_deployment`` call the fleet would actually see, rather than on
    which private helper was reached. ``floor_settings`` patches the floor up
    from the 1 ``mock_settings`` pins, which is what makes the branch
    reachable at all.

    Not duplicated here: that an early-returning iteration records no window
    sample, which ``TestScalingIterationEarlyReturnRecordsNoSample`` already
    pins independently of the floor.
    """

    async def test_a_below_floor_fleet_is_raised_by_one(
        self, autoscaler, mocker, floor_settings, cleanup_autoscaler
    ):
        # The dev observation, in direct-argument form: an idle fleet one pod
        # below the floor, with availability so far above the reserve that the
        # capacity trigger will never fire for it. It is even scale-DOWN
        # eligible on this reading (230 >= the 130 deadband), so the floor
        # branch has to claim the iteration ahead of the scale-down chain.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)

        await make_a_decision(
            autoscaler,
            time.time(),
            current_replicas=FLOOR_REPLICAS - 1,
            total_available=idle_available(),
        )

        mock_scale.assert_called_once_with(
            namespace="test-namespace",
            deployment_name="test-deployment",
            replicas=FLOOR_REPLICAS,
        )

    async def test_a_fleet_at_the_floor_is_not_scaled_in_either_direction(
        self, autoscaler, mocker, floor_settings, cleanup_autoscaler
    ):
        # The resting point the fix has to converge to. Every time-based
        # scale-down guard is elapsed first, so the min-replicas clamp is the
        # only thing left refusing the (otherwise eligible) scale-down, and the
        # floor branch is the only thing that could wrongly add a pod.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        now = time.time()
        expire_cooldowns(autoscaler, now)

        await make_a_decision(
            autoscaler,
            now,
            current_replicas=FLOOR_REPLICAS,
            total_available=idle_available(),
        )

        mock_scale.assert_not_called()

    async def test_a_fleet_above_the_floor_with_no_pressure_is_left_alone(
        self, autoscaler, mocker, floor_settings, cleanup_autoscaler
    ):
        # The branch is inert everywhere it has no business acting: above the
        # floor, above the scale-up threshold, below the scale-down deadband.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)

        await make_a_decision(
            autoscaler,
            time.time(),
            current_replicas=FLOOR_REPLICAS + 1,
            total_available=busy_available(),
        )

        mock_scale.assert_not_called()

    async def test_a_starting_pod_holds_the_floor_scale_up_off(
        self, autoscaler, mocker, floor_settings, cleanup_autoscaler
    ):
        # The floor goes through _try_scale_up, so it inherits that gate: a
        # floor that scaled past a booting pod would add the fleet's whole
        # deficit in one poll interval.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)

        await make_a_decision(
            autoscaler,
            time.time(),
            current_replicas=FLOOR_REPLICAS - 1,
            total_available=busy_available(),
            starting_pods=1,
        )

        mock_scale.assert_not_called()

    async def test_the_starting_pod_refusal_is_logged_as_a_skipped_scale_up(
        self,
        autoscaler,
        mocker,
        floor_settings,
        cleanup_autoscaler,
        autoscaler_info_logs,
    ):
        # The refusal is the same one the capacity path emits, so "why is the
        # fleet still below minReplicas" is answerable from the existing line.
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", AsyncMock())
        autoscaler_info_logs.clear()

        await make_a_decision(
            autoscaler,
            time.time(),
            current_replicas=FLOOR_REPLICAS - 1,
            total_available=busy_available(),
            starting_pods=1,
        )

        assert logged_lines(autoscaler_info_logs) == [
            ("INFO", "Skipping scale-up: 1 pod(s) still starting")
        ]

    async def test_the_scale_up_cooldown_holds_the_floor_scale_up_off(
        self, autoscaler, mocker, floor_settings, cleanup_autoscaler
    ):
        # The other inherited gate. A fleet two pods below the floor climbs one
        # cooldown at a time, exactly as a fleet under capacity pressure does.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        now = time.time()
        autoscaler._last_scale_up_time = now

        await make_a_decision(
            autoscaler,
            now,
            current_replicas=FLOOR_REPLICAS - 1,
            total_available=busy_available(),
        )

        mock_scale.assert_not_called()

    async def test_a_floor_above_the_ceiling_is_enforced_at_the_ceiling(
        self, autoscaler, mocker, inverted_replica_bounds, cleanup_autoscaler
    ):
        # min 5 > max 3: the enforced floor is the ceiling, so a fleet below it
        # still gets raised -- the misconfiguration costs no capacity.
        floor, ceiling = inverted_replica_bounds
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)

        await make_a_decision(
            autoscaler,
            time.time(),
            current_replicas=ceiling - 1,
            total_available=busy_available(),
        )

        mock_scale.assert_called_once_with(
            namespace="test-namespace",
            deployment_name="test-deployment",
            replicas=ceiling,
        )

    async def test_a_fleet_at_the_ceiling_below_the_floor_does_not_scale(
        self, autoscaler, mocker, inverted_replica_bounds, cleanup_autoscaler
    ):
        # Where such a fleet comes to rest: at the ceiling, still nominally
        # below the configured floor, asking for nothing further.
        floor, ceiling = inverted_replica_bounds
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)

        await make_a_decision(
            autoscaler,
            time.time(),
            current_replicas=ceiling,
            total_available=busy_available(),
        )

        mock_scale.assert_not_called()

    async def test_a_fleet_at_the_ceiling_below_the_floor_does_not_warn_each_poll(
        self,
        autoscaler,
        mocker,
        inverted_replica_bounds,
        cleanup_autoscaler,
        real_get_logger,
        caplog,
    ):
        # Why the floor is clamped to the ceiling rather than left to
        # _try_scale_up's max check: an unclamped floor would reach that check
        # on EVERY poll for the life of the process and log its WARNING every
        # time. The startup banner names the misconfiguration once instead.
        floor, ceiling = inverted_replica_bounds
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", AsyncMock())
        caplog.clear()

        await make_a_decision(
            autoscaler,
            time.time(),
            current_replicas=ceiling,
            total_available=busy_available(),
        )

        # A negative assertion also passes when nothing is captured, so it
        # depends on real_get_logger for its meaning.
        assert "Already at max replicas" not in caplog.text

    async def test_the_floor_scale_up_line_names_the_floor(
        self,
        autoscaler,
        mocker,
        floor_settings,
        cleanup_autoscaler,
        autoscaler_info_logs,
    ):
        # An operator reading Loki has to be able to tell a floor scale-up from
        # a capacity one: the line must not claim a capacity shortfall that did
        # not happen -- this fleet has 230 available against a minimum of 30.
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", AsyncMock())
        available = idle_available()
        autoscaler_info_logs.clear()

        await make_a_decision(
            autoscaler,
            time.time(),
            current_replicas=FLOOR_REPLICAS - 1,
            total_available=available,
        )

        assert logged_lines(autoscaler_info_logs) == [
            (
                "INFO",
                f"SCALE UP: {FLOOR_REPLICAS - 1} -> {FLOOR_REPLICAS} "
                f"(below min replicas {FLOOR_REPLICAS} (available {available}))",
            )
        ]

    async def test_the_capacity_pressure_line_is_byte_identical_to_before(
        self,
        autoscaler,
        mocker,
        floor_settings,
        cleanup_autoscaler,
        autoscaler_info_logs,
    ):
        # Moving the reason into a parameter must not have changed the line the
        # runbook and the existing Loki queries already match on. The expected
        # string is spelled out in the pre-change f-string's own form rather
        # than built from capacity_reason(), so a drift in the rendered text
        # fails here instead of moving silently with the helper.
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", AsyncMock())
        available = starved_available()
        min_capacity = scaler_settings.scaler_min_available_capacity
        autoscaler_info_logs.clear()

        await make_a_decision(
            autoscaler,
            time.time(),
            current_replicas=FLOOR_REPLICAS + 1,
            total_available=available,
        )

        assert logged_lines(autoscaler_info_logs) == [
            (
                "INFO",
                f"SCALE UP: {FLOOR_REPLICAS + 1} -> {FLOOR_REPLICAS + 2} "
                f"(available {available} < min {min_capacity})",
            )
        ]

    async def test_capacity_pressure_is_the_reason_when_both_conditions_hold(
        self,
        autoscaler,
        mocker,
        floor_settings,
        cleanup_autoscaler,
        autoscaler_info_logs,
    ):
        # Both branches would add exactly one pod, so the only thing their
        # order decides is which reason the operator gets -- and a fleet that
        # is both starved and below the floor is starved first.
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", AsyncMock())
        available = starved_available()
        min_capacity = scaler_settings.scaler_min_available_capacity
        autoscaler_info_logs.clear()

        await make_a_decision(
            autoscaler,
            time.time(),
            current_replicas=FLOOR_REPLICAS - 1,
            total_available=available,
        )

        assert logged_lines(autoscaler_info_logs) == [
            (
                "INFO",
                f"SCALE UP: {FLOOR_REPLICAS - 1} -> {FLOOR_REPLICAS} "
                f"(available {available} < min {min_capacity})",
            )
        ]

    async def test_the_floor_outranks_the_unknown_pod_capacity_suppression(
        self,
        autoscaler,
        mocker,
        floor_settings,
        cleanup_autoscaler,
        autoscaler_info_logs,
    ):
        # The other branch the floor was inserted ahead of. A pod_capacity of 0
        # means no reporter published a usable capacity, which normally answers
        # the iteration with a scale-down suppression WARNING -- but a fleet
        # below the floor is one the floor branch claims first, and it needs no
        # capacity reading of its own to do so. Reached only by a direct call:
        # _calculate_capacity cannot produce a (pod_capacity=0, total_available
        # >= min_capacity) pair. Comparing the whole line list is what pins the
        # WARNING's absence, not just the SCALE UP's presence.
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", AsyncMock())
        available = busy_available()
        autoscaler_info_logs.clear()

        # noinspection PyProtectedMember
        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=available,
            windowed_available=available,
            pod_capacity=0,
            max_available=MAX_AVAILABLE_PER_POD,
            current_replicas=FLOOR_REPLICAS - 1,
            starting_pods=0,
        )

        assert logged_lines(autoscaler_info_logs) == [
            (
                "INFO",
                f"SCALE UP: {FLOOR_REPLICAS - 1} -> {FLOOR_REPLICAS} "
                f"(below min replicas {FLOOR_REPLICAS} (available {available}))",
            )
        ]

    async def test_a_floor_scale_up_consumes_the_scale_up_cooldown(
        self, autoscaler, mocker, floor_settings, cleanup_autoscaler
    ):
        # A floor scale-up is a scale-up: it stamps the cooldown like any
        # other. The new pod cannot have booted a second later, so the fleet is
        # still below the floor on the next iteration and only the stamp can
        # stop a second pod being asked for.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        now = time.time()

        await make_a_decision(
            autoscaler,
            now,
            current_replicas=FLOOR_REPLICAS - 1,
            total_available=busy_available(),
        )
        await make_a_decision(
            autoscaler,
            now + 1,
            current_replicas=FLOOR_REPLICAS - 1,
            total_available=busy_available(),
        )

        mock_scale.assert_called_once_with(
            namespace="test-namespace",
            deployment_name="test-deployment",
            replicas=FLOOR_REPLICAS,
        )

    async def test_a_floor_scale_up_arms_the_stabilization_window(
        self, autoscaler, mocker, floor_settings, cleanup_autoscaler
    ):
        # The other stamp. Every scale-down guard is elapsed before the
        # scale-up, and the scale-down attempt is on a fleet well above the
        # floor, so the freshly armed anchor is the only thing that can refuse
        # it -- a floor scale-up must not be a pod the fleet can hand straight
        # back.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        now = time.time()
        expire_cooldowns(autoscaler, now)

        await make_a_decision(
            autoscaler,
            now,
            current_replicas=FLOOR_REPLICAS - 1,
            total_available=busy_available(),
        )
        mock_scale.reset_mock()
        await attempt_scale_down(autoscaler, now)

        mock_scale.assert_not_called()

    async def test_a_reporting_fleet_short_of_spec_replicas_does_not_trip_the_floor(
        self, new_autoscaler, floor_settings, freezer, poll_driver
    ):
        # The floor keys on spec.replicas, not on how many pods answered. Here
        # the deployment is AT the floor while only two pods report -- a pod
        # mid-restart, or one whose metrics endpoint is down -- and a floor
        # keyed on reporters would answer that by adding a fourth pod the
        # deployment does not need. Driven through the real iteration so the
        # replica count comes from get_deployment_replicas, as in production.
        autoscaler = new_autoscaler()
        driver = poll_driver(autoscaler, freezer, replicas=FLOOR_REPLICAS)

        await driver.poll(busy_available(), pod_count=FLOOR_REPLICAS - 1)

        driver.scale_deployment.assert_not_called()

    async def test_a_freshly_constructed_instance_enforces_the_floor_at_once(
        self, new_autoscaler, floor_settings, freezer, poll_driver
    ):
        # The floor is evaluated statelessly from the live settings and the
        # current replica count, so a scaler that has just restarted enforces
        # it on its first observed iteration with no state to rebuild -- the
        # same fail-safe amnesia the scale-up cooldown already has.
        #
        # This is also the non-vacuity twin of the test above: same two
        # reporting pods, same availability, only spec.replicas differs.
        autoscaler = new_autoscaler()
        driver = poll_driver(autoscaler, freezer, replicas=FLOOR_REPLICAS - 1)

        await driver.poll(busy_available(), pod_count=FLOOR_REPLICAS - 1)

        driver.scale_deployment.assert_called_once_with(
            namespace="test-namespace",
            deployment_name="test-deployment",
            replicas=FLOOR_REPLICAS,
        )

    async def test_a_no_metrics_iteration_performs_no_floor_scale_up(
        self, new_autoscaler, floor_settings, freezer, poll_driver
    ):
        # Enforcement is limited to iterations that reach a decision. A fleet
        # whose pods have all stopped answering is below the floor as far as
        # the metrics go, but the absence of an observation is not an
        # observation, and the test above shows this same instance and spec
        # DOES scale once an iteration reaches the decision.
        autoscaler = new_autoscaler()
        driver = poll_driver(autoscaler, freezer, replicas=FLOOR_REPLICAS - 1)

        await driver.poll_no_metrics()

        driver.scale_deployment.assert_not_called()

    async def test_a_no_running_pods_iteration_performs_no_floor_scale_up(
        self, new_autoscaler, floor_settings, freezer, poll_driver
    ):
        # The first early return, same contract: nothing was observed, so
        # nothing is decided -- not even the floor, which needs no metrics of
        # its own and would otherwise be tempting to enforce here.
        autoscaler = new_autoscaler()
        driver = poll_driver(autoscaler, freezer, replicas=FLOOR_REPLICAS - 1)

        await driver.poll_no_running_pods()

        driver.scale_deployment.assert_not_called()


@pytest.mark.asyncio
class TestScalingIteration:
    async def test_counts_only_non_reporting_pods_as_starting(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        # The starting-pods gate is only as good as this derivation: a pod that
        # DID report is settled even when it reports zero capacity, while a
        # Pending pod and a Running pod that stayed silent are both starting.
        pending_pod = make_pod_info(1, ready=False, phase="Pending")
        silent_pod = make_pod_info(2, ready=False)
        zero_capacity_pod = make_pod_info(3)
        healthy_pod = make_pod_info(4)
        mocker.patch.object(
            autoscaler._k8s_client,
            "list_pods",
            AsyncMock(
                return_value=[
                    pending_pod,
                    silent_pod,
                    zero_capacity_pod,
                    healthy_pod,
                ]
            ),
        )
        mocker.patch.object(
            autoscaler,
            "_collect_metrics",
            AsyncMock(return_value=[make_metrics(3, 0, 0), make_metrics(4, 50)]),
        )
        mocker.patch.object(autoscaler, "_update_pod_deletion_costs", AsyncMock())
        mocker.patch.object(
            autoscaler._k8s_client, "get_deployment_replicas", AsyncMock(return_value=4)
        )
        mock_decision = AsyncMock()
        mocker.patch.object(autoscaler, "_make_scaling_decision", mock_decision)

        await autoscaler._scaling_iteration()

        # Forwarded positionally as (now, total_available, windowed_available,
        # pod_capacity, max_available, current_replicas, starting_pods).
        assert mock_decision.call_args.args[6] == 2


@pytest.mark.asyncio
class TestScalingIterationExcludesTerminatingPods:
    """
    Pins that a terminating pod (metadata.deletionTimestamp set,
    ``PodInfo.terminating=True``) is dropped from the fleet view up front in
    ``_scaling_iteration``.

    A pod on SIGTERM stays phase=Running with a podIP and keeps answering
    /metrics/channel-pool with its full ``channels_total`` for the whole
    preStop + drain window while refusing every new stream, so its reported
    slots are phantom capacity. If they were summed into ``total_available``, a
    rolling-update / scale-down-under-load rollout would read plenty of free
    capacity and refuse to scale up while real clients are being turned away.

    Each test wires a fleet of one healthy Running pod near capacity (10 free)
    and one terminating Running pod reporting ~720 phantom-free. ``_collect_metrics``
    is stubbed with a side effect keyed on the pods it is actually handed, so a
    dropped filter is observable: the terminating pod would be scraped and its
    720 slots would surface. This is deliberately orthogonal to readiness --
    ``TestScalingIterationCountsUnreadyRunningPod`` pins that an unready (but not
    terminating) Running pod is still counted.
    """

    @staticmethod
    def _healthy_pod() -> PodInfo:
        # Running, ready, NOT terminating. 90/100 in use -> 10 available.
        return PodInfo("pod-1", "10.0.0.1", ready=True, phase="Running")

    @staticmethod
    def _terminating_pod() -> PodInfo:
        # Running with a podIP, but deletionTimestamp set -> terminating.
        return PodInfo(
            "pod-2", "10.0.0.2", ready=True, phase="Running", terminating=True
        )

    @staticmethod
    def _metrics_by_name() -> dict:
        # pod-2 reports 720 phantom-available; only the filter keeps it out.
        return {
            "pod-1": make_metrics(1, 90, 100),
            "pod-2": make_metrics(2, 0, 720),
        }

    def _stub_fleet(self, autoscaler, mocker, replicas: int = FLEET_REPLICAS):
        """Wire list_pods + a pod-keyed _collect_metrics; return the collect mock."""
        metrics_by_name = self._metrics_by_name()

        def fake_collect(pods):
            return [metrics_by_name[p.name] for p in pods if p.name in metrics_by_name]

        collect = AsyncMock(side_effect=fake_collect)
        mocker.patch.object(
            autoscaler._k8s_client,
            "list_pods",
            AsyncMock(return_value=[self._healthy_pod(), self._terminating_pod()]),
        )
        mocker.patch.object(autoscaler, "_collect_metrics", collect)
        mocker.patch.object(
            autoscaler._k8s_client,
            "get_deployment_replicas",
            AsyncMock(return_value=replicas),
        )
        return collect

    async def test_terminating_pod_not_counted_in_total_available(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        self._stub_fleet(autoscaler, mocker)
        mocker.patch.object(autoscaler, "_update_pod_deletion_costs", AsyncMock())
        mock_decision = AsyncMock()
        mocker.patch.object(autoscaler, "_make_scaling_decision", mock_decision)

        await autoscaler._scaling_iteration()

        # Only the healthy pod's 10 free slots count -- NOT 10 + 720. Forwarded
        # positionally as (now, total_available, ...).
        assert mock_decision.call_args.args[1] == 10

    async def test_terminating_pod_is_not_scraped_for_metrics(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        collect = self._stub_fleet(autoscaler, mocker)
        mocker.patch.object(autoscaler, "_update_pod_deletion_costs", AsyncMock())
        mocker.patch.object(autoscaler, "_make_scaling_decision", AsyncMock())

        await autoscaler._scaling_iteration()

        scraped = {p.name for p in collect.call_args.args[0]}
        assert scraped == {"pod-1"}

    async def test_terminating_pod_gets_no_deletion_cost_patch(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        self._stub_fleet(autoscaler, mocker)
        # Let the REAL _update_pod_deletion_costs run; stub only the K8s patch.
        patch_annotation = AsyncMock()
        mocker.patch.object(
            autoscaler._k8s_client, "patch_pod_annotation", patch_annotation
        )
        mocker.patch.object(autoscaler, "_make_scaling_decision", AsyncMock())

        await autoscaler._scaling_iteration()

        patched = {c.kwargs["pod_name"] for c in patch_annotation.call_args_list}
        assert patched == {"pod-1"}

    async def test_terminating_pod_lets_scale_up_fire(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        self._stub_fleet(autoscaler, mocker)
        mocker.patch.object(autoscaler, "_update_pod_deletion_costs", AsyncMock())
        scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", scale)

        # Real _make_scaling_decision: with the phantom 720 excluded,
        # total_available is 10 < min 30, so the scale-up branch fires. If the
        # terminating pod were counted (730 available) it would not.
        await autoscaler._scaling_iteration()

        scale.assert_called_once()
        assert scale.call_args.kwargs["replicas"] == FLEET_REPLICAS + 1


@pytest.mark.asyncio
class TestScalingIterationCountsUnreadyRunningPod:
    """
    Orthogonality guard: the terminating-pod filter is NOT a readiness filter.

    An unready (readinessProbe failing) but NOT terminating Running pod may be
    saturated -- it can be holding every one of its channels while reporting
    NOT_READY -- so it must stay in the capacity sum. Dropping unready pods
    would erase real load and manufacture phantom headroom, the exact mirror of
    the bug the terminating filter fixes.
    """

    async def test_unready_running_pod_still_counts(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        # Unready, Running, NOT terminating; reports 90/100 -> 10 available.
        unready = PodInfo("pod-1", "10.0.0.1", ready=False, phase="Running")
        mocker.patch.object(
            autoscaler._k8s_client, "list_pods", AsyncMock(return_value=[unready])
        )
        mocker.patch.object(
            autoscaler,
            "_collect_metrics",
            AsyncMock(return_value=[make_metrics(1, 90)]),
        )
        mocker.patch.object(autoscaler, "_update_pod_deletion_costs", AsyncMock())
        mocker.patch.object(
            autoscaler._k8s_client,
            "get_deployment_replicas",
            AsyncMock(return_value=FLEET_REPLICAS),
        )
        mock_decision = AsyncMock()
        mocker.patch.object(autoscaler, "_make_scaling_decision", mock_decision)

        await autoscaler._scaling_iteration()

        # Counted (total_available == 10) and NOT treated as a starting pod.
        assert mock_decision.call_args.args[1] == 10
        assert mock_decision.call_args.args[6] == 0


@pytest.mark.asyncio
class TestScalingIterationEarlyReturnRecordsNoSample:
    """
    Pins that an early-returning iteration joins NO sample to the window.

    ``_scaling_iteration`` has two early returns -- no running pods, and no
    metrics from any running pod -- and the ``_record_available_capacity``
    call sits AFTER both, on purpose: the source comment beside it states that
    the absence of an observation is not an observation of low load, and that
    recording one would let a metrics outage age into permission to scale
    down. That is an anti-flapping safety property (the 2026-08-11 sapi-bravo
    whipsaw class Epic 4 closed), yet nothing pinned it: a refactor that moved
    the recording call above the early returns would record ``total_available``
    from ``_calculate_capacity([])`` -- which is 0 -- and no test would fail.

    Each test warms the window with real idle polls past the no-amnesia
    horizon, so a recorded blackout sample would be OBSERVABLE in the
    forwarded minimum rather than hidden behind the None-returning warm-up
    guard, then interposes two blackout iterations and checks the forwarded
    ``windowed_available`` (positional arg 2 of ``_make_scaling_decision``) is
    unperturbed. The blackout gap is kept shorter than the window so a real
    idle sample still straddles the cutoff -- the blackout iterations are the
    only thing that could move the minimum.

    Honest asymmetry: the plausible regression is concentrated on the "no
    metrics" return, where a hoisted record would insert 0. A refactor moving
    the record above the "no running pods" return cannot even compute
    ``total_available`` there, so that test pins the early return's contract
    for completeness rather than guarding an equally-likely bug.
    """

    async def test_a_no_metrics_iteration_joins_no_sample_to_the_window(
        self,
        new_autoscaler,
        mocker,
        mock_settings,
        short_scale_down_window,
        freezer,
        poll_driver,
    ):
        # The load-bearing case: a hoisted record would insert
        # _calculate_capacity([]) == 0 twice inside the warmed window, dropping
        # the forwarded min() from idle_available() to 0.
        autoscaler = new_autoscaler()
        driver = poll_driver(autoscaler, freezer)
        window = short_scale_down_window
        mock_decision = AsyncMock()
        mocker.patch.object(autoscaler, "_make_scaling_decision", mock_decision)

        # Warm the window past the no-amnesia horizon so the forwarded minimum
        # is backed by observation -- not a vacuous None that would mask a
        # blackout insert.
        await driver.poll_for(idle_available(), duration=window + POLL_STEP_SECONDS)
        assert mock_decision.call_args.args[2] == idle_available()

        # Two blackout iterations, their combined gap kept shorter than the
        # window so a real idle sample still straddles the cutoff after them.
        await driver.poll_no_metrics()
        driver.advance(POLL_STEP_SECONDS)
        await driver.poll_no_metrics()

        driver.advance(POLL_STEP_SECONDS)
        await driver.poll(idle_available())

        # Identical to the pre-blackout baseline: the blackout iterations
        # recorded nothing, so the minimum never dropped toward 0.
        assert mock_decision.call_args.args[2] == idle_available()

    async def test_a_no_running_pods_iteration_joins_no_sample_to_the_window(
        self,
        new_autoscaler,
        mocker,
        mock_settings,
        short_scale_down_window,
        freezer,
        poll_driver,
    ):
        # Completeness: pins the first early return's contract. A record
        # hoisted above it has no metrics to derive total_available from, so
        # this guards the contract rather than an equally-likely regression.
        autoscaler = new_autoscaler()
        driver = poll_driver(autoscaler, freezer)
        window = short_scale_down_window
        mock_decision = AsyncMock()
        mocker.patch.object(autoscaler, "_make_scaling_decision", mock_decision)

        await driver.poll_for(idle_available(), duration=window + POLL_STEP_SECONDS)
        assert mock_decision.call_args.args[2] == idle_available()

        await driver.poll_no_running_pods()
        driver.advance(POLL_STEP_SECONDS)
        await driver.poll_no_running_pods()

        driver.advance(POLL_STEP_SECONDS)
        await driver.poll(idle_available())

        assert mock_decision.call_args.args[2] == idle_available()


@pytest.mark.asyncio
class TestScaleDownWindow:
    """
    Pins the landed smoothing of the scale-down input.

    It is a trailing MINIMUM over timestamped ``(now, total_available)``
    samples, pruned by AGE in seconds (``scaler_scale_down_window_seconds``)
    and never by sample count -- so the effective window does not change when
    adaptive polling switches between the hot and cold intervals. The sample is
    recorded in ``_scaling_iteration``, once per iteration that produced
    metrics, and the minimum that recording returns is handed to the decision
    as an argument. These tests therefore drive whole polls, or the recording
    seam itself, rather than calling the decision with a value of their own.

    The arithmetic every test below leans on: ``mock_settings`` patches
    ``scaler_min_available_capacity`` to 30, and a uniform pod capacity of 100
    puts the scale-down deadband at 130.
    """

    async def test_a_load_trough_shorter_than_the_window_does_not_scale_down(
        self,
        new_autoscaler,
        mock_settings,
        short_scale_down_window,
        freezer,
        poll_driver,
    ):
        # "Trough" is a trough in LOAD, which is a SPIKE in available capacity:
        # the incident's collapse from 140 streams to 10 sends total_available
        # up, not down. The window's job is to keep the busy reading in play
        # long enough that a spike shorter than the window cannot buy a pod
        # deletion, even with every other guard already elapsed.
        autoscaler = new_autoscaler()
        driver = poll_driver(autoscaler, freezer)
        window = short_scale_down_window

        await driver.poll_for(busy_available(), duration=window + POLL_STEP_SECONDS)
        await driver.poll_for(
            idle_available(), duration=window - POLL_STEP_SECONDS, clear_gates=True
        )

        driver.scale_deployment.assert_not_called()

    async def test_a_load_trough_longer_than_the_window_does_scale_down(
        self,
        new_autoscaler,
        mock_settings,
        short_scale_down_window,
        freezer,
        poll_driver,
    ):
        # The half that makes the previous test mean something: sustained
        # excess capacity is still allowed to give a pod back, so the test
        # above is not passing against an unconditional block.
        autoscaler = new_autoscaler()
        driver = poll_driver(autoscaler, freezer)
        window = short_scale_down_window

        await driver.poll_for(busy_available(), duration=window + POLL_STEP_SECONDS)
        await driver.poll_for(
            idle_available(), duration=window + 2 * POLL_STEP_SECONDS, clear_gates=True
        )

        assert (
            driver.scale_deployment.call_args.kwargs["replicas"] == FLEET_REPLICAS - 1
        )

    async def test_decision_uses_the_window_minimum_not_the_latest_or_the_mean(
        self, new_autoscaler, mocker, mock_settings, short_scale_down_window, freezer
    ):
        # The reduction discriminator. Against a deadband of 130 the retained
        # samples are 230, 31, 230: the latest (230) clears it and so does the
        # arithmetic mean (163), so a last-value-wins or EWMA implementation
        # would scale down here. Only the minimum (31) refuses.
        autoscaler = new_autoscaler()
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        window = short_scale_down_window
        freezer.tick(datetime.timedelta(seconds=window + 1))
        now = time.time()

        windowed = feed_window(
            autoscaler,
            [
                (window, idle_available()),
                (window // 2, busy_available()),
                (0, idle_available()),
            ],
            now,
        )
        expire_cooldowns(autoscaler, now)
        await autoscaler._make_scaling_decision(
            now=now,
            total_available=idle_available(),
            windowed_available=windowed,
            pod_capacity=DEFAULT_POD_CAPACITY,
            max_available=MAX_AVAILABLE_PER_POD,
            current_replicas=FLEET_REPLICAS,
            starting_pods=0,
        )

        mock_scale.assert_not_called()

    @pytest.mark.parametrize(
        "position,extra_age,expected_scaled",
        [
            ("at the cutoff", 0, False),
            ("one second past the cutoff", 1, True),
        ],
    )
    async def test_window_edge_retains_the_sample_covering_the_cutoff(
        self,
        new_autoscaler,
        mocker,
        mock_settings,
        short_scale_down_window,
        freezer,
        position,
        extra_age,
        expected_scaled,
    ):
        # The landed prune drops a sample only once the sample behind it has
        # itself aged to or past the cutoff, which keeps the newest reading at
        # or before the cutoff and so covers the window edge to edge. A busy
        # reading sitting exactly ``window`` seconds back therefore still holds
        # the minimum down; one second later, with a newer reading now covering
        # the cutoff, it is dropped.
        autoscaler = new_autoscaler()
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        window = short_scale_down_window
        freezer.tick(datetime.timedelta(seconds=window + extra_age + 1))
        now = time.time()

        windowed = feed_window(
            autoscaler,
            [(window + extra_age, busy_available())]
            + [
                (age, idle_available()) for age in range(window - 1 + extra_age, -1, -1)
            ],
            now,
        )
        expire_cooldowns(autoscaler, now)
        await autoscaler._make_scaling_decision(
            now=now,
            total_available=idle_available(),
            windowed_available=windowed,
            pod_capacity=DEFAULT_POD_CAPACITY,
            max_available=MAX_AVAILABLE_PER_POD,
            current_replicas=FLEET_REPLICAS,
            starting_pods=0,
        )

        assert mock_scale.called is expected_scaled

    async def test_readings_taken_on_the_scale_up_branch_still_hold_the_window_down(
        self,
        new_autoscaler,
        mock_settings,
        short_scale_down_window,
        freezer,
        poll_driver,
    ):
        # Every iteration that produced metrics records a sample, whichever
        # branch the decision then takes. Recording only on the scale-down
        # branch would leave a hole exactly where it hurts: the readings taken
        # while the fleet was starved would be missing, the window would still
        # hold the pre-burst high readings, and the first eligible read after
        # the burst would hand a pod straight back -- the whipsaw the window
        # exists to damp.
        autoscaler = new_autoscaler()
        driver = poll_driver(autoscaler, freezer)
        window = short_scale_down_window

        await driver.poll_for(idle_available(), duration=window + POLL_STEP_SECONDS)
        await driver.poll_for(starved_available(), duration=2 * POLL_STEP_SECONDS)
        driver.scale_deployment.reset_mock()
        driver.advance(POLL_STEP_SECONDS)
        expire_cooldowns(autoscaler)
        await driver.poll(idle_available())

        driver.scale_deployment.assert_not_called()

    async def test_a_zero_length_window_falls_back_to_the_instantaneous_reading(
        self, new_autoscaler, mocker, mock_settings, freezer, poll_driver
    ):
        # The operator escape hatch, and the one setting that could empty the
        # container the minimum is taken over: min() of an empty sequence
        # raises ValueError, which _main_loop's broad except would swallow into
        # a scaler that logs and never scales again. The prune keeps the newest
        # sample unconditionally and the elapsed-time guard cannot bite at zero,
        # so a zero-length window degenerates to the instantaneous reading with
        # no warm-up rather than raising.
        mocker.patch.object(scaler_settings, "scaler_scale_down_window_seconds", 0)
        autoscaler = new_autoscaler()
        driver = poll_driver(autoscaler, freezer)
        expire_cooldowns(autoscaler)

        await driver.poll(idle_available())

        assert (
            driver.scale_deployment.call_args.kwargs["replicas"] == FLEET_REPLICAS - 1
        )


@pytest.mark.asyncio
class TestScaleDownWindowInitialization:
    """
    Pins the window's no-amnesia initialization.

    The landed mechanism is an elapsed-time guard, not a synthetic seed sample
    and not a separate anchor field: ``_record_available_capacity`` returns
    None -- which forbids scale-down -- until ``now - _process_start_time``
    reaches the window length. A scaler that restarts mid-incident therefore
    has to watch a full window itself before it may hand any capacity back.

    A second None guard backs it up for the startup-blackout case, where the
    process is old enough but metrics only began arriving recently so no
    retained sample reaches back to the cutoff. The two overlap for an instance
    whose samples all postdate its own start, which is why the last test here
    -- feeding both instances a sample history that predates the younger one's
    construction -- is the one that isolates the elapsed-time guard.
    """

    async def test_a_fresh_instance_does_not_scale_down_before_the_window_elapses(
        self,
        new_autoscaler,
        mock_settings,
        short_scale_down_window,
        freezer,
        poll_driver,
    ):
        # Sustained, comfortably eligible readings from the first poll on, with
        # the cooldown and stabilization guards explicitly elapsed before every
        # poll, so the only thing left to refuse is the unwarmed window.
        autoscaler = new_autoscaler()
        driver = poll_driver(autoscaler, freezer)

        await driver.poll_for(
            idle_available(),
            duration=short_scale_down_window - POLL_STEP_SECONDS,
            clear_gates=True,
        )

        driver.scale_deployment.assert_not_called()

    async def test_a_fresh_instance_scales_down_once_the_window_elapses(
        self,
        new_autoscaler,
        mock_settings,
        short_scale_down_window,
        freezer,
        poll_driver,
    ):
        # The other half: the warm-up expires rather than blocking forever.
        autoscaler = new_autoscaler()
        driver = poll_driver(autoscaler, freezer)

        await driver.poll_for(
            idle_available(),
            duration=short_scale_down_window + 2 * POLL_STEP_SECONDS,
            clear_gates=True,
        )

        assert (
            driver.scale_deployment.call_args.kwargs["replicas"] == FLEET_REPLICAS - 1
        )

    @pytest.mark.parametrize(
        "process_age,expected_scaled",
        [("older than the window", True), ("just restarted", False)],
    )
    async def test_identical_samples_act_only_for_a_process_older_than_the_window(
        self,
        new_autoscaler,
        mocker,
        mock_settings,
        short_scale_down_window,
        freezer,
        process_age,
        expected_scaled,
    ):
        # The discriminator against a plain minimum over the retained samples.
        # Both instances are given the same readings at the same timestamps and
        # asked at the same "now"; the only difference is when the process
        # itself started, and a sample-only implementation would answer
        # identically for both.
        window = short_scale_down_window
        if process_age == "older than the window":
            autoscaler = new_autoscaler()
            freezer.tick(datetime.timedelta(seconds=window + 1))
        else:
            freezer.tick(datetime.timedelta(seconds=window + 1))
            autoscaler = new_autoscaler()
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        now = time.time()

        windowed = feed_window(
            autoscaler,
            [(age, idle_available()) for age in range(window, -1, -POLL_STEP_SECONDS)],
            now,
        )
        expire_cooldowns(autoscaler, now)
        await autoscaler._make_scaling_decision(
            now=now,
            total_available=idle_available(),
            windowed_available=windowed,
            pod_capacity=DEFAULT_POD_CAPACITY,
            max_available=MAX_AVAILABLE_PER_POD,
            current_replicas=FLEET_REPLICAS,
            starting_pods=0,
        )

        assert mock_scale.called is expected_scaled


class TestCooldownInitialization:
    """
    Pins SR-5.3: the scale-down cooldown is armed from process start.

    A scaler pod that restarts mid-incident has no memory of what the previous
    instance did, so a scale-down cooldown starting at the epoch reads as long
    expired and fires on the very first iteration after the restart. Each test
    builds its Autoscaler inside the test body, after the clock is frozen, and
    derives every timestamp from the frozen clock -- never by reading the
    instance's own ``_last_scale_down_time`` back, which would pass against any
    initial value including 0.
    """

    @pytest.mark.asyncio
    async def test_fresh_instance_does_not_scale_down_before_cooldown(
        self, new_autoscaler, mocker, mock_settings, freezer
    ):
        autoscaler = new_autoscaler()
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        cooldown = scaler_settings.scaler_scale_down_cooldown_seconds

        freezer.tick(datetime.timedelta(seconds=cooldown - 1))
        # The stabilization anchor is armed from process start too, and its
        # window is longer than the cooldown. Back-date it so the cooldown is
        # the only guard this test can be passing on.
        autoscaler._scale_up_stabilization_anchor = (
            time.time() - scaler_settings.scaler_post_scale_up_stabilization_seconds - 1
        )
        await autoscaler._try_scale_down(
            time.time(),
            current_replicas=5,
            windowed_available=200,
            total_available=200,
            pod_capacity=100,
            max_available=80,
            starting_pods=0,
        )

        mock_scale.assert_not_called()

    @pytest.mark.asyncio
    async def test_fresh_instance_scales_down_after_cooldown(
        self, new_autoscaler, mocker, mock_settings, freezer
    ):
        # The half that makes the previous test mean something: it separates a
        # cooldown armed from process start from an unconditional block.
        autoscaler = new_autoscaler()
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        cooldown = scaler_settings.scaler_scale_down_cooldown_seconds

        freezer.tick(datetime.timedelta(seconds=cooldown + 1))
        # Same reason as the previous test: without this the stabilization
        # window, not the cooldown, would decide the outcome.
        autoscaler._scale_up_stabilization_anchor = (
            time.time() - scaler_settings.scaler_post_scale_up_stabilization_seconds - 1
        )
        await autoscaler._try_scale_down(
            time.time(),
            current_replicas=5,
            windowed_available=200,
            total_available=200,
            pod_capacity=100,
            max_available=80,
            starting_pods=0,
        )

        mock_scale.assert_called_once_with(
            namespace="test-namespace",
            deployment_name="test-deployment",
            replicas=4,
        )

    @pytest.mark.asyncio
    async def test_fresh_instance_can_scale_up_immediately(
        self, new_autoscaler, mocker, mock_settings, freezer
    ):
        # Deliberate asymmetry: only the scale-DOWN cooldown is armed from
        # process start. Anchoring the scale-up cooldown too would delay
        # emergency scale-up by a full cooldown after every restart.
        autoscaler = new_autoscaler()
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)

        await autoscaler._try_scale_up(
            time.time(),
            current_replicas=3,
            starting_pods=0,
            reason=capacity_reason(),
        )

        mock_scale.assert_called_once_with(
            namespace="test-namespace",
            deployment_name="test-deployment",
            replicas=4,
        )


@pytest.mark.asyncio
class TestStabilizationWindow:
    """
    Pins the post-scale-up stabilization window.

    Landed as its own state -- ``_scale_up_stabilization_anchor``, initialized
    from ``_process_start_time`` -- and armed inside ``_try_scale_up`` only
    after ``scale_deployment`` has actually executed. It is deliberately NOT
    ``_last_scale_up_time``, which stays 0 so a restarted scaler can still
    scale up immediately and so ``_get_poll_interval`` does not read a fresh
    process as having just scaled. Its length is
    ``scaler_post_scale_up_stabilization_seconds``.
    """

    async def test_a_completed_scale_up_suppresses_a_later_scale_down(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        now = time.time()

        await complete_a_scale_up(autoscaler, now)
        mock_scale.reset_mock()
        await attempt_scale_down(autoscaler, now)

        mock_scale.assert_not_called()

    @pytest.mark.parametrize(
        "offset,expected_scaled",
        [(-1, False), (0, True), (1, True)],
    )
    async def test_suppression_lifts_when_the_window_has_elapsed(
        self,
        autoscaler,
        mocker,
        mock_settings,
        cleanup_autoscaler,
        offset,
        expected_scaled,
    ):
        # The boundary pair, offsets derived from the patched setting. The
        # comparison is "time since the scale-up < the window", so a scale-down
        # exactly one window later is already allowed.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        up_time = time.time()
        stabilization = scaler_settings.scaler_post_scale_up_stabilization_seconds

        await complete_a_scale_up(autoscaler, up_time)
        mock_scale.reset_mock()
        await attempt_scale_down(autoscaler, up_time + stabilization + offset)

        assert mock_scale.called is expected_scaled

    @pytest.mark.parametrize(
        "refusal,starting_pods,at_max_replicas,in_scale_up_cooldown",
        [
            ("pods are still starting", 1, False, False),
            ("already at max replicas", 0, True, False),
            ("inside the scale-up cooldown", 0, False, True),
        ],
    )
    async def test_a_refused_scale_up_does_not_arm_the_window(
        self,
        autoscaler,
        mocker,
        mock_settings,
        cleanup_autoscaler,
        refusal,
        starting_pods,
        at_max_replicas,
        in_scale_up_cooldown,
    ):
        # Only a scale-up that actually added capacity may hold scale-down off.
        # Stamping the anchor before _try_scale_up's early returns is an easy
        # slip and is otherwise invisible. The mock is deliberately not reset:
        # a scale-up that wrongly executed would both arm the window and show
        # up as a second call, so either failure mode breaks this assertion.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        now = time.time()
        expire_cooldowns(autoscaler, now)
        if in_scale_up_cooldown:
            autoscaler._last_scale_up_time = now
        current_replicas = (
            scaler_settings.scaler_max_replicas if at_max_replicas else FLEET_REPLICAS
        )

        await autoscaler._try_scale_up(
            now,
            current_replicas=current_replicas,
            starting_pods=starting_pods,
            reason=capacity_reason(),
        )
        await attempt_scale_down(autoscaler, now)

        mock_scale.assert_called_once_with(
            namespace="test-namespace",
            deployment_name="test-deployment",
            replicas=FLEET_REPLICAS - 1,
        )

    async def test_a_second_scale_up_is_allowed_while_scale_down_is_suppressed(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        # The window is state of its own with a duration of its own, pinned
        # without reading a private field: one scale-up cooldown after a
        # scale-up (10s as mock_settings patches it) the fleet may grow again,
        # while the much longer stabilization window (600s) still refuses to
        # let it shrink. Reusing _last_scale_up_time for both would make the
        # two decisions agree.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        up_time = time.time()
        await complete_a_scale_up(autoscaler, up_time)
        mock_scale.reset_mock()
        later = up_time + scaler_settings.scaler_scale_up_cooldown_seconds + 1

        await autoscaler._try_scale_up(
            later,
            current_replicas=FLEET_REPLICAS,
            starting_pods=0,
            reason=capacity_reason(),
        )
        await attempt_scale_down(autoscaler, later)

        # The single call is the scale-UP: the scale-down attempt at the same
        # instant added nothing.
        mock_scale.assert_called_once_with(
            namespace="test-namespace",
            deployment_name="test-deployment",
            replicas=FLEET_REPLICAS + 1,
        )

    async def test_a_scale_down_does_not_arm_the_window(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        # After a scale-down the next one is governed by the scale-down
        # cooldown alone. The assertion is unambiguous about which guard is
        # doing the work because mock_settings patches the stabilization window
        # (600s) longer than the cooldown (300s): had the scale-down armed the
        # window, the second attempt one cooldown later would still be inside
        # it and refused.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        first = time.time()
        expire_cooldowns(autoscaler, first)
        second = first + scaler_settings.scaler_scale_down_cooldown_seconds + 1

        await attempt_scale_down(autoscaler, first)
        await attempt_scale_down(autoscaler, second)

        assert mock_scale.call_count == 2

    @pytest.mark.parametrize(
        "offset,expected_scaled",
        [(-1, False), (0, True)],
    )
    async def test_a_fresh_instance_is_held_by_the_window_from_process_start(
        self,
        new_autoscaler,
        mocker,
        mock_settings,
        freezer,
        offset,
        expected_scaled,
    ):
        # The anchor's process-start initialization, made observable. At the
        # default sizing the process-start-armed scale-down cooldown dominates
        # and hides it, so the cooldown is patched short and the stabilization
        # window left long: past the cooldown but inside the window, the window
        # is the only guard that can still refuse. Had the anchor been left at
        # 0 -- amnesia across a restart -- the first case would scale down.
        mocker.patch.object(
            scaler_settings,
            "scaler_scale_down_cooldown_seconds",
            SHORT_COOLDOWN_SECONDS,
        )
        autoscaler = new_autoscaler()
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        stabilization = scaler_settings.scaler_post_scale_up_stabilization_seconds

        freezer.tick(datetime.timedelta(seconds=stabilization + offset))
        await attempt_scale_down(autoscaler, time.time())

        assert mock_scale.called is expected_scaled


@pytest.mark.asyncio
class TestScaleUpIsNotSmoothed:
    """
    Pins the asymmetry: only the scale-DOWN input is smoothed.

    Running out of capacity is an emergency, so the scale-up branch compares
    the instantaneous reading and nothing else -- no warm-up, no window delay.
    ``TestTryScaleUp`` and ``TestCooldownInitialization`` cover the up path's
    own guards; the subject here is that the window does not reach it.
    """

    async def test_a_stale_high_window_does_not_delay_scale_up(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        # The single most important assertion in the smoothing work: the fleet
        # has just fallen below the minimum while the trailing minimum is still
        # reporting the pre-incident glut, and the scale-up happens on that
        # very read.
        mock_scale_up = AsyncMock()
        mocker.patch.object(autoscaler, "_try_scale_up", mock_scale_up)

        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=starved_available(),
            windowed_available=idle_available(),
            pod_capacity=DEFAULT_POD_CAPACITY,
            max_available=MAX_AVAILABLE_PER_POD,
            current_replicas=FLEET_REPLICAS,
            starting_pods=0,
        )

        mock_scale_up.assert_called_once()

    async def test_a_stale_low_window_does_not_provoke_scale_up(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        # The converse: the smoothed value must not leak into the up
        # comparison. Recovered capacity means no scale-up, whatever the
        # trailing minimum still remembers.
        mock_scale_up = AsyncMock()
        mocker.patch.object(autoscaler, "_try_scale_up", mock_scale_up)

        await autoscaler._make_scaling_decision(
            now=time.time(),
            total_available=idle_available(),
            windowed_available=starved_available(),
            pod_capacity=DEFAULT_POD_CAPACITY,
            max_available=MAX_AVAILABLE_PER_POD,
            current_replicas=FLEET_REPLICAS,
            starting_pods=0,
        )

        mock_scale_up.assert_not_called()

    async def test_a_fresh_instance_scales_up_on_its_very_first_poll(
        self, new_autoscaler, mock_settings, freezer, poll_driver
    ):
        # End to end, through the real recording seam: on the first poll the
        # window has no minimum to give (it reports None until the process has
        # been up a full window), and the scale-up still fires immediately.
        autoscaler = new_autoscaler()
        driver = poll_driver(autoscaler, freezer)

        await driver.poll(starved_available())

        driver.scale_deployment.assert_called_once_with(
            namespace="test-namespace",
            deployment_name="test-deployment",
            replicas=FLEET_REPLICAS + 1,
        )


@pytest.mark.asyncio
class TestScaleDownGateObservability:
    """
    The two observable consequences of the scale-down guard ORDER.

    The guards are side-effect-free early returns, so their order has no
    observable consequence beyond (i) whether a refused decision consumed the
    cooldown and (ii) which reason is logged. Nothing here asserts branch order
    through patched private methods, which would only restate the source.
    """

    async def test_a_refusal_by_the_stabilization_window_does_not_consume_the_cooldown(
        self, autoscaler, mocker, mock_settings, cleanup_autoscaler
    ):
        # The stabilization guard sits ahead of the cooldown so the cooldown is
        # never consulted or stamped for a decision that was going to be
        # refused anyway. If the refused attempt had stamped
        # _last_scale_down_time, the second attempt -- at the same instant,
        # with only the stabilization window lifted -- would land inside a
        # fresh cooldown and be refused too.
        mock_scale = AsyncMock()
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", mock_scale)
        now = time.time()
        await complete_a_scale_up(autoscaler, now)
        mock_scale.reset_mock()

        await attempt_scale_down(autoscaler, now)
        autoscaler._scale_up_stabilization_anchor = (
            now - scaler_settings.scaler_post_scale_up_stabilization_seconds - 1
        )
        await attempt_scale_down(autoscaler, now)

        mock_scale.assert_called_once_with(
            namespace="test-namespace",
            deployment_name="test-deployment",
            replicas=FLEET_REPLICAS - 1,
        )

    async def test_the_reported_reason_when_two_guards_refuse_at_once(
        self,
        autoscaler,
        mocker,
        mock_settings,
        cleanup_autoscaler,
        autoscaler_info_logs,
    ):
        # The one logging assertion this file spends, and the only way the
        # relative order of two side-effect-free guards is observable at all.
        # The instance is simultaneously inside the stabilization window and
        # inside the scale-down cooldown; the stabilization guard is checked
        # first, so it is the reason the operator gets. The LEVEL is as much
        # the contract as the text: "why did it not scale down" is answerable
        # from Loki only above DEBUG, which is what production runs at (see the
        # Grafana production diagnostics runbook). The surrounding refusals
        # stay DEBUG deliberately; this one does not, so an INFO-level capture
        # of the whole attempt must contain exactly this line.
        mocker.patch.object(autoscaler._k8s_client, "scale_deployment", AsyncMock())
        now = time.time()
        stabilization = scaler_settings.scaler_post_scale_up_stabilization_seconds
        await complete_a_scale_up(autoscaler, now)
        autoscaler._last_scale_down_time = now
        autoscaler_info_logs.clear()

        await attempt_scale_down(autoscaler, now)

        assert [
            f"{record.levelname} {record.message}"
            for record in autoscaler_info_logs.records
        ] == [f"INFO In post-scale-up stabilization window (0s/{stabilization}s)"]


class TestLogStartupConfig:
    """
    The startup banner is the operator's record of what the scaler is actually
    running with, and the production diagnostics runbook reads it back out of
    Loki when reconstructing an incident. Both scale-down suppression tunables
    have to appear in it, with their units, or a timeline cannot tell a
    correctly-configured refusal from a misconfigured one.
    """

    def test_reports_the_scale_down_suppression_tunables(
        self, mock_settings, autoscaler_info_logs
    ):
        Autoscaler._log_startup_config()

        window = scaler_settings.scaler_scale_down_window_seconds
        stabilization = scaler_settings.scaler_post_scale_up_stabilization_seconds
        assert (
            f"scale_down_window={window}s, "
            f"post_scale_up_stabilization={stabilization}s"
        ) in autoscaler_info_logs.text

    def test_a_floor_above_the_ceiling_is_reported_once(
        self, inverted_replica_bounds, autoscaler_info_logs
    ):
        # A cross-field validator would raise at import time and crash the
        # scaler rather than degrade it, so the misconfiguration is reported
        # here instead -- exactly once per process, at startup, not once per
        # poll from the decision path.
        floor, ceiling = inverted_replica_bounds
        autoscaler_info_logs.clear()

        Autoscaler._log_startup_config()

        assert [
            message
            for level, message in logged_lines(autoscaler_info_logs)
            if level == "WARNING"
        ] == [
            f"min_replicas ({floor}) is above max_replicas ({ceiling}); the two "
            f"directions are enforced at different values: upward floor "
            f"enforcement is capped at max_replicas ({ceiling}), while "
            f"scale-down still refuses to shrink below the configured "
            f"min_replicas ({floor}), so the fleet holds wherever it already "
            f"sits within [{ceiling}, {floor}]"
        ]

    @pytest.mark.parametrize("floor, ceiling", [(1, 10), (3, 3)])
    def test_a_floor_not_above_the_ceiling_is_not_warned_about(
        self, mocker, mock_settings, autoscaler_info_logs, floor, ceiling
    ):
        # The banner stays quiet for every configuration that is not actually
        # inverted, so the WARNING above is a real signal rather than noise
        # every scaler emits at startup. Both sides of the boundary are here:
        # the ordinary relationship, and the equal pair a pinned-size
        # deployment (minReplicas == maxReplicas) configures -- which is a
        # legitimate deployment, not a misconfiguration, and would WARN on
        # every startup if the predicate were >= rather than >.
        mocker.patch.object(scaler_settings, "scaler_min_replicas", floor)
        mocker.patch.object(scaler_settings, "scaler_max_replicas", ceiling)
        autoscaler_info_logs.clear()

        Autoscaler._log_startup_config()

        assert not [
            message
            for level, message in logged_lines(autoscaler_info_logs)
            if level == "WARNING"
        ]


class TestGetPollInterval:
    def test_returns_hot_interval_after_recent_scale_up(
        self, autoscaler, mocker, cleanup_autoscaler
    ):
        mocker.patch.object(scaler_settings, "scaler_poll_interval_hot_seconds", 2.0)
        mocker.patch.object(scaler_settings, "scaler_scale_up_cooldown_seconds", 10)
        mocker.patch.object(scaler_settings, "scaler_min_available_capacity", 50)

        autoscaler._last_scale_up_time = time.time()
        autoscaler._last_available_capacity = 200  # High capacity but recent scale

        result = autoscaler._get_poll_interval()

        assert result == 2.0

    def test_returns_hot_interval_when_capacity_low(
        self, autoscaler, mocker, cleanup_autoscaler
    ):
        mocker.patch.object(scaler_settings, "scaler_poll_interval_hot_seconds", 2.0)
        mocker.patch.object(scaler_settings, "scaler_scale_up_cooldown_seconds", 10)
        mocker.patch.object(scaler_settings, "scaler_min_available_capacity", 50)

        autoscaler._last_scale_up_time = 0  # Long ago
        autoscaler._last_available_capacity = 80  # Below 2x min (100)

        result = autoscaler._get_poll_interval()

        assert result == 2.0

    def test_returns_cold_interval_when_stable(
        self, autoscaler, mocker, cleanup_autoscaler
    ):
        mocker.patch.object(scaler_settings, "scaler_poll_interval_cold_seconds", 15.0)
        mocker.patch.object(scaler_settings, "scaler_scale_up_cooldown_seconds", 10)
        mocker.patch.object(scaler_settings, "scaler_min_available_capacity", 50)

        # No recent scale and plenty of capacity
        autoscaler._last_scale_up_time = 0
        autoscaler._last_available_capacity = 200  # Above 2x min (100)

        result = autoscaler._get_poll_interval()

        assert result == 15.0

    @pytest.mark.asyncio
    async def test_returns_cold_interval_for_freshly_constructed_instance(
        self, new_autoscaler, mocker
    ):
        # Arming the scale-DOWN cooldown from process start left the poll
        # interval alone: _get_poll_interval reads _last_scale_up_time, which
        # still initializes to 0, so a fresh scaler with ample capacity starts
        # out polling cold exactly as before.
        mocker.patch.object(scaler_settings, "scaler_poll_interval_cold_seconds", 15.0)
        mocker.patch.object(scaler_settings, "scaler_scale_up_cooldown_seconds", 10)
        mocker.patch.object(scaler_settings, "scaler_min_available_capacity", 50)
        autoscaler = new_autoscaler()
        autoscaler._last_available_capacity = 200  # Above 2x min (100)

        result = autoscaler._get_poll_interval()

        assert result == 15.0


class TestRunAutoscaler:
    def test_raises_error_when_deployment_not_set(self, mocker):
        mocker.patch.object(scaler_settings, "scaler_target_deployment", "")

        with pytest.raises(ValueError) as exc:
            import asyncio

            asyncio.get_event_loop().run_until_complete(run_autoscaler())

        assert "OMNI_EVENTS_SCALER_TARGET_DEPLOYMENT must be set" in str(exc.value)

    def test_raises_error_when_min_capacity_is_zero(self, mocker):
        mocker.patch.object(scaler_settings, "scaler_target_deployment", "test")
        mocker.patch.object(scaler_settings, "scaler_min_available_capacity", 0)

        with pytest.raises(ValueError) as exc:
            import asyncio

            asyncio.get_event_loop().run_until_complete(run_autoscaler())

        assert "OMNI_EVENTS_SCALER_MIN_AVAILABLE_CAPACITY must be at least 1" in str(
            exc.value
        )

    def test_configures_logging_like_main_service(self, mocker):
        import logging

        mocker.patch.object(scaler_settings, "scaler_target_deployment", "")
        mocker.patch.object(scaler_settings, "scaler_log_level", "WARNING")

        mock_basic_config = mocker.patch.object(logging, "basicConfig")

        with pytest.raises(ValueError):
            import asyncio

            asyncio.get_event_loop().run_until_complete(run_autoscaler())

        # Uses the same format as logging_config.configure_logging()
        expected_format = "%(asctime)s - " + logging.BASIC_FORMAT
        mock_basic_config.assert_called_once_with(
            level=logging.WARNING,
            format=expected_format,
        )
