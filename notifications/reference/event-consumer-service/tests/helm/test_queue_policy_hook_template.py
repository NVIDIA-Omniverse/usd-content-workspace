# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Chart-side pins for the event-queue policy hook Job (issue b.9dd).

The Job in ``templates/queue-policy-hook-job.yaml`` applies a broker policy that
bounds the durable event queues and auto-reaps abandoned ones. Everything it
does is decided at RENDER time, so the render is where the invariants can be
pinned; the Job itself only forwards what the chart handed it.

Four claims carry real risk and are the reason this file exists:

1. **The match pattern is derived from the app's queue-naming contract.** The
   template hardcodes ``^event-queue:`` because it is not an operator knob, so
   nothing in the chart notices if ``QUEUE_NAME_PREFIX`` is renamed in
   ``app/lib/queue_names.py`` (where it is defined; ``rabbitmq_base`` only
   re-exports it) -- every event queue would silently go un-bounded. The
   pattern test IMPORTS that constant rather than quoting the literal a second
   time, which is what makes it a drift guard rather than a tautology.
2. **``apply-to: quorum_queues`` is the only thing keeping the policy off the
   non-durable queues.** Durable and non-durable queues share the
   ``event-queue:`` name prefix, so the pattern cannot separate them; only the
   durable declaration branch sets ``x-queue-type: quorum``. Widening the scope
   to ``queues`` would start drop-head-ing a live consumer's backlog on queues
   that have no dead-letter exchange to catch it, so the rendered scope is
   pinned here and the pattern is shown NOT to exclude a non-durable name on its
   own. There is deliberately no ``applyTo`` VALUE -- the template hardcodes
   ``quorum_queues``, which is why the pin is on the render rather than on a
   default, and why widening it takes a template edit that this test blocks by
   failing. (The other
   half of the separation -- which declaration carries ``x-queue-type: quorum``
   -- belongs to the app and is pinned in
   ``tests/app/lib/rabbitmq/test_rabbitmq_base.py::TestCreateQueue``.)
3. **Numbers must render as integers.** Helm parses YAML ints as float64, so a
   bare ``| quote`` turns 86400000 into ``"8.64e+07"`` -- a value the broker
   rejects. The template runs every numeric knob through ``| int64 | quote``;
   the integer forms are pinned here so a dropped ``int64`` is caught by a test
   and not by a failed release.
4. **The entrypoint has to survive `.dockerignore` and the publish denylist.**
   The Job runs
   ``python -m public_scripts.apply_queue_policy`` out of the service image, and
   the image ships app code by ``COPY . /code`` rather than installing the
   project, so a path excluded from the build context simply does not exist at
   runtime -- the Job then dies on ``No module named
   public_scripts.apply_queue_policy``, which fails the release with a message
   about none of this. The module lives in its own top-level ``public_scripts``
   package precisely so that no ignore rule reaches it: ``scripts`` (the dev
   tooling) is excluded wholesale, and a re-include underneath an excluded
   directory would never be consulted. The rendered command is walked back to a
   repo path and checked against the ignore rules, so excluding the module,
   moving it back under an excluded directory, renaming it, or retargeting the
   command all break a test instead of a deploy. Shipping the FILE is only half
   of it: the image installs ``uv export --no-dev --locked``, so the module's
   third-party imports have to be DECLARED dependencies. One that only reaches
   the image transitively keeps working in dev and dies at import in the Job the
   moment the package that dragged it in stops doing so -- also pinned here.
   The published source folder is the same hazard once more: the chart it
   carries names this module, so the publish denylist must not drop the
   module's package. That check reads the internal publish tooling, so it lives
   with the tooling's tests under ``tests/scripts/``, which are not published.

