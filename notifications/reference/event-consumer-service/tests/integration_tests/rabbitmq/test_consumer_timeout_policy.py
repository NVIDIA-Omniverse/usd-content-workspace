# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
END-TO-END verification of the consumer-timeout policy (PRD AC 5, issue b.9gj).

AC 5's chain has four links and, until this module, nothing tested more than one
of them at a time:

1. The chart renders ``policyHook.policy.definition.consumerTimeoutMs`` into the
   hook Job's environment -- covered by ``tests/helm/``.
2. ``public_scripts/apply_queue_policy.py`` turns that into a
   ``consumer-timeout`` key in a management-API policy PUT -- covered by
   ``tests/public_scripts/test_apply_queue_policy.py``, against a fake pool.
3. RabbitMQ, given that policy, actually cancels a consumer that withholds an
   ack -- covered NOWHERE. It was an inference from the RabbitMQ docs, and the
   inference had already been wrong once: the previous implementation passed
   ``x-consumer-timeout`` as a ``basic.consume`` argument, where the broker
   silently ignores it. The whole issue is that a plausible-looking ack-timeout
   lever did nothing.
4. Our consume loop notices that cancellation and ends the client's stream with
   a reconnect signal instead of hanging -- covered by
   ``test_consumer_liveness.py``, but only for cancellations this test suite
   induced itself (a queue delete, a client-side ``basic.cancel``), never for
   one the broker decided on.

This module joins links 3 and 4 against a LIVE broker: it applies a real policy
through the real management API, runs the REAL consume loop, withholds the ack
the way a stuck client would, and waits for the broker to act.

Measured behaviour (RabbitMQ 4.0.5, quorum queue, ``consumer-timeout`` 60000)
----------------------------------------------------------------------------
- The broker enforces on a coarse tick, roughly once a minute, so where a 60 s
  timeout lands depends on how the delivery fell against that tick: measured at
  60.0 s after delivery on one run and 120.2 s on another. That is why
  ``ENFORCEMENT_TIMEOUT_S`` is minutes wide and why this arm is opt-in.
- The enforcement is a CHANNEL CLOSE carrying exactly the documented wire
  message -- ``PRECONDITION_FAILED - delivery acknowledgement on channel 1
  timed out. Timeout value used: 60000 ms`` -- and the broker requeues the
  withheld delivery as part of it (the queue goes to ``messages_ready`` 1,
  ``messages_unacknowledged`` 0). The echoed ``60000`` is the strongest
  available evidence that the value THIS module put in the policy is the value
  the broker enforced.
- aio-pika's ``RobustChannel`` reopens that channel IN PLACE ~75 ms later. Our
  liveness poll runs on a ~1 s cadence, so by the time it looks, ``is_closed``
  reads False again on a fresh underlay with an empty consumer registry. The
  loss is still detected, and it is classified off the channel INCARNATION
  pinned at consume start rather than off that misleading reading, so the
  reopen does not disguise the death:
  ``ConsumerCancelledError(channel_closed=True)``. See
  ``test_the_error_records_the_channel_death_through_the_reopen``, which
  asserts both halves -- the classification and the reopen it has to survive.

Requirements and why this arm is opt-in
---------------------------------------
- It needs the RabbitMQ MANAGEMENT HTTP API, which is the only reach that
  applies a policy without node access (the same reason
  ``apply_queue_policy.py`` uses it). CI's ``integration:`` job therefore runs
  the ``rabbitmq:4.1.3-management`` image rather than the plain one, which
  ships the plugin disabled -- on the bare image this whole module skips and
  the job goes green having never exercised it. A local broker without the
  plugin skips the module the same way.
- The slow arm additionally waits out the broker's enforcement tick -- minutes,
  not seconds -- so it is gated behind ``OMNI_EVENTS_RUN_SLOW_LIVE_TESTS`` and
  skips by default rather than adding minutes to every ``pytest tests/`` run.
  The FAST arm below is deliberately structured to survive that skip: it still
  applies the policy, still declares the quorum queue, still proves the policy
  landed on it, and still proves the real consume loop delivered through it.

Blast radius
------------
The policy this module creates is scoped by pattern to its own uuid-named queue
and nothing else, is given a priority above any deployed policy so an
unforeseen pattern overlap would resolve in its favour rather than silently
against it, and is deleted in teardown. ``TestTheNarrowPolicyIsNarrow`` asserts
that every pre-existing policy on the broker -- ``event-queue-limits`` on a real
deployment -- came through untouched, so a future edit that widened the pattern
would fail here instead of retuning a production policy.

Scope notes
-----------
- The autouse fakes from the parent conftests are bypassed the same way
  ``test_consumer_liveness.py`` bypasses them: the connection comes from
  ``class_live_broker_connection`` and the consume generator is the pre-patch
  reference captured at import.
