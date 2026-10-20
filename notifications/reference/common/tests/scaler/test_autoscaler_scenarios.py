# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Scenario-level tests: the 2026-08-11 incident load shape, replayed.

These are load-shape tests, not unit tests. They drive the real
``Autoscaler._scaling_iteration`` through the scenario harness against a
simulated fleet on a simulated clock, and assert STRUCTURAL properties of the
resulting trajectory -- did the fleet reverse direction mid-burst, did it hand
a pod back while it was still growing, did it converge once the load was
durably gone -- rather than a fixed replica sequence per capacity. The
fine-grained unit coverage of the individual gates lives in
``test_autoscaler.py`` and is not duplicated here.

**Non-vacuity is the whole game.** "No scale-down happened" is worthless
unless the scenario actually reached a state the pre-Epic-4 logic would have
acted on. Every no-scale-down claim here is paired with an assertion over
``TickRecord.unsmoothed_scale_down_eligible`` -- the pre-Epic-4 predicate,
``total_available >= min_capacity + pod_capacity`` -- and the headline replay
claim strengthens that pairing to ticks that were also above the replica floor
and past the process-start-anchored scale-down cooldown, so it cannot be
satisfied by ticks some OTHER gate would have refused anyway.

These tests run with the SHIPPED defaults (floor 3, scale-down cooldown 1800,
window 300, stabilization 600), pinned by the harness's ``scenario_settings``
fixture. ``test_autoscaler.py``'s ``mock_settings`` deliberately pins the OLD
defaults and is deliberately not used here.
"""

import pytest
import pytest_asyncio

from notification_common.scaler.scaler_settings import scaler_settings
from tests.scaler.scenario_harness import (
    CHANNEL_UNIT_POD_CAPACITY,
    POD_READY_AFTER_SECONDS,
    POD_VISIBLE_AFTER_SECONDS,
    STREAM_UNIT_POD_CAPACITY,
    LoadPoint,
    ScaleEvent,
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

# Every test here drives the async iteration seam.
pytestmark = pytest.mark.asyncio

# ---------------------------------------------------------------------------
# Settings-derived offsets. Never literals: Task 3 flipped two of these
# defaults and Epic 6 (t1.tj2.wi) will retune more. ``shipped_default`` reads
# the value DECLARED on ScalerSettings, which is exactly what the harness's
# ``scenario_settings`` fixture patches the live settings object to.
# ---------------------------------------------------------------------------
MIN_CAPACITY = shipped_default("scaler_min_available_capacity")
MIN_REPLICAS = shipped_default("scaler_min_replicas")
MAX_REPLICAS = shipped_default("scaler_max_replicas")
UP_COOLDOWN = shipped_default("scaler_scale_up_cooldown_seconds")
DOWN_COOLDOWN = shipped_default("scaler_scale_down_cooldown_seconds")
WINDOW = shipped_default("scaler_scale_down_window_seconds")
STABILIZATION = shipped_default("scaler_post_scale_up_stabilization_seconds")
HOT_POLL = shipped_default("scaler_poll_interval_hot_seconds")
COLD_POLL = shipped_default("scaler_poll_interval_cold_seconds")

# Every guard that can refuse a scale-down on a fleet above the floor is
# anchored at process start, so this is how long a freshly constructed scaler
# must run before any of them can permit one.
GUARD_HORIZON = max(DOWN_COOLDOWN, WINDOW, STABILIZATION)


# ---------------------------------------------------------------------------
# 1. The incident timeline, as data.
#
# Provenance. The ~8-checkout baseline, the 322->500->600 step in ~30 s, the
# 140->10 cliff in under 15 s and the directly-logged `Capacity: 600/480 used`
# peak are quoted from
# docs/runbooks/incident-2026-08-11-sapi-bravo-channel-exhaustion.md in the
# service repo. The 11:12-12:12 EDT hour of 2<->4 replica oscillation, the
# ~60 s peak / ~30 s trough cadence within it, and the healthy 5->8 scale-up
# in ~27 s logged with "0 starting" are from the Grafana diagnosis recorded on
# Plan Bee b.tj2; they are NOT in the incident-record file itself.
#
# All demand figures are FLEET-WIDE concurrent stream checkouts, never per-pod
# availability, so the one dataset replays unchanged at both per-pod
# capacities. All times are seconds since t0.
# ---------------------------------------------------------------------------

# Baseline concurrency -- incident record, "a baseline of ~8".
BASELINE_STREAMS = 8

# The step-jump ramp: one desktop client's scene load -- incident record,
# "a 322->500->600 step in ~30 s".
RAMP_STREAMS = (322, 500, 600)

# The directly-logged peak -- incident record, `Capacity: 600/480 used`.
PEAK_STREAMS = RAMP_STREAMS[-1]

# The cliff -- incident record, "140->10 in <15 s ... all 3 pods were still up;
# the scale-down came 15 s later -- load dropped on its own".
CLIFF_STREAMS = (140, 10)
TROUGH_STREAMS = CLIFF_STREAMS[-1]

# Phase A -- baseline, held past max(window, stabilization) so the fleet enters
# the burst fully warmed: no window amnesia, no stabilization residue.
# Deliberately NOT held past the scale-down cooldown. A fleet warmed past
# 1800 s at this baseline is scale-down eligible on every Phase-A tick and
# would legitimately shed a pod BEFORE the burst even starts, which is neither
# what the incident showed nor what this replay is about; an un-elapsed
# scale-down cooldown at burst onset is the realistic entry condition.
PHASE_A_START = 0.0
PHASE_A_SECONDS = max(WINDOW, STABILIZATION) + 4 * COLD_POLL

# Phase B -- the step-jump ramp, one step per cold poll so every step of the
# ~30 s ramp is sampled at least once whatever the adaptive cadence does.
PHASE_B_START = PHASE_A_START + PHASE_A_SECONDS
RAMP_STEP_SECONDS = COLD_POLL
PHASE_B_SECONDS = len(RAMP_STREAMS) * RAMP_STEP_SECONDS

# Phase C -- the cliff. CONSTRUCTED rather than measured as a single trace:
# the record reports the 600 peak and the 140->10 cliff as two independent
# measurements. The peak is carried in from Phase B, then the load steps down
# through 140 to 10 inside the measured "under 15 s".
PHASE_C_START = PHASE_B_START + PHASE_B_SECONDS
CLIFF_STEP_SECONDS = 12.0
PHASE_C_SECONDS = 2 * CLIFF_STEP_SECONDS + COLD_POLL / 2

# Phase D -- the oscillation hour: the load returns and cliffs again, over and
# over, inside the same load phase.
#
# Each trough is SHORTER than scaler_scale_down_window_seconds. That is what
# the measured shape actually was -- tens of seconds, not minutes -- and it is
# precisely the property that makes the min-over-window damping the thing
# under test: every trailing window still contains a peak sample, so the
# window minimum never clears the deadband however often the load collapses.
# Peak and trough are whole multiples of the cold poll interval so each one is
# sampled even when the cadence is at its slowest.
PHASE_D_START = PHASE_C_START + PHASE_C_SECONDS
PHASE_D_PEAK_SECONDS = 4 * COLD_POLL
PHASE_D_TROUGH_SECONDS = 2 * COLD_POLL
PHASE_D_CYCLE_SECONDS = PHASE_D_PEAK_SECONDS + PHASE_D_TROUGH_SECONDS

# Run the oscillation past the process-start-anchored scale-down cooldown plus
# a full window beyond it, so the phase contains ticks on which nothing but
# the window minimum is left to refuse a scale-down. Without that, a "no
# scale-down during the burst" claim would pass on the cooldown alone.
PHASE_D_CYCLES = int(
    -(-(DOWN_COOLDOWN + WINDOW - PHASE_D_START) // PHASE_D_CYCLE_SECONDS)
)
PHASE_D_SECONDS = PHASE_D_CYCLES * PHASE_D_CYCLE_SECONDS

# Phase E -- the quiet tail. Load settles back to baseline and stays there for
# two scale-down cooldowns, so a LEGITIMATE convergence is allowed to happen
# and can be checked for monotonicity and cooldown spacing.
PHASE_E_START = PHASE_D_START + PHASE_D_SECONDS
PHASE_E_SECONDS = 2 * DOWN_COOLDOWN
REPLAY_END = PHASE_E_START + PHASE_E_SECONDS


def _incident_load() -> list[LoadPoint]:
    """
    Build the shared incident timeline.

    :return: The fleet-wide demand steps for Phases A-E, in time order
    """
    points = [LoadPoint(PHASE_A_START, BASELINE_STREAMS)]
    for step, streams in enumerate(RAMP_STREAMS):
        points.append(LoadPoint(PHASE_B_START + step * RAMP_STEP_SECONDS, streams))
    for step, streams in enumerate(CLIFF_STREAMS):
        points.append(LoadPoint(PHASE_C_START + step * CLIFF_STEP_SECONDS, streams))
    for cycle in range(PHASE_D_CYCLES):
        cycle_start = PHASE_D_START + cycle * PHASE_D_CYCLE_SECONDS
        points.append(LoadPoint(cycle_start, PEAK_STREAMS))
        trough_start = cycle_start + PHASE_D_PEAK_SECONDS
        points.append(LoadPoint(trough_start, TROUGH_STREAMS))
    points.append(LoadPoint(PHASE_E_START, BASELINE_STREAMS))
    return points


INCIDENT_LOAD: list[LoadPoint] = _incident_load()

# The phases, for the sampling self-check and for scoping assertions.
PHASES = (
    ("A-baseline", PHASE_A_START, PHASE_B_START),
    ("B-ramp", PHASE_B_START, PHASE_C_START),
    ("C-cliff", PHASE_C_START, PHASE_D_START),
    ("D-oscillation", PHASE_D_START, PHASE_E_START),
    ("E-tail", PHASE_E_START, REPLAY_END),
)


# ---------------------------------------------------------------------------
# Replay arms.
#
# The timeline is one shared dataset. What varies per arm is the per-pod
# capacity, the starting fleet and the pod-visibility profile -- none of which
# are part of the load shape.
#
# The starting fleet is parameterized alongside the capacity because the two
# runs legitimately produce different trajectories: at ~80 the incident's 600
# checkouts exceed a small fleet and force scale-ups, while post-packing at
# ~720 they are sub-one-pod and force nothing, so the ~720 arm needs floor
# headroom of its own for its convergence to be observable at all. 4 replicas
# at ~80 is the top of the incident's measured 2<->4 flapping band and sits
# above the new floor of 3.
#
# The visibility profile is parameterized because it decides whether the
# starting-pods gate can bind on the UP path, and that in turn decides the
# fleet the burst settles at. A pod's podIP lands within a poll or two of
# creation, so the gate normally has a real window; the harness's default of
# 25 s is fitted to the incident's "0 starting" log lines, and at that value
# the scale-up cooldown (10 s) fires up to three times before the first new
# pod is even visible. Both are replayed: the structural claims hold at both,
# and each arm's distinct outcome is asserted where it belongs.
# ---------------------------------------------------------------------------
GATED_BOOT_VISIBLE_AFTER = 2.0

# The top of the incident's measured 2<->4 replica flapping band, and the
# fleet the burst is entered with at channel-unit capacity.
INCIDENT_FLAP_BAND_TOP = 4

# Floor headroom for the stream-unit arm: three pods' worth, so the fleet has
# somewhere to converge to and the convergence claims are not answered by the
# replica floor alone.
STREAM_UNIT_FLOOR_HEADROOM = 3

# Per-arm scale-up threshold. Each replay arm carries its own
# ``scaler_min_available_capacity``, applied over the shipped default inside the
# ``replay`` fixture, so the retune is expressed entirely in this file.
#
# Task 1 (subtask t3.tj2.wi.9w.kk) retuned the DEPLOYED threshold -- helm
# ``channelPoolAutoscaling.minAvailableCapacity`` -- from 30 to 800 stream units
# for the post-Epic-3 packing (one pod's soft threshold =
# ``channel_pool_size 100 x channel_client_limit 8``). That decision was
# values-only: the notification_common library default
# (``scaler_settings.scaler_min_available_capacity``) is deliberately left at 30
# as a never-hit fallback, so ``shipped_default("scaler_min_available_capacity")``
# (== ``MIN_CAPACITY``) still reads 30 and the stream-unit arm must pin the
# retuned value explicitly. The channel-unit arms track the shipped default so a
# later default change flows through without a literal here.
STREAM_UNIT_MIN_CAPACITY = 800
CHANNEL_UNIT_MIN_CAPACITY = MIN_CAPACITY

CHANNEL_UNIT_ARM = (
    CHANNEL_UNIT_POD_CAPACITY,
    INCIDENT_FLAP_BAND_TOP,
    GATED_BOOT_VISIBLE_AFTER,
    CHANNEL_UNIT_MIN_CAPACITY,
)
CHANNEL_UNIT_LATE_ARM = (
    CHANNEL_UNIT_POD_CAPACITY,
    INCIDENT_FLAP_BAND_TOP,
    POD_VISIBLE_AFTER_SECONDS,
    CHANNEL_UNIT_MIN_CAPACITY,
)
# The visibility profile is immaterial on the stream-unit arm: even at the
# retuned 800 threshold the burst never drives availability below the minimum
# (600 concurrent streams are sub-one-pod at 720 per pod), so it never scales up
# and no pod ever boots.
STREAM_UNIT_ARM = (
    STREAM_UNIT_POD_CAPACITY,
    MIN_REPLICAS + STREAM_UNIT_FLOOR_HEADROOM,
    POD_VISIBLE_AFTER_SECONDS,
    STREAM_UNIT_MIN_CAPACITY,
)

REPLAY_ARMS = [CHANNEL_UNIT_ARM, CHANNEL_UNIT_LATE_ARM, STREAM_UNIT_ARM]
REPLAY_ARM_IDS = [
    "channel-unit-gated-boot",
    "channel-unit-late-visibility",
    "stream-unit-gated-boot",
]

# The channel-unit arm polls hot through most of the burst, so a whole replay
# costs far more iterations than it has phases.
REPLAY_MAX_TICKS = 4000


def ticks_in(driver: ScenarioDriver, start: float, end: float) -> list[TickRecord]:
    """
    Every tick recorded in the half-open scenario-time interval.

    :param driver: The driver that ran the scenario
    :param start: Inclusive lower bound, in seconds since t0
    :param end: Exclusive upper bound, in seconds since t0
    :return: The matching ticks, oldest first
    """
    return [record for record in driver.ticks if start <= record.t < end]


def events_in(driver: ScenarioDriver, start: float, end: float) -> list[ScaleEvent]:
    """
    Every scale event recorded in the half-open scenario-time interval.

    :param driver: The driver that ran the scenario
    :param start: Inclusive lower bound, in seconds since t0
    :param end: Exclusive upper bound, in seconds since t0
    :return: The matching events, oldest first
    """
    return [event for event in driver.scale_events if start <= event.t < end]


def scale_downs(driver: ScenarioDriver) -> list[ScaleEvent]:
    """
    :param driver: The driver that ran the scenario
    :return: Every scale-down the scaler executed, oldest first
    """
    return [event for event in driver.scale_events if event.direction == "down"]


def scale_ups(driver: ScenarioDriver) -> list[ScaleEvent]:
    """
    :param driver: The driver that ran the scenario
    :return: Every scale-up the scaler executed, oldest first
    """
    return [event for event in driver.scale_events if event.direction == "up"]


def unrefusable_ticks(
    driver: ScenarioDriver, start: float, end: float
) -> list[TickRecord]:
    """
    Ticks the pre-Epic-4 rule would have scaled down on, for its own reason.

    The strengthened non-vacuity discriminator: eligible under the old
    predicate, above the replica floor, and past the process-start-anchored
    scale-down cooldown -- so neither the floor nor the cooldown can be the
    reason nothing happened, and only the smoothing window is left.

    :param driver: The driver that ran the scenario
    :param start: Inclusive lower bound, in seconds since t0
    :param end: Exclusive upper bound, in seconds since t0
    :return: The discriminating ticks, oldest first
    """
    return [
        record
        for record in ticks_in(driver, start, end)
        if record.unsmoothed_scale_down_eligible
        and record.replicas > MIN_REPLICAS
        and record.t >= DOWN_COOLDOWN
    ]


@pytest_asyncio.fixture
async def replay(request, mocker, scenario) -> ScenarioDriver:  # noqa: F811
    """
    Drive the whole incident timeline at one arm's capacity and fleet.

    Parameterized indirectly with a
    ``(pod_capacity, initial_replicas, visible_after, min_available_capacity)``
    tuple. The per-arm ``min_available_capacity`` is patched over the shipped
    default that ``scenario_settings`` (pulled in via ``scenario``) has already
    pinned, so the stream-unit arm re-drives the incident at Task 1's retuned
    threshold while the channel-unit arms stay at the shipped default -- the
    same per-arm ``mocker.patch.object`` idiom ``restart_driver`` and
    ``undamped_settings`` use. The autoscaler reads ``scaler_settings`` live at
    decision time, and the harness computes each ``TickRecord``'s predicate
    flags against the same live value, so the override flows through both.
    """
    pod_capacity, initial_replicas, visible_after, min_capacity = request.param
    mocker.patch.object(scaler_settings, "scaler_min_available_capacity", min_capacity)
    driver = scenario(
        pod_capacity=pod_capacity,
        load=INCIDENT_LOAD,
        initial_replicas=initial_replicas,
        visible_after=visible_after,
    )

    await driver.run_until(REPLAY_END, max_ticks=REPLAY_MAX_TICKS)
    return driver


class TestIncidentTimeline:
    """
    The dataset's own preconditions, which the replay claims depend on.

    Declared as tests rather than as comments because they are derived from
    settings Epic 6 (t1.tj2.wi) is expected to retune: a retune that inverts
    one of these relationships silently turns a replay claim vacuous, and
    should fail here instead.

    These five are synchronous checks over module constants; they carry the
    module's asyncio marker only because it is applied module-wide.
    """

    async def test_the_troughs_are_shorter_than_the_smoothing_window(self):
        assert PHASE_D_TROUGH_SECONDS < WINDOW

    async def test_the_oscillation_outlasts_the_scale_down_cooldown(self):
        assert PHASE_E_START >= DOWN_COOLDOWN + WINDOW

    async def test_the_baseline_outlasts_the_window_and_the_stabilization(self):
        assert PHASE_A_SECONDS > max(WINDOW, STABILIZATION)

    async def test_the_baseline_stops_short_of_the_scale_down_cooldown(self):
        assert PHASE_B_START < DOWN_COOLDOWN

    async def test_the_tail_spans_two_scale_down_cooldowns(self):
        assert PHASE_E_SECONDS >= 2 * DOWN_COOLDOWN


@pytest.mark.parametrize("replay", REPLAY_ARMS, ids=REPLAY_ARM_IDS, indirect=True)
class TestIncidentLoadShapeReplay:
    """
    The capacity-invariant structural claims, asserted at every arm.

    None of these encode a replica sequence: they are properties of the shape
    of the trajectory, which is what the Epic actually promises.
    """

    async def test_every_phase_is_sampled(self, replay):
        assert all(ticks_in(replay, start, end) for _, start, end in PHASES)

    async def test_every_oscillation_trough_is_sampled(self, replay):
        troughs = [
            (
                PHASE_D_START + cycle * PHASE_D_CYCLE_SECONDS + PHASE_D_PEAK_SECONDS,
                PHASE_D_START + (cycle + 1) * PHASE_D_CYCLE_SECONDS,
            )
            for cycle in range(PHASE_D_CYCLES)
        ]

        assert all(ticks_in(replay, start, end) for start, end in troughs)

    async def test_the_fleet_never_reverses_direction_twice(self, replay):
        # The Epic's headline claim in one observable: the fleet grows, then
        # once the load is durably gone it shrinks. It never reverses twice.
        assert replay.direction_changes() <= 1

    async def test_no_scale_down_inside_a_stabilization_window(self, replay):
        forbidden = [
            event
            for up in scale_ups(replay)
            for event in events_in(replay, up.t, up.t + STABILIZATION)
            if event.direction == "down"
        ]

        assert forbidden == []

    async def test_the_tail_converges(self, replay):
        assert scale_downs(replay)

    async def test_the_tail_only_ever_scales_down(self, replay):
        tail = events_in(replay, PHASE_E_START, REPLAY_END)

        assert {event.direction for event in tail} == {"down"}

    async def test_the_convergence_has_more_than_one_step(self, replay):
        # Non-vacuity for the two pairwise claims below. Both iterate over
        # CONSECUTIVE PAIRS of scale-downs, so a trajectory with a single
        # scale-down makes each of them an empty ``all()`` and passes them
        # without ever comparing anything. Every arm currently converges in
        # more than one step; if an Epic 6 (t1.tj2.wi) retune collapses one of
        # them to a single step, this fails here rather than quietly hollowing
        # out the two claims that follow.
        assert len(scale_downs(replay)) >= 2

    async def test_scale_downs_are_strictly_monotone(self, replay):
        replicas = [event.to_replicas for event in scale_downs(replay)]

        assert len(replicas) >= 2 and all(
            later < earlier for earlier, later in zip(replicas, replicas[1:])
        )

    async def test_scale_downs_are_cooldown_spaced(self, replay):
        moments = [event.t for event in scale_downs(replay)]
        gaps = [later - earlier for earlier, later in zip(moments, moments[1:])]

        assert gaps and all(gap >= DOWN_COOLDOWN for gap in gaps)

    async def test_the_fleet_never_drops_below_the_floor(self, replay):
        assert min(record.replicas for record in replay.ticks) >= MIN_REPLICAS

    async def test_churn_stays_bounded(self, replay):
        # A structural bound, not a fitted one: the fleet cannot grow past
        # max_replicas from where it started, and the process-start-anchored
        # cooldown admits at most one scale-down per cooldown of scenario
        # time. The measured incident put roughly a dozen replica changes into
        # its oscillation hour alone, so a flapping implementation blows past
        # this even when it somehow satisfies the direction-change count.
        initial_replicas = replay.ticks[0].replicas
        bound = (MAX_REPLICAS - initial_replicas) + int(REPLAY_END // DOWN_COOLDOWN)

        assert len(replay.scale_events) <= bound


@pytest.mark.parametrize(
    "replay", [CHANNEL_UNIT_ARM], ids=REPLAY_ARM_IDS[:1], indirect=True
)
class TestIncidentLoadShapeReplayUnderPressure:
    """
    The pressure-conditional half of the replay: the incident-closure claim.

    Asserted only where the burst actually creates capacity pressure. This
    stays gated to the channel-unit arm even after Task 1 (t3.tj2.wi.9w.kk)
    landed the min_available_capacity retune (30 -> 800 stream units): 600
    concurrent streams are sub-one-pod at ~720 per pod (600 < 720), so at the
    retuned 800 threshold the burst STILL never makes the stream-unit fleet
    tight -- and converging toward the replica floor under the baseline tail is
    CORRECT behavior there, not the incoherent collapse Epic 6 (t1.tj2.wi)
    fixes. That collapse pathology needs sustained ABOVE-one-pod load, which the
    incident's sub-one-pod profile does not contain; the retuned-threshold
    re-derivation for this arm is pinned in
    TestStreamUnitReplayAtRetunedThreshold so the gating is justified by running
    assertions rather than silently left weakened. It is also asserted at the
    gated-boot arm rather than the late-visibility one:
    with pod visibility fitted at 25 s the up path outruns its own gate and
    overshoots the fleet past what the peak needs, which makes a later trim
    correct rather than a regression. That arm's outcome is pinned separately
    in TestLateVisibilityOvershootTrim.
    """

    async def test_no_scale_down_during_the_burst_and_the_oscillation(self, replay):
        burst = events_in(replay, PHASE_B_START, PHASE_E_START)

        assert [event for event in burst if event.direction == "down"] == []

    async def test_the_burst_reached_a_state_the_old_rule_would_have_acted_on(
        self, replay
    ):
        # The pairing that makes the claim above worth anything: the old rule
        # WOULD have given a pod back here, and neither the floor nor the
        # cooldown was standing in its way.
        assert unrefusable_ticks(replay, PHASE_C_START, PHASE_E_START)

    async def test_the_window_minimum_is_what_refused_the_scale_down(self, replay):
        # ... and this is the mechanism. On those same ticks the trailing
        # minimum sits below the deadband the instantaneous reading cleared,
        # because every window still contains a peak sample.
        refused = unrefusable_ticks(replay, PHASE_C_START, PHASE_E_START)

        assert all(
            record.windowed_available < MIN_CAPACITY + record.pod_capacity_reported
            for record in refused
        )

    async def test_the_burst_forced_the_fleet_to_grow(self, replay):
        assert scale_ups(replay)


@pytest.mark.parametrize(
    "replay", [STREAM_UNIT_ARM], ids=REPLAY_ARM_IDS[2:3], indirect=True
)
class TestStreamUnitReplayAtRetunedThreshold:
    """
    The stream-unit arm re-driven at Task 1's retuned min_available_capacity.

    Task 1 (t3.tj2.wi.9w.kk) retuned the deployed
    ``channelPoolAutoscaling.minAvailableCapacity`` from 30 to 800 stream units,
    values-only -- the notification_common library default stays 30, so the
    ``replay`` fixture pins 800 on this arm explicitly. Re-driven at 800, the
    incident's ~600-stream burst is still sub-one-pod at 720 per pod
    (600 < 720): it never drives fleet-wide availability below the 800 threshold
    and forces no growth, which is exactly the condition
    ``TestIncidentLoadShapeReplayUnderPressure`` is gated away from this arm for.

    These are the running assertions behind that gating decision -- the
    re-derivation the Subtask requires so no capacity-conditional claim is left
    silently weakened. The convergence-above-floor picture the retune produces
    for a genuinely above-one-pod sustained load is NOT reachable from this
    sub-one-pod incident profile (8 baseline streams need a single pod, so the
    coherent level here IS the replica floor); that above-floor coherence is
    exercised by the deadband unit tests in ``test_autoscaler.py``, not by this
    replay. What this arm proves is the AC-4 tuning half at the retuned
    threshold: re-driven at 800, the fleet still never oscillates.
    """

    async def test_the_retuned_threshold_is_live_on_this_arm(self, replay):
        # The per-arm override reached the live settings object the decision
        # reads, so this really is the 800-unit re-drive, not the 30 default.
        assert scaler_settings.scaler_min_available_capacity == STREAM_UNIT_MIN_CAPACITY

    async def test_the_burst_never_read_below_the_retuned_threshold(self, replay):
        # Non-vacuity for the gating: at 720 per pod the incident load genuinely
        # stayed at or above the retuned 800 threshold on every tick, which is
        # WHY the pressure-conditional claims stay channel-unit-only rather than
        # being re-gated onto this arm.
        assert not any(record.below_min_capacity for record in replay.ticks)

    async def test_the_burst_forced_no_growth(self, replay):
        # ... and the direct consequence: a sub-one-pod burst adds no pod even
        # at the retuned threshold.
        assert scale_ups(replay) == []

    async def test_the_fleet_converges_without_oscillating(self, replay):
        # AC-4 tuning half at the retuned threshold: the fleet only ever gives
        # pods back once the load is durably gone, never interleaving a
        # scale-up, so the direction never reverses.
        assert replay.direction_changes() == 0 and scale_downs(replay)

    async def test_the_convergence_target_is_the_sub_one_pod_floor(self, replay):
        # The coherent convergence target at the retuned threshold for this
        # sub-one-pod load: the baseline tail (8 streams) needs a single pod, so
        # the fleet trims its floor headroom back down to the replica floor.
        # This is correct convergence, not the pre-retune deadband-[30, 750]
        # collapse -- the two are indistinguishable ON THIS LOAD because the
        # load never justifies staying above the floor. Re-derived from Task 1's
        # value rather than restated from the Epic-4 before-picture.
        assert replay.ticks[-1].replicas == MIN_REPLICAS


@pytest.mark.parametrize(
    "replay", [CHANNEL_UNIT_LATE_ARM], ids=REPLAY_ARM_IDS[1:2], indirect=True
)
class TestLateVisibilityOvershootTrim:
    """
    What the fitted 25 s pod-visibility profile costs, measured.

    With the scale-up cooldown (10 s) well below the delay before a new pod
    appears in ``list_pods``, the up path fires several times before its own
    starting-pods gate can bind and the fleet lands one pod above what the
    burst needs. The landed scale-down logic then trims exactly that surplus,
    once, at the first cooldown boundary -- and the surviving fleet still
    holds the phase's own peak. That is convergence from an overshoot, not the
    flap this Epic closes, and these are the claims that separate the two.
    Recorded here as Epic 6's (t1.tj2.wi) gate-cost before-picture.
    """

    async def test_no_scale_down_leaves_the_fleet_short_of_the_peak(self, replay):
        pod_capacity = replay.pod_capacity
        burst = events_in(replay, PHASE_B_START, PHASE_E_START)

        assert all(
            event.to_replicas * pod_capacity >= PEAK_STREAMS + MIN_CAPACITY
            for event in burst
            if event.direction == "down"
        )

    async def test_no_trough_ever_bought_a_scale_down(self, replay):
        # The distinguishing property. A trough-bought deletion is one decided
        # on a reading taken while the load was collapsed; every deletion here
        # is decided on a window minimum strictly below the instantaneous
        # reading, which means the window carried a peak sample into it.
        deciding = [
            record
            for event in events_in(replay, PHASE_B_START, PHASE_E_START)
            if event.direction == "down"
            for record in replay.ticks
            if record.t == event.t
        ]

        assert deciding and all(
            record.windowed_available < record.total_available for record in deciding
        )

    async def test_the_surplus_is_trimmed_no_faster_than_the_cooldown(self, replay):
        # At most one deletion per cooldown boundary inside the phase, which
        # is what makes this a trim and not a flap.
        burst = events_in(replay, PHASE_B_START, PHASE_E_START)
        boundaries = int((PHASE_E_START - PHASE_B_START) // DOWN_COOLDOWN) + 1

        assert len([e for e in burst if e.direction == "down"]) <= boundaries

    async def test_the_overshoot_is_what_made_the_trim_legitimate(self, replay):
        # Non-vacuity for the three claims above: the fleet really did grow
        # past what the peak needed, so there really was a surplus to trim.
        needed = -(-(PEAK_STREAMS + MIN_CAPACITY) // replay.pod_capacity)

        assert max(record.replicas for record in replay.ticks) > needed


# ---------------------------------------------------------------------------
# 3. Scale-up promptness: the good behaviour has to survive the damping.
# ---------------------------------------------------------------------------

# A fleet that comfortably holds the ramp's first step and is one step below
# what the peak needs -- the incident's own 5-replica starting point.
LATENCY_START_REPLICAS = 5

# What the ~600-checkout peak actually needs at channel-unit capacity, with
# the minimum buffer still intact: ceil((600 + 30) / 80) = 8, the incident's
# measured 5->8.
LATENCY_TARGET_REPLICAS = -(-(PEAK_STREAMS + MIN_CAPACITY) // CHANNEL_UNIT_POD_CAPACITY)
LATENCY_GROWTH_STEPS = LATENCY_TARGET_REPLICAS - LATENCY_START_REPLICAS

# The peak arrives a few cold polls in, so the run has a settled before-state.
LATENCY_PEAK_AT = 4 * COLD_POLL

# Deadlines, measured from the first tick that reads below the minimum.
#
# At the fitted 25 s visibility the incident's own cadence applies: one
# scale-up per scale-up cooldown, nothing gating them, so three steps cost
# three cooldowns plus the poll that carries the last one. The incident
# measured this real-world 5->8 at roughly 27 s with "0 starting" logged
# throughout, which is exactly this profile.
#
# At a realistic 2 s visibility the starting-pods gate binds instead, and
# every step after the first waits out a full pod boot -- the up-path cost of
# the now-symmetric gates, and Epic 6's (t1.tj2.wi) input.
LATENCY_LATE_DEADLINE = LATENCY_GROWTH_STEPS * UP_COOLDOWN + HOT_POLL
LATENCY_GATED_DEADLINE = (
    (LATENCY_GROWTH_STEPS - 1) * POD_READY_AFTER_SECONDS + UP_COOLDOWN + 2 * HOT_POLL
)

LATENCY_ARMS = [
    (POD_VISIBLE_AFTER_SECONDS, LATENCY_LATE_DEADLINE),
    (GATED_BOOT_VISIBLE_AFTER, LATENCY_GATED_DEADLINE),
]
LATENCY_ARM_IDS = ["late-visibility", "gated-boot"]


@pytest_asyncio.fixture
async def latency_driver(request, scenario) -> ScenarioDriver:  # noqa: F811
    """Replay the ramp's tail: a settled fleet, then the ~600 peak."""
    visible_after = request.param
    driver = scenario(
        pod_capacity=CHANNEL_UNIT_POD_CAPACITY,
        load=[
            LoadPoint(0, RAMP_STREAMS[0]),
            LoadPoint(LATENCY_PEAK_AT, PEAK_STREAMS),
        ],
        initial_replicas=LATENCY_START_REPLICAS,
        visible_after=visible_after,
    )

    await driver.run_until(
        LATENCY_PEAK_AT + max(deadline for _, deadline in LATENCY_ARMS)
    )
    return driver


