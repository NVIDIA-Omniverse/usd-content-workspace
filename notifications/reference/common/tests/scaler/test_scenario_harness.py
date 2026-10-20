# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Self-checks for the scenario harness.

These prove the simulator is not lying about the invariants the load-shape
scenarios rely on: pod lifecycle, deletion order, capacity-clamped load
distribution, clock advance and the runaway guard. Every assertion reads a
value the real ``_scaling_iteration`` produced, never the harness's own fleet
list -- checking the fleet list against itself would be a tautology.

This is not a place to re-test the autoscaler.
"""

import pytest
import pytest_asyncio

from tests.scaler.scenario_harness import (
    CHANNEL_UNIT_POD_CAPACITY,
    STREAM_UNIT_POD_CAPACITY,
    LoadPoint,
    ScenarioDriver,
    TickRecord,
    shipped_default,
)

# The harness's fixtures have to be visible in this module's namespace for
# pytest to resolve them by name -- including the two ``scenario`` itself
# depends on, which pytest looks up in the requesting module, not in the module
# where ``scenario`` is defined.
from tests.scaler.scenario_harness import fake_clock  # noqa: F401
from tests.scaler.scenario_harness import scenario  # noqa: F401
from tests.scaler.scenario_harness import scenario_settings  # noqa: F401

# Lifecycle delays for the boot scenario, deliberately far apart and far below
# the incident-derived defaults so several polls land inside each phase.
BOOT_VISIBLE_AFTER = 10.0
BOOT_READY_AFTER = 20.0

# A fleet at the replica floor, and exactly the demand that saturates it, so
# the first poll reports zero availability and must scale up.
BOOT_FLEET = 3
BOOT_DEMAND = BOOT_FLEET * CHANNEL_UNIT_POD_CAPACITY

# Demand well past what a channel-unit fleet of that size can hold, and
# comfortably inside what the same fleet holds in stream units.
CLAMP_FLEET = 3
CLAMP_DEMAND = 600

# The pod given a narrower capacity of its own, standing in for a mid-rollout
# fleet. Pod 1 is created first, so it reports at fleet index 0.
NARROW_POD_INDEX = 1

# A deliberately uneven fleet: an even split would tie every pod for the
# most-available-first deletion rule and make it untestable. The last pod is
# the least loaded, so it is the one a scale-down must remove.
UNEVEN_LOADS = (70, 60, 50, 40, 10)

# Free (unpinned) demand low enough that the fleet stays scale-down eligible
# and the survivors can absorb the deleted pod's share.
ABSORBED_DEMAND = 200

# Demand no surviving fleet of this size can hold, stepped in after the
# scale-down so the shortfall shows up as unplaceable. The deadband makes this
# regime unreachable from a scale-down alone -- a fleet may only give a pod
# back while the survivors can absorb its load -- so the load has to step up
# afterwards for the clamped regime to exist at all.
OVERSUBSCRIBED_DEMAND = 1000
OVERSUBSCRIBED_FROM_SECONDS = 1900
OVERSUBSCRIBED_RUN_SECONDS = 1920

# The scale-down cooldown is armed from process start and is the last guard to
# elapse, so nothing can be given back before it does. Run just past it.
SCALE_DOWN_RUN_SECONDS = 2000

# Long enough for a burst to scale the fleet up and for the collapsed load to
# then age past every scale-down guard.
FLAP_RUN_SECONDS = 1900
FLAP_COLLAPSE_AT = 60.0

# Every test here drives the async iteration seam, so the marker is applied
# once for the module rather than repeated on sixteen tests.
pytestmark = pytest.mark.asyncio


def ticks_in(driver: ScenarioDriver, start: float, end: float) -> list[TickRecord]:
    """
    Every tick recorded in the half-open scenario-time interval.

    :param driver: The driver that ran the scenario
    :param start: Inclusive lower bound, in seconds since t0
    :param end: Exclusive upper bound, in seconds since t0
    :return: The matching ticks, oldest first
    """
    return [record for record in driver.ticks if start <= record.t < end]


def tick_at(driver: ScenarioDriver, t: float) -> TickRecord:
    """
    The tick recorded at an exact scenario time.

    :param driver: The driver that ran the scenario
    :param t: The scenario time, in seconds since t0
    :return: The tick recorded then
    """
    for record in driver.ticks:
        if record.t == t:
            return record
    raise AssertionError(f"No tick at t={t}")


def tick_from(driver: ScenarioDriver, t: float) -> TickRecord:
    """
    The first tick recorded at or after a scenario time.

    :param driver: The driver that ran the scenario
    :param t: The scenario time, in seconds since t0
    :return: The first tick from then on
    """
    for record in driver.ticks:
        if record.t >= t:
            return record
    raise AssertionError(f"No tick at or after t={t}")


def tick_after(driver: ScenarioDriver, t: float) -> TickRecord:
    """
    The first tick recorded after a scenario time.

    :param driver: The driver that ran the scenario
    :param t: The scenario time to look past, in seconds since t0
    :return: The next tick
    """
    for record in driver.ticks:
        if record.t > t:
            return record
    raise AssertionError(f"No tick after t={t}")


@pytest_asyncio.fixture
async def boot_driver(scenario) -> ScenarioDriver:  # noqa: F811
    """A saturated fleet that scales up, run through the new pod's whole boot."""
    driver = scenario(
        pod_capacity=CHANNEL_UNIT_POD_CAPACITY,
        load=[LoadPoint(0, BOOT_DEMAND)],
        initial_replicas=BOOT_FLEET,
        visible_after=BOOT_VISIBLE_AFTER,
        ready_after=BOOT_READY_AFTER,
    )
    await driver.run_until(BOOT_READY_AFTER + 1)
    return driver