- Fixtures are CLASS-scoped with matching ``loop_scope`` so one live episode
  serves every assertion in its class. See ``test_consumer_liveness.py`` for the
  measured caveat about xdist's ``--dist load`` rebuilding class fixtures per
  test -- it matters more here than anywhere, since the slow episode is minutes
  long. Run the slow arm with ``-n0`` so the episode is driven once.
"""

import asyncio
import json
import logging
import os
import urllib.error
import urllib.parse
import urllib.request
import uuid
from types import SimpleNamespace

import aio_pika
import pytest
import pytest_asyncio
from notification_common.config.common_settings import common_settings

from app.config import settings
from app.lib.rabbitmq.ack_tracker import AckTracker
from app.lib.rabbitmq.rabbitmq_base import ConsumerCancelledError, RabbitMQBase

# Captured at import time, before the autouse ``mock_queue_iterator`` fixture in
# ``tests/integration_tests/conftest.py`` replaces the attribute with a canned
# generator. Calling this reference runs the REAL consume loop.
_REAL_CONSUME_WITH_CLIENT_ACKS = RabbitMQBase._consume_with_client_acks

# The opt-in for the minutes-long enforcement arm. Unset by default so a plain
# `uv run pytest tests/` stays fast; set it to run the end-to-end verification:
#   OMNI_EVENTS_RUN_SLOW_LIVE_TESTS=1 uv run pytest \
#       tests/integration_tests/rabbitmq/test_consumer_timeout_policy.py -n0
_SLOW_ARM_ENV = "OMNI_EVENTS_RUN_SLOW_LIVE_TESTS"

# Management-API overrides. Both default to values derived from the AMQP URI the
# service itself uses, so a rig that set OMNI_EVENTS_RABBITMQ_URI needs no
# further configuration; the overrides exist for a managed broker that serves
# its management API from a different host or credential (the same case
# `apply_queue_policy.cross_check_target` warns about).
_MANAGEMENT_URL_ENV = "OMNI_EVENTS_RABBITMQ_MANAGEMENT_URL"
_MANAGEMENT_USERNAME_ENV = "OMNI_EVENTS_RABBITMQ_MANAGEMENT_USERNAME"
_MANAGEMENT_PASSWORD_ENV = "OMNI_EVENTS_RABBITMQ_MANAGEMENT_PASSWORD"

DEFAULT_MANAGEMENT_PORT = 15672

# Generous: the management API's queue endpoint is served off the stats
# collector, which has been measured taking seconds under load on a busy host.
MANAGEMENT_TIMEOUT_S = 30.0

# Transport-level retries for a management call, mirroring the retry budget
# `apply_queue_policy._request` gives the same API in production. A container
# rig reaching a broker across a host gateway sees occasional transient
# unreachability (observed as `OSError: [Errno 101] Network is unreachable` on a
# host that answered a second earlier), and without a retry a blip like that
# fails the run for a reason that has nothing to do with the code under test.
MANAGEMENT_ATTEMPTS = 3
MANAGEMENT_RETRY_DELAY_S = 1.0

# The ack timeout under test, in the units the policy takes. Held AT RabbitMQ's
# documented 60000 ms minimum -- the same floor `apply_queue_policy
# ._require_consumer_timeout` refuses to go below -- because a shorter one is
# unsupported and a longer one only makes this arm slower.
CONSUMER_TIMEOUT_MS = 60000

# Every policy this module creates is named with this prefix, which is also how
# the blast-radius snapshots tell "a policy this module owns" from "a policy that
# was already here". That distinction has to cover SIBLINGS, not just this rig's
# own policy: under xdist another worker runs this same module concurrently and
# its policy is live on the broker at the same time.
POLICY_NAME_PREFIX = "ctimeout-policy-"

# Above any deployed policy. Only one policy applies to a queue (the
# highest-priority match), so if a future deployment shipped a pattern that
# happened to cover this module's queue, a lower priority would leave the test
# silently measuring THAT policy instead of its own.
POLICY_PRIORITY = 100

# How long to wait for the management API to report the policy as effective on
# the queue. The broker applies it in well under a second; this budget is for
# the stats collector that the queue endpoint reads from, measured at ~5 s.
POLICY_EFFECTIVE_TIMEOUT_S = 60.0
POLICY_EFFECTIVE_POLL_S = 1.0

# Held AT the settings floor (``consumer_liveness_poll_seconds`` is ``ge=1.0``)
# so the detection latency measured after the broker's enforcement is one a real
# deployment could configure.
TEST_POLL_SECONDS = 1.0

# The ceiling on the broker acting. Enforcement runs on a coarse tick rather
# than a per-delivery timer, so where it lands depends on how the delivery fell
# against that tick: measured at 60.0 s on one run and 120.2 s on another for
# the same 60 s timeout, making the worst case the timeout plus a full tick.
# Bounded well above that so a loaded broker cannot flake it, and bounded at all
# so a policy that never bites fails as a NAMED timeout instead of hanging.
ENFORCEMENT_TIMEOUT_S = 240.0

# Slack on the "it waited out the configured timeout" lower bound. The broker's
# clock starts when it hands the delivery to the socket; ours starts when the
# message comes back out of the consume generator, tens of milliseconds later.
# When the enforcement tick happens to land right on the timeout -- measured at
# exactly 60.0 s on one run against 120.2 s on another -- that skew makes our
# figure fractionally SHORTER than the broker's, so an exact >= 60 s comparison
# would fail on clock skew rather than on behaviour. Two seconds absorbs the
# skew while staying nowhere near anything else that could take a consumer away.
ENFORCEMENT_CLOCK_SKEW_S = 2.0

# The budget for our loop to notice an enforcement the broker already made. One
# poll plus the classifying round trip; several times that is a wide margin that
# still fails a fix which only terminated "eventually".
PROMPT_DETECTION_BUDGET_S = 8.0

# The non-vacuity control: how long to wait for the withheld message to arrive
# through the real generator in the first place.
BASELINE_TIMEOUT_S = 30.0

BASELINE_BODY = b'{"event_type": "ack-timeout-probe", "message": {"test": "m"}}'


class ManagementApiUnavailable(Exception):
    """The RabbitMQ management HTTP API could not be reached or used."""


class ConsumerTimeoutRig(SimpleNamespace):
    """The live queue, policy and stream this module measures against."""


class EnforcementEpisode(SimpleNamespace):
    """The measured record of one broker ack-timeout enforcement."""


def _slow_arm_enabled() -> bool:
    """True when the environment opts in to the minutes-long enforcement arm."""
    return os.environ.get(_SLOW_ARM_ENV, "").strip().lower() in (
        "1",
        "true",
        "yes",
        "on",
    )


def _management_credentials() -> tuple[str, str, str, str]:
    """
    The (base URL, vhost, username, password) to drive the management API with.

    Derived from the AMQP URI the service consumes from, so the policy is aimed
    at the same broker and the same vhost the streams under test live on -- the
    cross-check ``apply_queue_policy.cross_check_target`` performs in
    production, done here by construction rather than by assertion.
    """
    parsed = urllib.parse.urlsplit(common_settings.rabbitmq_uri)
    path = parsed.path.lstrip("/")
    vhost = urllib.parse.unquote(path) if path else "/"
    base_url = os.environ.get(_MANAGEMENT_URL_ENV, "").strip() or (
        f"http://{parsed.hostname or 'localhost'}:{DEFAULT_MANAGEMENT_PORT}"
    )
    username = (
        os.environ.get(_MANAGEMENT_USERNAME_ENV, "").strip()
        or parsed.username
        or "guest"
    )
    password = (
        os.environ.get(_MANAGEMENT_PASSWORD_ENV, "").strip()
        or parsed.password
        or "guest"
    )
    return base_url.rstrip("/"), vhost, username, password


def _management_call(method: str, path: str, body=None) -> tuple[int, object]:
    """
    One management-API call, returning ``(status, parsed body)``.

    A non-2xx status is RETURNED rather than raised, because a non-200 is a
    meaningful answer here: 404 means the policy does not exist yet, and 401
    means the credentials are wrong -- both are things the caller reports
    precisely rather than retries. Only a failure to REACH the API raises
    :class:`ManagementApiUnavailable`.
    """
    base_url, _vhost, username, password = _management_credentials()
    request = urllib.request.Request(
        f"{base_url}{path}",
        method=method,
        data=None if body is None else json.dumps(body).encode(),
        headers={"content-type": "application/json"},
    )
    password_manager = urllib.request.HTTPPasswordMgrWithDefaultRealm()
    password_manager.add_password(None, base_url, username, password)
    opener = urllib.request.build_opener(
        urllib.request.HTTPBasicAuthHandler(password_manager)
    )
    try:
        with opener.open(request, timeout=MANAGEMENT_TIMEOUT_S) as response:
            raw = response.read()
            return response.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as exc:
        raw = exc.read()
        return exc.code, (json.loads(raw) if raw else None)
    # URLError (unreachable host, refused connection, DNS) and read timeouts are
    # both OSError subclasses, and HTTPError is caught above, so what reaches
    # here is always "could not reach the API" rather than "the API said no".
    except OSError as exc:
        raise ManagementApiUnavailable(
            f"{method} {path} could not reach the management API ({exc!r})"
        ) from exc


async def _management_request(method: str, path: str, body=None) -> tuple[int, object]:
    """:func:`_management_call` with a small retry budget for transport blips."""
    for attempt in range(MANAGEMENT_ATTEMPTS):
        try:
            return _management_call(method, path, body)
        except ManagementApiUnavailable:
            if attempt + 1 == MANAGEMENT_ATTEMPTS:
                raise
            await asyncio.sleep(MANAGEMENT_RETRY_DELAY_S)


async def _require_management_api() -> None:
    """Skip the module unless a usable management API is present."""
    base_url, _vhost, _username, _password = _management_credentials()
    try:
        status, _body = await _management_request("GET", "/api/overview")
    except ManagementApiUnavailable as exc:
        pytest.skip(
            f"RabbitMQ management API unreachable at {base_url!r} after "
            f"{MANAGEMENT_ATTEMPTS} attempts: {exc}. This module applies a real "
            "broker policy, which has no AMQP equivalent, so it cannot run "
            "without it. Enable the management plugin (in CI, run the "
            "integration job against rabbitmq:<version>-management) or point "
            f"{_MANAGEMENT_URL_ENV} at a broker that has it; remove this skip "
            "once that is the default."
        )
    if status != 200:
        pytest.skip(
            f"RabbitMQ management API at {base_url!r} answered HTTP {status} for "
            "/api/overview; this module needs a working management API, and "
            "credentials that may administer policies, to apply its policy. "
            f"Override them with {_MANAGEMENT_USERNAME_ENV} / "
            f"{_MANAGEMENT_PASSWORD_ENV}."
        )


async def _policy_fingerprints() -> dict:
    """Every policy on the vhost, keyed by name, as comparable JSON."""
    _base_url, vhost, _username, _password = _management_credentials()
    status, policies = await _management_request(
        "GET", f"/api/policies/{urllib.parse.quote(vhost, safe='')}"
    )
    if status != 200 or not isinstance(policies, list):
        raise ManagementApiUnavailable(
            f"listing policies returned HTTP {status}: {policies!r}"
        )
    return {
        policy["name"]: {
            key: policy.get(key)
            for key in ("pattern", "apply-to", "priority", "definition")
        }
        for policy in policies
    }


def _foreign_policies(fingerprints: dict) -> dict:
    """
    The policies this module does NOT own -- the set it must leave untouched.

    Every ``POLICY_NAME_PREFIX`` policy is dropped, not merely this rig's own:
    under xdist a sibling worker running this same module has its policy live on
    the broker at the same time, and a raw before/after comparison would then
    fail on a neighbour's presence rather than on any damage this module did.
    """
    return {
        name: fingerprint
        for name, fingerprint in fingerprints.items()
        if not name.startswith(POLICY_NAME_PREFIX)
    }


async def _await_effective_policy(queue_name: str, policy_name: str):
    """
    Poll the queue's management record until it reports our policy in force.

    Returns ``(policy, effective_policy_definition)`` as last seen, so a failing
    assertion reports what the broker really said rather than just "not yet".
    The poll is necessary because the queue endpoint is served from the stats
    collector, which lags the policy application by seconds -- the broker itself
    applies it immediately.
    """
    _base_url, vhost, _username, _password = _management_credentials()
    path = (
        f"/api/queues/{urllib.parse.quote(vhost, safe='')}/"
        f"{urllib.parse.quote(queue_name, safe='')}"
    )
    loop = asyncio.get_running_loop()
    deadline = loop.time() + POLICY_EFFECTIVE_TIMEOUT_S
    applied_policy = None
    definition = None
    while True:
        status, record = await _management_request("GET", path)
        if status == 200 and isinstance(record, dict):
            applied_policy = record.get("policy")
            definition = record.get("effective_policy_definition")
            if applied_policy == policy_name and definition:
                return applied_policy, definition
        if loop.time() >= deadline:
            return applied_policy, definition
        await asyncio.sleep(POLICY_EFFECTIVE_POLL_S)


async def _consumer_tags_on(channel) -> set:
    """The consumer tags the live channel's aiormq underlay still carries."""
    try:
        underlay = await channel.get_underlay_channel()
    except aio_pika.exceptions.ChannelInvalidStateError:
        return set()
    return set(underlay.consumers)