The render guards are asserted through ``helm template``'s exit status and
stderr, since "fails to render" IS the contract for them and the message is the
only thing the operator gets. They cover the required broker coordinates plus
every bound the BROKER would happily accept and that is silently destructive: a
negative ``deliveryLimit`` (rabbitmq-server#14202) and a null-or-zero
``messageTtlMs`` / ``maxLength``. Null is guarded separately from the range in
each case because Helm's ``int64`` coerces nil to 0, so ``--set ...=null`` would
otherwise render a real bound of 0 rather than failing. A non-numeric KIND is
guarded separately again and for the same coercion: ``int`` maps an unparseable
string to 0 too, so ``deliveryLimit=abc`` would clear a ``>= 0`` range check and
ship a bound the operator never wrote. That check sits AFTER each null check so
that a null keeps its own distinct message, and the ordering is pinned here
rather than assumed. ``expiresMs`` is the one knob where null is legitimate --
it is the documented omit-the-bound idiom -- so it takes the type check and
neither of the others; that asymmetry is pinned too, since a null guard copied
onto it would break the idiom.

``consumerTimeoutMs`` is the third shape: null is legitimate AND it is the
default, but a small numeric value is neither omitted nor rejected by the broker
-- it is accepted and destructive, since a ``consumer-timeout`` below RabbitMQ's
documented 60000 ms floor cancels healthy consumers mid-delivery (at 0, every
consumer on every durable queue, immediately). So it carries the null idiom, the
type check, AND a range floor, and all three are pinned.

The Job's ``activeDeadlineSeconds`` and the script's own retry budget are two
halves of one number and are pinned together here (see
``TestRetryBudgetFitsTheJobDeadline``): the deadline exists so the script's
diagnostic is what an operator sees, which only holds while the script can spend
its whole budget inside it.
"""

import ast
import fnmatch
import importlib.metadata
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path, PurePosixPath

import pytest
import yaml

from app.lib.queue_names import QUEUE_NAME_PREFIX
from app.lib.rabbitmq.non_durable_queue_consumer import NonDurableQueueConsumer
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase
from public_scripts import apply_queue_policy
from tests.helm.conftest import select_tokenless_auth

REPO_ROOT = Path(__file__).resolve().parents[2]
CHART_PATH = REPO_ROOT / "helm" / "event-consumer-service"
DOCKERIGNORE = REPO_ROOT / ".dockerignore"
PYPROJECT = REPO_ROOT / "pyproject.toml"

# A development-only script, used to show that `scripts/` as a whole still stays
# out of the image now that the entrypoint no longer lives there.
DEV_SCRIPT = PurePosixPath("scripts/manual_grpc_tests.py")

# The third-party imports the entrypoint carries today. Asserted to be FOUND by
# the scan below, so that a scan which quietly returned nothing cannot satisfy
# the "every import is declared" check vacuously.
KNOWN_ENTRYPOINT_IMPORTS = {"certifi", "urllib3", "tenacity"}

HOOK_JOB_TEMPLATE = "templates/queue-policy-hook-job.yaml"
SMOKE_TEST_TEMPLATE = "templates/tests/smoke-test.yaml"

QUEUE_POLICY_COMPONENT = "queue-policy"

RELEASE = "ec"
# 38 characters, the longest release name the chart is likely to meet in the
# field (helm itself caps release names at 53).
LONG_RELEASE = "a-very-long-release-name-for-consumers"

# Kubernetes rejects a Job whose name exceeds the label-value limit, because the
# name becomes the `job-name` label on every pod it creates.
MAX_JOB_NAME_LENGTH = 63
DNS_1123_SUBDOMAIN = re.compile(r"^[a-z0-9]([-a-z0-9]*[a-z0-9])?$")

# The broker coordinates have no defaults on purpose, so every enabled render
# has to supply them. Values chosen to be obviously fake.
REQUIRED_SET = (
    "policyHook.managementApi.baseUrl=https://broker.example.com:15672",
    "policyHook.managementApi.vhost=events",
    "policyHook.managementApi.existingSecret.name=rmq-policy-admin",
)

# `helm.sh/hook-weight` orders the policy Job ahead of the smoke-test Job, which
# carries no weight (implicit 0). A string, because annotation values are.
EXPECTED_HOOK_WEIGHT = "-5"

# The escape hatch every render-guard message must point the operator at.
ESCAPE_HATCH = "policyHook.enabled=false"

pytestmark = pytest.mark.skipif(
    shutil.which("helm") is None,
    reason="helm CLI not available",
)


def _helm_template(
    *extra_set: str, show_only: str | None = None, release: str = RELEASE
):
    args = ["helm", "template", release, str(CHART_PATH), "--namespace", "storage-api"]
    if show_only is not None:
        args.extend(["--show-only", show_only])
    select_tokenless_auth(args)
    for kv in extra_set:
        args.extend(["--set", kv])
    return subprocess.run(args, capture_output=True, text=True)


def _render(
    *extra_set: str, show_only: str | None = None, release: str = RELEASE
) -> str:
    result = _helm_template(*extra_set, show_only=show_only, release=release)
    assert result.returncode == 0, (
        f"helm template failed:\nstdout:\n{result.stdout}\n" f"stderr:\n{result.stderr}"
    )
    return result.stdout


def _render_error(*extra_set: str) -> str:
    """Render expecting a `fail`, returning stderr for the message assertions."""
    result = _helm_template(*extra_set)
    assert result.returncode != 0, f"expected the render to fail:\n{result.stdout}"
    return result.stderr


def _documents(rendered: str) -> list[dict]:
    return [doc for doc in yaml.safe_load_all(rendered) if doc]


def _hook_job(*extra_set: str, release: str = RELEASE) -> dict:
    """The rendered policy hook Job, with the required broker values supplied."""
    rendered = _render(
        *REQUIRED_SET, *extra_set, show_only=HOOK_JOB_TEMPLATE, release=release
    )
    docs = _documents(rendered)
    assert (
        len(docs) == 1
    ), f"expected exactly one document, got {len(docs)}:\n{rendered}"
    return docs[0]


def _hook_container(*extra_set: str) -> dict:
    containers = _hook_job(*extra_set)["spec"]["template"]["spec"]["containers"]
    assert len(containers) == 1, f"expected a single container, got {len(containers)}"
    return containers[0]


def _literal_env(*extra_set: str) -> dict[str, str]:
    """Container env vars carrying a literal `value` (secretKeyRef rows omitted)."""
    env = _hook_container(*extra_set)["env"]
    return {row["name"]: row["value"] for row in env if "value" in row}


def _env_names(*extra_set: str) -> set[str]:
    return {row["name"] for row in _hook_container(*extra_set)["env"]}


@pytest.fixture(scope="module")
def hook_env() -> dict[str, str]:
    """Literal env of the stock enabled render. Read-only across these tests."""
    return _literal_env()


class TestPolicyPattern:
    """The pattern must track the app's queue-naming contract, not a literal.

    `QUEUE_NAME_PREFIX` is imported from its home in `app/lib/queue_names.py`
    so that renaming it in the app breaks this test; re-quoting
    `"^event-queue:"` here would assert nothing.
    """

    def test_pattern_is_derived_from_the_queue_name_prefix(self, hook_env):
        assert hook_env["QUEUE_POLICY_PATTERN"] == f"^{QUEUE_NAME_PREFIX}:"

    @pytest.mark.asyncio
    async def test_pattern_matches_a_durable_queue_name(self, hook_env, mock_queue_id):
        name = await RabbitMQBase._queue_name(mock_queue_id)

        assert re.match(hook_env["QUEUE_POLICY_PATTERN"], name)


class TestApplyToScope:
    """`apply-to: quorum_queues` is load-bearing scoping, not a tuning knob.

    The name pattern deliberately cannot separate durable from non-durable
    queues, so the queue TYPE is the whole separation. This class owns the
    chart's half of that: the rendered scope, and the fact that the pattern
    alone does NOT exclude a non-durable queue name. The app's half -- durable
    declarations carrying `x-queue-type: quorum` and non-durable ones carrying
    no queue type -- is pinned by
    `tests/app/lib/rabbitmq/test_rabbitmq_base.py::TestCreateQueue`
    (`test_declares_durable_queue_with_quorum_and_dlx_args` and
    `test_declares_non_durable_queue_with_name_and_ttl`), which assert the whole
    `declare_queue` call rather than one argument.
    """

    def test_apply_to_is_quorum_queues(self, hook_env):
        assert hook_env["QUEUE_POLICY_APPLY_TO"] == "quorum_queues"

    def test_the_scope_is_not_reachable_as_a_value(self):
        # There is no `policyHook.policy.applyTo`; the template hardcodes the
        # scope. Helm accepts a `--set` at an unknown path silently, so an
        # operator who believes it is a knob gets no error -- the render simply
        # has to ignore him. This also fails if the value plumbing is ever
        # restored, which is the change that would make widening possible again.
        env = _literal_env("policyHook.policy.applyTo=queues")

        assert env["QUEUE_POLICY_APPLY_TO"] == "quorum_queues"

    @pytest.mark.asyncio
    async def test_pattern_alone_does_not_exclude_non_durable_queues(
        self, hook_env, mock_queue_id
    ):
        name = await NonDurableQueueConsumer._queue_name(mock_queue_id)

        assert re.match(hook_env["QUEUE_POLICY_PATTERN"], name)


class TestHookDisabled:
    """`policyHook.enabled=false` is the escape hatch: it must render nothing."""

    def test_no_job_document_is_rendered(self):
        rendered = _render("policyHook.enabled=false", show_only=HOOK_JOB_TEMPLATE)

        assert _documents(rendered) == []

    def test_no_queue_policy_job_appears_in_the_full_release(self):
        docs = _documents(_render("policyHook.enabled=false"))
        components = {
            doc.get("metadata", {}).get("labels", {}).get("app.kubernetes.io/component")
            for doc in docs
        }

        assert QUEUE_POLICY_COMPONENT not in components

    def test_required_values_are_not_demanded_when_disabled(self):
        # The whole point of the hatch: a disabled hook must not make the
        # broker coordinates mandatory.
        result = _helm_template("policyHook.enabled=false")

        assert result.returncode == 0, result.stderr


class TestHookOrdering:
    """The bounds must be in place before the smoke test exercises the queues."""

    def test_hook_weight_orders_the_job_first(self):
        annotations = _hook_job()["metadata"]["annotations"]

        assert annotations["helm.sh/hook-weight"] == EXPECTED_HOOK_WEIGHT

    def test_hook_weight_is_a_string(self):
        # An unquoted -5 renders as an int and Kubernetes rejects a non-string
        # annotation value outright.
        annotations = _hook_job()["metadata"]["annotations"]

        assert isinstance(annotations["helm.sh/hook-weight"], str)

    def test_smoke_test_job_carries_no_hook_weight(self):
        # The ordering claim is relative: it only holds while the smoke test
        # stays at the implicit weight 0.
        docs = _documents(
            _render(
                "policyHook.enabled=false",
                "tests.enabled=true",
                show_only=SMOKE_TEST_TEMPLATE,
            )
        )
        assert len(docs) == 1, f"expected one smoke-test document, got {len(docs)}"

        assert "helm.sh/hook-weight" not in docs[0]["metadata"]["annotations"]

    def test_job_runs_on_install_and_upgrade(self):
        annotations = _hook_job()["metadata"]["annotations"]

        assert annotations["helm.sh/hook"] == "post-install,post-upgrade"

    def test_previous_job_is_deleted_before_the_next_run(self):
        # Jobs are immutable, so without this the second `helm upgrade` fails.
        annotations = _hook_job()["metadata"]["annotations"]

        assert annotations["helm.sh/hook-delete-policy"] == "before-hook-creation"


# (label, --set overrides that leave one required value empty, message fragment)
MISSING_REQUIRED_VALUES = [
    (
        "baseUrl",
        ("policyHook.managementApi.baseUrl=",),
        "policyHook.managementApi.baseUrl",
    ),
    (
        "vhost",
        ("policyHook.managementApi.vhost=",),
        "policyHook.managementApi.vhost",
    ),
    (
        "existingSecret.name",
        ("policyHook.managementApi.existingSecret.name=",),
        "policyHook.managementApi.existingSecret.name",
    ),
]
MISSING_REQUIRED_IDS = [label for label, _, _ in MISSING_REQUIRED_VALUES]


class TestRequiredValueGuards:
    """An incompletely configured hook fails the render, never installs a no-op.

    A hook that silently does nothing is exactly the failure this feature
    exists to remove, so "refuses to render" is the contract.
    """

    @pytest.mark.parametrize(
        "label, overrides, fragment",
        MISSING_REQUIRED_VALUES,
        ids=MISSING_REQUIRED_IDS,
    )
    def test_render_fails_when_a_required_value_is_empty(
        self, label, overrides, fragment
    ):
        stderr = _render_error(*REQUIRED_SET, *overrides)

        assert fragment in stderr

    @pytest.mark.parametrize(
        "label, overrides, fragment",
        MISSING_REQUIRED_VALUES,
        ids=MISSING_REQUIRED_IDS,
    )
    def test_guard_names_the_escape_hatch(self, label, overrides, fragment):
        stderr = _render_error(*REQUIRED_SET, *overrides)

        assert ESCAPE_HATCH in stderr

    def test_bare_default_render_fails_without_broker_coordinates(self):
        # The shipped default is `enabled: true` with empty coordinates, so a
        # bare render must fail -- this is why every other tests/helm module
        # passes `policyHook.enabled=false`.
        result = _helm_template()

        assert result.returncode != 0


DEFINITION = "policyHook.policy.definition"

# Bound values the BROKER accepts and that are silently destructive, so the only
# place they can be stopped is the render. A null sits beside each range
# violation because Helm's `int64` coerces nil to 0: without an explicit nil
# test, `--set ...=null` renders a REAL bound of 0 rather than failing, which is
# the more dangerous half of each pair -- the operator asked for "unset".
# `consumerTimeoutMs` is the exception with no `_null` twin, and only that one:
# null is its DEFAULT and its omit-the-bound idiom, so a nil is legitimate there
# and never reaches the range check (pinned in `TestConsumerTimeoutFloor`).
# (label, --set override, the values path the message must name)
DESTRUCTIVE_BOUNDS = [
    ("deliveryLimit_negative", f"{DEFINITION}.deliveryLimit=-1", "deliveryLimit"),
    ("deliveryLimit_null", f"{DEFINITION}.deliveryLimit=null", "deliveryLimit"),
    ("messageTtlMs_zero", f"{DEFINITION}.messageTtlMs=0", "messageTtlMs"),
    ("messageTtlMs_null", f"{DEFINITION}.messageTtlMs=null", "messageTtlMs"),
    ("maxLength_zero", f"{DEFINITION}.maxLength=0", "maxLength"),
    ("maxLength_null", f"{DEFINITION}.maxLength=null", "maxLength"),
    (
        "consumerTimeoutMs_zero",
        f"{DEFINITION}.consumerTimeoutMs=0",
        "consumerTimeoutMs",
    ),
    (
        "consumerTimeoutMs_below_floor",
        f"{DEFINITION}.consumerTimeoutMs=59999",
        "consumerTimeoutMs",
    ),
]
DESTRUCTIVE_BOUND_IDS = [label for label, _, _ in DESTRUCTIVE_BOUNDS]

# The null overrides restated on their own, because their MESSAGES are the
# claim: each one has to keep the text it had before the type check existed,
# which only holds while that check sits after the nil check. Reordering the
# two is invisible otherwise -- the render still fails, just with the wrong
# explanation, sending the operator after a typo he did not make.
NULL_BOUNDS = [
    ("deliveryLimit", f"{DEFINITION}.deliveryLimit=null"),
    ("messageTtlMs", f"{DEFINITION}.messageTtlMs=null"),
    ("maxLength", f"{DEFINITION}.maxLength=null"),
]
NULL_BOUND_IDS = [label for label, _ in NULL_BOUNDS]

# Values of the wrong KIND. Helm's `int` maps any unparseable string to 0, the
# same coercion that makes the nulls above dangerous, so none of the range
# checks can see one: `deliveryLimit=abc` passes `>= 0` and ships
# `delivery-limit: 0`. `=""` is the same hazard reached by quoting rather than
# by a typo, which is the form an operator's own values file produces, so it is
# pinned on the knob whose 0 is silently destructive (`deliveryLimit`) and on
# the one whose 0 the broker rejects outright (`expiresMs`).
# (label, --set override, the values path the message must name)
NON_NUMERIC_BOUNDS = [
    ("deliveryLimit_string", f"{DEFINITION}.deliveryLimit=abc", "deliveryLimit"),
    ("messageTtlMs_string", f"{DEFINITION}.messageTtlMs=abc", "messageTtlMs"),
    ("maxLength_string", f"{DEFINITION}.maxLength=abc", "maxLength"),
    ("expiresMs_string", f"{DEFINITION}.expiresMs=abc", "expiresMs"),
    (
        "consumerTimeoutMs_string",
        f"{DEFINITION}.consumerTimeoutMs=abc",
        "consumerTimeoutMs",
    ),
    ("deliveryLimit_empty", f"{DEFINITION}.deliveryLimit=", "deliveryLimit"),
    ("expiresMs_empty", f"{DEFINITION}.expiresMs=", "expiresMs"),
    (
        "consumerTimeoutMs_empty",
        f"{DEFINITION}.consumerTimeoutMs=",
        "consumerTimeoutMs",
    ),
]
NON_NUMERIC_BOUND_IDS = [label for label, _, _ in NON_NUMERIC_BOUNDS]

# Every guarded override, for the check that has to hold across all of them: the
# escape hatch each message names actually works.
ALL_GUARDED_BOUNDS = DESTRUCTIVE_BOUNDS + NON_NUMERIC_BOUNDS
ALL_GUARDED_BOUND_IDS = [label for label, _, _ in ALL_GUARDED_BOUNDS]

# The phrase that distinguishes the type refusal from every range/null refusal.
# Used both ways: asserted PRESENT for a wrong-kind value, and ABSENT for a
# null, which is what pins the two guards' order.
TYPE_GUARD_FRAGMENT = "must be a number"

# The kind Helm reports for anything `--set` could not parse as a number. The
# operator needs it: "must be a number" against a value that looks like one in
# his values file only makes sense once he is told it arrived as a string.
REPORTED_KIND = "got string"

# The lowest value each bound still accepts. The guards are range checks, so the
# boundary is where an off-by-one turns a legal policy into a refused render.
# (label, --set override, env var, the value it must render as)
LOWEST_LEGAL_BOUNDS = [
    (
        "deliveryLimit",
        f"{DEFINITION}.deliveryLimit=0",
        "QUEUE_POLICY_DELIVERY_LIMIT",
        "0",
    ),
    (
        "messageTtlMs",
        f"{DEFINITION}.messageTtlMs=1",
        "QUEUE_POLICY_MESSAGE_TTL_MS",
        "1",
    ),
    ("maxLength", f"{DEFINITION}.maxLength=1", "QUEUE_POLICY_MAX_LENGTH", "1"),
]
LOWEST_LEGAL_IDS = [label for label, _, _, _ in LOWEST_LEGAL_BOUNDS]


class TestDestructiveBoundGuards:
    """Bounds the broker accepts but that silently destroy events are refused.

    Nothing downstream catches any of these: the PUT succeeds, this fail-closed
    hook reports success, the release goes green, and the damage surfaces later
    as missing events with no deploy-time signal at all. `message-ttl: 0`
    expires every message the instant it is published; `max-length: 0`
    dead-letters every message on every durable queue (it can never hold one); a
    negative `delivery-limit` triggers the RabbitMQ 4.0.5-4.1.4 policy-repair
    loop (rabbitmq-server#14202), which resets the `expires` idle clock every 5 s
    so no queue ever auto-reaps; a `consumer-timeout` below RabbitMQ's documented
    60000 ms floor cancels healthy consumers mid-delivery, and at 0 cancels every
    consumer on every durable queue at once. Refusing to render is the only
    defense.

    The consumer-timeout cases sit here rather than only in
    `TestConsumerTimeoutFloor` because they share both claims below: the message
    names the knob, and it names the escape hatch. What is particular to that
    knob -- the legitimate null, the floor's own wording, the boundary from the
    legal side -- stays in its own class.
    """

    @pytest.mark.parametrize(
        "label, override, knob", DESTRUCTIVE_BOUNDS, ids=DESTRUCTIVE_BOUND_IDS
    )
    def test_the_render_fails_and_names_the_knob(self, label, override, knob):
        # `_render_error` asserts the non-zero exit; the knob name is what turns
        # a stack of Helm template noise into something actionable.
        stderr = _render_error(*REQUIRED_SET, override)

        assert knob in stderr

    @pytest.mark.parametrize(
        "label, override, knob", DESTRUCTIVE_BOUNDS, ids=DESTRUCTIVE_BOUND_IDS
    )
    def test_the_guard_names_the_escape_hatch(self, label, override, knob):
        stderr = _render_error(*REQUIRED_SET, override)

        assert ESCAPE_HATCH in stderr

    @pytest.mark.parametrize(
        "label, override, knob", ALL_GUARDED_BOUNDS, ids=ALL_GUARDED_BOUND_IDS
    )
    def test_disabling_the_hook_bypasses_the_guard(self, label, override, knob):
        # Every message above tells the operator to disable the hook. If any
        # `fail` sat outside the `if .Values.policyHook.enabled` branch that
        # advice would be a dead end, and an operator who manages the policy
        # himself could not install the chart at all. Spans the wrong-KIND
        # overrides too (`TestNonNumericBoundGuards`): those refuse from a
        # shared named template, which is one more place a guard could end up
        # outside the branch, and it is the same claim either way.
        result = _helm_template("policyHook.enabled=false", override)

        assert result.returncode == 0, result.stderr

    @pytest.mark.parametrize(
        "label, override, env_name, expected",
        LOWEST_LEGAL_BOUNDS,
        ids=LOWEST_LEGAL_IDS,
    )
    def test_the_lowest_legal_value_still_renders(
        self, label, override, env_name, expected
    ):
        env = _literal_env(override)

        assert env[env_name] == expected


class TestDeliveryLimitGuardMessages:
    """The two delivery-limit refusals are different failures and say so.

    They cannot share a message. A NEGATIVE limit is the #14202 landmine and the
    operator needs to be told why -1 is banned, or it just looks arbitrary. A
    NULL is not that at all -- it is an omission Helm silently turns into a legal
    `delivery-limit: 0`, which dead-letters every requeued message on its first
    redelivery. Handing the null case the policy-repair explanation would send
    the operator after a bug that is not there.
    """

    def test_the_negative_guard_cites_the_expires_reset(self):
        stderr = _render_error(*REQUIRED_SET, f"{DEFINITION}.deliveryLimit=-1")

        assert "expires" in stderr

    def test_the_negative_guard_names_the_policy_repair_loop(self):
        # Pinned so the contrast below stays meaningful: if this phrase ever
        # left the negative message, the null test would start passing for the
        # wrong reason.
        stderr = _render_error(*REQUIRED_SET, f"{DEFINITION}.deliveryLimit=-1")

        assert "policy-repair" in stderr

    def test_the_null_guard_does_not_cite_the_policy_repair_loop(self):
        stderr = _render_error(*REQUIRED_SET, f"{DEFINITION}.deliveryLimit=null")

        assert "policy-repair" not in stderr

    def test_the_null_guard_explains_the_zero_coercion(self):
        # Without this the operator sees "must be an integer >= 0" against a
        # value he deliberately left unset, and has no way to know that Helm
        # turned it into a 0 rather than an omission.
        stderr = _render_error(*REQUIRED_SET, f"{DEFINITION}.deliveryLimit=null")

        assert "coerces nil" in stderr


class TestNonNumericBoundGuards:
    """A value of the wrong KIND is refused, not coerced into a real bound.

    Helm's `int` does not fail on garbage -- it returns 0. So every range check
    in this template is blind to a non-numeric value: `deliveryLimit=abc`
    satisfies `>= 0` and ships `delivery-limit: 0`, the same silent destruction
    the null guards exist to stop, arrived at from the other direction. The
    operator gets no signal at all, because the render succeeds and the hook
    reports success. `expiresMs` fails differently but no better: its 0 is a
    value the BROKER rejects, so the release dies on a broker-side error that
    names neither the knob nor the typo behind it.

    An empty string is the same fault without a typo -- it is what a quoted or
    blanked-out entry in an operator's own values file produces -- so it is
    pinned alongside.
    """

    @pytest.mark.parametrize(
        "label, override, knob", NON_NUMERIC_BOUNDS, ids=NON_NUMERIC_BOUND_IDS
    )
    def test_the_render_fails_and_names_the_knob(self, label, override, knob):
        stderr = _render_error(*REQUIRED_SET, override)

        assert knob in stderr

    @pytest.mark.parametrize(
        "label, override, knob", NON_NUMERIC_BOUNDS, ids=NON_NUMERIC_BOUND_IDS
    )
    def test_the_message_reports_the_kind_that_arrived(self, label, override, knob):
        # "must be a number" alone reads as nonsense against a values file that
        # plainly contains a number; the kind is what tells the operator it
        # reached the chart quoted.
        stderr = _render_error(*REQUIRED_SET, override)

        assert REPORTED_KIND in stderr

    @pytest.mark.parametrize(
        "label, override, knob", NON_NUMERIC_BOUNDS, ids=NON_NUMERIC_BOUND_IDS
    )
    def test_the_guard_names_the_escape_hatch(self, label, override, knob):
        stderr = _render_error(*REQUIRED_SET, override)

        assert ESCAPE_HATCH in stderr

    @pytest.mark.parametrize("knob, override", NULL_BOUNDS, ids=NULL_BOUND_IDS)
    def test_a_null_keeps_its_own_message(self, knob, override):
        # The placement claim, stated as its own test because reordering the
        # two checks does not change WHETHER the render fails -- only what it
        # says. A null that got answered with "must be a number, but got
        # <nil>" would send the operator looking for a quoting mistake in a
        # value he deliberately left unset, and would bury the thing he
        # actually needs to know: that Helm turned it into a 0.
        stderr = _render_error(*REQUIRED_SET, override)

        assert TYPE_GUARD_FRAGMENT not in stderr


class TestExpiresAutoReap:
    """`expiresMs` absence is meaningful and must not degrade into a zero.

    The chart tests explicitly for nil rather than using `with`, because `with`
    treats 0 as absent -- an operator who sets 0 has to see the broker reject it
    rather than silently receive a policy with the reaper switched off.

    Null being legitimate here is what makes this the one knob with no null and
    no range guard, so the remaining check -- the value must still be a NUMBER
    -- is pinned beside the null case rather than only in
    `TestNonNumericBoundGuards`, where the asymmetry is not visible.
    """

    def test_default_ships_a_twenty_four_hour_expires(self, hook_env):
        assert hook_env["QUEUE_POLICY_EXPIRES_MS"] == "86400000"

    def test_null_omits_the_expires_variable_entirely(self):
        names = _env_names("policyHook.policy.definition.expiresMs=null")

        assert "QUEUE_POLICY_EXPIRES_MS" not in names

    def test_a_non_numeric_value_is_refused_even_though_null_is_accepted(self):
        # Deliberately adjacent to the test above, because the two together are
        # the claim. Exempting `expiresMs` from the null guard is a statement
        # about NIL only; it is easy to read as "this knob is unguarded" and
        # drop the type check with it. A string is not an omission -- Helm
        # coerces it to 0, and `expires: 0` is rejected by the broker, so the
        # release fails either way. The difference is whether the operator is
        # told which value he got wrong.
        stderr = _render_error(*REQUIRED_SET, f"{DEFINITION}.expiresMs=abc")

        assert "expiresMs" in stderr

    def test_null_leaves_the_other_definition_knobs_intact(self):
        # Omission must be surgical: the rest of the policy still ships.
        env = _literal_env("policyHook.policy.definition.expiresMs=null")

        assert env["QUEUE_POLICY_MESSAGE_TTL_MS"] == "86400000"

    def test_zero_is_forwarded_rather_than_silently_dropped(self):
        env = _literal_env("policyHook.policy.definition.expiresMs=0")

        assert env["QUEUE_POLICY_EXPIRES_MS"] == "0"


class TestConsumerTimeoutFloor:
    """`consumerTimeoutMs` carries the null idiom AND a floor, unlike any other.

    It is the only bound where a small numeric value is neither omitted nor
    rejected: the broker ACCEPTS a `consumer-timeout` below its own documented
    60000 ms floor and then cancels healthy consumers mid-delivery. At 0 that is
    every consumer on every durable queue, immediately, on first delivery -- a
    permanent kill/reconnect loop across all clients, while the hook Job reports
    success and the release goes green. Nothing downstream catches it, so the
    render is the only place it can be stopped.

    Null is the DEFAULT here, not merely permitted: absent the key, the broker's
    own default applies, which is the behaviour every existing deployment has.
    So the omission is pinned as hard as the floor -- a render that started
    shipping a key would retune the ack timeout on every matched queue of every
    installation, unasked.

    The floor stops at the upstream hard bound. The softer ">= 300000 (5
    minutes)" recommendation and the prefetch-aware sizing formula stay guidance
    in values.yaml, because the safe value depends on the client's per-message
    processing time the chart cannot see -- so 60000 must still render.

    Only what is particular to this knob lives here. The two claims it shares
    with every other destructive bound -- the refusal names the knob, and it
    names `policyHook.enabled=false` -- are parameterized over
    `DESTRUCTIVE_BOUNDS`, which carries the below-floor cases.
    """

    def test_the_default_omits_the_variable_entirely(self, hook_env):
        assert "QUEUE_POLICY_CONSUMER_TIMEOUT_MS" not in hook_env

    def test_an_explicit_null_omits_the_variable_entirely(self):
        # Same as the default, but reached by an operator writing it out --
        # which is the form that would break first if the nil test were ever
        # swapped for a `with`.
        names = _env_names(f"{DEFINITION}.consumerTimeoutMs=null")

        assert "QUEUE_POLICY_CONSUMER_TIMEOUT_MS" not in names

    def test_a_set_value_renders_as_an_integer_string(self):
        # Helm parses YAML ints as float64, so a `| quote` without `| int64`
        # would render "3e+05" and the broker would reject the whole policy.
        env = _literal_env(f"{DEFINITION}.consumerTimeoutMs=300000")

        assert env["QUEUE_POLICY_CONSUMER_TIMEOUT_MS"] == "300000"

    def test_the_floor_itself_still_renders(self):
        # The boundary, from the legal side: 60000 is below the RECOMMENDED
        # 300000 and is still the operator's call to make, so the guard must
        # enforce the hard floor only.
        env = _literal_env(f"{DEFINITION}.consumerTimeoutMs=60000")

        assert env["QUEUE_POLICY_CONSUMER_TIMEOUT_MS"] == "60000"

    def test_one_millisecond_below_the_floor_is_refused(self):
        # The boundary from the other side. Stated separately from the
        # parameterized destructive-bound cases because an off-by-one in the
        # comparison is exactly what would let an unsupported value through
        # while every other case still failed.
        stderr = _render_error(*REQUIRED_SET, f"{DEFINITION}.consumerTimeoutMs=59999")

        assert "must be an integer >= 60000" in stderr

    def test_the_refusal_says_what_a_low_value_does(self):
        # Without the reason the floor reads as an arbitrary ban and the next
        # operator reaches for a way around it.
        stderr = _render_error(*REQUIRED_SET, f"{DEFINITION}.consumerTimeoutMs=0")

        assert "cancels healthy consumers" in stderr

    def test_the_refusal_names_the_null_way_out(self):
        # The operator who hit this floor usually wanted the broker default,
        # and `null` is how he gets it -- but nothing else in the message says
        # so, and the values path is not self-describing.
        stderr = _render_error(*REQUIRED_SET, f"{DEFINITION}.consumerTimeoutMs=0")

        assert "null to omit the key" in stderr

    def test_the_refusal_points_at_the_sizing_input(self):
        # "Pick a bigger number" is not actionable alone; the prefetch count is
        # the half of the sizing the operator cannot guess.
        stderr = _render_error(*REQUIRED_SET, f"{DEFINITION}.consumerTimeoutMs=0")

        assert "durable_consumer_prefetch_count" in stderr

    def test_a_null_is_not_answered_with_the_type_guard(self):
        # The placement claim, as for `expiresMs`: null is legitimate here, so
        # it must render rather than trip the shared "must be a number" check.
        result = _helm_template(*REQUIRED_SET, f"{DEFINITION}.consumerTimeoutMs=null")

        assert result.returncode == 0, result.stderr

    def test_a_non_numeric_value_is_refused_even_though_null_is_accepted(self):
        # Deliberately adjacent to the test above, because the two together are
        # the claim. Exempting the knob from the NIL guard is easy to read as
        # "this knob is unguarded"; a string is not an omission, and Helm
        # coerces it to 0 -- the single worst value in the range.
        stderr = _render_error(*REQUIRED_SET, f"{DEFINITION}.consumerTimeoutMs=abc")

        assert "consumerTimeoutMs" in stderr

    def test_the_type_guard_names_this_knobs_own_consequence(self):
        # The shared type guard enumerates what a coerced 0 means for each
        # bound. Without this knob's entry the operator is told about
        # deliveryLimit and expiresMs and left to work out that HIS typo means
        # every consumer gets cancelled.
        stderr = _render_error(*REQUIRED_SET, f"{DEFINITION}.consumerTimeoutMs=abc")

        assert "cancels every consumer on every durable queue" in stderr

    def test_a_set_value_leaves_the_other_definition_knobs_intact(self):
        env = _literal_env(f"{DEFINITION}.consumerTimeoutMs=300000")

        assert env["QUEUE_POLICY_MESSAGE_TTL_MS"] == "86400000"


# Every numeric knob that reaches the Job as a string. Helm parses YAML ints as
# float64, so a `| quote` without `| int64` renders "8.64e+07" and the broker
# rejects the policy.
NUMERIC_ENV_DEFAULTS = [
    ("QUEUE_POLICY_PRIORITY", "0"),
    ("QUEUE_POLICY_DELIVERY_LIMIT", "10"),
    ("QUEUE_POLICY_MESSAGE_TTL_MS", "86400000"),
    ("QUEUE_POLICY_MAX_LENGTH", "100000"),
    ("QUEUE_POLICY_EXPIRES_MS", "86400000"),
]


class TestNumericValuesRenderAsIntegers:
    """No scientific notation, on the defaults or on an overridden value."""

    @pytest.mark.parametrize(
        "name, expected", NUMERIC_ENV_DEFAULTS, ids=[n for n, _ in NUMERIC_ENV_DEFAULTS]
    )
    def test_default_renders_as_an_integer_string(self, hook_env, name, expected):
        assert hook_env[name] == expected

    def test_an_overridden_large_value_renders_as_an_integer_string(self):
        env = _literal_env("policyHook.policy.definition.messageTtlMs=604800000")

        assert env["QUEUE_POLICY_MESSAGE_TTL_MS"] == "604800000"

    def test_job_level_deadline_renders_as_an_integer(self):
        # A literal on purpose: this is a pin on the shipped chart DEFAULT, so
        # reading it back out of values.yaml would assert nothing. What the
        # number has to be compatible with is asserted in
        # TestRetryBudgetFitsTheJobDeadline.
        assert _hook_job()["spec"]["activeDeadlineSeconds"] == 180

    def test_job_level_backoff_limit_renders_as_an_integer(self):
        # 1, i.e. two pod attempts in total.
        assert _hook_job()["spec"]["backoffLimit"] == 1

    def test_ttl_after_finished_renders_as_an_integer(self):
        assert _hook_job()["spec"]["ttlSecondsAfterFinished"] == 900

    def test_zero_ttl_after_finished_omits_the_field(self):
        # 0 means "keep the pod for its logs", not "delete immediately".
        spec = _hook_job("policyHook.ttlSecondsAfterFinished=0")["spec"]

        assert "ttlSecondsAfterFinished" not in spec


# Kubernetes' initial back-off between failed pod attempts of a Job. It doubles
# from there, but the deadline below is reached long before a third attempt
# could be scheduled, so the first interval is the only one that counts.
POD_RETRY_BACKOFF_SECONDS = 10


def _script_worst_case_seconds() -> float:
    """Wall clock the apply module can burn in a single pod, worst case.

    Derived from the module's OWN constants rather than re-typed here, so the
    chart-side deadline is the only fixed number in the comparison and a retuned
    retry budget moves this with it.
    """
    per_attempt = (
        apply_queue_policy._TIMEOUT.connect_timeout
        + apply_queue_policy._TIMEOUT.read_timeout
    )
    attempts = apply_queue_policy._MAX_ATTEMPTS
    # tenacity waits BETWEEN attempts, not after the last one, and
    # `wait_random_exponential(max=...)` caps each of those waits.
    per_call = (
        attempts * per_attempt
        + (attempts - 1) * apply_queue_policy._MAX_BACKOFF_SECONDS
    )
    # The GET fails open and falls through to the PUT (see `policy_is_current`),
    # so the two calls' budgets add rather than sharing one.
    return 2 * per_call


class TestRetryBudgetFitsTheJobDeadline:
    """The script's retry budget and the Job's deadline are one number, split.

    `activeDeadlineSeconds` exists so a broker outage fails the release instead
    of hanging it. But it kills the pod with a bare `DeadlineExceeded`, which
    says nothing about WHY the broker would not answer; the script's own logged
    error is the only useful diagnostic, and an operator sees it only if the
    script can spend its ENTIRE retry budget and still report before the
    deadline lands. Either side can be retuned without the other noticing, and
    the mismatch stays invisible until the day a broker is actually down --
    which is the one day the diagnostic is needed. So the relationship is
    asserted here rather than left to the comments in values.yaml.
    """

    def test_one_pod_run_finishes_inside_the_deadline(self):
        deadline = _hook_job()["spec"]["activeDeadlineSeconds"]

        assert _script_worst_case_seconds() < deadline

    def test_every_pod_attempt_fits_inside_the_deadline(self):
        # The deadline spans ALL pod attempts, so backoffLimit buys nothing
        # unless the deadline covers the retries too -- the extra attempts are
        # simply cut off mid-run, and the last one dies without reporting.
        spec = _hook_job()["spec"]
        attempts = spec["backoffLimit"] + 1
        elapsed = (
            attempts * _script_worst_case_seconds()
            + (attempts - 1) * POD_RETRY_BACKOFF_SECONDS
        )

        assert elapsed <= spec["activeDeadlineSeconds"]


class TestJobPodContract:
    """The Job runs the service image with the apply module as its entrypoint."""

    def test_entrypoint_runs_the_apply_module(self):
        # The production image is distroless, so there is no shell to wrap this.
        assert _hook_container()["command"] == [
            "python",
            "-u",
            "-m",
            "public_scripts.apply_queue_policy",
        ]

    def test_pod_does_not_restart_on_failure(self):
        # backoffLimit governs retries; a restarting pod would bypass it.
        pod_spec = _hook_job()["spec"]["template"]["spec"]

        assert pod_spec["restartPolicy"] == "Never"

    def test_service_account_token_is_not_mounted(self):
        # The Job talks only to the broker; it needs no Kubernetes API access.
        pod_spec = _hook_job()["spec"]["template"]["spec"]

        assert pod_spec["automountServiceAccountToken"] is False

    def test_job_name_does_not_collide_with_the_smoke_test_job(self):
        policy_name = _hook_job()["metadata"]["name"]
        smoke = _documents(
            _render(
                "policyHook.enabled=false",
                "tests.enabled=true",
                show_only=SMOKE_TEST_TEMPLATE,
            )
        )[0]

        assert policy_name != smoke["metadata"]["name"]

    def test_job_name_does_not_collide_with_the_deployment(self):
        deployment = _documents(
            _render(*REQUIRED_SET, show_only="templates/deployment.yaml")
        )[0]

        assert _hook_job()["metadata"]["name"] != deployment["metadata"]["name"]

    def test_a_long_release_name_stays_within_the_job_name_limit(self):
        # The helper truncates at 63; a longer name is rejected by the API
        # server at install time, which is a broken release, not a warning.
        name = _hook_job(release=LONG_RELEASE)["metadata"]["name"]

        assert len(name) <= MAX_JOB_NAME_LENGTH

    def test_a_truncated_job_name_is_still_a_valid_kubernetes_name(self):
        # Truncation can land on the separating "-", which would be rejected;
        # the helper trims a trailing dash for exactly that reason.
        name = _hook_job(release=LONG_RELEASE)["metadata"]["name"]

        assert DNS_1123_SUBDOMAIN.match(name)

    def test_resources_come_from_the_hooks_own_values(self):
        # The override is what makes this a path check: a template that read
        # `.Values.resources` instead would render the service's block and
        # ignore this entirely, with nothing else in the chart noticing.
        container = _hook_container("policyHook.resources.requests.cpu=17m")

        assert container["resources"]["requests"]["cpu"] == "17m"

    def test_resources_are_not_the_services_own_request(self):
        # The Job makes two HTTP calls and exits; inheriting the service's much
        # larger request would have every deploy wait on that much schedulable
        # room for a pod that needs none of it.
        deployment_container = _documents(
            _render(*REQUIRED_SET, show_only="templates/deployment.yaml")
        )[0]["spec"]["template"]["spec"]["containers"][0]

        assert _hook_container()["resources"] != deployment_container["resources"]

    @pytest.mark.parametrize(
        "field, override, expected",
        [
            ("nodeSelector", "nodeSelector.disktype=ssd", {"disktype": "ssd"}),
            ("tolerations", "tolerations[0].key=broker", [{"key": "broker"}]),
            (
                "affinity",
                "affinity.nodeAffinity.preferred=zone-a",
                {"nodeAffinity": {"preferred": "zone-a"}},
            ),
        ],
        ids=["nodeSelector", "tolerations", "affinity"],
    )
    def test_pod_scheduling_values_reach_the_hook_pod(self, field, override, expected):
        # The hook pod runs the same image as the service, so wherever the
        # service can be scheduled the hook must be schedulable too -- on a
        # tainted or GPU-labelled node pool a hook that ignored these would sit
        # Pending until activeDeadlineSeconds failed the release.
        pod_spec = _hook_job(override)["spec"]["template"]["spec"]

        assert pod_spec[field] == expected

    @pytest.mark.parametrize(
        "field", ["nodeSelector", "tolerations", "affinity"], ids=str
    )
    def test_unset_scheduling_values_render_no_empty_block(self, field):
        # `with` guards the whole block: an empty `affinity: {}` reaching the
        # API server is noise at best and a schema rejection at worst.
        pod_spec = _hook_job()["spec"]["template"]["spec"]

        assert field not in pod_spec


def _entrypoint_module_path() -> PurePosixPath:
    """The repo path of the module the rendered Job's `command` actually runs.

    Read out of the render rather than hardcoded, so renaming the module or
    retargeting the command moves this path with it.
    """
    command = _hook_container()["command"]
    assert "-m" in command, f"the entrypoint does not run a module: {command}"
    module = command[command.index("-m") + 1]
    return PurePosixPath(*module.split(".")).with_suffix(".py")


def _dockerignore_patterns() -> list[str]:
    lines = DOCKERIGNORE.read_text().splitlines()
    return [
        stripped
        for line in lines
        if (stripped := line.strip()) and not stripped.startswith("#")
    ]


def _pattern_matches(pattern: str, path: PurePosixPath) -> bool:
    """Whether a `.dockerignore` pattern matches `path`, by Docker's rules.

    Matching is segment-wise, and a pattern that matches a PARENT of the path
    takes the whole tree below it -- that second half is the trap the rules
    exist to avoid, so it is modelled here rather than assumed away.
    """
    pattern_parts = PurePosixPath(pattern).parts
    path_parts = path.parts
    if len(pattern_parts) > len(path_parts):
        return False
    return all(
        fnmatch.fnmatchcase(actual, expected)
        for expected, actual in zip(pattern_parts, path_parts)
    )


def _matching_patterns(path: PurePosixPath) -> list[str]:
    """Every `.dockerignore` pattern that matches `path`, in file order."""
    return [
        pattern
        for pattern in _dockerignore_patterns()
        if _pattern_matches(pattern.removeprefix("!"), path)
    ]


def _last_matching_pattern(path: PurePosixPath) -> str | None:
    """The pattern that decides `path`: the last one to match it, if any."""
    matching = _matching_patterns(path)
    return matching[-1] if matching else None


def _third_party_imports(path: Path) -> set[str]:
    """Top-level import names in `path` that are not standard library.

    Read out of the source rather than listed here, so an import added to the
    entrypoint later is covered without anyone remembering to add it.
    """
    tree = ast.parse(path.read_text())
    roots: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            roots.update(alias.name.split(".")[0] for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            roots.add(node.module.split(".")[0])
    return {
        root
        for root in roots
        if root not in sys.stdlib_module_names
        and root not in ("app", "scripts", "public_scripts")
    }


def _normalized(name: str) -> str:
    """A distribution name in PEP 503 comparison form."""
    return re.sub(r"[-_.]+", "-", name).lower()


# The distribution name at the head of a PEP 508 requirement string, before any
# extras, version specifier, or environment marker.
REQUIREMENT_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*")


def _declared_distributions() -> set[str]:
    """Normalized distribution names in `[project].dependencies`."""
    project = tomllib.loads(PYPROJECT.read_text())["project"]
    return {
        _normalized(REQUIREMENT_NAME.match(requirement).group())
        for requirement in project["dependencies"]
    }


def _providing_distributions(import_name: str) -> set[str]:
    """Distributions that could provide a top-level import name.

    Nearly all map one-to-one; `packages_distributions()` covers the ones that
    do not (`yaml` -> `pyyaml`), and the import name itself is the fallback when
    the metadata knows nothing about it.
    """
    providers = importlib.metadata.packages_distributions().get(import_name)
    return {_normalized(name) for name in providers or [import_name]}


class TestEntrypointShipsInTheImage:
    """The Job's entrypoint module must survive the Docker build context.

    The image ships app code by `COPY . /code`, so `.dockerignore` decides
    whether the module exists at runtime at all. Nothing else in the build
    notices its absence: the image builds clean and the Job dies on `No module
    named public_scripts.apply_queue_policy` on the first `helm upgrade` that
    runs it.

    The module earns this by living OUTSIDE `scripts/`, which is excluded
    wholesale. That separation is the thing being defended: a re-include cannot
    substitute for it, because Docker prunes an excluded directory before any
    negation below it is consulted.
    """

    def test_the_entrypoint_module_exists_in_the_repo(self):
        # Catches a rename or a typo in the template's `command` on its own.
        assert (REPO_ROOT / _entrypoint_module_path()).is_file()

    def test_the_entrypoint_module_is_not_ignored_by_the_build_context(self):
        # Nothing in `.dockerignore` reaches this path, so no pattern decides
        # it and it lands in the build context by default. A re-include would
        # also do (the last pattern to match wins), which is why the assertion
        # is on the DECISION rather than on the absence of a match.
        deciding = _last_matching_pattern(_entrypoint_module_path())

        assert deciding is None or deciding.startswith("!"), (
            f"{_entrypoint_module_path()} is excluded from the Docker build "
            f"context by {deciding!r}; the hook Job cannot import it"
        )

    def test_no_parent_directory_of_the_entrypoint_is_excluded(self):
        # The hazard the layout exists to avoid: Docker skips an excluded
        # directory WHOLESALE, before any negation underneath it is consulted,
        # so a module moved back under `scripts/` (or under any other excluded
        # parent) would silently vanish from the image no matter what
        # re-include followed. Every parent must therefore go unmatched.
        parents = [
            parent for parent in _entrypoint_module_path().parents if str(parent) != "."
        ]
        excluded = {
            str(parent): matching
            for parent in parents
            if (matching := _matching_patterns(parent))
        }

        assert excluded == {}, (
            f"{_entrypoint_module_path()} sits under a directory the Docker "
            f"build context drops: {excluded}; the re-include of the module "
            f"itself would never be consulted"
        )

    def test_the_other_development_scripts_stay_out_of_the_image(self):
        # The other half of the separation: `scripts/` is dev/manual tooling
        # and has to stay out of a production image. Anchored on the literal
        # directory rather than on the entrypoint's neighbours, since the
        # entrypoint no longer lives there.
        deciding = _last_matching_pattern(DEV_SCRIPT)

        assert deciding is not None and not deciding.startswith("!")

    def test_the_entrypoint_does_not_live_with_the_development_scripts(self):
        # States the layout invariant directly, so moving the module back under
        # `scripts/` fails on a readable message rather than only through the
        # ignore-rule arithmetic above.
        assert _entrypoint_module_path().parts[0] != DEV_SCRIPT.parts[0]

    def test_every_third_party_import_is_a_declared_dependency(self):
        # The image installs `uv export --no-dev --locked`, so only the resolved
        # dependency set reaches it. An import that is merely transitive today
        # (certifi arrives under httpx/requests/sentry-sdk) works everywhere in
        # dev and vanishes from the image the moment its carrier drops it --
        # surfacing as `ModuleNotFoundError` in the hook Job, which fails the
        # release. A direct declaration is the only thing that holds.
        declared = _declared_distributions()
        imports = _third_party_imports(REPO_ROOT / _entrypoint_module_path())
        undeclared = {
            name for name in imports if not _providing_distributions(name) & declared
        }

        assert undeclared == set(), (
            f"{sorted(undeclared)} imported by {_entrypoint_module_path()} but not "
            f"in [project].dependencies of pyproject.toml; the hook Job would "
            f"fail at import if the transitive path to them ever disappeared"
        )

    def test_the_import_scan_sees_the_entrypoints_dependencies(self):
        # Guards the check above from passing for the wrong reason: a scan that
        # silently found nothing would report nothing undeclared.
        imports = _third_party_imports(REPO_ROOT / _entrypoint_module_path())

        assert imports >= KNOWN_ENTRYPOINT_IMPORTS


def _value_from(container: dict, name: str) -> dict:
    rows = [row for row in container["env"] if row["name"] == name]
    assert len(rows) == 1, f"expected one {name} row, got {len(rows)}"
    return rows[0]["valueFrom"]


class TestCredentialAndCrossCheckWiring:
    """The admin credential and the AMQP URI arrive by reference, never inline."""

    def test_username_comes_from_the_configured_secret(self):
        secret_ref = _value_from(_hook_container(), "QUEUE_POLICY_MANAGEMENT_USERNAME")[
            "secretKeyRef"
        ]

        assert secret_ref["name"] == "rmq-policy-admin"

    def test_username_key_is_configurable(self):
        container = _hook_container(
            "policyHook.managementApi.existingSecret.usernameKey=admin-user"
        )
        secret_ref = _value_from(container, "QUEUE_POLICY_MANAGEMENT_USERNAME")

        assert secret_ref["secretKeyRef"]["key"] == "admin-user"

    def test_password_key_is_configurable(self):
        container = _hook_container(
            "policyHook.managementApi.existingSecret.passwordKey=admin-pass"
        )
        secret_ref = _value_from(container, "QUEUE_POLICY_MANAGEMENT_PASSWORD")

        assert secret_ref["secretKeyRef"]["key"] == "admin-pass"

    def test_password_is_never_rendered_as_a_literal(self, hook_env):
        assert "QUEUE_POLICY_MANAGEMENT_PASSWORD" not in hook_env

    def test_amqp_uri_is_supplied_for_the_vhost_cross_check(self):
        # Without this row the Job's vhost cross-check silently no-ops and
        # "applied successfully to the wrong vhost" goes undetected.
        secret_ref = _value_from(_hook_container(), "OMNI_EVENTS_RABBITMQ_URI")

        assert secret_ref["secretKeyRef"]["key"] == "rabbitmq_uri"
