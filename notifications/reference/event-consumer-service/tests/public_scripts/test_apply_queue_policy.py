# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for the event-queue policy applier (issue b.9dd).

:mod:`public_scripts.apply_queue_policy` runs in the chart's
post-install/post-upgrade hook Job. It is entirely env-driven and its only side
effect is one or two calls to the RabbitMQ management API, so the whole module is
exercised here with the environment set by fixture and a hand-written fake
standing in at the ``urllib3.PoolManager`` boundary. Every test therefore asserts
what the module would put on the wire, which is the actual contract.

The claims worth defending:

- **The read-before-write guard.** Applying a policy resets the ``expires`` idle
  clock on every matched queue. A deploy cadence shorter than ``expires`` would
  then keep abandoned queues alive forever and auto-reap would never fire, so an
  unchanged policy must produce a GET and NO PUT. Because the comparison is over
  parsed JSON, key order must not matter -- a test drives that explicitly, since
  a naive string compare would pass every other test in this file and still
  re-PUT on every single deploy. The comparison is forgiving at exactly one
  level: unknown TOP-LEVEL fields the broker echoes back are ignored, while the
  ``definition`` object is matched key-for-key, so a hand-edited bound is a
  difference and gets reverted.
- **Fail-closed on the read, fail-loud on the write.** An unreadable existing
  policy still applies (better a redundant PUT than a missing policy), while a
  non-2xx PUT raises and takes the exit code -- and with it the Job and the
  release -- non-zero.
- **The vhost cross-check.** "Applied successfully to the wrong vhost" is
  otherwise silent: the PUT succeeds, the Job goes green, and no queue is ever
  bounded. A mismatch must abort before any request is made. A HOST mismatch, by
  contrast, is legitimate on a managed broker and may only warn.
- **``expires`` omission vs. zero.** Absent or blank means ship no ``expires``
  key at all. An explicit ``0`` is forwarded rather than dropped, so the broker
  rejects it loudly instead of the operator silently getting a policy with the
  reaper switched off.
- **``consumer-timeout`` omission vs. a value below the floor.** Absent is the
  DEFAULT here and means ship no key at all, leaving the broker's own default in
  force. A value RabbitMQ documents as unsupported is refused rather than
  forwarded, because unlike ``expires`` the broker ACCEPTS it -- and then cancels
  every consumer on every durable queue.
- **The negative delivery-limit refusal, again, here.** The chart guards this at
  render time, but the chart is not the only way this module runs: it is a plain
  entrypoint that anything -- a debug pod, a hand-written Job, a future caller --
  can start with its own environment. And the value is a landmine rather than a
  preference, so the refusal is duplicated deliberately and pinned on both sides.
- **The real transport.** Every test above replaces ``pool.request``, so nothing
  in them would notice a URL assembled wrong at the urllib3 boundary, a
  credential that never became a header, or a body that never got encoded.
  ``TestAgainstALiveManagementApi`` runs the module end-to-end against an
  ephemeral stdlib HTTP server on a real socket to close that gap.