async def _drive_to_termination(generator) -> SimpleNamespace:
    """
    Step the generator once more and record how, and when, it ended.

    Records rather than asserts, so the expensive live episode runs once per
    class and each test reads a field (the record-then-assert pattern of
    ``test_consumer_liveness.py``).
    """
    loop = asyncio.get_running_loop()
    try:
        record = await asyncio.wait_for(anext(generator), timeout=ENFORCEMENT_TIMEOUT_S)
    except asyncio.TimeoutError:
        return SimpleNamespace(
            error=None, record=None, timed_out=True, ended_at=loop.time()
        )
    except StopAsyncIteration:
        return SimpleNamespace(
            error=None, record=None, timed_out=False, ended_at=loop.time()
        )
    # Any other exception IS the observation under test, so it is recorded
    # rather than allowed to error the fixture.
    except Exception as exc:  # noqa: BLE001
        return SimpleNamespace(
            error=exc, record=None, timed_out=False, ended_at=loop.time()
        )
    return SimpleNamespace(
        error=None, record=record, timed_out=False, ended_at=loop.time()
    )


async def _best_effort_delete_queue(connection, queue_name) -> None:
    """Delete a test-created queue on its own channel, ignoring failures."""
    try:
        channel = await connection.channel()
    except Exception:  # noqa: BLE001 - cleanup is best-effort
        return
    try:
        await channel.queue_delete(queue_name)
    except Exception:  # noqa: BLE001 - cleanup is best-effort
        logging.getLogger(__name__).debug(
            "Best-effort cleanup could not delete queue %r", queue_name, exc_info=True
        )
    finally:
        try:
            await channel.close()
        except Exception:  # noqa: BLE001 - cleanup is best-effort
            pass


