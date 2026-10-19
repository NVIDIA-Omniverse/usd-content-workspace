# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Helm-template pins for the graceful-drain DEPLOYMENT surface (SR-6).

The endpoint-settle delay is applied by the APPLICATION on SIGTERM (the
`OMNI_EVENTS_SHUTDOWN_SETTLE_DELAY_SECONDS` setting), not by a Kubernetes
`lifecycle.preStop.sleep` handler. The Deployment therefore carries no
`lifecycle` block at all: the container takes SIGTERM immediately on pod
deletion and spends the settle, the drain and the server teardown itself. What
the Deployment does carry is the pod-spec `terminationGracePeriodSeconds` and
three render-time guards, and this module locks that render and those guards.

The three guards, in the order deployment.yaml evaluates them:

1. The removed `shutdown.preStopDelaySeconds` key `fail`s the render. An
   operator who still sets it would otherwise get the settle default silently
   layered on top of a value that no longer does anything.
2. Each `shutdown.*Seconds` value is checked for numeric text, then for
   finiteness, then bounds-checked (`>= 0` for the settle and the drain window,
   `> 0` for the poll interval). These reach the app as ConfigMap strings rather
   than as API-server-validated pod fields, so without this a bad value renders
   cleanly and then CrashLoopBackOffs at process start. The finiteness check
   sits between the other two because a magnitude outside float64's range is
   numeric text that converts to 0: it would clear the text test, satisfy the
   bound and the invariant below as 0, and reach pydantic as `inf`, which does
   NOT crash the pod -- it settles until SIGKILL on every shutdown.
3. The coupled-knob invariant: the grace period must cover
   settleDelaySeconds + drainWindowSeconds + the fixed post-drain teardown tail,
   because the three run in that order inside one grace budget.

SCOPE: this file owns the DEPLOYMENT-level render (grace period, the absence of
a lifecycle hook) and all three render-time guards. The settings-ConfigMap
shutdown rows (`OMNI_EVENTS_SHUTDOWN_SETTLE_DELAY_SECONDS`,
`..._DRAIN_WINDOW_SECONDS`, `..._DRAIN_POLL_INTERVAL_SECONDS`) are pinned
per-row by test_drain_settings_template.py; the ConfigMap assertions here are
deliberately light/structural (presence only) to avoid double-coverage.