"""

from __future__ import annotations

import base64
import json
import logging
import threading
import urllib.parse
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import certifi
import pytest
import tenacity
import urllib3

from public_scripts import apply_queue_policy
from public_scripts.apply_queue_policy import PolicyApplyError, TransientPolicyError

MANAGEMENT_URL = "https://broker.example.com:15672"
VHOST = "events"
POLICY_NAME = "event-queue-limits"
PATTERN = "^event-queue:"
USERNAME = "policy-admin"
PASSWORD = "s3cret-pw"

# The AMQP URI the service itself uses; its path segment IS the vhost.
SERVICE_URI = f"amqps://svc:pw@broker.example.com/{VHOST}"

POLICY_PATH = f"/api/policies/{VHOST}/{POLICY_NAME}"
POLICY_URL = f"{MANAGEMENT_URL}{POLICY_PATH}"

# Mirrors the chart's stock render of the policyHook block.
DEFAULT_ENV = {
    "QUEUE_POLICY_MANAGEMENT_URL": MANAGEMENT_URL,
    "QUEUE_POLICY_MANAGEMENT_USERNAME": USERNAME,
    "QUEUE_POLICY_MANAGEMENT_PASSWORD": PASSWORD,
    "QUEUE_POLICY_VHOST": VHOST,
    "QUEUE_POLICY_NAME": POLICY_NAME,
    "QUEUE_POLICY_PATTERN": PATTERN,
    "QUEUE_POLICY_PRIORITY": "0",
    "QUEUE_POLICY_APPLY_TO": "quorum_queues",
    "QUEUE_POLICY_DELIVERY_LIMIT": "10",
    "QUEUE_POLICY_MESSAGE_TTL_MS": "86400000",
    "QUEUE_POLICY_MAX_LENGTH": "100000",
    "QUEUE_POLICY_OVERFLOW": "drop-head",
    "QUEUE_POLICY_EXPIRES_MS": "86400000",
    "OMNI_EVENTS_RABBITMQ_URI": SERVICE_URI,
}

# Variables the chart's stock render does NOT emit, cleared so a shell that
# happens to export one cannot change what these tests are testing.
# `consumerTimeoutMs` defaults to null, and the template omits the whole env row
# rather than rendering it blank, so its absence here IS the stock render.
OMITTED_ENV = ("QUEUE_POLICY_CONSUMER_TIMEOUT_MS",)

# RabbitMQ's documented hard floor for `consumer-timeout`, and one below it.
CONSUMER_TIMEOUT_FLOOR_MS = 60000
BELOW_CONSUMER_TIMEOUT_FLOOR_MS = CONSUMER_TIMEOUT_FLOOR_MS - 1

EXPECTED_DEFINITION = {
    "delivery-limit": 10,
    "message-ttl": 86400000,
    "max-length": 100000,
    "overflow": "drop-head",
    "expires": 86400000,
}

EXPECTED_POLICY = {
    "pattern": PATTERN,
    "apply-to": "quorum_queues",
    "priority": 0,
    "definition": EXPECTED_DEFINITION,
}

# Read from the module rather than re-typed. The claim under test is that the
# decorator is actually WIRED to this budget and reraises once it is spent, not
# that the number happens to be 3 -- and the number itself is not free: it is
# sized against the Job's `activeDeadlineSeconds`, which
# `tests/helm/test_queue_policy_hook_template.py::TestRetryBudgetFitsTheJobDeadline`
# pins against the render.
RETRY_ATTEMPTS = apply_queue_policy._MAX_ATTEMPTS


@dataclass
class _Call:
    method: str
    url: str
    headers: dict = field(default_factory=dict)
    body: bytes | None = None
    timeout: object = None


class _FakeResponse:
    """The two attributes the module reads off a urllib3 response."""

    def __init__(self, status: int, data: object = b""):
        self.status = status
        if isinstance(data, (bytes, str)):
            self.data = data if isinstance(data, bytes) else data.encode()
        else:
            self.data = json.dumps(data).encode()


class _FakeBroker:
    """Stands in for ``urllib3.PoolManager`` at the management-API boundary.

    Each of ``get``/``put`` takes a single outcome (reused for every call) or a
    list consumed in order; an ``Exception`` outcome is raised instead of
    returned, which is how transport failures are simulated.
    """

    def __init__(self, *, get: object = None, put: object = None):
        self._outcomes = {
            "GET": _as_outcomes(get if get is not None else _FakeResponse(404)),
            "PUT": _as_outcomes(put if put is not None else _FakeResponse(204)),
        }
        self.calls: list[_Call] = []

    def request(self, method, url, headers=None, body=None, timeout=None):
        self.calls.append(_Call(method, url, dict(headers or {}), body, timeout))
        outcomes = self._outcomes[method]
        outcome = outcomes.pop(0) if len(outcomes) > 1 else outcomes[0]
        if isinstance(outcome, Exception):
            raise outcome
        return outcome

    @property
    def methods(self) -> list[str]:
        return [call.method for call in self.calls]

    def call(self, method: str) -> _Call:
        matching = [call for call in self.calls if call.method == method]
        assert matching, f"no {method} was issued; calls were {self.methods}"
        return matching[0]

    def sent_policy(self) -> dict:
        return json.loads(self.call("PUT").body)


def _as_outcomes(outcome: object) -> list:
    return list(outcome) if isinstance(outcome, list) else [outcome]


def _broker_policy(**overrides) -> dict:
    """An existing-policy body as the management API returns it.

    The broker echoes back TOP-LEVEL fields that were never PUT (``vhost``,
    ``name``), and the comparison ignores those. Everything the chart owns --
    ``pattern``, ``apply-to``, ``priority``, and the whole ``definition``
    object -- is compared exactly.
    """
    body = {
        "vhost": VHOST,
        "name": POLICY_NAME,
        **json.loads(json.dumps(EXPECTED_POLICY)),
    }
    body.update(overrides)
    return body


@pytest.fixture(autouse=True)
def policy_env(monkeypatch):
    """The chart-supplied environment, isolated from whatever the shell has."""
    for name, value in DEFAULT_ENV.items():
        monkeypatch.setenv(name, value)
    for name in OMITTED_ENV:
        monkeypatch.delenv(name, raising=False)
    return monkeypatch


@pytest.fixture(autouse=True)
def instant_retries(mocker):
    """Keep tenacity's real retry policy but drop the backoff sleeps."""
    mocker.patch.object(apply_queue_policy._request.retry, "wait", tenacity.wait_none())


@pytest.fixture
def broker(mocker):
    """Install a fake management API and hand the fake back to the test."""

    def _install(**kwargs) -> _FakeBroker:
        fake = _FakeBroker(**kwargs)
        mocker.patch.object(apply_queue_policy, "_pool_manager", return_value=fake)
        return fake

    return _install