@pytest_asyncio.fixture
async def uneven_scale_down_driver(scenario) -> ScenarioDriver:  # noqa: F811
    """A fleet with pinned, unequal load, run until it gives a pod back."""
    driver = scenario(
        pod_capacity=CHANNEL_UNIT_POD_CAPACITY,
        load=[LoadPoint(0, 0)],
        initial_replicas=len(UNEVEN_LOADS),
    )
    for pod_index, in_use in enumerate(UNEVEN_LOADS, start=1):
        driver.pin_pod_load(pod_index, in_use)

    await driver.run_until(SCALE_DOWN_RUN_SECONDS)
    return driver


class TestScenarioDriver:
    async def test_saturated_fleet_scales_up_once(self, boot_driver):
        assert [event.direction for event in boot_driver.scale_events] == ["up"]

    async def test_new_pod_is_not_counted_as_starting_while_invisible(
        self, boot_driver
    ):
        invisible = ticks_in(boot_driver, 0, BOOT_VISIBLE_AFTER)

        assert {record.starting_pods for record in invisible} == {0}

    async def test_new_pod_counts_as_starting_once_visible(self, boot_driver):
        booting = ticks_in(boot_driver, BOOT_VISIBLE_AFTER, BOOT_READY_AFTER)

        assert {record.starting_pods for record in booting} == {1}

    async def test_new_pod_is_not_yet_reporting_while_starting(self, boot_driver):
        booting = ticks_in(boot_driver, BOOT_VISIBLE_AFTER, BOOT_READY_AFTER)

        assert {record.reporting_pods for record in booting} == {BOOT_FLEET}

    async def test_new_pod_reports_once_ready(self, boot_driver):
        assert tick_at(boot_driver, BOOT_READY_AFTER).reporting_pods == BOOT_FLEET + 1

    async def test_saturated_tick_flags_the_scale_up_predicate(self, boot_driver):
        assert boot_driver.ticks[0].below_min_capacity

    async def test_idle_tick_flags_the_unsmoothed_scale_down_predicate(
        self, uneven_scale_down_driver
    ):
        assert uneven_scale_down_driver.ticks[0].unsmoothed_scale_down_eligible

    @pytest.mark.parametrize(
        "pod_capacity, expected_available",
        [
            # 3 x 80 = 240 slots against 600 demand: every pod is clamped full.
            (CHANNEL_UNIT_POD_CAPACITY, 0),
            # 3 x 720 = 2160 slots against the same 600 demand.
            (STREAM_UNIT_POD_CAPACITY, 3 * STREAM_UNIT_POD_CAPACITY - CLAMP_DEMAND),
        ],
    )
    async def test_load_distribution_is_clamped_at_pod_capacity(
        self, scenario, pod_capacity, expected_available  # noqa: F811
    ):
        driver = scenario(
            pod_capacity=pod_capacity,
            load=[LoadPoint(0, CLAMP_DEMAND)],
            initial_replicas=CLAMP_FLEET,
        )

        await driver.run_until(0.5)

        assert driver.ticks[0].total_available == expected_available

    @pytest.mark.parametrize(
        "pod_capacity, expected_unplaceable",
        [
            (CHANNEL_UNIT_POD_CAPACITY, CLAMP_DEMAND - 3 * CHANNEL_UNIT_POD_CAPACITY),
            (STREAM_UNIT_POD_CAPACITY, 0),
        ],
    )
    async def test_demand_beyond_fleet_capacity_is_recorded_as_unplaceable(
        self, scenario, pod_capacity, expected_unplaceable  # noqa: F811
    ):
        driver = scenario(
            pod_capacity=pod_capacity,
            load=[LoadPoint(0, CLAMP_DEMAND)],
            initial_replicas=CLAMP_FLEET,
        )

        await driver.run_until(0.5)

        assert driver.ticks[0].unplaceable_streams == expected_unplaceable

    async def test_scale_down_gives_a_pod_back(self, uneven_scale_down_driver):
        assert [event.direction for event in uneven_scale_down_driver.scale_events] == [
            "down"
        ]

    async def test_scale_down_leaves_one_fewer_reporting_pod(
        self, uneven_scale_down_driver
    ):
        deletion = uneven_scale_down_driver.scale_events[0]

        after = tick_after(uneven_scale_down_driver, deletion.t)

        assert after.reporting_pods == len(UNEVEN_LOADS) - 1

    async def test_scale_down_removes_the_least_loaded_pod(
        self, uneven_scale_down_driver
    ):
        deletion = uneven_scale_down_driver.scale_events[0]

        after = tick_after(uneven_scale_down_driver, deletion.t)

        assert after.total_in_use == sum(UNEVEN_LOADS) - min(UNEVEN_LOADS)

    async def test_survivors_absorb_the_removed_pods_load(self, scenario):  # noqa: F811
        driver = scenario(
            pod_capacity=CHANNEL_UNIT_POD_CAPACITY,
            load=[LoadPoint(0, ABSORBED_DEMAND)],
            initial_replicas=len(UNEVEN_LOADS),
        )

        await driver.run_until(SCALE_DOWN_RUN_SECONDS)
        after = tick_after(driver, driver.scale_events[0].t)

        assert (after.total_in_use, after.unplaceable_streams) == (ABSORBED_DEMAND, 0)

    async def test_demand_above_surviving_capacity_is_clamped_not_absorbed(
        self, scenario  # noqa: F811
    ):
        driver = scenario(
            pod_capacity=CHANNEL_UNIT_POD_CAPACITY,
            load=[
                LoadPoint(0, ABSORBED_DEMAND),
                LoadPoint(OVERSUBSCRIBED_FROM_SECONDS, OVERSUBSCRIBED_DEMAND),
            ],
            initial_replicas=len(UNEVEN_LOADS),
        )

        await driver.run_until(OVERSUBSCRIBED_RUN_SECONDS)
        oversubscribed = tick_from(driver, OVERSUBSCRIBED_FROM_SECONDS)
        survivors = len(UNEVEN_LOADS) - 1

        assert (oversubscribed.total_in_use, oversubscribed.unplaceable_streams) == (
            survivors * CHANNEL_UNIT_POD_CAPACITY,
            OVERSUBSCRIBED_DEMAND - survivors * CHANNEL_UNIT_POD_CAPACITY,
        )

    @pytest.mark.parametrize(
        "demand, expected_interval",
        [
            (CLAMP_DEMAND, shipped_default("scaler_poll_interval_hot_seconds")),
            (0, shipped_default("scaler_poll_interval_cold_seconds")),
        ],
    )
    async def test_clock_advances_by_the_scalers_own_poll_interval(
        self, scenario, demand, expected_interval  # noqa: F811
    ):
        driver = scenario(
            pod_capacity=CHANNEL_UNIT_POD_CAPACITY,
            load=[LoadPoint(0, demand)],
            initial_replicas=CLAMP_FLEET,
        )

        await driver.run_until(40)

        assert driver.ticks[2].t - driver.ticks[1].t == expected_interval

    async def test_iteration_that_returns_early_is_still_recorded(
        self, scenario  # noqa: F811
    ):
        driver = scenario(
            pod_capacity=CHANNEL_UNIT_POD_CAPACITY,
            load=[LoadPoint(0, 0)],
            initial_replicas=0,
        )

        await driver.run_until(0.5)

        assert driver.ticks[0].decision_reached is False

    async def test_run_until_fails_fast_when_max_ticks_is_exceeded(
        self, scenario  # noqa: F811
    ):
        driver = scenario(
            pod_capacity=CHANNEL_UNIT_POD_CAPACITY,
            load=[LoadPoint(0, 0)],
            initial_replicas=CLAMP_FLEET,
        )

        with pytest.raises(AssertionError, match="max_ticks=5"):
            await driver.run_until(10_000, max_ticks=5)

    async def test_terminating_pod_can_be_placed_at_metrics_index_zero(
        self, scenario  # noqa: F811
    ):
        driver = scenario(
            pod_capacity=CHANNEL_UNIT_POD_CAPACITY,
            load=[LoadPoint(0, 0)],
            initial_replicas=len(UNEVEN_LOADS),
        )
        marked = driver.mark_terminating(pod_index=3, first=True)

        await driver.run_until(0.5)
        first_reporter = driver.ticks[0].metrics[0]

        assert (first_reporter.pod_name, first_reporter.channels_total) == (
            f"pod-{marked}",
            0,
        )

    async def test_a_pinned_capacity_is_what_that_pod_reports(
        self, scenario  # noqa: F811
    ):
        driver = scenario(
            pod_capacity=STREAM_UNIT_POD_CAPACITY,
            load=[LoadPoint(0, 0)],
            initial_replicas=CLAMP_FLEET,
        )
        driver.pin_pod_capacity(NARROW_POD_INDEX, CHANNEL_UNIT_POD_CAPACITY)
        driver.pin_pod_load(NARROW_POD_INDEX, 0)

        await driver.run_until(0.5)

        assert driver.ticks[0].metrics[0].channels_total == CHANNEL_UNIT_POD_CAPACITY

    async def test_a_pinned_capacity_does_not_lower_the_fleets_pod_capacity(
        self, scenario  # noqa: F811
    ):
        # The landed rule takes the widest healthy reporter, so a narrow pod
        # in an otherwise-wide fleet must leave the deadband at the wide value.
        driver = scenario(
            pod_capacity=STREAM_UNIT_POD_CAPACITY,
            load=[LoadPoint(0, 0)],
            initial_replicas=CLAMP_FLEET,
        )
        driver.pin_pod_capacity(NARROW_POD_INDEX, CHANNEL_UNIT_POD_CAPACITY)
        driver.pin_pod_load(NARROW_POD_INDEX, 0)

        await driver.run_until(0.5)

        assert driver.ticks[0].pod_capacity_reported == STREAM_UNIT_POD_CAPACITY

    async def test_a_burst_and_its_collapse_produce_one_direction_change(
        self, scenario  # noqa: F811
    ):
        driver = scenario(
            pod_capacity=CHANNEL_UNIT_POD_CAPACITY,
            load=[LoadPoint(0, BOOT_DEMAND), LoadPoint(FLAP_COLLAPSE_AT, 0)],
            initial_replicas=BOOT_FLEET,
        )

        await driver.run_until(FLAP_RUN_SECONDS)

        assert driver.direction_changes() == 1