def first_below_min(driver: ScenarioDriver) -> TickRecord:
    """
    The first tick that read below the minimum available capacity.

    :param driver: The driver that ran the scenario
    :return: That tick
    """
    for record in driver.ticks:
        if record.below_min_capacity:
            return record
    raise AssertionError("No tick ever read below the minimum available capacity")


@pytest.mark.parametrize(
    "latency_driver, deadline",
    LATENCY_ARMS,
    ids=LATENCY_ARM_IDS,
    indirect=["latency_driver"],
)
class TestScaleUpLatencyUnderReplay:
    """
    The direct rebuttal to "the fix made us slower".

    Nothing added on the scale-down side may damp the up path: it reads the
    instantaneous capacity, and it reacts on the tick that reading first goes
    short.
    """

    async def test_the_first_scale_up_lands_on_the_first_short_tick(
        self, latency_driver, deadline
    ):
        assert latency_driver.scale_events[0].t == first_below_min(latency_driver).t

    async def test_the_first_scale_event_is_a_scale_up(self, latency_driver, deadline):
        assert latency_driver.scale_events[0].direction == "up"

    async def test_the_fleet_reaches_what_the_peak_needs_within_the_deadline(
        self, latency_driver, deadline
    ):
        grown = [
            record
            for record in latency_driver.ticks
            if record.replicas >= LATENCY_TARGET_REPLICAS
        ]

        assert grown and grown[0].t <= first_below_min(latency_driver).t + deadline

    async def test_no_scale_down_interleaves_the_growth(self, latency_driver, deadline):
        # A guard rather than a discriminator, and deliberately so: on a
        # horizon this short the process-start-anchored cooldown would forbid
        # a deletion by itself. It is here to catch a future change that makes
        # the up path hand a pod back mid-climb through some other route.
        assert scale_downs(latency_driver) == []