The failure modes guarded: a dropped `terminationGracePeriodSeconds` reverts the
pod to the kubelet's 30s default, which no longer covers settle + drain +
teardown and so SIGKILLs the pod mid-shutdown; a resurrected preStop hook
reintroduces the `PodLifecycleSleepAction` feature-gate dependency that made the
chart un-installable on a default-gated k8s 1.29 API server; and a mis-set or
mistyped shutdown value must be caught at `helm template` time rather than in
production, which is what the inline `fail` guards enforce.
"""

import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"

DEPLOYMENT_TEMPLATE = "templates/deployment.yaml"
SETTINGS_CONFIGMAP_TEMPLATE = "templates/settings-configmap.yaml"

# The recorded chart defaults (values.yaml shutdown.*), read verbatim, not
# assumed: grace 60s, settle 15s, drain window 30s.
DEFAULT_GRACE_SECONDS = 60
DEFAULT_SETTLE_SECONDS = 15
DEFAULT_DRAIN_WINDOW_SECONDS = 30

# The post-drain server-teardown tail. A template-local constant in
# deployment.yaml rather than a value, because it is a fixed property of the
# application's shutdown implementation (the gRPC stop grace and the REST
# lifespan shutdown sleep, which run concurrently once the drain ends) and not
# something an operator tunes.
TEARDOWN_TAIL_SECONDS = 10

# The invariant's floor at the shipped defaults: 15 + 30 + 10 = 55, so the
# shipped 60s grace period holds it with 5s of headroom.
DEFAULT_INVARIANT_FLOOR_SECONDS = (
    DEFAULT_SETTLE_SECONDS + DEFAULT_DRAIN_WINDOW_SECONDS + TEARDOWN_TAIL_SECONDS
)

VALUES_GRACE = "shutdown.terminationGracePeriodSeconds"
VALUES_SETTLE = "shutdown.settleDelaySeconds"
VALUES_DRAIN_WINDOW = "shutdown.drainWindowSeconds"
VALUES_DRAIN_POLL_INTERVAL = "shutdown.drainPollIntervalSeconds"

# Removed by Issue b.wj6, superseded by VALUES_SETTLE. Named here because the
# guard that rejects it is part of the contract, not because the key still
# exists.
REMOVED_VALUES_PRESTOP = "shutdown.preStopDelaySeconds"

# Distinct from every default above (and from each other) so a hardcoded literal
# or a cross-wired values key cannot pass by coincidence. Both keep the
# invariant satisfied so the render still succeeds: 90 >= 15 + 30 + 10 and
# 90 >= 25 + 30 + 10.
OVERRIDE_GRACE_SECONDS = 90
OVERRIDE_SETTLE_SECONDS = 25

# A grace period far below the invariant's floor (10 < 15 + 30 + 10), used to
# trip the coupled-knob guard well clear of any boundary case.
UNDERSIZED_GRACE_SECONDS = 10

# (values key, the bound the template enforces). The accepted ranges mirror
# notification_common's CommonSettings: the settle and the drain window take any
# value >= 0 (0 skips that phase), while the poll interval is a cadence and must
# be strictly positive. That zero asymmetry is pinned by the two dedicated
# classes below rather than as a column here.
BOUNDED_VALUES = [
    (VALUES_SETTLE, ">= 0"),
    (VALUES_DRAIN_WINDOW, ">= 0"),
    (VALUES_DRAIN_POLL_INTERVAL, "> 0"),
]

BOUNDED_VALUE_IDS = [key for key, _ in BOUNDED_VALUES]

# A value the app would accept as a duration but the template must not: a
# negative shrinks the invariant's floor instead of tripping it, and pydantic
# rejects it at startup.
NEGATIVE_VALUE = "-0.5"

# Typo shapes that reach the template as strings and would otherwise render into
# the ConfigMap unchallenged: a word, a Go/k8s-style unit suffix, and an empty
# value from a bare `--set key=`.
NON_NUMERIC_VALUES = ["abc", "15s", ""]

# Magnitudes outside float64's range. They are numeric TEXT -- the guard's
# pattern accepts exponent notation, because `1e-3` is a legitimate cadence --
# but sprig's `float64` swallows the range error these produce and yields 0. So
# they would pass the text test, then satisfy the grace invariant as 0, and the
# ConfigMap would carry the text on to pydantic, which reads `1e400` as `inf`
# and settles until SIGKILL. Numeric text that does not spell zero yet converts
# to 0 is exactly that overflow (or underflow) signature.
NON_FINITE_VALUES = ["1e400", "1e-400"]

# Further spellings of an out-of-range magnitude: an explicit sign, a capital
# exponent marker, and an exponent applied to a fractional mantissa. Each is a
# distinct branch of the numeric-text pattern, so each has to reach the finite
# check rather than being waved through by a narrower match.
NON_FINITE_SPELLINGS = ["+1e400", "1E400", "0.1e400"]

# The largest magnitude that is still IN float64's range. It converts to itself
# rather than to 0, so it must clear the finite check and be judged by the
# coupled-knob invariant instead -- the check is about non-finiteness, not size.
LARGE_FINITE_VALUE = "1e308"

# Zero written in exponent form: numeric text that does not spell "0" as a bare
# digit yet legitimately converts to 0. It is the case the finite check must NOT
# intercept, since it is a real off-switch rather than an overflow.
EXPONENT_ZERO = "0e5"

# Decimal spellings `--set` delivers as strings rather than numbers. All are
# legitimate durations, so the guard must not reject them as it does a typo.
# `1e-3` is the exponent form that carries its weight twice over: it also pins
# that the finite check reads the CONVERTED value (0.001, not 0) rather than
# rejecting exponent notation wholesale.
ACCEPTED_DECIMAL_FORMS = ["0.5", ".25", "1e-3", "+0.5"]

ENV_SETTLE_DELAY = "OMNI_EVENTS_SHUTDOWN_SETTLE_DELAY_SECONDS"
ENV_DRAIN_WINDOW = "OMNI_EVENTS_SHUTDOWN_DRAIN_WINDOW_SECONDS"
ENV_DRAIN_POLL_INTERVAL = "OMNI_EVENTS_SHUTDOWN_DRAIN_POLL_INTERVAL_SECONDS"

pytestmark = pytest.mark.skipif(
    shutil.which("helm") is None,
    reason="helm CLI not available",
)


def _render_raw(
    *extra_set: str,
    show_only: str | None = None,
    set_json: tuple[str, ...] = (),
):
    """Run `helm template` and return the CompletedProcess unchecked.

    Used by the guard tests, which assert on a NON-zero exit; the checked
    `_render` wrapper below is for the success-path renders.

    `set_json` overrides go through `--set-json`, which delivers a real JSON
    number where `--set` would deliver a string. That is the shape a values
    FILE produces, and the bounds guard reads the value through `printf "%v"`,
    so the two shapes are worth exercising separately.
    """
    args = ["helm", "template", "ec", str(CHART_PATH), "--namespace", "storage-api"]
    if show_only is not None:
        args.extend(["--show-only", show_only])
    # Before the caller's own overrides: helm takes the last --set, so an
    # explicit policyHook setting in extra_set still wins.
    disable_policy_hook(args)
    select_tokenless_auth(args)
    for kv in extra_set:
        args.extend(["--set", kv])
    for kv in set_json:
        args.extend(["--set-json", kv])
    return subprocess.run(args, capture_output=True, text=True)


def _render(
    *extra_set: str,
    show_only: str | None = None,
    set_json: tuple[str, ...] = (),
) -> str:
    result = _render_raw(*extra_set, show_only=show_only, set_json=set_json)
    assert result.returncode == 0, (
        f"helm template failed:\nstdout:\n{result.stdout}\n" f"stderr:\n{result.stderr}"
    )
    return result.stdout


def _deployment(*extra_set: str, set_json: tuple[str, ...] = ()) -> dict:
    out = _render(*extra_set, show_only=DEPLOYMENT_TEMPLATE, set_json=set_json)
    docs = [d for d in yaml.safe_load_all(out) if d]
    assert len(docs) == 1, f"expected exactly one document, got {len(docs)}:\n{out}"
    assert docs[0]["kind"] == "Deployment", f"unexpected kind:\n{out}"
    return docs[0]


def _pod_spec(*extra_set: str, set_json: tuple[str, ...] = ()) -> dict:
    return _deployment(*extra_set, set_json=set_json)["spec"]["template"]["spec"]


def _settings_data(*extra_set: str, set_json: tuple[str, ...] = ()) -> dict:
    """Return the settings ConfigMap's `data` mapping from a scoped render."""
    out = _render(*extra_set, show_only=SETTINGS_CONFIGMAP_TEMPLATE, set_json=set_json)
    docs = [d for d in yaml.safe_load_all(out) if d]
    assert len(docs) == 1, f"expected exactly one document, got {len(docs)}:\n{out}"
    assert docs[0]["kind"] == "ConfigMap", f"unexpected kind:\n{out}"
    return docs[0]["data"]