class TestBuildDefinition:
    """The policy definition body assembled from the environment."""

    def test_carries_every_bound_from_the_environment(self):
        assert apply_queue_policy.build_definition() == EXPECTED_DEFINITION

    def test_omits_expires_when_the_variable_is_absent(self, policy_env):
        policy_env.delenv("QUEUE_POLICY_EXPIRES_MS")

        assert "expires" not in apply_queue_policy.build_definition()

    def test_omits_expires_when_the_variable_is_blank(self, policy_env):
        policy_env.setenv("QUEUE_POLICY_EXPIRES_MS", "")

        assert "expires" not in apply_queue_policy.build_definition()

    def test_leaves_the_other_bounds_intact_without_expires(self, policy_env):
        policy_env.delenv("QUEUE_POLICY_EXPIRES_MS")

        assert apply_queue_policy.build_definition()["message-ttl"] == 86400000

    def test_forwards_an_explicit_zero_expires(self, policy_env):
        # Deliberate: the broker rejects a zero `expires`, and that loud
        # rejection is preferable to silently shipping no reaper at all.
        policy_env.setenv("QUEUE_POLICY_EXPIRES_MS", "0")

        assert apply_queue_policy.build_definition()["expires"] == 0

    def test_rejects_a_non_integer_bound(self, policy_env):
        policy_env.setenv("QUEUE_POLICY_MAX_LENGTH", "1e5")

        with pytest.raises(PolicyApplyError, match="must be an integer"):
            apply_queue_policy.build_definition()

    def test_rejects_a_missing_bound(self, policy_env):
        policy_env.delenv("QUEUE_POLICY_OVERFLOW")

        with pytest.raises(PolicyApplyError, match="must be set and non-empty"):
            apply_queue_policy.build_definition()

    def test_omits_consumer_timeout_when_the_variable_is_absent(self):
        # The DEFAULT, and the one that matters: with no `consumer-timeout` key
        # the broker's own default stays in force. Shipping a key here on a
        # stock render would silently retune the ack timeout on every queue the
        # policy matches.
        assert "consumer-timeout" not in apply_queue_policy.build_definition()

    def test_omits_consumer_timeout_when_the_variable_is_blank(self, policy_env):
        # A blank is what a hand-written Job or a blanked-out values entry
        # produces. It must read as "omit", not as a 0 the broker would accept
        # and then use to cancel every consumer it has.
        policy_env.setenv("QUEUE_POLICY_CONSUMER_TIMEOUT_MS", "")

        assert "consumer-timeout" not in apply_queue_policy.build_definition()

    def test_leaves_the_other_bounds_intact_without_consumer_timeout(self):
        # Omission must be surgical: the rest of the policy still ships.
        assert apply_queue_policy.build_definition() == EXPECTED_DEFINITION

    def test_carries_the_consumer_timeout_when_it_is_set(self, policy_env):
        policy_env.setenv("QUEUE_POLICY_CONSUMER_TIMEOUT_MS", "300000")

        assert apply_queue_policy.build_definition()["consumer-timeout"] == 300000


class TestConsumerTimeoutRefusal:
    """A consumer-timeout below RabbitMQ's floor is refused here, not shipped.

    The chart guards this at render time, but this module is a plain entrypoint
    that anything -- a debug pod, a hand-written Job, a future caller -- can
    start with its own environment. And the value is a landmine rather than a
    preference: RabbitMQ documents anything under 60000 ms as unsupported but
    does NOT reject it, so a plain typo of `0` applies cleanly and then cancels
    every consumer on every durable queue the instant it takes a delivery -- a
    permanent kill/reconnect loop across all clients, with this fail-closed
    script still reporting success. Nothing downstream would catch it.

    Only the upstream hard floor is enforced. The softer ">= 300000 (5 minutes)"
    recommendation stays guidance, because the safe value depends on client
    per-message processing time this script cannot see -- so 60000 must still be
    accepted.
    """

    def test_a_zero_consumer_timeout_is_rejected(self, policy_env):
        # The reachable typo, and the worst value in the range.
        policy_env.setenv("QUEUE_POLICY_CONSUMER_TIMEOUT_MS", "0")

        with pytest.raises(PolicyApplyError, match="must be >= 60000"):
            apply_queue_policy.build_definition()

    def test_one_millisecond_below_the_floor_is_rejected(self, policy_env):
        # The boundary: the guard is a range check, so an off-by-one here is
        # what lets an unsupported value through.
        policy_env.setenv(
            "QUEUE_POLICY_CONSUMER_TIMEOUT_MS", str(BELOW_CONSUMER_TIMEOUT_FLOOR_MS)
        )

        with pytest.raises(PolicyApplyError, match="must be >= 60000"):
            apply_queue_policy.build_definition()

    def test_the_floor_itself_is_accepted(self, policy_env):
        # The other side of the same boundary, and the reason the check stops
        # at the hard floor: 60000 is below the recommended 300000 and is still
        # the operator's call to make.
        policy_env.setenv(
            "QUEUE_POLICY_CONSUMER_TIMEOUT_MS", str(CONSUMER_TIMEOUT_FLOOR_MS)
        )

        definition = apply_queue_policy.build_definition()

        assert definition["consumer-timeout"] == CONSUMER_TIMEOUT_FLOOR_MS

    def test_the_refusal_names_the_variable(self, policy_env):
        policy_env.setenv("QUEUE_POLICY_CONSUMER_TIMEOUT_MS", "0")

        with pytest.raises(PolicyApplyError, match="QUEUE_POLICY_CONSUMER_TIMEOUT_MS"):
            apply_queue_policy.build_definition()

    def test_the_refusal_says_what_a_low_value_does(self, policy_env):
        # Without the reason, the floor reads as an arbitrary ban and the next
        # operator reaches for a way around it.
        policy_env.setenv("QUEUE_POLICY_CONSUMER_TIMEOUT_MS", "0")

        with pytest.raises(PolicyApplyError, match="cancels healthy consumers"):
            apply_queue_policy.build_definition()

    def test_the_refusal_points_at_the_sizing_input(self, policy_env):
        # "Pick a bigger number" is not actionable on its own; the prefetch
        # count is the half of the sizing the operator cannot guess.
        policy_env.setenv("QUEUE_POLICY_CONSUMER_TIMEOUT_MS", "0")

        with pytest.raises(PolicyApplyError, match="durable_consumer_prefetch_count"):
            apply_queue_policy.build_definition()

    def test_no_request_reaches_the_broker(self, broker, policy_env):
        # The refusal has to land BEFORE any HTTP call: a policy PUT rolled
        # back afterwards is not equivalent, because the damaging policy exists
        # in the broker in between.
        policy_env.setenv("QUEUE_POLICY_CONSUMER_TIMEOUT_MS", "0")
        fake = broker()

        with pytest.raises(PolicyApplyError):
            apply_queue_policy.apply_policy()

        assert fake.calls == []

    def test_the_exit_code_is_non_zero(self, broker, policy_env):
        policy_env.setenv("QUEUE_POLICY_CONSUMER_TIMEOUT_MS", "0")
        broker()

        assert apply_queue_policy.main() == 1

    def test_the_reason_is_logged_at_error(self, broker, policy_env, caplog):
        # The Job's pod log is the only place an operator can read this from.
        policy_env.setenv("QUEUE_POLICY_CONSUMER_TIMEOUT_MS", "0")
        broker()

        with caplog.at_level(logging.ERROR, logger="apply_queue_policy"):
            apply_queue_policy.main()

        assert "cancels healthy consumers" in caplog.text