# ---------------------------------------------------------------------------
# 4. SR-5.1 end to end: a reporter that is not the widest must not price the
#    deadband. The direct-argument unit twin lives in test_autoscaler.py.
# ---------------------------------------------------------------------------

# The narrow reporter's capacity: an old-image pod mid-rollout, publishing the
# channel-unit value while the rest of the fleet publishes the stream-unit one.
NARROW_POD_CAPACITY = CHANNEL_UNIT_POD_CAPACITY

# Pod 1 is created first, so it reports at metrics index 0 -- where the old
# rule read the fleet's pod capacity from.
INDEX_ZERO_POD = 1

# Clear of both replica guards, so neither the floor nor the ceiling can be
# the reason a deletion did or did not happen.
REPORTER_FLEET_REPLICAS = MIN_REPLICAS + 1

# Availability placed squarely between the two deadbands: above the old
# metrics[0] one (min + the index-0 reporter's capacity), which would have
# scaled down, and below the landed one (min + the widest reporter's
# capacity), which does not.
NARROW_TARGET_AVAILABLE = (
    (MIN_CAPACITY + NARROW_POD_CAPACITY) + (MIN_CAPACITY + STREAM_UNIT_POD_CAPACITY)
) // 2

# The narrow pod is pinned full, so the whole demand rides the wide pods and
# the fleet's availability is entirely theirs.
NARROW_WIDE_PODS = REPORTER_FLEET_REPLICAS - 1
NARROW_DEMAND = NARROW_WIDE_PODS * STREAM_UNIT_POD_CAPACITY - NARROW_TARGET_AVAILABLE