@pytest_asyncio.fixture(scope="class", loop_scope="class")
async def consumer_timeout_rig(class_live_broker_connection, class_mocker):
    """
    A quorum queue under a real ``consumer-timeout`` policy, with a live stream.

    Steps, all against the live broker:

    1. Record every policy already on the vhost, then PUT a narrow one --
       pattern anchored to this rig's uuid-named queue, priority above anything
       deployed, definition ``{"consumer-timeout": CONSUMER_TIMEOUT_MS}``.
    2. Declare that queue as a QUORUM queue, matching what
       ``RabbitMQBase._create_queue`` declares for durable consumers; the policy
       lever is only interesting for the queue type production actually uses.
    3. Wait for the management API to report the policy in force on the queue.
       Without this the whole module could pass vacuously against a policy that
       never applied.
    4. Start the REAL ``_consume_with_client_acks``, publish one message, and
       require it to come out of the generator -- the non-vacuity control. It is
       then NEVER acked, which is exactly the stuck client the timeout exists
       for: ``_consume_with_client_acks`` hands acking to the client, so simply
       not acking is the production-shaped fault, not a simulation of one.

    Yields the rig; teardown closes the generator, deletes the queue and deletes
    the policy.
    """
    await _require_management_api()
    connection = class_live_broker_connection

    class_mocker.patch.object(
        settings.settings, "consumer_liveness_poll_seconds", TEST_POLL_SECONDS
    )
    class_mocker.patch.object(RabbitMQBase.connector, "_connection", connection)
    class_mocker.patch.object(RabbitMQBase.connector, "closing", False)

    suffix = uuid.uuid4()
    queue_name = f"ctimeout-q-{suffix}"
    policy_name = f"{POLICY_NAME_PREFIX}{suffix}"
    _base_url, vhost, _username, _password = _management_credentials()
    policy_path = (
        f"/api/policies/{urllib.parse.quote(vhost, safe='')}/"
        f"{urllib.parse.quote(policy_name, safe='')}"
    )
    desired_definition = {"consumer-timeout": CONSUMER_TIMEOUT_MS}

    consume_channel = None
    generator = None
    policy_created = False
    try:
        policies_before = _foreign_policies(await _policy_fingerprints())
        status, body = await _management_request(
            "PUT",
            policy_path,
            {
                # Anchored to this rig's queue and nothing else. Widening this
                # pattern would retune whatever else it matched, including a
                # deployed policy's queues.
                "pattern": f"^{queue_name}$",
                "apply-to": "queues",
                "priority": POLICY_PRIORITY,
                "definition": desired_definition,
            },
        )
        if status not in (200, 201, 204):
            pytest.skip(
                f"could not create the test policy (HTTP {status}: {body!r}); "
                "the management-API user needs policy-management rights on "
                f"vhost {vhost!r}"
            )
        policy_created = True

        consume_channel = await connection.channel()
        queue = await consume_channel.declare_queue(
            queue_name,
            durable=True,
            arguments={"x-queue-type": "quorum"},
            robust=False,
        )

        applied_policy, effective_definition = await _await_effective_policy(
            queue_name, policy_name
        )

        async def _authz(_message):
            return True

        generator = _REAL_CONSUME_WITH_CLIENT_ACKS(queue, _authz, AckTracker())

        await consume_channel.default_exchange.publish(
            aio_pika.Message(body=BASELINE_BODY), routing_key=queue_name
        )
        loop = asyncio.get_running_loop()
        try:
            baseline_record = await asyncio.wait_for(
                anext(generator), timeout=BASELINE_TIMEOUT_S
            )
        except asyncio.TimeoutError:
            pytest.fail(
                "the real consume generator never delivered the baseline "
                f"message within {BASELINE_TIMEOUT_S}s; the live rig is broken, "
                "so nothing this module measures would mean anything"
            )
        delivered_at = loop.time()

        policies_now = await _policy_fingerprints()
        yield ConsumerTimeoutRig(
            connection=connection,
            channel=consume_channel,
            queue=queue,
            queue_name=queue_name,
            policy_name=policy_name,
            desired_definition=desired_definition,
            applied_policy=applied_policy,
            effective_definition=effective_definition,
            policies_before=policies_before,
            policies_after=_foreign_policies(policies_now),
            our_policy=policies_now.get(policy_name),
            generator=generator,
            baseline_record=baseline_record,
            delivered_at=delivered_at,
        )
    except ManagementApiUnavailable as exc:
        # The API answered ``/api/overview`` a moment ago, so this is a
        # transport blip that outlasted MANAGEMENT_ATTEMPTS rather than a
        # missing management plugin. Skipping keeps it from reading as a
        # failure of the code under test, which is what an ERROR out of a
        # fixture would look like.
        pytest.skip(
            f"the management API became unreachable mid-rig ({exc}); this "
            "module cannot apply or verify its policy without it"
        )
    finally:
        if generator is not None:
            await generator.aclose()
        await _best_effort_delete_queue(connection, queue_name)
        if policy_created:
            # Broad on purpose: this is the last chance to take the policy back
            # off a possibly SHARED broker, so no failure mode here is worth
            # skipping the attempt over. It is still only best-effort -- a run
            # killed outright (a SIGPIPE from `pytest ... | head` will do it)
            # never reaches this at all, which is how a stray
            # `ctimeout-policy-<uuid>` can survive. Such a leftover is inert,
            # since its pattern is anchored to a queue that no longer exists,
            # but it should be deleted by hand.
            try:
                await _management_request("DELETE", policy_path)
            except Exception:  # noqa: BLE001 - cleanup is best-effort
                logging.getLogger(__name__).warning(
                    "Could not delete the test policy %r; it is scoped to the "
                    "deleted queue %r and is inert, but should be removed by "
                    "hand.",
                    policy_name,
                    queue_name,
                    exc_info=True,
                )
        if consume_channel is not None:
            try:
                await consume_channel.close()
            except Exception:  # noqa: BLE001 - cleanup is best-effort
                pass