class TestDeliveryLimitRefusal:
    """A negative delivery-limit is refused here too, not only by the chart.

    On RabbitMQ 4.0.5-4.1.4 a matching `delivery-limit: -1` makes the
    policy-repair tick rewrite the queue config every 5 s (rabbitmq-server#14202),
    which resets the `expires` idle clock -- so no queue ever auto-reaps and the
    whole point of the policy is silently lost. The chart refuses the value at
    render time, but this module is a plain entrypoint that anything can run with
    its own environment, and nothing downstream would catch it: the broker
    accepts the policy and the Job reports success.

    The refusal has to land BEFORE any HTTP call. A policy PUT that is rolled
    back afterwards is not equivalent -- the damaging policy exists in the
    broker in between, and the rollback is exactly what fails if the process
    dies.
    """

    def test_a_negative_delivery_limit_is_rejected(self, policy_env):
        policy_env.setenv("QUEUE_POLICY_DELIVERY_LIMIT", "-1")

        with pytest.raises(PolicyApplyError, match="must be >= 0"):
            apply_queue_policy.build_definition()

    def test_the_refusal_names_the_variable(self, policy_env):
        policy_env.setenv("QUEUE_POLICY_DELIVERY_LIMIT", "-1")

        with pytest.raises(PolicyApplyError, match="QUEUE_POLICY_DELIVERY_LIMIT"):
            apply_queue_policy.build_definition()

    def test_the_refusal_cites_the_expires_reset(self, policy_env):
        # Without the reason, -1 reads as an arbitrary ban and the next operator
        # reaches for a way around it.
        policy_env.setenv("QUEUE_POLICY_DELIVERY_LIMIT", "-1")

        with pytest.raises(PolicyApplyError, match="expires idle clock"):
            apply_queue_policy.build_definition()

    def test_zero_is_accepted(self, policy_env):
        # The boundary: >= 0 is the rule, and 0 is legal (if drastic).
        policy_env.setenv("QUEUE_POLICY_DELIVERY_LIMIT", "0")

        assert apply_queue_policy.build_definition()["delivery-limit"] == 0

    def test_no_request_reaches_the_broker(self, broker, policy_env):
        policy_env.setenv("QUEUE_POLICY_DELIVERY_LIMIT", "-1")
        fake = broker()

        with pytest.raises(PolicyApplyError):
            apply_queue_policy.apply_policy()

        assert fake.calls == []

    def test_the_exit_code_is_non_zero(self, broker, policy_env):
        policy_env.setenv("QUEUE_POLICY_DELIVERY_LIMIT", "-1")
        broker()

        assert apply_queue_policy.main() == 1

    def test_the_reason_is_logged_at_error(self, broker, policy_env, caplog):
        # The Job's pod log is the only place an operator can read this from.
        policy_env.setenv("QUEUE_POLICY_DELIVERY_LIMIT", "-1")
        broker()

        with caplog.at_level(logging.ERROR, logger="apply_queue_policy"):
            apply_queue_policy.main()

        assert "expires idle clock" in caplog.text


class TestParseAmqpUri:
    """The vhost the service's AMQP URI actually addresses."""

    @pytest.mark.parametrize(
        "uri, expected",
        [
            ("amqps://user:pw@host/events", "events"),
            # A vhost containing `/` is percent-encoded in the URI.
            ("amqps://user:pw@host/%2Fprod", "/prod"),
            # No path at all means the default vhost.
            ("amqp://user:pw@host:5672", "/"),
            ("amqp://user:pw@host:5672/", "/"),
        ],
        ids=["plain", "percent_encoded", "no_path", "empty_path"],
    )
    def test_returns_the_decoded_vhost(self, uri, expected):
        assert apply_queue_policy.parse_amqp_uri(uri)[1] == expected

    def test_returns_the_host(self):
        host, _ = apply_queue_policy.parse_amqp_uri(SERVICE_URI)

        assert host == "broker.example.com"