# Long enough that every process-start-anchored guard has elapsed and the
# decision is down to the deadband alone.
REPORTER_RUN_SECONDS = GUARD_HORIZON + 2 * COLD_POLL

# When the zero-capacity reporter appears. Before the scale-down cooldown
# elapses, because a terminating pod removes a full pod-step of capacity: the
# fleet has to sit one step ABOVE the discriminating band beforehand, and a
# fleet left there past the cooldown would legitimately shed a pod first.
ZERO_REPORTER_MARKED_AT = DOWN_COOLDOWN / 2

# Post-termination availability, mid-band: at or above the minimum (so the
# scale-UP branch does not claim the decision) and below the landed deadband.
ZERO_REPORTER_ARMS = [CHANNEL_UNIT_POD_CAPACITY, STREAM_UNIT_POD_CAPACITY]
ZERO_REPORTER_ARM_IDS = ["channel-unit-capacity", "stream-unit-capacity"]


def zero_reporter_demand(pod_capacity: int) -> int:
    """
    Demand that lands the surviving fleet inside the discriminating band.

    :param pod_capacity: The per-pod capacity of the healthy reporters
    :return: The fleet-wide demand to hold
    """
    survivors = (REPORTER_FLEET_REPLICAS - 1) * pod_capacity
    return survivors - MIN_CAPACITY - pod_capacity // 2