class TestTheNarrowPolicyIsNarrow:
    """The test's own policy touched nothing that was already on the broker."""

    @pytest.mark.asyncio(loop_scope="class")
    async def test_no_other_policy_was_added_or_changed(self, consumer_timeout_rig):
        # A deployment carries `event-queue-limits` (pattern `^event-queue:`),
        # and reapplying a policy resets every matched queue's `expires` idle
        # clock -- so a test that widened its pattern or reused a deployed name
        # would do real damage on a shared broker. This is the guard that makes
        # that a red test rather than an incident.
        #
        # Both snapshots are `_foreign_policies` -- see there for why they
        # exclude every policy this module owns, not just this rig's own. What
        # is left is exactly the set this module must not disturb.
        rig = consumer_timeout_rig
        assert rig.policies_after == rig.policies_before

    @pytest.mark.asyncio(loop_scope="class")
    async def test_our_policy_is_anchored_to_this_rigs_queue_alone(
        self, consumer_timeout_rig
    ):
        # The narrowness itself, read back off the broker rather than trusted
        # from the request body: an anchored uuid pattern can match this rig's
        # queue and nothing else, which is what makes the priority, the
        # definition and the whole episode safe to run against a shared broker.
        rig = consumer_timeout_rig
        assert rig.our_policy == {
            "pattern": f"^{rig.queue_name}$",
            "apply-to": "queues",
            "priority": POLICY_PRIORITY,
            "definition": rig.desired_definition,
        }