class TestCrossCheckTarget:
    """A policy aimed at a vhost the service never uses must not be applied."""

    def test_matching_vhost_passes(self):
        assert apply_queue_policy.cross_check_target(VHOST, MANAGEMENT_URL) is None

    def test_mismatched_vhost_raises(self, policy_env):
        policy_env.setenv("OMNI_EVENTS_RABBITMQ_URI", "amqps://broker/other-vhost")

        with pytest.raises(PolicyApplyError, match="vhost mismatch"):
            apply_queue_policy.cross_check_target(VHOST, MANAGEMENT_URL)

    def test_mismatch_names_the_value_to_fix(self, policy_env):
        policy_env.setenv("OMNI_EVENTS_RABBITMQ_URI", "amqps://broker/other-vhost")

        with pytest.raises(PolicyApplyError, match="policyHook.managementApi.vhost"):
            apply_queue_policy.cross_check_target(VHOST, MANAGEMENT_URL)

    def test_differing_host_is_accepted(self, policy_env):
        # A managed broker legitimately serves its management API from a
        # different hostname than its AMQP endpoint.
        policy_env.setenv("OMNI_EVENTS_RABBITMQ_URI", f"amqps://amqp.internal/{VHOST}")

        assert (
            apply_queue_policy.cross_check_target(VHOST, "https://mgmt.example.com")
            is None
        )

    def test_differing_host_warns(self, policy_env, caplog):
        policy_env.setenv("OMNI_EVENTS_RABBITMQ_URI", f"amqps://amqp.internal/{VHOST}")

        apply_queue_policy.cross_check_target(VHOST, "https://mgmt.example.com")

        assert "amqp.internal" in caplog.text

    def test_absent_service_uri_skips_the_check(self, policy_env):
        policy_env.delenv("OMNI_EVENTS_RABBITMQ_URI")

        assert (
            apply_queue_policy.cross_check_target("any-vhost", MANAGEMENT_URL) is None
        )

    def test_absent_service_uri_warns(self, policy_env, caplog):
        policy_env.delenv("OMNI_EVENTS_RABBITMQ_URI")

        apply_queue_policy.cross_check_target("any-vhost", MANAGEMENT_URL)

        assert "OMNI_EVENTS_RABBITMQ_URI" in caplog.text


class TestApplyPolicySkipsAnUnchangedPolicy:
    """The read-before-write guard: an identical policy must NOT be re-PUT.

    A PUT resets every matched queue's `expires` idle clock, so re-applying on
    each deploy would keep abandoned queues alive indefinitely and auto-reap
    would never fire.
    """

    def test_no_put_is_issued(self, broker):
        fake = broker(get=_FakeResponse(200, _broker_policy()))

        apply_queue_policy.apply_policy()

        assert fake.methods == ["GET"]

    def test_key_order_does_not_count_as_a_difference(self, broker):
        # The comparison must be over parsed JSON. A string compare would pass
        # every other test here and still re-PUT on every deploy.
        shuffled_definition = dict(reversed(list(EXPECTED_DEFINITION.items())))
        existing = _broker_policy(definition=shuffled_definition)
        fake = broker(get=_FakeResponse(200, dict(reversed(list(existing.items())))))

        apply_queue_policy.apply_policy()

        assert fake.methods == ["GET"]

    def test_extra_broker_supplied_fields_do_not_force_a_reapply(self, broker):
        # The management API echoes back TOP-LEVEL fields that were never PUT
        # (`vhost`, `name`, and whatever a future release adds), so those are
        # ignored; counting them as differences would re-PUT on every deploy
        # after a broker upgrade. The tolerance stops at the top level --
        # inside `definition` the comparison is exact, which is what
        # TestDefinitionIsComparedExactly pins.
        existing = _broker_policy(**{"some-future-broker-field": 1})
        fake = broker(get=_FakeResponse(200, existing))

        apply_queue_policy.apply_policy()

        assert fake.methods == ["GET"]

    def test_get_targets_the_policy_url(self, broker):
        fake = broker(get=_FakeResponse(200, _broker_policy()))

        apply_queue_policy.apply_policy()

        assert fake.call("GET").url == POLICY_URL


class TestDefinitionIsComparedExactly:
    """Inside ``definition`` the comparison is exact, in BOTH directions.

    The two levels have deliberately different semantics, and only one of them
    is forgiving: unknown TOP-LEVEL fields are ignored (the broker echoes them
    back and a future release will add more), while ``definition`` -- the
    bounds the chart actually declares -- must match key-for-key. A subset
    check there would let a hand-added `max-length-bytes` sit on the chart's own
    policy forever, and a superset check would let a bound the operator deleted
    stay deleted, in both cases with the hook Job reporting success.
    """

    def test_a_hand_added_definition_key_triggers_a_put(self, broker):
        hand_edited = dict(EXPECTED_DEFINITION, **{"max-length-bytes": 1048576})
        fake = broker(get=_FakeResponse(200, _broker_policy(definition=hand_edited)))

        apply_queue_policy.apply_policy()

        assert "PUT" in fake.methods

    def test_the_reapply_reverts_to_the_declared_definition(self, broker):
        # The chart's declared state wins: the hand-added key is not merged
        # forward, it is written away.
        hand_edited = dict(EXPECTED_DEFINITION, **{"max-length-bytes": 1048576})
        fake = broker(get=_FakeResponse(200, _broker_policy(definition=hand_edited)))

        apply_queue_policy.apply_policy()

        assert fake.sent_policy()["definition"] == EXPECTED_DEFINITION

    def test_a_missing_definition_key_triggers_a_put(self, broker):
        # The other direction: a bound the chart declares but the broker does
        # not carry. Nothing else notices -- the queues are simply unbounded.
        stripped = {
            key: value
            for key, value in EXPECTED_DEFINITION.items()
            if key != "max-length"
        }
        fake = broker(get=_FakeResponse(200, _broker_policy(definition=stripped)))

        apply_queue_policy.apply_policy()

        assert "PUT" in fake.methods

    def test_an_absent_definition_triggers_a_put(self, broker):
        fake = broker(get=_FakeResponse(200, _broker_policy(definition={})))

        apply_queue_policy.apply_policy()

        assert "PUT" in fake.methods


