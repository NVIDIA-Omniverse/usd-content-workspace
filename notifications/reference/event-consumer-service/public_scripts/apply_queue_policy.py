# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Apply the event-queue bounds policy through the RabbitMQ management API.

Run by the chart's ``post-install,post-upgrade`` hook Job
(``templates/queue-policy-hook-job.yaml``), which supplies every parameter as an
environment variable. The Job runs the service image, so this module ships in
that image rather than in a ConfigMap: there it is unit-tested, linted, and
versioned in lockstep with the service, and it lands in the published source
mirror (``public_scripts/``) that customers read. (The image being distroless
rules out a shell-*script* ConfigMap in particular; a ConfigMap-mounted Python
file would run fine without a shell.)

Queue arguments are immutable once a queue exists, so the bounds are carried by
a broker *policy* -- the only form that can be retuned on a running cluster.
The management HTTP API is the only reach that also works against a managed
broker (no node or Erlang access), which is why it is used instead of
``rabbitmqctl``.

Failure is loud by design: a non-zero exit fails the Job, which fails the Helm
release, rather than leaving a cluster silently running without the bounds.
"""

from __future__ import annotations

import json
import logging
import os
import sys
import urllib.parse

import certifi
import urllib3
from tenacity import (
    before_sleep_log,
    retry,
    retry_if_exception_type,
    stop_after_attempt,
    wait_random_exponential,
)

logger = logging.getLogger("apply_queue_policy")

ENV_MANAGEMENT_URL = "QUEUE_POLICY_MANAGEMENT_URL"
ENV_MANAGEMENT_USERNAME = "QUEUE_POLICY_MANAGEMENT_USERNAME"
ENV_MANAGEMENT_PASSWORD = "QUEUE_POLICY_MANAGEMENT_PASSWORD"
ENV_VHOST = "QUEUE_POLICY_VHOST"
ENV_NAME = "QUEUE_POLICY_NAME"
ENV_PATTERN = "QUEUE_POLICY_PATTERN"
ENV_PRIORITY = "QUEUE_POLICY_PRIORITY"
ENV_APPLY_TO = "QUEUE_POLICY_APPLY_TO"
ENV_DELIVERY_LIMIT = "QUEUE_POLICY_DELIVERY_LIMIT"
ENV_MESSAGE_TTL_MS = "QUEUE_POLICY_MESSAGE_TTL_MS"
ENV_MAX_LENGTH = "QUEUE_POLICY_MAX_LENGTH"
ENV_OVERFLOW = "QUEUE_POLICY_OVERFLOW"
# Absence is meaningful: the chart omits this variable entirely when
# `policyHook.policy.definition.expiresMs` is null, which means "ship no
# `expires` key". A zero would be rejected by the broker, so it is never sent.
ENV_EXPIRES_MS = "QUEUE_POLICY_EXPIRES_MS"
# Absence is meaningful here too, and it is the DEFAULT: the chart omits this
# variable unless an operator sets
# `policyHook.policy.definition.consumerTimeoutMs`, which means "ship no
# `consumer-timeout` key" and leaves the broker's own default in force. The
# policy is the only lever for this bound that can be retuned on a running
# cluster -- the queue argument of the same name is immutable once a queue
# exists, and a basic.consume argument is silently ignored.
ENV_CONSUMER_TIMEOUT_MS = "QUEUE_POLICY_CONSUMER_TIMEOUT_MS"
# The AMQP URI the SERVICE uses. Read only to cross-check that the policy is
# aimed at the vhost the service actually consumes from.
ENV_RABBITMQ_URI = "OMNI_EVENTS_RABBITMQ_URI"

# Retry budget. Sized so the WHOLE script -- both calls, every attempt, every
# backoff wait -- finishes well inside the Job's `activeDeadlineSeconds`
# (default 180 in values.yaml). If it did not, a stalling broker would get the
# pod killed as DeadlineExceeded before the script could log why, which is the
# one diagnostic an operator actually needs:
#
#   per attempt      4 s connect + 8 s read                       = 12 s
#   per call         3 attempts + backoff waits of 1 s and 2 s    = 39 s
#   whole script     GET (fails open) then PUT                    = 78 s
#
# The bound is deliberately per-call rather than global: the GET can burn its
# full budget and still fall through to the PUT (see `policy_is_current`), so
# the two budgets add.
_MAX_ATTEMPTS = 3
_MAX_BACKOFF_SECONDS = 2
_TIMEOUT = urllib3.Timeout(connect=4.0, read=8.0)

# Module-level tuple so the `except` below stays a name, not an inline
# `except (A, B):` that Black 25 strips parens from on py3.14 (see b.8qg).
_JSON_PARSE_EXC = (ValueError, TypeError)


class PolicyApplyError(Exception):
    """The policy could not be applied. Fatal -- fails the Job and the release."""


class TransientPolicyError(PolicyApplyError):
    """A management-API call failed in a way that is worth retrying."""


def _require_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise PolicyApplyError(f"{name} must be set and non-empty")
    return value


def _require_int_env(name: str) -> int:
    raw = _require_env(name)
    try:
        return int(raw)
    except ValueError as exc:
        raise PolicyApplyError(f"{name} must be an integer, got {raw!r}") from exc


def _require_delivery_limit() -> int:
    """Return the delivery limit, refusing a negative one before any HTTP call.

    The chart's render-time guard is the primary defense, but this module is a
    plain entrypoint that anything can run with its own environment, and the
    value it guards against is a landmine rather than a preference: on RabbitMQ
    4.0.5-4.1.4 a matching ``delivery-limit: -1`` makes the policy-repair tick
    rewrite the queue config every 5 s, which resets the ``expires`` idle clock,
    so no queue ever auto-reaps and the whole point of the policy is silently
    lost (rabbitmq-server#14202).
    """
    delivery_limit = _require_int_env(ENV_DELIVERY_LIMIT)
    if delivery_limit < 0:
        raise PolicyApplyError(
            f"{ENV_DELIVERY_LIMIT} is {delivery_limit}, but it must be >= 0. A "
            "negative delivery-limit triggers the RabbitMQ 4.0.5-4.1.4 "
            "policy-repair loop, which resets the expires idle clock every 5 "
            "seconds so abandoned queues never auto-reap. Refusing to apply "
            "the policy."
        )
    return delivery_limit


def _require_consumer_timeout() -> int:
    """Return the consumer timeout, refusing one below the upstream floor.

    The chart's render-time guard is the primary defense, but this module is a
    plain entrypoint that anything can run with its own environment, and the
    value it guards against is a landmine rather than a preference: RabbitMQ
    documents values under 60000 ms (60 s) as unsupported but does not reject
    them, so a plain typo of ``0`` applies cleanly and then cancels every
    consumer on every durable queue the instant it takes a delivery -- a
    permanent kill/reconnect loop across all clients, with this fail-closed
    script still reporting success.

    Only the upstream hard floor is enforced. The softer ">= 300000 (5
    minutes)" recommendation stays guidance, because the safe value depends on
    client per-message processing time this script cannot see.
    """
    consumer_timeout = _require_int_env(ENV_CONSUMER_TIMEOUT_MS)
    if consumer_timeout < 60000:
        raise PolicyApplyError(
            f"{ENV_CONSUMER_TIMEOUT_MS} is {consumer_timeout}, but it must be "
            ">= 60000 (60 seconds). RabbitMQ does not support a "
            "consumer-timeout below that, and a low value cancels healthy "
            "consumers mid-delivery -- at 0 it cancels every consumer on every "
            "durable queue immediately. Size it above "
            "durable_consumer_prefetch_count x your worst-case per-message "
            "processing time. Refusing to apply the policy."
        )
    return consumer_timeout


def build_definition() -> dict[str, int | str]:
    """Assemble the policy definition body from the chart-supplied environment."""
    definition: dict[str, int | str] = {
        "delivery-limit": _require_delivery_limit(),
        "message-ttl": _require_int_env(ENV_MESSAGE_TTL_MS),
        "max-length": _require_int_env(ENV_MAX_LENGTH),
        "overflow": _require_env(ENV_OVERFLOW),
    }
    if os.environ.get(ENV_EXPIRES_MS, "").strip():
        definition["expires"] = _require_int_env(ENV_EXPIRES_MS)
    if os.environ.get(ENV_CONSUMER_TIMEOUT_MS, "").strip():
        definition["consumer-timeout"] = _require_consumer_timeout()
    return definition


def parse_amqp_uri(uri: str) -> tuple[str | None, str]:
    """Return the (host, vhost) an AMQP URI addresses.

    The vhost is the single percent-decoded path segment; an empty path means
    the default vhost ``/``.
    """
    parsed = urllib.parse.urlsplit(uri)
    path = parsed.path.lstrip("/")
    vhost = urllib.parse.unquote(path) if path else "/"
    return parsed.hostname, vhost


def cross_check_target(vhost: str, management_url: str) -> None:
    """Fail if the policy targets a vhost the service does not consume from.

    "Applied successfully to the wrong vhost" is otherwise completely silent:
    the PUT succeeds, the Job goes green, and no queue is ever bounded. Host
    mismatch is only a warning -- a managed broker legitimately serves its
    management API from a different hostname than its AMQP endpoint.
    """
    uri = os.environ.get(ENV_RABBITMQ_URI, "").strip()
    if not uri:
        logger.warning(
            "%s is not set; skipping the vhost cross-check.", ENV_RABBITMQ_URI
        )
        return

    amqp_host, amqp_vhost = parse_amqp_uri(uri)
    if amqp_vhost != vhost:
        raise PolicyApplyError(
            f"vhost mismatch: the policy targets {vhost!r} but the service's "
            f"AMQP URI consumes from {amqp_vhost!r}. Applying it would bound a "
            "vhost the service never uses. Fix "
            "policyHook.managementApi.vhost."
        )

    management_host = urllib.parse.urlsplit(management_url).hostname
    if amqp_host and management_host and amqp_host != management_host:
        logger.warning(
            "Management API host %s differs from the AMQP host %s. This is "
            "normal for a managed broker, but check it is the same cluster.",
            management_host,
            amqp_host,
        )


def _pool_manager() -> urllib3.PoolManager:
    # Certificate verification is deliberately NOT bypassed: this call carries
    # a broker administrator credential.
    return urllib3.PoolManager(
        cert_reqs="CERT_REQUIRED", ca_certs=certifi.where(), retries=False
    )


@retry(
    stop=stop_after_attempt(_MAX_ATTEMPTS),
    wait=wait_random_exponential(multiplier=1, max=_MAX_BACKOFF_SECONDS),
    retry=retry_if_exception_type(TransientPolicyError),
    before_sleep=before_sleep_log(logger, logging.WARNING),
    reraise=True,
)
def _request(
    pool: urllib3.PoolManager,
    method: str,
    url: str,
    headers: dict[str, str],
    body: bytes | None = None,
) -> urllib3.HTTPResponse:
    try:
        response = pool.request(
            method, url, headers=headers, body=body, timeout=_TIMEOUT
        )
    except urllib3.exceptions.HTTPError as exc:
        raise TransientPolicyError(f"{method} {url} failed: {exc}") from exc
    if response.status == 429 or response.status >= 500:
        raise TransientPolicyError(f"{method} {url} returned HTTP {response.status}")
    return response


def policy_is_current(
    pool: urllib3.PoolManager,
    url: str,
    headers: dict[str, str],
    desired: dict[str, object],
) -> bool:
    """Return True when the broker already carries exactly the desired policy.

    Applying a policy resets every matched queue's ``expires`` idle clock, so an
    unconditional PUT on a deploy cadence shorter than ``expires`` would keep
    abandoned queues alive forever and auto-reap would never fire. Any read
    failure returns False so the PUT still runs -- fail-closed on the apply.

    The comparison is *semantic equality*, in both directions, over the four
    fields this chart owns: ``pattern``, ``apply-to``, ``priority``, and the
    whole ``definition`` object. ``definition`` is compared as parsed JSON, so
    key order cannot matter, and by exact equality, so a key an operator added
    by hand inside the chart's own policy is a difference and gets reverted --
    the chart's declared state wins. Only TOP-LEVEL fields the broker echoes
    back but never received (``vhost``, ``name``, and whatever a future release
    adds) are ignored; treating those as differences would re-PUT on every
    deploy and defeat the idle-clock guard above.
    """
    try:
        response = _request(pool, "GET", url, headers)
    except TransientPolicyError as exc:
        logger.warning("Could not read the existing policy (%s); applying it.", exc)
        return False
    if response.status != 200:
        # 404 means the policy does not exist yet. Any other status falls
        # through to the PUT, which reports the real failure.
        return False
    try:
        existing = json.loads(response.data)
    except _JSON_PARSE_EXC:
        return False
    if not isinstance(existing, dict):
        return False
    if existing.get("definition") != desired["definition"]:
        # `!=` on two parsed dicts is exact: set-of-keys AND values. An extra
        # definition key an operator added by hand, a missing one, or a retuned
        # value all read as a difference -- there is no subset check here.
        return False
    return all(
        existing.get(key) == value
        for key, value in desired.items()
        if key != "definition"
    )


def apply_policy() -> None:
    base_url = _require_env(ENV_MANAGEMENT_URL).rstrip("/")
    vhost = _require_env(ENV_VHOST)
    name = _require_env(ENV_NAME)
    desired: dict[str, object] = {
        "pattern": _require_env(ENV_PATTERN),
        "apply-to": _require_env(ENV_APPLY_TO),
        "priority": _require_int_env(ENV_PRIORITY),
        "definition": build_definition(),
    }

    cross_check_target(vhost, base_url)

    quoted_vhost = urllib.parse.quote(vhost, safe="")
    quoted_name = urllib.parse.quote(name, safe="")
    url = f"{base_url}/api/policies/{quoted_vhost}/{quoted_name}"

    username = _require_env(ENV_MANAGEMENT_USERNAME)
    password = _require_env(ENV_MANAGEMENT_PASSWORD)
    headers = urllib3.util.make_headers(basic_auth=f"{username}:{password}")
    headers["content-type"] = "application/json"

    pool = _pool_manager()
    if policy_is_current(pool, url, headers, desired):
        logger.info(
            "Policy %r on vhost %r already matches; skipping the PUT so matched "
            "queues keep their expires idle clocks.",
            name,
            vhost,
        )
        return

    response = _request(pool, "PUT", url, headers, json.dumps(desired).encode())
    if response.status not in (200, 201, 204):
        raise PolicyApplyError(
            f"PUT {url} returned HTTP {response.status}: "
            f"{response.data.decode(errors='replace')[:500]}"
        )
    logger.info("Applied policy %r on vhost %r: %s", name, vhost, desired)


def main() -> int:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
        stream=sys.stdout,
    )
    try:
        apply_policy()
    except PolicyApplyError as exc:
        logger.error("Failed to apply the event-queue policy: %s", exc)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