class TestThePolicyReallyApplied:
    """
    The broker really put OUR ``consumer-timeout`` on the queue.

    This is the fast arm, and it is the one that survives when the slow arm is
    skipped. On its own it does not prove the broker acts on the value -- that
    is what ``TestTheBrokerEnforcesTheAckTimeout`` is for -- but it does close
    the gap the old ``x-consumer-timeout`` bug lived in: a lever that was
    accepted and then silently ignored. A ``consumer-timeout`` visible in the
    queue's own effective policy is the broker confirming it holds the setting.
    """

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_queue_reports_our_policy_as_the_effective_one(
        self, consumer_timeout_rig
    ):
        # Names the policy, not just "some policy": only the highest-priority
        # match applies, so a lower-priority test policy would leave the queue
        # governed by a deployed one and every measurement below would be about
        # that policy instead.
        assert consumer_timeout_rig.applied_policy == consumer_timeout_rig.policy_name

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_effective_definition_carries_the_consumer_timeout(
        self, consumer_timeout_rig
    ):
        assert consumer_timeout_rig.effective_definition == (
            consumer_timeout_rig.desired_definition
        ), (
            "the broker did not report consumer-timeout "
            f"{CONSUMER_TIMEOUT_MS} as effective on queue "
            f"{consumer_timeout_rig.queue_name!r}; it reported "
            f"{consumer_timeout_rig.effective_definition!r}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_real_stream_delivered_through_the_policied_queue(
        self, consumer_timeout_rig
    ):
        # Non-vacuity for the whole module: a consumer the broker cancels for
        # withholding an ack has to have been given something to withhold.
        assert consumer_timeout_rig.baseline_record.message["event_type"] == (
            "ack-timeout-probe"
        )