class TestApplyPolicyWritesWhenTheBrokerDiffers:
    """Absent, changed, or unreadable: the policy gets PUT."""

    def test_absent_policy_is_created(self, broker):
        fake = broker(get=_FakeResponse(404))

        apply_queue_policy.apply_policy()

        assert fake.methods == ["GET", "PUT"]

    def test_changed_definition_triggers_a_put(self, broker):
        stale = dict(EXPECTED_DEFINITION, **{"max-length": 5})
        fake = broker(get=_FakeResponse(200, _broker_policy(definition=stale)))

        apply_queue_policy.apply_policy()

        assert "PUT" in fake.methods

    def test_changed_pattern_triggers_a_put(self, broker):
        fake = broker(get=_FakeResponse(200, _broker_policy(pattern="^other:")))

        apply_queue_policy.apply_policy()

        assert "PUT" in fake.methods

    def test_widened_apply_to_triggers_a_put(self, broker):
        fake = broker(get=_FakeResponse(200, _broker_policy(**{"apply-to": "queues"})))

        apply_queue_policy.apply_policy()

        assert "PUT" in fake.methods

    def test_malformed_existing_body_triggers_a_put(self, broker):
        fake = broker(get=_FakeResponse(200, b"<html>not json</html>"))

        apply_queue_policy.apply_policy()

        assert "PUT" in fake.methods

    def test_unreadable_existing_policy_still_applies(self, broker):
        # Fail-closed on the read: a redundant PUT beats a missing policy.
        fake = broker(get=_FakeResponse(503))

        apply_queue_policy.apply_policy()

        assert "PUT" in fake.methods

    def test_body_carries_the_whole_policy(self, broker):
        fake = broker()

        apply_queue_policy.apply_policy()

        assert fake.sent_policy() == EXPECTED_POLICY

    def test_body_carries_the_priority_as_an_integer(self, broker, policy_env):
        policy_env.setenv("QUEUE_POLICY_PRIORITY", "7")
        fake = broker()

        apply_queue_policy.apply_policy()

        assert fake.sent_policy()["priority"] == 7

    def test_body_omits_expires_when_it_is_unset(self, broker, policy_env):
        policy_env.delenv("QUEUE_POLICY_EXPIRES_MS")
        fake = broker()

        apply_queue_policy.apply_policy()

        assert "expires" not in fake.sent_policy()["definition"]

    def test_put_targets_the_policy_url(self, broker):
        fake = broker()

        apply_queue_policy.apply_policy()

        assert fake.call("PUT").url == POLICY_URL

    def test_trailing_slash_on_the_base_url_does_not_double_up(
        self, broker, policy_env
    ):
        policy_env.setenv("QUEUE_POLICY_MANAGEMENT_URL", f"{MANAGEMENT_URL}/")
        fake = broker()

        apply_queue_policy.apply_policy()

        assert fake.call("PUT").url == POLICY_URL


class TestApplyPolicyRequestHeaders:
    """The call carries a broker administrator credential; the wiring matters."""

    def test_credential_is_sent_as_basic_auth(self, broker):
        fake = broker()

        apply_queue_policy.apply_policy()

        encoded = fake.call("PUT").headers["authorization"].split(" ", 1)[1]
        assert base64.b64decode(encoded).decode() == f"{USERNAME}:{PASSWORD}"

    def test_body_is_declared_as_json(self, broker):
        fake = broker()

        apply_queue_policy.apply_policy()

        assert fake.call("PUT").headers["content-type"] == "application/json"

    def test_the_read_is_authenticated_too(self, broker):
        fake = broker()

        apply_queue_policy.apply_policy()

        assert "authorization" in fake.call("GET").headers


class TestRequestBudget:
    """Every call is time-bounded, so a hung broker cannot outlast the Job.

    The Job's `activeDeadlineSeconds` (180 in the chart) is the outer bound, but
    it kills the pod rather than reporting anything useful. A request with no
    timeout would hang against a broker that accepts the connection and never
    answers, and the release would fail on the deadline with no diagnosis. That
    the per-call budget actually FITS inside the deadline is asserted against the
    render in
    `tests/helm/test_queue_policy_hook_template.py::TestRetryBudgetFitsTheJobDeadline`.
    """

    def test_the_read_is_time_bounded(self, broker):
        fake = broker()

        apply_queue_policy.apply_policy()

        assert fake.call("GET").timeout is apply_queue_policy._TIMEOUT

    def test_the_write_is_time_bounded(self, broker):
        fake = broker()

        apply_queue_policy.apply_policy()

        assert fake.call("PUT").timeout is apply_queue_policy._TIMEOUT


class TestPoolManager:
    """TLS verification is never bypassed on a credential-bearing call."""

    def test_certificates_are_required(self):
        pool = apply_queue_policy._pool_manager()

        assert pool.connection_pool_kw["cert_reqs"] == "CERT_REQUIRED"

    def test_the_certifi_bundle_is_used(self):
        pool = apply_queue_policy._pool_manager()

        assert pool.connection_pool_kw["ca_certs"] == certifi.where()

    def test_urllib3_does_not_retry_underneath_tenacity(self):
        # urllib3's default `Retry(3)` would multiply tenacity's 3 attempts into
        # as many as 12 requests, each with its own timeout -- 144 s of connect
        # and read budget on a single call, which blows past the Job's 180 s
        # activeDeadlineSeconds before tenacity has even finished the GET.
        # `retries=False` is normalized by PoolManager into a Retry whose
        # `total` is the False sentinel, which is what disables it.
        pool = apply_queue_policy._pool_manager()

        assert pool.connection_pool_kw["retries"].total is False