@pytest_asyncio.fixture
async def narrow_reporter_driver(scenario) -> ScenarioDriver:  # noqa: F811
    """A mid-rollout fleet: one narrow reporter at index 0, the rest wide."""
    driver = scenario(
        pod_capacity=STREAM_UNIT_POD_CAPACITY,
        load=[LoadPoint(0, NARROW_DEMAND)],
        initial_replicas=REPORTER_FLEET_REPLICAS,
    )
    driver.pin_pod_capacity(INDEX_ZERO_POD, NARROW_POD_CAPACITY)
    driver.pin_pod_load(INDEX_ZERO_POD, NARROW_POD_CAPACITY)

    await driver.run_until(REPORTER_RUN_SECONDS)
    return driver


@pytest_asyncio.fixture
async def zero_reporter_driver(request, scenario) -> ScenarioDriver:  # noqa: F811
    """A steady fleet that acquires a terminating, zero-capacity reporter."""
    pod_capacity = request.param
    driver = scenario(
        pod_capacity=pod_capacity,
        load=[LoadPoint(0, zero_reporter_demand(pod_capacity))],
        initial_replicas=REPORTER_FLEET_REPLICAS,
    )

    await driver.run_until(ZERO_REPORTER_MARKED_AT)
    driver.mark_terminating(pod_index=INDEX_ZERO_POD, channels_in_use=0, first=True)
    await driver.run_until(REPORTER_RUN_SECONDS)
    return driver


class TestNarrowestReporterAtIndexZeroScenario:
    """
    The discriminating half of SR-5.1, end to end.

    It rides a NON-ZERO narrow reporter deliberately. Reverting the source to
    the old ``metrics[0]`` rule with a zero-capacity reporter present yields
    ``pod_capacity == 0``, which the "capacity unknown" suppression guard
    catches on its own, so a zero-reporter scenario cannot tell the two rules
    apart. A narrow but healthy reporter can: the old rule prices a deletion
    at 80 and the landed one at 720.
    """

    async def test_no_scale_down_with_a_narrow_reporter_at_index_zero(
        self, narrow_reporter_driver
    ):
        assert narrow_reporter_driver.scale_events == ()

    async def test_the_narrow_reporter_really_is_at_index_zero(
        self, narrow_reporter_driver
    ):
        settled = narrow_reporter_driver.ticks[-1]

        assert settled.metrics[0].channels_total == NARROW_POD_CAPACITY

    async def test_the_old_index_zero_deadband_would_have_been_cleared(
        self, narrow_reporter_driver
    ):
        # Computed from the tick's own recorded values, not restated from the
        # source: this is what the pre-Epic-4 rule would have compared.
        settled = narrow_reporter_driver.ticks[-1]

        assert (
            settled.total_available
            >= scaler_settings.scaler_min_available_capacity
            + settled.metrics[0].channels_total
        )

    async def test_the_landed_widest_reporter_deadband_was_not_cleared(
        self, narrow_reporter_driver
    ):
        settled = narrow_reporter_driver.ticks[-1]

        assert (
            settled.total_available
            < scaler_settings.scaler_min_available_capacity
            + settled.pod_capacity_reported
        )

    async def test_the_window_cannot_be_the_reason(self, narrow_reporter_driver):
        # The load is constant, so the trailing minimum equals the
        # instantaneous reading and smoothing has nothing to suppress. Read
        # back off the tick rather than taken on faith.
        settled = narrow_reporter_driver.ticks[-1]

        assert settled.windowed_available == settled.total_available

    async def test_the_run_outlasted_every_process_start_anchored_guard(
        self, narrow_reporter_driver
    ):
        assert narrow_reporter_driver.ticks[-1].t >= GUARD_HORIZON