@pytest_asyncio.fixture(scope="class", loop_scope="class")
async def enforcement_episode(consumer_timeout_rig):
    """
    Withhold the ack and let the broker act, timing both sides.

    The BROKER's action is timed off the channel's close callback rather than by
    polling. That is not a convenience: the enforcement closes the channel and
    aio-pika's ``RobustChannel`` reopens it in place within ~75 ms, so a poll
    coarser than that would miss the close entirely and report a per-consumer
    cancel that never happened. The callback also hands over the broker's own
    wire message, which is what lets this module assert the timeout the broker
    enforced was the one the policy carried.

    OUR reaction is timed off the generator ending. Separating the two is what
    makes the claim checkable: "the broker enforced at T+120 s and our loop
    ended the stream 1.0 s later" is a different statement from "something ended
    the stream after two minutes", and only the first verifies AC 5.
    """
    rig = consumer_timeout_rig
    enforcement = SimpleNamespace(at=None, error=None)

    def _on_channel_close(_sender, exc):
        # First close only: the fixture teardown closes this channel too.
        if enforcement.at is None:
            enforcement.at = asyncio.get_running_loop().time()
            enforcement.error = exc

    rig.channel.close_callbacks.add(_on_channel_close)
    try:
        outcome = await _drive_to_termination(rig.generator)
    finally:
        rig.channel.close_callbacks.discard(_on_channel_close)

    episode = EnforcementEpisode(
        rig=rig,
        outcome=outcome,
        enforced_at=enforcement.at,
        enforcement_error=enforcement.error,
        tags_after=await _consumer_tags_on(rig.channel),
        channel_closed_after=rig.channel.is_closed,
        enforcement_latency=(
            None if enforcement.at is None else enforcement.at - rig.delivered_at
        ),
        detection_latency=(
            None if enforcement.at is None else outcome.ended_at - enforcement.at
        ),
    )
    # Emitted, not just asserted on: the timings are the deliverable here, and a
    # green run should still hand them over (`pytest -o log_cli=true
    # --log-cli-level=INFO`). The assertions restate them on failure.
    logging.getLogger(__name__).info("%s", _measurement(episode))
    yield episode


def _measurement(episode) -> str:
    """The measured numbers, restated on every failure so the run is the record."""
    enforcement = (
        "never"
        if episode.enforcement_latency is None
        else f"{episode.enforcement_latency:.1f}s after delivery"
    )
    detection = (
        "n/a"
        if episode.detection_latency is None
        else f"{episode.detection_latency:.3f}s later"
    )
    return (
        f"[measured] consumer-timeout={CONSUMER_TIMEOUT_MS}ms; broker enforced "
        f"{enforcement} with {episode.enforcement_error!r}; stream ended "
        f"{detection} with {episode.outcome.error!r}; channel is_closed after "
        f"the reopen: {episode.channel_closed_after}; consumer tags left: "
        f"{episode.tags_after}"
    )