class TestApplyPolicyFailsLoudly:
    """A cluster running without the bounds is the condition being prevented."""

    def test_rejected_put_raises(self, broker):
        broker(put=_FakeResponse(403, b"insufficient permissions"))

        with pytest.raises(PolicyApplyError, match="HTTP 403"):
            apply_queue_policy.apply_policy()

    def test_rejected_put_surfaces_the_broker_response(self, broker):
        broker(put=_FakeResponse(403, b"user lacks the policymaker tag"))

        with pytest.raises(PolicyApplyError, match="policymaker"):
            apply_queue_policy.apply_policy()

    def test_a_forbidden_put_is_not_retried(self, broker):
        # 4xx other than 429 is a configuration error; retrying only delays the
        # failure past the Job's deadline.
        fake = broker(put=_FakeResponse(403))

        with pytest.raises(PolicyApplyError):
            apply_queue_policy.apply_policy()

        assert fake.methods.count("PUT") == 1

    def test_persistent_server_error_raises_after_the_retries(self, broker):
        broker(put=_FakeResponse(500))

        with pytest.raises(TransientPolicyError):
            apply_queue_policy.apply_policy()

    def test_persistent_server_error_exhausts_the_retry_budget(self, broker):
        fake = broker(put=_FakeResponse(500))

        with pytest.raises(TransientPolicyError):
            apply_queue_policy.apply_policy()

        assert fake.methods.count("PUT") == RETRY_ATTEMPTS

    def test_a_transport_failure_is_retried_then_succeeds(self, broker):
        transport_error = urllib3.exceptions.HTTPError("no route")
        fake = broker(put=[transport_error, _FakeResponse(204)])

        apply_queue_policy.apply_policy()

        assert fake.methods.count("PUT") == 2

    def test_vhost_mismatch_never_reaches_the_broker(self, broker, policy_env):
        # The cross-check has to run BEFORE any request, or a wrong-vhost PUT
        # lands and the Job still goes green.
        policy_env.setenv("OMNI_EVENTS_RABBITMQ_URI", "amqps://broker/other-vhost")
        fake = broker()

        with pytest.raises(PolicyApplyError, match="vhost mismatch"):
            apply_queue_policy.apply_policy()

        assert fake.calls == []

    def test_missing_management_url_raises(self, broker, policy_env):
        policy_env.delenv("QUEUE_POLICY_MANAGEMENT_URL")
        broker()

        with pytest.raises(PolicyApplyError, match="QUEUE_POLICY_MANAGEMENT_URL"):
            apply_queue_policy.apply_policy()

    def test_missing_credential_raises(self, broker, policy_env):
        policy_env.setenv("QUEUE_POLICY_MANAGEMENT_PASSWORD", "")
        broker()

        with pytest.raises(PolicyApplyError, match="QUEUE_POLICY_MANAGEMENT_PASSWORD"):
            apply_queue_policy.apply_policy()


class TestMain:
    """The exit code is what fails the Job and, through it, the Helm release."""

    def test_returns_zero_when_the_policy_is_applied(self, broker):
        broker()

        assert apply_queue_policy.main() == 0

    def test_returns_zero_when_the_policy_was_already_current(self, broker):
        broker(get=_FakeResponse(200, _broker_policy()))

        assert apply_queue_policy.main() == 0

    def test_returns_non_zero_when_the_broker_rejects_the_put(self, broker):
        broker(put=_FakeResponse(403))

        assert apply_queue_policy.main() == 1

    def test_returns_non_zero_when_the_broker_stays_unavailable(self, broker):
        broker(put=_FakeResponse(500))

        assert apply_queue_policy.main() == 1

    def test_returns_non_zero_on_a_vhost_mismatch(self, broker, policy_env):
        policy_env.setenv("OMNI_EVENTS_RABBITMQ_URI", "amqps://broker/other-vhost")
        broker()

        assert apply_queue_policy.main() == 1

    def test_returns_non_zero_on_incomplete_configuration(self, broker, policy_env):
        policy_env.delenv("QUEUE_POLICY_NAME")
        broker()

        assert apply_queue_policy.main() == 1

    def test_logs_the_failure_reason(self, broker, caplog):
        broker(put=_FakeResponse(403))

        with caplog.at_level(logging.ERROR, logger="apply_queue_policy"):
            apply_queue_policy.main()

        assert "Failed to apply the event-queue policy" in caplog.text


class TestPolicyUrlEncoding:
    """Names and vhosts that need escaping must not break out of the path."""

    @pytest.mark.parametrize(
        "vhost, service_path",
        [
            # The default vhost is a bare "/", which would otherwise open a
            # second path segment in the management URL.
            ("/", ""),
            ("/prod", "%2Fprod"),
            ("team a", "team%20a"),
        ],
        ids=["default", "slash", "space"],
    )
    def test_vhost_is_percent_encoded(self, broker, policy_env, vhost, service_path):
        policy_env.setenv("QUEUE_POLICY_VHOST", vhost)
        policy_env.setenv(
            "OMNI_EVENTS_RABBITMQ_URI", f"amqps://broker.example.com/{service_path}"
        )
        fake = broker()

        apply_queue_policy.apply_policy()

        expected = urllib.parse.quote(vhost, safe="")
        assert f"/api/policies/{expected}/" in fake.call("PUT").url

    def test_policy_name_is_percent_encoded(self, broker, policy_env):
        policy_env.setenv("QUEUE_POLICY_NAME", "event queue/limits")
        fake = broker()

        apply_queue_policy.apply_policy()

        assert fake.call("PUT").url.endswith("/event%20queue%2Flimits")


@dataclass
class _WireRequest:
    """What actually came off the socket, as the server read it."""

    method: str
    path: str
    headers: dict
    body: bytes