@pytest.mark.parametrize(
    "zero_reporter_driver",
    ZERO_REPORTER_ARMS,
    ids=ZERO_REPORTER_ARM_IDS,
    indirect=True,
)
class TestZeroCapacityReporterScenario:
    """
    The combined-safety half of SR-5.1: a terminating pod at metrics index 0.

    A pod that is terminating, restarting, or has not built its pool yet
    reports ``channels_total=0``. Sorted first, it is exactly what the old
    rule read the fleet's pod capacity from.
    """

    async def test_no_scale_down_once_the_zero_reporter_appears(
        self, zero_reporter_driver
    ):
        assert (
            events_in(
                zero_reporter_driver, ZERO_REPORTER_MARKED_AT, REPORTER_RUN_SECONDS
            )
            == []
        )

    async def test_no_scale_down_at_all(self, zero_reporter_driver):
        assert zero_reporter_driver.scale_events == ()

    async def test_the_zero_reporter_really_is_at_index_zero(
        self, zero_reporter_driver
    ):
        settled = zero_reporter_driver.ticks[-1]

        assert settled.metrics[0].channels_total == 0

    async def test_the_old_index_zero_deadband_would_have_been_cleared(
        self, zero_reporter_driver
    ):
        settled = zero_reporter_driver.ticks[-1]

        assert (
            settled.total_available
            >= scaler_settings.scaler_min_available_capacity
            + settled.metrics[0].channels_total
        )

    async def test_the_landed_widest_reporter_deadband_was_not_cleared(
        self, zero_reporter_driver
    ):
        settled = zero_reporter_driver.ticks[-1]

        assert (
            settled.total_available
            < scaler_settings.scaler_min_available_capacity
            + settled.pod_capacity_reported
        )

    async def test_the_window_cannot_be_the_reason(self, zero_reporter_driver):
        settled = zero_reporter_driver.ticks[-1]

        assert settled.windowed_available == settled.total_available


@pytest.mark.parametrize("pod_capacity", ZERO_REPORTER_ARMS, ids=ZERO_REPORTER_ARM_IDS)
class TestHealthyFleetStillScalesDown:
    """
    The positive twin for both reporter scenarios.

    Without the odd reporter, and with availability past the landed deadband,
    the same warmed fleet does give a pod back. Without this the negative
    halves prove nothing.

    It runs its OWN scenario at each reporter arm's capacity rather than
    requesting ``zero_reporter_driver``: the only thing it ever took from that
    fixture was the arm's per-pod capacity, which is the parameter itself, so
    riding it just paid for a full terminating-reporter replay per arm and
    threw the result away.
    """

    async def test_a_warmed_fleet_scales_down_when_the_deadband_is_cleared(
        self, pod_capacity, scenario  # noqa: F811
    ):
        # One clear pod-step above the deadband, on an untouched fleet.
        demand = REPORTER_FLEET_REPLICAS * pod_capacity - 2 * (
            MIN_CAPACITY + pod_capacity
        )
        healthy = scenario(
            pod_capacity=pod_capacity,
            load=[LoadPoint(0, demand)],
            initial_replicas=REPORTER_FLEET_REPLICAS,
        )

        await healthy.run_until(REPORTER_RUN_SECONDS)

        assert [event.direction for event in healthy.scale_events] == ["down"]


# ---------------------------------------------------------------------------
# 5. SR-5.3 end to end: a scaler that restarts mid-load cannot bypass the
#    guards by having forgotten them.
# ---------------------------------------------------------------------------

RESTART_REPLICAS = 5

# Load low enough that the fleet is scale-down eligible on the very first
# iteration and stays that way: this scenario is only interesting while the
# instantaneous reading is continuously begging for a deletion.
RESTART_DEMAND = 40

RESTART_TAIL_SECONDS = 2 * COLD_POLL

# The three guards a restart must not be able to bypass, all of them anchored
# at _process_start_time.
RESTART_GUARD_FIELDS = (
    "scaler_scale_down_cooldown_seconds",
    "scaler_post_scale_up_stabilization_seconds",
    "scaler_scale_down_window_seconds",
)

# One arm per guard. At the shipped defaults the cooldown dominates the other
# two outright (1800 > 600 > 300), so a single arm would only ever prove the
# cooldown and the tests would pass against a build whose other two anchors
# had been reset to the epoch. Each arm therefore switches off the guards
# above it at their documented disabling value, leaving the next one down as
# the only thing between a freshly restarted scaler and a deletion. The
# arms are named for the guard each one leaves standing.
#
# Caveat on the last arm's name. Through this seam the pods report from t0, so
# the trailing window fills from t0 too: its elapsed-time guard and the
# startup-blackout guard become indistinguishable here -- either one alone
# produces the same refusal, and "window-no-amnesia" therefore does NOT
# discriminate between them. It is a real end-to-end check that SOMETHING
# window-side survives a restart, not evidence about which. The discriminating
# coverage is unit-level, in test_autoscaler.py.
RESTART_ARMS = [
    (),
    ("scaler_scale_down_cooldown_seconds",),
    (
        "scaler_scale_down_cooldown_seconds",
        "scaler_post_scale_up_stabilization_seconds",
    ),
]
RESTART_ARM_IDS = ["scale-down-cooldown", "stabilization-anchor", "window-no-amnesia"]


def live_guard_horizon() -> float:
    """
    How long a fresh process must run before any guard can permit a deletion.

    Read off the live settings rather than the declared defaults, because the
    restart arms disable guards one at a time.

    :return: The longest process-start-anchored guard currently configured
    """
    return max(getattr(scaler_settings, field) for field in RESTART_GUARD_FIELDS)


@pytest_asyncio.fixture
async def restart_driver(
    request, mocker, scenario, scenario_settings  # noqa: F811
) -> ScenarioDriver:
    """
    A fresh Autoscaler under the harness clock, on immediately eligible load.

    The scenario factory constructs the instance itself, after the fake clock
    is installed, so ``_process_start_time`` is the scenario's t0 -- which is
    what makes "the scaler just restarted" the state under test. It is never
    the ``autoscaler`` fixture from ``test_autoscaler.py``, whose construction
    order relative to time control is not guaranteed. Nothing here reads a
    private timestamp back off the instance; every offset comes from the
    patched settings and t0.
    """
    for field in request.param:
        mocker.patch.object(scaler_settings, field, 0)

    driver = scenario(
        pod_capacity=CHANNEL_UNIT_POD_CAPACITY,
        load=[LoadPoint(0, RESTART_DEMAND)],
        initial_replicas=RESTART_REPLICAS,
    )

    await driver.run_until(live_guard_horizon())
    return driver


@pytest.mark.parametrize(
    "restart_driver", RESTART_ARMS, ids=RESTART_ARM_IDS, indirect=True
)
class TestScalerRestartMidLoad:
    async def test_no_scale_down_before_the_guard_horizon(self, restart_driver):
        assert restart_driver.scale_events == ()

    async def test_the_fleet_was_eligible_the_whole_time(self, restart_driver):
        # Non-vacuity: the pre-Epic-4 rule would have deleted a pod on the
        # first iteration after the restart, and on every one after it.
        assert all(
            record.unsmoothed_scale_down_eligible for record in restart_driver.ticks
        )

    async def test_the_fleet_stayed_above_the_floor_the_whole_time(
        self, restart_driver
    ):
        # ... and the replica floor was never the reason, either.
        assert all(record.replicas > MIN_REPLICAS for record in restart_driver.ticks)

    async def test_the_scale_down_fires_once_the_horizon_passes(self, restart_driver):
        # The positive half. Without it this whole class passes against a
        # scaler that never scales down at all. Only the FIRST event is
        # pinned: the arms that disable the cooldown legitimately keep
        # deleting on subsequent ticks until the fleet reaches the floor.
        await restart_driver.run_until(live_guard_horizon() + RESTART_TAIL_SECONDS)
        directions = [event.direction for event in restart_driver.scale_events]

        assert directions[:1] == ["down"]

    async def test_the_scale_down_waits_for_the_whole_horizon(self, restart_driver):
        horizon = live_guard_horizon()

        await restart_driver.run_until(horizon + RESTART_TAIL_SECONDS)

        assert scale_downs(restart_driver)[0].t >= horizon


