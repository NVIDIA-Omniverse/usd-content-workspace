# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Helm-template pins for the graceful-shutdown DEPLOYMENT surface.

templates/deployment.yaml carries the pod-spec `terminationGracePeriodSeconds`,
a render-time invariant guard that `fail`s the template when the grace period
cannot cover the whole shutdown sequence, and a bounds check on each shutdown
value it templates. This module locks that Deployment render and both guards.

SCOPE: this file owns the DEPLOYMENT-level render (the grace period) and the
render-time GUARDS. The settings-ConfigMap rows -- including
`OMNI_EVENTS_SHUTDOWN_SETTLE_DELAY_SECONDS`,
`OMNI_EVENTS_SHUTDOWN_DRAIN_WINDOW_SECONDS` and
`OMNI_EVENTS_SHUTDOWN_DRAIN_POLL_INTERVAL_SECONDS` -- are pinned per-row by
test_drain_settings_template.py and are deliberately NOT asserted here to avoid
double-coverage.

The failure modes guarded: a dropped `terminationGracePeriodSeconds` reverts the
pod to the kubelet's 30s default, which risks a SIGKILL mid-shutdown; a grace
period that no longer covers the settle, the drain and the post-drain teardown
must be caught at `helm template` time rather than in production, which is what
the inline `fail` guard enforces; a shutdown value that is negative, non-numeric
or outside float64 range would otherwise render cleanly and CrashLoopBackOff the
pod at settings load, or -- for the grace period, which the guards read through
sprig's float64 -- be misreported as a coverage shortfall against the wrong knob
(TestShutdownValueBounds), while that same guard grown too
strict would refuse the documented 0 that skips the settle or the drain phase
(TestShutdownValueBoundsAccepts); and a container `lifecycle`
block must stay absent, because the native `sleep` preStop handler this chart
once carried is rejected by the API server below Kubernetes 1.30 and this is a
self-hosted product (TestNoContainerLifecycle).
"""

from typing import NamedTuple

import pytest

from tests.helm.conftest import render, render_raw, requires_helm, single_document

DEPLOYMENT_TEMPLATE = "templates/deployment.yaml"

# The landed values.yaml shutdown.* defaults, read verbatim, not assumed.
DEFAULT_GRACE_SECONDS = 60
DEFAULT_SETTLE_SECONDS = 15
DEFAULT_DRAIN_WINDOW_SECONDS = 5

VALUES_GRACE = "shutdown.terminationGracePeriodSeconds"
VALUES_SETTLE = "shutdown.settleDelaySeconds"
VALUES_DRAIN_WINDOW = "shutdown.drainWindowSeconds"
VALUES_POLL_INTERVAL = "shutdown.drainPollIntervalSeconds"

# The fixed post-drain teardown term the template reserves on top of the two
# knobs, mirroring deployment.yaml's `$teardownTailSeconds` constant. It covers
# the gRPC server's stop grace and the REST lifespan shutdown, which run
# concurrently once the drain ends. It is a property of the library's shutdown
# implementation rather than a chart knob, so it is duplicated here rather than
# read from values.yaml -- and this duplication is deliberate: the template
# restates the number in prose it cannot interpolate, so
# test_trip_message_names_the_teardown_tail below is the one surface that fails
# loudly when the template's constant is re-derived and the prose is not.
TEARDOWN_TAIL_SECONDS = 10

# The three knobs the coverage guard's failure message must name. All three are
# coupled, so naming fewer would leave the operator guessing which to change.
INVARIANT_KEYS = [VALUES_GRACE, VALUES_SETTLE, VALUES_DRAIN_WINDOW]

# An override distinct from every default above so a hardcoded literal or a
# cross-wired values key cannot pass by coincidence. It keeps the invariant
# satisfied so the render still succeeds: 90 >= 15 + 5 + 10.
OVERRIDE_GRACE_SECONDS = 90

# The exact invariant boundary at the shipped defaults, derived from the three
# terms rather than written as a literal, so the arithmetic under test is stated
# once. The guard is a strict `lt`, so equality renders.
BOUNDARY_GRACE_SECONDS = (
    DEFAULT_SETTLE_SECONDS + DEFAULT_DRAIN_WINDOW_SECONDS + TEARDOWN_TAIL_SECONDS
)

# The shortfall override the guard tests trip the render with: 29 < 30. It is
# one second under the boundary, which is also what proves every term is really
# summed -- a guard that dropped the settle or the teardown tail would compute a
# floor of 15 or 20 and let this render through.
SHORTFALL_GRACE_SECONDS = BOUNDARY_GRACE_SECONDS - 1

# The drain window that turns the boundary grace period into a shortfall by a
# fraction of a second: 30 < 15 + 5.5 + 10.
SHORTFALL_DRAIN_WINDOW_SECONDS = 5.5

# A fractional grace period. It clears the coverage invariant (60.5 >= 30), so
# only the whole-number guard can reject it -- the pod-spec field is an int64
# and would otherwise render here and be refused by the API server at apply
# time.
FRACTIONAL_GRACE_SECONDS = 60.5

# The substring of the whole-number guard's message that states the
# requirement, matched loosely so a reworded message does not false-fail.
WHOLE_NUMBER_MESSAGE = "must be a whole number"

# The substring of the coverage guard's message that states the teardown tail.
# Asserted on the number, not on the whole sentence, so a reworded message does
# not false-fail while a re-derived constant still does.
TEARDOWN_TAIL_MESSAGE = f"{TEARDOWN_TAIL_SECONDS}s post-drain"

# A fractional drain window, paired below with a whole grace period: the
# whole-number check is deliberately on the grace period alone, because the
# shutdown.* knobs are floats and a fractional value is legitimate.
FRACTIONAL_DRAIN_WINDOW_SECONDS = 5.5


class BoundsCase(NamedTuple):
    """One out-of-bounds shutdown value and the message it must be refused with."""

    id: str
    values_key: str
    value: str
    message: str


# The bounds loop's accepted ranges mirror notification_common CommonSettings
# for the three float knobs: the settle and the drain window take any value
# >= 0, while the poll interval is a cadence and must be > 0. The grace period
# is the loop's fourth key on the >= 0 band, and it is in the loop for a
# different reason -- see the grace row below. These rows cover both bands, both
# rejection shapes the pattern has to catch (a sign and a non-number), and the
# float64 range check that a purely textual pattern would let through --
# "1e400" is numeric text that sprig's float64 converts to 0, which would
# otherwise satisfy the coverage invariant here and reach pydantic as `inf`.
BOUNDS_CASES = [
    BoundsCase(
        "negative-settle",
        VALUES_SETTLE,
        "-1",
        f"{VALUES_SETTLE} must be a number >= 0",
    ),
    BoundsCase(
        "non-numeric-drain-window",
        VALUES_DRAIN_WINDOW,
        "abc",
        f"{VALUES_DRAIN_WINDOW} must be a number >= 0",
    ),
    BoundsCase(
        "zero-poll-interval",
        VALUES_POLL_INTERVAL,
        "0",
        f"{VALUES_POLL_INTERVAL} must be a number > 0",
    ),
    # The grace period is a pod field the API server does validate, so unlike
    # the three knobs above it cannot CrashLoopBackOff the pod. What it can do
    # is reach the guards below through sprig's float64, which yields 0 for any
    # text it cannot parse: without the loop entry this same "abc" is reported
    # as a coverage shortfall against the settle-plus-drain-plus-teardown floor,
    # which sends the operator to the wrong knob. Asserting the message -- not
    # just the non-zero exit, which the coverage guard also produces -- is what
    # pins the loop as the surface that refuses it. The row deliberately sets no
    # other override: here the grace period IS the value under test.
    BoundsCase(
        "non-numeric-grace",
        VALUES_GRACE,
        "abc",
        f"{VALUES_GRACE} must be a number >= 0",
    ),
    BoundsCase(
        "non-finite-settle",
        VALUES_SETTLE,
        "1e400",
        f"{VALUES_SETTLE} must be a finite number",
    ),
]

BOUNDS_IDS = [case.id for case in BOUNDS_CASES]


class AcceptedCase(NamedTuple):
    """One in-bounds shutdown value the bounds loop must let through."""

    id: str
    values_key: str
    value: str


# The accepted side of the same loop, which BOUNDS_CASES above never reaches. 0
# is the documented "skip this phase" setting for both the settle and the drain
# window, so refusing it would break a supported configuration at render time.
# Two independent dimensions are covered:
#
#   * the per-key `$exclusive` flag. The settle and the drain window are >= 0
#     bands, so each needs its own row at 0 -- flipping either flag to the poll
#     interval's > 0 band would otherwise pass unnoticed.
#   * the template's zero-spelling test, which is one regex shared by every key
#     in the loop, so the spellings need not be repeated per key. "0" covers its
#     leading-digit branch, "0.0" the trailing-zeros branch and ".0" the
#     bare-decimal branch. Breaking any of them routes a legitimate zero into
#     the non-finite branch -- numeric, not spelling zero, converting to 0.0 --
#     and refuses it as out of float64 range.
#
# "+0" is deliberately not a row: `helm --set` normalizes the sign away before
# the template sees the value, so it renders as the "0" row already here.
# Confirmed by rendering, as were the three spellings below.
ACCEPTED_CASES = [
    AcceptedCase("settle-zero", VALUES_SETTLE, "0"),
    AcceptedCase("settle-trailing-zero", VALUES_SETTLE, "0.0"),
    AcceptedCase("drain-window-zero", VALUES_DRAIN_WINDOW, "0"),
    AcceptedCase("drain-window-bare-decimal-zero", VALUES_DRAIN_WINDOW, ".0"),
]

ACCEPTED_IDS = [case.id for case in ACCEPTED_CASES]

pytestmark = requires_helm


def _deployment(*extra_set: str) -> dict:
    out = render(*extra_set, show_only=DEPLOYMENT_TEMPLATE)
    return single_document(out, "Deployment")


def _pod_spec(*extra_set: str) -> dict:
    return _deployment(*extra_set)["spec"]["template"]["spec"]


@pytest.fixture(scope="module")
def default_pod_spec() -> dict:
    """The default-render pod spec. Read-only; shared to avoid re-rendering."""
    return _pod_spec()


@pytest.fixture(scope="module")
def default_container(default_pod_spec) -> dict:
    """The default-render container. Read-only; shared to avoid re-rendering."""
    containers = default_pod_spec["containers"]
    assert len(containers) == 1, f"expected exactly one container: {containers}"
    return containers[0]


@pytest.fixture(scope="module")
def grace_shortfall_result():
    """One shared render of the tripped guard, unchecked.

    The guard-trip tests below all assert on the same `grace=29` render -- one on
    its exit status, one (parametrized per invariant key) on its stderr, one on
    the teardown tail it names -- so the `helm template` subprocess runs once for
    the whole group.
    """
    return render_raw(
        f"{VALUES_GRACE}={SHORTFALL_GRACE_SECONDS}", show_only=DEPLOYMENT_TEMPLATE
    )


@pytest.fixture(scope="module")
def fractional_grace_result():
    """One shared render of the tripped whole-number guard, unchecked.

    Three tests below assert on this same `grace=60.5` render -- its exit
    status, and two properties of its stderr -- so the `helm template`
    subprocess runs once for the group.
    """
    return render_raw(
        f"{VALUES_GRACE}={FRACTIONAL_GRACE_SECONDS}", show_only=DEPLOYMENT_TEMPLATE
    )


@pytest.fixture(scope="module")
def bounds_results() -> dict:
    """One unchecked render per out-of-bounds row, keyed by case id.

    Both bounds classes below assert on the same renders -- one on the exit
    status, one on the message -- so each `helm template` subprocess runs once.
    """
    return {
        case.id: render_raw(
            f"{case.values_key}={case.value}", show_only=DEPLOYMENT_TEMPLATE
        )
        for case in BOUNDS_CASES
    }


@pytest.fixture(scope="module")
def accepted_results() -> dict:
    """One unchecked render per in-bounds row, keyed by case id.

    Mirrors `bounds_results` above for the loop's accepted side. Every row
    overrides the grace period well clear of its floor, so a non-zero exit here
    can only come from the bounds loop and never from the coverage invariant.
    """
    return {
        case.id: render_raw(
            f"{case.values_key}={case.value}",
            f"{VALUES_GRACE}={OVERRIDE_GRACE_SECONDS}",
            show_only=DEPLOYMENT_TEMPLATE,
        )
        for case in ACCEPTED_CASES
    }


class TestTerminationGracePeriod:
    """The pod spec carries terminationGracePeriodSeconds at its chart default.

    Absence is the primary bug: without the key the pod silently reverts to the
    kubelet's 30s default, which leaves the whole shutdown sequence -- the
    settle, the drain, and the post-drain teardown -- racing the SIGKILL
    deadline. Both servers stop together at the end of the drain, so the REST
    teardown is part of that post-drain tail rather than running concurrently
    with the drain as it did before the pin moved to notification-common 6.12.0.
    """

    def test_present_in_default_render(self, default_pod_spec):
        assert "terminationGracePeriodSeconds" in default_pod_spec

    def test_default_matches_recorded_default(self, default_pod_spec):
        assert (
            default_pod_spec["terminationGracePeriodSeconds"] == DEFAULT_GRACE_SECONDS
        )

    def test_follows_its_values_key(self):
        # Proves the number follows shutdown.terminationGracePeriodSeconds
        # rather than a hardcoded literal that merely matches the default.
        spec = _pod_spec(f"{VALUES_GRACE}={OVERRIDE_GRACE_SECONDS}")

        assert spec["terminationGracePeriodSeconds"] == OVERRIDE_GRACE_SECONDS


class TestNoContainerLifecycle:
    """The container carries no `lifecycle` block at all.

    An earlier revision of this chart shipped a native `lifecycle.preStop.sleep`
    hook. The API server rejects that handler below Kubernetes 1.30 and this is
    a self-hosted product, so the hook was removed deliberately rather than
    version-gated; the app-side shutdown.settleDelaySeconds now covers the same
    de-routing lag from inside the process. This pins the removal: a
    reintroduced preStop would otherwise come back silently, break installs on
    older clusters, and stack its delay on top of the settle inside one grace
    period.
    """

    def test_default_render_container_has_no_lifecycle_key(self, default_container):
        assert "lifecycle" not in default_container


class TestInvariantGuardTrips:
    """The inline render-time `fail` guards reject a grace period that cannot
    cover the settle, the drain and the post-drain teardown, or that is not a
    whole number of seconds, turning a production SIGKILL-mid-shutdown or an
    apply-time API-server rejection into a template-time error."""

    def test_trips_when_grace_below_the_invariant(self, grace_shortfall_result):
        # 29 < 15 + 5 + 10: the guard must fail the render non-zero.
        assert grace_shortfall_result.returncode != 0

    @pytest.mark.parametrize("key", INVARIANT_KEYS)
    def test_trip_message_names_the_invariant(self, grace_shortfall_result, key):
        # The three knobs are coupled, so the render error must name all of
        # them to be actionable for the operator.
        assert key in grace_shortfall_result.stderr

    def test_trip_message_names_the_teardown_tail(self, grace_shortfall_result):
        # The tail is a template constant with no values key, so naming it in
        # the message is the only way an operator can reconstruct the floor --
        # and this assertion is what fails loudly if the constant is
        # re-derived without the prose around it being updated.
        assert TEARDOWN_TAIL_MESSAGE in grace_shortfall_result.stderr

    def test_trips_on_float_boundary_shortfall(self):
        # Float arithmetic, not truncated ints: 30 < 15 + 5.5 + 10. A guard
        # that compared as ints would false-pass this (30 >= 30).
        result = render_raw(
            f"{VALUES_GRACE}={BOUNDARY_GRACE_SECONDS}",
            f"{VALUES_DRAIN_WINDOW}={SHORTFALL_DRAIN_WINDOW_SECONDS}",
            show_only=DEPLOYMENT_TEMPLATE,
        )

        assert result.returncode != 0

    def test_trips_on_a_fractional_grace_period(self, fractional_grace_result):
        # 60.5 clears the coverage invariant, so this can only be the
        # whole-number guard: the pod-spec field is an int64.
        assert fractional_grace_result.returncode != 0

    def test_fractional_trip_message_names_the_grace_key(self, fractional_grace_result):
        assert VALUES_GRACE in fractional_grace_result.stderr

    def test_fractional_trip_message_states_the_whole_number_requirement(
        self, fractional_grace_result
    ):
        assert WHOLE_NUMBER_MESSAGE in fractional_grace_result.stderr


class TestShutdownValueBounds:
    """Each shutdown value is range-checked at render time.

    The three shutdown.* floats reach the app as ConfigMap strings rather than
    as API-server-validated pod fields, so without this guard a negative,
    non-numeric or out-of-range value renders cleanly and is only rejected by
    the library's field validation at process start -- a CrashLoopBackOff and a
    stalled rollout instead of an immediate template error. A negative settle
    would additionally shrink the grace-period floor the invariant guard
    computes, so the two guards are not independent. The grace period is in the
    loop for the converse reason: it is a validated pod field, but an unparsable
    one reaches the guards below as 0 and is blamed on the settle and the drain
    unless the loop names it first.
    """

    @pytest.mark.parametrize("case", BOUNDS_CASES, ids=BOUNDS_IDS)
    def test_render_fails(self, bounds_results, case):
        assert bounds_results[case.id].returncode != 0

    @pytest.mark.parametrize("case", BOUNDS_CASES, ids=BOUNDS_IDS)
    def test_message_names_the_key_and_its_bound(self, bounds_results, case):
        # Naming the key alone is not enough: the settle and the drain window
        # accept 0 while the poll interval does not, so the message has to say
        # which band was violated for the operator to know what to set.
        assert case.message in bounds_results[case.id].stderr


class TestShutdownValueBoundsAccepts:
    """What the bounds loop must NOT reject.

    TestShutdownValueBounds above pins only the refused side, which leaves the
    opposite failure -- a guard that is too strict -- entirely silent. 0 is the
    documented setting for skipping the settle or the drain phase, stated in
    both values.yaml and the template's own prose, so a band narrowed to > 0 or
    a broken zero-spelling test would refuse a supported configuration at
    render time and strand the operator with no way to switch a phase off.
    """

    @pytest.mark.parametrize("case", ACCEPTED_CASES, ids=ACCEPTED_IDS)
    def test_render_succeeds(self, accepted_results, case):
        result = accepted_results[case.id]

        assert result.returncode == 0, result.stderr


class TestInvariantGuardPasses:
    """What the guards must NOT reject.

    The coverage guard is a strict `lt`, so equality passes: at the exact
    boundary the render succeeds and emits the grace period. The whole-number
    guard is scoped to the grace period alone, so a fractional drain window
    still renders.
    """

    def test_passes_at_exact_boundary(self):
        # 30 == 15 + 5 + 10: equality must render, not trip.
        spec = _pod_spec(f"{VALUES_GRACE}={BOUNDARY_GRACE_SECONDS}")

        assert spec["terminationGracePeriodSeconds"] == BOUNDARY_GRACE_SECONDS

    def test_passes_on_a_fractional_drain_window(self):
        # The whole-number check is on the grace period only: a fractional
        # drain window under a whole grace period must still render, or the
        # float knob would be silently narrowed to integers.
        spec = _pod_spec(
            f"{VALUES_DRAIN_WINDOW}={FRACTIONAL_DRAIN_WINDOW_SECONDS}",
            f"{VALUES_GRACE}={DEFAULT_GRACE_SECONDS}",
        )

        assert spec["terminationGracePeriodSeconds"] == DEFAULT_GRACE_SECONDS