def _bounds_message(values_key: str, bound: str, value: str) -> str:
    """The exact `fail` text the bounds guard emits for a rejected value."""
    return f"{values_key} must be a number {bound} (got {value})"


def _finite_message(values_key: str, value: str) -> str:
    """The exact `fail` text the finite check emits for an out-of-range value.

    Deliberately distinct from `_bounds_message`: an operator who wrote a
    magnitude no float64 can hold needs to be told that, not that their value
    is out of bounds, which would send them looking at the wrong end of it.
    """
    return f"{values_key} must be a finite number (got {value})"


@pytest.fixture(scope="module")
def default_pod_spec() -> dict:
    """The default-render pod spec. Read-only; shared to avoid re-rendering."""
    return _pod_spec()


@pytest.fixture(scope="module")
def default_container(default_pod_spec) -> dict:
    """The default-render server container. Read-only; shared."""
    containers = default_pod_spec["containers"]
    assert len(containers) == 1, f"expected exactly one container: {containers}"
    return containers[0]


@pytest.fixture(scope="module")
def default_deployment_text() -> str:
    """The default render of deployment.yaml as raw text. Read-only; shared."""
    return _render(show_only=DEPLOYMENT_TEMPLATE)


@pytest.fixture(scope="module")
def removed_prestop_render() -> subprocess.CompletedProcess:
    """The FAILING render of a chart that still sets the removed preStop key.

    Read-only; shared, because the exit code and each fragment of the `fail`
    message are asserted separately and a `helm template` subprocess is the
    dominant cost of this file.
    """
    return _render_raw(
        f"{REMOVED_VALUES_PRESTOP}={DEFAULT_SETTLE_SECONDS}",
        show_only=DEPLOYMENT_TEMPLATE,
    )