# ---------------------------------------------------------------------------
# 6. SR-5.2 end to end: the fleet must not give a pod back while another one
#    is still booting.
# ---------------------------------------------------------------------------

STARTING_GATE_REPLICAS = 5

# Quiet demand: below the scale-down deadband (so nothing happens while the
# process-start cooldown runs out) and above the scale-up threshold.
STARTING_GATE_QUIET_DEMAND = (
    STARTING_GATE_REPLICAS * CHANNEL_UNIT_POD_CAPACITY - MIN_CAPACITY - 1
)

# Then a step that puts the fleet short and makes it add a pod.
STARTING_GATE_BURST_DEMAND = (
    STARTING_GATE_REPLICAS * CHANNEL_UNIT_POD_CAPACITY - MIN_CAPACITY + 1
)

# ... and, while that pod is still booting, the load collapses, stepping
# availability across the deadband in a single poll. This is the shape the
# gate exists for: a fleet whose capacity is about to jump by a full pod-step
# must not delete a pod first.
STARTING_GATE_TROUGH_DEMAND = TROUGH_STREAMS

# The burst is timed for the moment the process-start cooldown has elapsed, so
# the cooldown cannot be the reason for any refusal that follows.
STARTING_GATE_BURST_AT = DOWN_COOLDOWN
STARTING_GATE_TROUGH_AT = STARTING_GATE_BURST_AT + UP_COOLDOWN

# The booting pod's metrics endpoint answers this long after it was created,
# which is what keeps it counted as starting across the collapse.
STARTING_GATE_READY_AFTER = 30.0
STARTING_GATE_WINDOW_END = STARTING_GATE_BURST_AT + STARTING_GATE_READY_AFTER
STARTING_GATE_RUN_SECONDS = STARTING_GATE_WINDOW_END + 2 * COLD_POLL

# The twin: the pod reports the instant it becomes visible, so the fleet never
# has a starting pod at all and nothing is left to refuse the deletion.
STARTING_GATE_ARMS = [(STARTING_GATE_READY_AFTER, False), (None, True)]
STARTING_GATE_ARM_IDS = ["pod-still-starting", "no-pod-starting"]


@pytest.fixture
def undamped_settings(mocker, scenario_settings) -> None:  # noqa: F811
    """
    Leave the starting-pods gate as the only thing that can refuse.

    The trailing window and the post-scale-up stabilization window are the
    other two refusals that could produce the same "nothing happened", so both
    are switched off at their documented disabling value; the process-start
    scale-down cooldown is elapsed by simulated time instead, since it has no
    such value. Every offset in this scenario is derived from these settings.
    """
    mocker.patch.object(scaler_settings, "scaler_scale_down_window_seconds", 0)
    mocker.patch.object(
        scaler_settings, "scaler_post_scale_up_stabilization_seconds", 0
    )


@pytest_asyncio.fixture
async def starting_gate_driver(
    request, scenario, undamped_settings  # noqa: F811
) -> ScenarioDriver:
    """A fleet that adds a pod and then loses its load while it boots."""
    ready_after = request.param
    driver = scenario(
        pod_capacity=CHANNEL_UNIT_POD_CAPACITY,
        load=[
            LoadPoint(0, STARTING_GATE_QUIET_DEMAND),
            LoadPoint(STARTING_GATE_BURST_AT, STARTING_GATE_BURST_DEMAND),
            LoadPoint(STARTING_GATE_TROUGH_AT, STARTING_GATE_TROUGH_DEMAND),
        ],
        initial_replicas=STARTING_GATE_REPLICAS,
        visible_after=GATED_BOOT_VISIBLE_AFTER,
        ready_after=ready_after or GATED_BOOT_VISIBLE_AFTER,
    )

    await driver.run_until(STARTING_GATE_RUN_SECONDS)
    return driver


@pytest.mark.parametrize(
    "starting_gate_driver, scales_down",
    STARTING_GATE_ARMS,
    ids=STARTING_GATE_ARM_IDS,
    indirect=["starting_gate_driver"],
)
class TestStartingPodsScaleDownGateScenario:
    async def test_the_collapse_only_buys_a_deletion_with_no_pod_starting(
        self, starting_gate_driver, scales_down
    ):
        collapse = events_in(
            starting_gate_driver, STARTING_GATE_TROUGH_AT, STARTING_GATE_WINDOW_END
        )

        assert bool([e for e in collapse if e.direction == "down"]) is scales_down

    async def test_the_deciding_ticks_were_eligible(
        self, starting_gate_driver, scales_down
    ):
        # Non-vacuity for the gated arm and a sanity check for the twin: the
        # collapse really did step availability across the pre-Epic-4
        # threshold.
        collapse = ticks_in(
            starting_gate_driver, STARTING_GATE_TROUGH_AT, STARTING_GATE_WINDOW_END
        )

        assert collapse and any(
            record.unsmoothed_scale_down_eligible for record in collapse
        )

    async def test_the_window_was_neutralized(self, starting_gate_driver, scales_down):
        # With the trailing window disabled the decision runs on the
        # instantaneous reading, so smoothing cannot be the reason for a
        # refusal. Confirmed from the tick record rather than assumed.
        collapse = ticks_in(
            starting_gate_driver, STARTING_GATE_TROUGH_AT, STARTING_GATE_WINDOW_END
        )

        assert all(
            record.windowed_available == record.total_available for record in collapse
        )

    async def test_the_starting_pod_count_matches_the_arm(
        self, starting_gate_driver, scales_down
    ):
        collapse = ticks_in(
            starting_gate_driver, STARTING_GATE_TROUGH_AT, STARTING_GATE_WINDOW_END
        )
        eligible = [
            record for record in collapse if record.unsmoothed_scale_down_eligible
        ]

        assert all((record.starting_pods == 0) is scales_down for record in eligible)


# ---------------------------------------------------------------------------
# 7. The post-scale-up stabilization window, end to end.
#
# The replay asserts that no scale-down lands inside a stabilization window,
# but on the incident timeline the scale-down cooldown outlives the
# stabilization window every time, so that claim there is a guard rather than
# a discriminator. This scenario is built so the stabilization window is the
# ONLY guard left standing: the cooldown has already elapsed when the fleet
# grows, and the load then stays collapsed long enough for the trailing window
# to fill and start permitting the deletion the window is still refusing.
# ---------------------------------------------------------------------------

STABILIZATION_REPLICAS = 5
STABILIZATION_CAPACITY = STABILIZATION_REPLICAS * CHANNEL_UNIT_POD_CAPACITY

# Quiet demand: below the scale-down deadband so nothing happens while the
# process-start cooldown runs out, and comfortably above the scale-up
# threshold so nothing grows either.
STABILIZATION_QUIET_DEMAND = (
    STABILIZATION_CAPACITY - MIN_CAPACITY - CHANNEL_UNIT_POD_CAPACITY // 2
)

# Then the fleet goes short and adds a pod, the moment the cooldown elapses.
STABILIZATION_BURST_DEMAND = STABILIZATION_CAPACITY - MIN_CAPACITY + 1
STABILIZATION_SCALE_UP_AT = DOWN_COOLDOWN

# ... and the load collapses right after, and stays collapsed. This is the
# case the stabilization window exists for: the pod-step the scale-up just
# added ages into the trailing window and would otherwise reverse the very
# scale-up that produced it, exactly one window later.
STABILIZATION_COLLAPSE_AT = STABILIZATION_SCALE_UP_AT + UP_COOLDOWN
STABILIZATION_WINDOW_SATISFIED_AT = STABILIZATION_COLLAPSE_AT + WINDOW
STABILIZATION_ENDS_AT = STABILIZATION_SCALE_UP_AT + STABILIZATION
STABILIZATION_RUN_SECONDS = STABILIZATION_ENDS_AT + 2 * COLD_POLL