@pytest.mark.skipif(
    not _slow_arm_enabled(),
    reason=(
        "the broker enforces consumer-timeout on a coarse tick, so this arm "
        f"takes minutes. Set {_SLOW_ARM_ENV}=1 to run it, preferably with -n0 "
        "so its class fixture drives the live episode once instead of once per "
        "test. The fast arm above still proves the policy applied to the queue; "
        "remove this gate only if the suite gains a slow lane that can absorb "
        "the wait."
    ),
)
class TestTheBrokerEnforcesTheAckTimeout:
    """
    The end-to-end claim of PRD AC 5, with every link real.

    A client that withholds an ack past the policy's ``consumer-timeout`` has
    its consumer taken away by the BROKER, and our consume loop ends that
    client's stream with the reconnect signal instead of leaving it open on a
    consumer nothing feeds.
    """

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_broker_acted_on_the_withheld_ack(self, enforcement_episode):
        # THE link that was never tested. The previous implementation passed
        # this bound as a `basic.consume` argument, where the broker ignores it
        # silently -- so "we set an ack timeout" was true and meaningless. This
        # asserts the broker acted.
        assert enforcement_episode.enforced_at is not None, (
            "the broker never acted within "
            f"{ENFORCEMENT_TIMEOUT_S}s of the ack being withheld, despite "
            f"reporting consumer-timeout={CONSUMER_TIMEOUT_MS}ms as effective "
            f"on the queue; the policy is not biting. "
            f"{_measurement(enforcement_episode)}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_broker_enforced_the_value_this_policy_carried(
        self, enforcement_episode
    ):
        # The strongest evidence available that the number travelled all the way
        # from the policy definition into the broker's enforcement: RabbitMQ
        # echoes the timeout it used back in the close reason. Without this the
        # arm would only show that SOME timeout fired, which the broker's own
        # 30-minute default would also satisfy.
        assert str(CONSUMER_TIMEOUT_MS) in str(enforcement_episode.enforcement_error), (
            f"the broker's enforcement did not name the {CONSUMER_TIMEOUT_MS}ms "
            "timeout this module's policy set, so it may have enforced its own "
            f"default instead. {_measurement(enforcement_episode)}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_enforcement_waited_out_the_configured_timeout(
        self, enforcement_episode
    ):
        # Ties the event to THIS policy rather than to anything else that could
        # take a consumer away. An enforcement arriving before the configured
        # timeout had elapsed would not be the ack timeout at all, and the arm
        # would be passing for the wrong reason.
        floor = CONSUMER_TIMEOUT_MS / 1000 - ENFORCEMENT_CLOCK_SKEW_S
        assert enforcement_episode.enforcement_latency >= floor, (
            "the consumer was taken away more than "
            f"{ENFORCEMENT_CLOCK_SKEW_S}s before the configured ack timeout had "
            "elapsed, so something other than the policy did it. "
            f"{_measurement(enforcement_episode)}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_stream_does_not_stay_open(self, enforcement_episode):
        # THE regression assertion. Pre-fix the loop blocked forever on
        # `messages_queue.get()`: a client that stopped acking kept a stream
        # that looked healthy against a consumer the broker had already taken
        # away -- which is the shape of the 47-hour zombie in issue b.9gj.
        assert enforcement_episode.outcome.timed_out is False, (
            "the consume generator was still open "
            f"{ENFORCEMENT_TIMEOUT_S}s after the ack was withheld. "
            f"{_measurement(enforcement_episode)}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_stream_ends_with_consumer_cancelled(self, enforcement_episode):
        # The remedy the client can act on. The queue is untouched, so the
        # answer is RECONNECT -- and, pre-fix, the answer was nothing at all.
        assert isinstance(enforcement_episode.outcome.error, ConsumerCancelledError), (
            "expected ConsumerCancelledError after the broker enforced the ack "
            f"timeout. {_measurement(enforcement_episode)}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_stream_ends_within_one_liveness_poll_of_the_enforcement(
        self, enforcement_episode
    ):
        # Measured from the BROKER's action, not from the start of the wait, so
        # the two-minute enforcement tick is not credited to our loop.
        assert enforcement_episode.detection_latency < PROMPT_DETECTION_BUDGET_S, (
            "our loop took longer than one liveness poll to notice an "
            "enforcement the broker had already made. "
            f"{_measurement(enforcement_episode)}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_error_records_the_channel_death_through_the_reopen(
        self, enforcement_episode
    ):
        """
        The enforcement reaches cleanup as the channel death it is.

        The broker's enforcement is a channel CLOSE (see the wire message the
        test above asserts on), and closing a channel requeues every unacked
        delivery on it. ``ConsumerCancelledError.channel_closed`` carries that
        fact to cleanup, which then skips its now-futile nacks
        (``RabbitMQBase._should_nack_outstanding``).

        The reopen race is what makes this worth asserting live rather than
        only in unit tests. ``RobustChannel`` reopens the channel in place
        ~75 ms after the close while the liveness poll runs on a ~1 s cadence,
        so ``is_closed`` is False again on a fresh incarnation by the time
        anything looks -- and the second assertion below measures exactly that.
        ``channel_closed`` is therefore an INCARNATION comparison, pinned at
        consume start, not a reading of the wrapper: the death survives the
        reopen and the nacks are skipped.

        Reading the wrapper instead reported ``channel_closed=False`` here, and
        cleanup then nacked delivery tags the dead incarnation had taken with
        it -- each failing with ``ChannelInvalidStateError``, caught and logged
        by ``AckTracker.nack_all`` at ERROR level once per in-flight delivery,
        up to ``durable_consumer_prefetch_count`` of them every time a stuck
        client was timed out. The two assertions differ for that reason and
        must stay different: the first is the pinned fact, the second is the
        live-broker measurement it has to survive.
        """
        assert enforcement_episode.outcome.error.channel_closed is True
        assert enforcement_episode.channel_closed_after is False