class _RecordingHandler(BaseHTTPRequestHandler):
    # HTTP/1.1 with an explicit content-length on every reply, so the connection
    # stays keep-alive. A close after each response would leave urllib3 holding
    # a stale pooled connection for the PUT that follows the GET, and the module
    # runs the pool with `retries=False` -- it would surface as an intermittent
    # transport error rather than a test failure worth reading.
    protocol_version = "HTTP/1.1"

    def _record_and_reply(self) -> None:
        length = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(length) if length else b""
        self.server.requests.append(
            _WireRequest(self.command, self.path, dict(self.headers), body)
        )
        payload = self.server.payloads.get(self.command, b"")
        self.send_response(self.server.statuses[self.command])
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    do_GET = _record_and_reply
    do_PUT = _record_and_reply

    def log_message(self, *args) -> None:
        """Silence the per-request stderr line; the assertions are the output."""


class _RecordingServer(ThreadingHTTPServer):
    """An ephemeral management API on a real socket.

    Bound to port 0 so `pytest-xdist` workers cannot collide on a fixed port.
    """

    daemon_threads = True

    def __init__(self, statuses: dict[str, int], payloads: dict[str, bytes]):
        super().__init__(("127.0.0.1", 0), _RecordingHandler)
        self.statuses = statuses
        self.payloads = payloads
        self.requests: list[_WireRequest] = []

    @property
    def base_url(self) -> str:
        host, port = self.server_address[:2]
        return f"http://{host}:{port}"

    @property
    def methods(self) -> list[str]:
        return [request.method for request in self.requests]

    def received(self, method: str) -> _WireRequest:
        matching = [request for request in self.requests if request.method == method]
        assert matching, f"no {method} arrived; the server saw {self.methods}"
        return matching[0]


@pytest.fixture
def live_management_api(policy_env):
    """Start a real HTTP server and point the module's environment at it."""
    servers: list[_RecordingServer] = []

    def _start(
        *, statuses: dict | None = None, payloads: dict | None = None
    ) -> _RecordingServer:
        server = _RecordingServer(
            {"GET": 404, "PUT": 204, **(statuses or {})}, payloads or {}
        )
        servers.append(server)
        # A short poll interval keeps teardown's `shutdown()` from waiting out
        # the 0.5 s default.
        threading.Thread(
            target=server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True
        ).start()
        policy_env.setenv("QUEUE_POLICY_MANAGEMENT_URL", server.base_url)
        # Keep the vhost cross-check on the happy path: same host, same vhost,
        # so nothing here is testing the mismatch warning by accident.
        policy_env.setenv("OMNI_EVENTS_RABBITMQ_URI", f"amqps://127.0.0.1/{VHOST}")
        return server

    yield _start

    for server in servers:
        server.shutdown()
        server.server_close()


class TestAgainstALiveManagementApi:
    """One end-to-end pass over the REAL transport, on a real socket.

    Every other test in this file replaces `pool.request`, which means the whole
    urllib3 boundary is assumed rather than exercised: a URL assembled into
    something urllib3 parses differently, a credential that never became a
    header, a body that was never encoded, or a response whose status the module
    reads off the wrong attribute would all sail through them. None of that is
    hypothetical -- it is precisely the layer the fake removes.

    Plain HTTP, not TLS: `_pool_manager`'s certificate settings are already
    pinned directly in `TestPoolManager`, and standing up a certificate
    authority here would buy a slower, flakier test for a claim that is already
    made. What this class adds is that the request the module builds survives an
    actual socket round trip.
    """

    def test_the_write_arrives_at_the_policy_path(self, live_management_api):
        server = live_management_api()

        apply_queue_policy.apply_policy()

        assert server.received("PUT").path == POLICY_PATH

    def test_the_write_carries_the_basic_auth_credential(self, live_management_api):
        server = live_management_api()

        apply_queue_policy.apply_policy()

        header = server.received("PUT").headers["authorization"]
        encoded = header.split(" ", 1)[1]
        assert base64.b64decode(encoded).decode() == f"{USERNAME}:{PASSWORD}"

    def test_the_write_declares_a_json_content_type(self, live_management_api):
        server = live_management_api()

        apply_queue_policy.apply_policy()

        assert server.received("PUT").headers["content-type"] == "application/json"

    def test_the_body_arrives_as_the_whole_policy(self, live_management_api):
        server = live_management_api()

        apply_queue_policy.apply_policy()

        assert json.loads(server.received("PUT").body) == EXPECTED_POLICY

    def test_the_read_precedes_the_write(self, live_management_api):
        server = live_management_api()

        apply_queue_policy.apply_policy()

        assert server.methods == ["GET", "PUT"]

    def test_an_unchanged_policy_read_off_the_wire_skips_the_write(
        self, live_management_api
    ):
        # The comparison runs on bytes the module decoded itself here, rather
        # than on a dict handed straight to it.
        server = live_management_api(
            statuses={"GET": 200},
            payloads={"GET": json.dumps(_broker_policy()).encode()},
        )

        apply_queue_policy.apply_policy()

        assert server.methods == ["GET"]

    def test_a_rejected_write_raises(self, live_management_api):
        server = live_management_api(statuses={"PUT": 403})

        with pytest.raises(PolicyApplyError, match="HTTP 403"):
            apply_queue_policy.apply_policy()

        assert server.methods == ["GET", "PUT"]

    def test_the_exit_code_is_zero_over_the_real_transport(self, live_management_api):
        live_management_api()

        assert apply_queue_policy.main() == 0