@pytest_asyncio.fixture
async def stabilization_driver(scenario) -> ScenarioDriver:  # noqa: F811
    """A fleet that grows just as its cooldown expires, then loses its load."""
    driver = scenario(
        pod_capacity=CHANNEL_UNIT_POD_CAPACITY,
        load=[
            LoadPoint(0, STABILIZATION_QUIET_DEMAND),
            LoadPoint(STABILIZATION_SCALE_UP_AT, STABILIZATION_BURST_DEMAND),
            LoadPoint(STABILIZATION_COLLAPSE_AT, TROUGH_STREAMS),
        ],
        initial_replicas=STABILIZATION_REPLICAS,
        # The new pod reports as soon as it is visible, so the starting-pods
        # gate is never what refuses: this scenario is about the window.
        visible_after=GATED_BOOT_VISIBLE_AFTER,
        ready_after=GATED_BOOT_VISIBLE_AFTER,
    )

    await driver.run_until(STABILIZATION_RUN_SECONDS)
    return driver


class TestPostScaleUpStabilizationScenario:
    async def test_the_stabilization_window_outlasts_the_trailing_window(self):
        # The dataset precondition that makes the interval below non-empty.
        assert STABILIZATION_ENDS_AT > STABILIZATION_WINDOW_SATISFIED_AT

    async def test_the_fleet_grew_when_the_load_arrived(self, stabilization_driver):
        assert [event.direction for event in stabilization_driver.scale_events][
            0
        ] == "up"

    async def test_no_scale_down_while_only_stabilization_is_left(
        self, stabilization_driver
    ):
        assert (
            events_in(
                stabilization_driver,
                STABILIZATION_WINDOW_SATISFIED_AT,
                STABILIZATION_ENDS_AT,
            )
            == []
        )

    async def test_the_trailing_window_had_already_stopped_refusing(
        self, stabilization_driver
    ):
        # Non-vacuity: on those ticks the smoothed value itself cleared the
        # deadband, so smoothing was no longer the reason -- nor was the
        # cooldown, which elapsed before the scale-up. Only the stabilization
        # window is left.
        permitted = [
            record
            for record in ticks_in(
                stabilization_driver,
                STABILIZATION_WINDOW_SATISFIED_AT,
                STABILIZATION_ENDS_AT,
            )
            if record.windowed_available is not None
            and record.windowed_available >= MIN_CAPACITY + record.pod_capacity_reported
        ]

        assert permitted

    async def test_the_scale_down_fires_once_the_window_closes(
        self, stabilization_driver
    ):
        # The positive half.
        assert [
            event.direction
            for event in events_in(
                stabilization_driver, STABILIZATION_ENDS_AT, STABILIZATION_RUN_SECONDS
            )
        ] == ["down"]


# ---------------------------------------------------------------------------
# 8. b.qwa end to end: a fresh deployment that starts below the replica floor.
#
# The dev observation, replayed. The chart omits ``spec.replicas`` so that a
# helm upgrade cannot fight the autoscaler, which leaves Kubernetes defaulting a
# fresh Deployment to ONE pod -- below the configured floor of 3. At the
# stream-unit values (720 per pod, threshold 800) an idle single-pod fleet is
# genuinely short (712 < 800) and climbs to 2 under capacity pressure, and there
# it stopped: two pods report 1432 available, which clears the 800 threshold and
# still sits inside the 1520 scale-down deadband, so under the pre-fix rule no
# branch ever acted again and the fleet rested one pod below minReplicas.
#
# The floor is enforced only as a clamp on the way DOWN, so nothing raised it.
# This scenario drives the whole trajectory through the real _scaling_iteration
# and asserts it now converges on the floor and stays there -- and, because the
# resting point is the whole point, that it stays there past every guard that
# could otherwise be the reason nothing happened.
# ---------------------------------------------------------------------------

# Kubernetes' own default for a Deployment with no spec.replicas.
FLOOR_CLIMB_INITIAL_REPLICAS = 1

# An idle fleet: the incident's own baseline concurrency, which one pod serves
# comfortably. The climb here is therefore never demand-driven past the first
# step -- exactly the case the capacity trigger cannot reach.
FLOOR_CLIMB_DEMAND = BASELINE_STREAMS

# Long enough that every process-start-anchored guard has elapsed while the
# fleet sits at the floor, so "it held" is not just "no guard had expired yet".
FLOOR_CLIMB_RUN_SECONDS = GUARD_HORIZON + 2 * COLD_POLL


def tick_at(driver: ScenarioDriver, t: float) -> TickRecord:
    """
    The tick recorded at a scenario time.

    Scale events carry the scenario time of the tick that produced them, so
    this is how a claim about an event is grounded in what the scaler actually
    saw when it made it.

    :param driver: The driver that ran the scenario
    :param t: The scenario time, in seconds since t0
    :return: The matching tick
    """
    for record in driver.ticks:
        if record.t == t:
            return record
    raise AssertionError(f"No tick recorded at t={t:.1f}s")


@pytest_asyncio.fixture
async def floor_climb_driver(mocker, scenario) -> ScenarioDriver:  # noqa: F811
    """
    A fresh single-pod deployment, idle, at the retuned stream-unit threshold.

    The per-arm ``min_available_capacity`` override is the same idiom the
    ``replay`` fixture uses: the library default (30) is a never-hit fallback,
    and the deployed value is Task 1's retuned 800. The replica floor is the
    SHIPPED default that ``scenario_settings`` already pinned.

    The new pod is visible quickly but reports on the default schedule, so the
    starting-pods gate is live throughout the climb -- as it is in production,
    and as the incident's own boot profile showed.
    """
    mocker.patch.object(
        scaler_settings, "scaler_min_available_capacity", STREAM_UNIT_MIN_CAPACITY
    )
    driver = scenario(
        pod_capacity=STREAM_UNIT_POD_CAPACITY,
        load=[LoadPoint(0, FLOOR_CLIMB_DEMAND)],
        initial_replicas=FLOOR_CLIMB_INITIAL_REPLICAS,
        visible_after=GATED_BOOT_VISIBLE_AFTER,
    )

    await driver.run_until(FLOOR_CLIMB_RUN_SECONDS)
    return driver


class TestFreshDeploymentClimbsToTheReplicaFloor:
    """
    The b.qwa trajectory: 1 -> 2 -> 3, then rest.

    The consistency the fix has to produce across the whole range is asserted
    here in the only form that proves it end to end -- a fleet that starts at 1
    and is left alone once it reaches 3, having passed through the exact state
    (2 replicas, idle) the pre-fix rule got stuck in.
    """

    async def test_the_fleet_reaches_the_floor(self, floor_climb_driver):
        assert floor_climb_driver.ticks[-1].replicas == MIN_REPLICAS

    async def test_it_climbs_one_pod_at_a_time(self, floor_climb_driver):
        assert [event.to_replicas for event in scale_ups(floor_climb_driver)] == [
            FLOOR_CLIMB_INITIAL_REPLICAS + 1,
            MIN_REPLICAS,
        ]

    async def test_it_never_hands_a_pod_back(self, floor_climb_driver):
        assert scale_downs(floor_climb_driver) == []

    async def test_the_step_off_two_was_not_bought_by_capacity_pressure(
        self, floor_climb_driver
    ):
        # Non-vacuity, first half: the deciding tick was NOT short of capacity,
        # so the capacity trigger cannot be what added the third pod. Without
        # this the scenario would pass on a fleet that simply grew under load.
        step = scale_ups(floor_climb_driver)[1]

        assert not tick_at(floor_climb_driver, step.t).below_min_capacity

    async def test_the_step_off_two_was_not_scale_down_eligible_either(
        self, floor_climb_driver
    ):
        # Non-vacuity, second half: that same tick was inside the scale-down
        # deadband too, which is precisely the state the pre-fix rule rested in
        # -- neither branch could act, so the fleet stayed one pod below the
        # floor indefinitely.
        step = scale_ups(floor_climb_driver)[1]

        assert not tick_at(floor_climb_driver, step.t).unsmoothed_scale_down_eligible

    async def test_it_holds_at_the_floor_past_every_guard(self, floor_climb_driver):
        settled = [
            record for record in floor_climb_driver.ticks if record.t >= GUARD_HORIZON
        ]

        assert settled and all(record.replicas == MIN_REPLICAS for record in settled)

    async def test_the_min_replicas_clamp_is_what_held_the_tail(
        self, floor_climb_driver
    ):
        # Non-vacuity for the tail: once the fleet is at 3 the idle load leaves
        # it comfortably past the scale-down deadband, and by then every
        # process-start-anchored guard has elapsed -- so the clamp really is
        # the only thing still refusing to shrink it.
        settled = [
            record for record in floor_climb_driver.ticks if record.t >= GUARD_HORIZON
        ]

        assert all(record.unsmoothed_scale_down_eligible for record in settled)