@pytest.fixture(scope="module")
def undersized_grace_render() -> subprocess.CompletedProcess:
    """The FAILING render of a grace period far below the invariant's floor.

    10 < 15 + 30 + 10. Read-only; shared across the exit-code and message
    assertions on the coupled-knob guard.
    """
    return _render_raw(
        f"{VALUES_GRACE}={UNDERSIZED_GRACE_SECONDS}", show_only=DEPLOYMENT_TEMPLATE
    )


class TestTerminationGracePeriod:
    """The pod spec carries terminationGracePeriodSeconds at its chart default.

    Absence is the primary bug: without the key the pod silently reverts to the
    kubelet's 30s default, which is shorter than settle(15) + drain(30) +
    teardown(10) and so SIGKILLs the pod mid-shutdown.
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


class TestNoPreStopLifecycleHook:
    """The container carries no lifecycle hook: SIGTERM arrives immediately.

    The settle it used to buy is now spent by the application after SIGTERM, so
    the hook is not merely unused -- it must be ABSENT. Kubernetes gates the
    native `sleep` handler behind `PodLifecycleSleepAction`, which is alpha and
    off by default in 1.29; on such an API server `dropDisabledFields` strips
    the sleep and validation then rejects the pod for having a preStop with no
    handler type, so a resurrected hook does not degrade gracefully -- it makes
    the chart un-installable there. The chart declares no `kubeVersion` floor.

    What the app-side settle is FOR is unchanged, and is worth restating because
    it is the reason this delay is not the knob to reach for on a rolling-upgrade
    503 (the distinction Issue b.7mz corrected). The budget is for the
    readiness-gated MAIN Service (service.yaml) and its direct in-cluster
    clients: that endpoint's ready flag flips false the moment the pod goes
    Terminating, and the change has to reach every watcher before the app stops
    serving. It does NOT cover the HTTPProxy path. Those routes target the
    `-envoy` Service, which sets publishNotReadyAddresses: true, so the pod stays
    in Envoy's load-balancer set until the pod OBJECT is deleted -- after this
    container exits. A longer settle postpones the app stop and the deletion by
    the same amount and closes nothing there; the remedy for that residual window
    is the catch-all route's httpProxy.unary.retryPolicy, pinned in
    tests/helm/test_http_proxy_streaming_template.py.
    """

    def test_container_has_no_lifecycle_block(self, default_container):
        assert "lifecycle" not in default_container

    def test_render_names_no_prestop_handler_anywhere(self, default_deployment_text):
        # Structural absence on the one container is not quite enough: a hook
        # reintroduced on a sidecar, or under an `initContainers` entry, would
        # slip past the assertion above and still carry the feature-gate
        # dependency. The rendered text sees any of those.
        assert "preStop" not in default_deployment_text


class TestRemovedPreStopKeyIsRejected:
    """Setting the removed `shutdown.preStopDelaySeconds` fails the render.

    Silently ignoring it is the trap: an operator upgrading from a chart where
    that key WAS the settle would get `settleDelaySeconds`' own default layered
    on top of whatever they had tuned, with no signal that their setting stopped
    doing anything. The guard turns that into a template-time error naming the
    replacement.
    """

    def test_render_fails_when_the_removed_key_is_set(self, removed_prestop_render):
        assert removed_prestop_render.returncode != 0

    def test_render_fails_when_the_removed_key_is_set_to_zero(self):
        # Zero is the case a truthiness test would wave through, and it is a
        # meaningful setting on the old key (no preStop sleep at all), so an
        # operator really can be holding it.
        result = _render_raw(
            f"{REMOVED_VALUES_PRESTOP}=0", show_only=DEPLOYMENT_TEMPLATE
        )

        assert result.returncode != 0

    def test_failure_names_the_removed_key(self, removed_prestop_render):
        assert REMOVED_VALUES_PRESTOP in removed_prestop_render.stderr

    def test_failure_names_the_replacement_key(self, removed_prestop_render):
        assert VALUES_SETTLE in removed_prestop_render.stderr

    def test_failure_states_the_new_grace_bound(self, removed_prestop_render):
        # The other half of the migration: anyone who had tuned the grace period
        # to the old two-term bound has to re-check it against the new one, so
        # the message carries the new bound rather than just the rename.
        assert (
            f"settleDelaySeconds + drainWindowSeconds + "
            f"{TEARDOWN_TAIL_SECONDS}s post-drain teardown"
            in removed_prestop_render.stderr
        )

    def test_an_explicit_null_renders_normally(self):
        # `--set key=null` is how an operator (or an umbrella chart) blanks an
        # inherited value, and it leaves the key unset rather than set. The
        # guard tests for a set value, not for the key's presence in the merged
        # values, so this must render.
        spec = _pod_spec(f"{REMOVED_VALUES_PRESTOP}=null")

        assert spec["terminationGracePeriodSeconds"] == DEFAULT_GRACE_SECONDS


class TestShutdownValueBoundsGuardRejectsNegatives:
    """A negative value for any bounded shutdown knob fails the render.

    A negative is worse than merely invalid: it SHRINKS the invariant's floor,
    so a negative settle would let a too-short grace period sail through the
    coupled-knob guard below and only surface as a pydantic error at process
    start -- a CrashLoopBackOff and a stalled rollout.
    """

    @pytest.mark.parametrize("values_key, bound", BOUNDED_VALUES, ids=BOUNDED_VALUE_IDS)
    def test_render_fails(self, values_key, bound):
        result = _render_raw(
            f"{values_key}={NEGATIVE_VALUE}", show_only=DEPLOYMENT_TEMPLATE
        )

        assert result.returncode != 0

    @pytest.mark.parametrize("values_key, bound", BOUNDED_VALUES, ids=BOUNDED_VALUE_IDS)
    def test_message_names_the_key_and_its_bound(self, values_key, bound):
        # Asserted on the message, not just the exit code, because the coupled
        # -knob guard also fails this render's neighbours -- only the text
        # proves the render failed for the reason under test.
        result = _render_raw(
            f"{values_key}={NEGATIVE_VALUE}", show_only=DEPLOYMENT_TEMPLATE
        )

        assert _bounds_message(values_key, bound, NEGATIVE_VALUE) in result.stderr


class TestShutdownValueBoundsGuardRejectsNonNumerics:
    """A non-numeric value for any bounded shutdown knob fails the render.

    These values reach the app as ConfigMap strings, not as API-server-validated
    pod fields, so nothing between `helm template` and process start would catch
    a `15s` unit suffix or an empty `--set key=`.
    """

    @pytest.mark.parametrize("value", NON_NUMERIC_VALUES, ids=repr)
    @pytest.mark.parametrize("values_key, bound", BOUNDED_VALUES, ids=BOUNDED_VALUE_IDS)
    def test_message_names_the_key_and_the_rejected_value(
        self, values_key, bound, value
    ):
        result = _render_raw(f"{values_key}={value}", show_only=DEPLOYMENT_TEMPLATE)

        assert _bounds_message(values_key, bound, value) in result.stderr

    def test_a_values_file_boolean_is_rejected(self):
        # `--set-json` delivers a real JSON type, the shape a values FILE
        # produces. A bare `true` in YAML is the classic accidental-type foot-gun
        # and must not pass a guard that only looked at digits.
        result = _render_raw(
            show_only=DEPLOYMENT_TEMPLATE,
            set_json=(f"{VALUES_SETTLE}=true",),
        )

        assert _bounds_message(VALUES_SETTLE, ">= 0", "true") in result.stderr


class TestShutdownValueBoundsGuardRejectsNonFiniteMagnitudes:
    """A magnitude outside float64's range fails the render on every key.

    This is the hole the numeric-text pattern leaves open: it accepts exponent
    notation (it must -- `1e-3` is a legitimate cadence), so `1e400` reads as
    numeric, and sprig's `float64` then swallows ParseFloat's range error and
    hands back 0. Nothing downstream would catch it: the grace invariant would
    compute against 0 and pass, and the ConfigMap would carry the TEXT to
    pydantic, which parses `1e400` as `inf` -- a pod that settles until SIGKILL
    on every shutdown. The underflow twin `1e-400` collapses to 0 the same way.
    """

    @pytest.mark.parametrize("value", NON_FINITE_VALUES)
    @pytest.mark.parametrize("values_key, bound", BOUNDED_VALUES, ids=BOUNDED_VALUE_IDS)
    def test_render_fails(self, values_key, bound, value):
        result = _render_raw(f"{values_key}={value}", show_only=DEPLOYMENT_TEMPLATE)

        assert result.returncode != 0

    @pytest.mark.parametrize("value", NON_FINITE_VALUES)
    @pytest.mark.parametrize("values_key, bound", BOUNDED_VALUES, ids=BOUNDED_VALUE_IDS)
    def test_message_names_the_key_and_the_rejected_value(
        self, values_key, bound, value
    ):
        # The exit code alone would not distinguish this guard from its
        # neighbours -- and would not catch the regression that matters most,
        # a value rejected for the wrong reason and reported as out of bounds.
        result = _render_raw(f"{values_key}={value}", show_only=DEPLOYMENT_TEMPLATE)

        assert _finite_message(values_key, value) in result.stderr

    @pytest.mark.parametrize("value", NON_FINITE_SPELLINGS)
    def test_other_out_of_range_spellings_are_rejected_too(self, value):
        # One key is enough here: the check is inside the loop over all three,
        # which the parametrization above already pins. What these add is the
        # spelling coverage -- a sign, a capital `E`, a fractional mantissa.
        result = _render_raw(f"{VALUES_SETTLE}={value}", show_only=DEPLOYMENT_TEMPLATE)

        assert _finite_message(VALUES_SETTLE, value) in result.stderr

    def test_a_large_but_in_range_magnitude_reaches_the_coupled_knob_guard(self):
        # 1e308 is finite, so it converts to itself rather than to 0 and must
        # pass this check and be judged by the invariant instead (which it
        # fails, hugely). Pins that the check keys on the conversion-to-zero
        # signature and has not become a cap on how large a value may be.
        result = _render_raw(
            f"{VALUES_SETTLE}={LARGE_FINITE_VALUE}", show_only=DEPLOYMENT_TEMPLATE
        )

        assert f"{TEARDOWN_TAIL_SECONDS}s post-drain teardown" in result.stderr


class TestShutdownValueBoundsGuardAcceptsZeroWhereTheLibraryDoes:
    """Zero renders for the settle and the drain window.

    Both are documented off-switches -- 0 skips that phase entirely -- so a
    guard that rejected zero would remove a supported configuration. Asserted
    through the rendered ConfigMap row rather than the exit code alone, so a
    guard that let zero through but dropped the row would still fail.
    """

    def test_zero_settle_delay_renders(self):
        assert _settings_data(f"{VALUES_SETTLE}=0")[ENV_SETTLE_DELAY] == "0"

    def test_zero_drain_window_renders(self):
        assert _settings_data(f"{VALUES_DRAIN_WINDOW}=0")[ENV_DRAIN_WINDOW] == "0"

    def test_an_exponent_form_zero_settle_delay_renders(self):
        # The off-switch written as `0e5`. It converts to 0 like any other
        # zero, so the finite check -- which rejects numeric text that converts
        # to 0 -- must recognise it as a zero LITERAL and stand down, or the
        # supported off-switch becomes unspellable in exponent form.
        data = _settings_data(f"{VALUES_SETTLE}={EXPONENT_ZERO}")

        assert data[ENV_SETTLE_DELAY] == EXPONENT_ZERO

    def test_an_exponent_form_zero_drain_window_renders(self):
        data = _settings_data(f"{VALUES_DRAIN_WINDOW}={EXPONENT_ZERO}")

        assert data[ENV_DRAIN_WINDOW] == EXPONENT_ZERO


class TestShutdownValueBoundsGuardRejectsAZeroPollInterval:
    """Zero fails for the poll interval, whose bound is strictly positive.

    It is a re-read cadence, not a duration: zero would busy-loop the drain
    rather than switch a phase off, and the library's own field bound rejects
    it. This is the one asymmetry in the bounds table, so it is pinned apart
    from the shared parametrization above.
    """

    def test_render_fails(self):
        result = _render_raw(
            f"{VALUES_DRAIN_POLL_INTERVAL}=0", show_only=DEPLOYMENT_TEMPLATE
        )

        assert result.returncode != 0

    def test_message_states_the_exclusive_bound(self):
        result = _render_raw(
            f"{VALUES_DRAIN_POLL_INTERVAL}=0", show_only=DEPLOYMENT_TEMPLATE
        )

        assert _bounds_message(VALUES_DRAIN_POLL_INTERVAL, "> 0", "0") in result.stderr

    def test_a_values_file_zero_is_rejected_too(self):
        # The `--set` form above arrives as the string "0"; a values file
        # delivers the number 0.0, which the guard has to catch through the same
        # test rather than by string comparison.
        result = _render_raw(
            show_only=DEPLOYMENT_TEMPLATE,
            set_json=(f"{VALUES_DRAIN_POLL_INTERVAL}=0.0",),
        )

        assert result.returncode != 0

    def test_an_exponent_form_zero_states_the_exclusive_bound(self):
        # `0e5` is a zero literal that also converts to 0, so it is the one
        # input both this guard and the finite check could claim. The finite
        # check must not intercept it: an operator who wrote a zero cadence has
        # a bound problem, not an out-of-range magnitude, and the message they
        # get decides which of the two they go and fix.
        result = _render_raw(
            f"{VALUES_DRAIN_POLL_INTERVAL}={EXPONENT_ZERO}",
            show_only=DEPLOYMENT_TEMPLATE,
        )

        assert (
            _bounds_message(VALUES_DRAIN_POLL_INTERVAL, "> 0", EXPONENT_ZERO)
            in result.stderr
        )


class TestShutdownValueBoundsGuardAcceptsLegitimateDecimals:
    """Ordinary decimal spellings render rather than tripping the guard.

    `--set` delivers a decimal as a STRING, so the guard reads text and could
    easily be too strict: a leading-dot `.25`, an exponent `1e-3` and an explicit
    `+0.5` are all valid Go floats an operator may write, and rejecting them
    would be a false trip on a correct configuration.
    """

    @pytest.mark.parametrize("value", ACCEPTED_DECIMAL_FORMS)
    def test_poll_interval_renders(self, value):
        data = _settings_data(f"{VALUES_DRAIN_POLL_INTERVAL}={value}")

        assert data[ENV_DRAIN_POLL_INTERVAL] == value

    def test_a_values_file_float_renders(self):
        # The number form: `--set-json` (and a values file) deliver a float64,
        # which the guard sees through `printf "%v"`.
        data = _settings_data(set_json=(f"{VALUES_SETTLE}=0.5",))

        assert data[ENV_SETTLE_DELAY] == "0.5"


class TestGraceCoversAllThreePhasesAtDefaults:
    """The SR-6 invariant holds at the shipped defaults.

    grace >= settle + drain + teardown, asserted from the rendered manifests
    (the grace period from the Deployment, the settle and drain from the
    settings ConfigMap the app actually reads) plus the template's teardown
    constant. A future defaults change that eats the headroom is caught here as
    well as by the render-time guard below.
    """

    def test_grace_covers_settle_plus_drain_plus_teardown(self, default_pod_spec):
        data = _settings_data()
        grace = default_pod_spec["terminationGracePeriodSeconds"]

        assert grace >= (
            float(data[ENV_SETTLE_DELAY])
            + float(data[ENV_DRAIN_WINDOW])
            + TEARDOWN_TAIL_SECONDS
        )


class TestInvariantGuardTrips:
    """The inline render-time `fail` guard rejects a grace period that cannot
    cover settle + drain + teardown, turning a production SIGKILL-mid-shutdown
    into a template-time error."""

    def test_trips_when_grace_is_below_the_bound(self, undersized_grace_render):
        # 10 < 15 + 30 + 10: the guard must fail the render non-zero.
        assert undersized_grace_render.returncode != 0

    def test_trips_when_only_the_teardown_tail_is_uncovered(self):
        # 54 clears the OLD two-term bound (54 >= 15 + 30 = 45) and fails the
        # new one by a single second. Pins that the tail is really in the sum
        # rather than absorbed by the default's headroom.
        result = _render_raw(
            f"{VALUES_GRACE}={DEFAULT_INVARIANT_FLOOR_SECONDS - 1}",
            show_only=DEPLOYMENT_TEMPLATE,
        )

        assert result.returncode != 0

    def test_trips_when_a_raised_settle_delay_pushes_past_the_bound(self):
        # 55 is exactly the floor at the default settle, so raising only the
        # settle must trip it. Proves the sum reads shutdown.settleDelaySeconds
        # rather than carrying the old key or a literal 15.
        result = _render_raw(
            f"{VALUES_GRACE}={DEFAULT_INVARIANT_FLOOR_SECONDS}",
            f"{VALUES_SETTLE}={DEFAULT_SETTLE_SECONDS + 1}",
            show_only=DEPLOYMENT_TEMPLATE,
        )

        assert result.returncode != 0

    def test_trip_message_names_the_invariant(self, undersized_grace_render):
        assert (
            VALUES_GRACE in undersized_grace_render.stderr
            and VALUES_SETTLE in undersized_grace_render.stderr
            and VALUES_DRAIN_WINDOW in undersized_grace_render.stderr
        )

    def test_trip_message_names_the_teardown_tail(self, undersized_grace_render):
        # The third term is a template constant with no values key, so an
        # operator reading only the message has to be told it exists -- otherwise
        # the arithmetic in the message does not add up to the bound it reports.
        assert (
            f"{TEARDOWN_TAIL_SECONDS}s post-drain teardown"
            in undersized_grace_render.stderr
        )

    def test_trips_on_float_boundary_shortfall(self):
        # Float arithmetic, not truncated ints: 55 < 15 + 30.5 + 10. A guard
        # that compared as ints would false-pass this (55 >= 15 + 30 + 10).
        result = _render_raw(
            f"{VALUES_GRACE}={DEFAULT_INVARIANT_FLOOR_SECONDS}",
            f"{VALUES_DRAIN_WINDOW}={DEFAULT_DRAIN_WINDOW_SECONDS + 0.5}",
            show_only=DEPLOYMENT_TEMPLATE,
        )

        assert result.returncode != 0


class TestInvariantGuardPasses:
    """The guard is strict `lt`, so equality passes: at the exact boundary the
    render succeeds and emits the grace period."""

    def test_passes_at_exact_boundary(self):
        # 55 == 15 + 30 + 10: equality must render, not trip.
        spec = _pod_spec(f"{VALUES_GRACE}={DEFAULT_INVARIANT_FLOOR_SECONDS}")

        assert spec["terminationGracePeriodSeconds"] == DEFAULT_INVARIANT_FLOOR_SECONDS

    def test_passes_when_a_raised_settle_delay_is_matched_by_the_grace_period(self):
        # The documented migration for an operator who lengthens the settle:
        # 90 >= 25 + 30 + 10. Pins that the guard is a bound, not a cap on the
        # settle itself.
        spec = _pod_spec(
            f"{VALUES_GRACE}={OVERRIDE_GRACE_SECONDS}",
            f"{VALUES_SETTLE}={OVERRIDE_SETTLE_SECONDS}",
        )

        assert spec["terminationGracePeriodSeconds"] == OVERRIDE_GRACE_SECONDS


class TestDrainSettingsRenderedIntoConfigMap:
    """Light structural presence check on the settings ConfigMap's shutdown rows.

    Per-row value/type/ordering coverage lives in
    test_drain_settings_template.py; here we only confirm the three shutdown env
    keys reach the same ConfigMap the Deployment mounts, so this file's
    Deployment surface and the settings surface are pinned as one coherent
    render.
    """

    def test_settle_delay_key_present_in_default_render(self):
        # The row that replaces the preStop hook: without it the app falls back
        # to the library default of 0 and the settle silently disappears, which
        # is exactly the regression this Issue's fix must not reintroduce.
        assert ENV_SETTLE_DELAY in _settings_data()

    def test_drain_window_key_present_in_default_render(self):
        assert ENV_DRAIN_WINDOW in _settings_data()

    def test_drain_poll_interval_key_present_in_default_render(self):
        assert ENV_DRAIN_POLL_INTERVAL in _settings_data()
