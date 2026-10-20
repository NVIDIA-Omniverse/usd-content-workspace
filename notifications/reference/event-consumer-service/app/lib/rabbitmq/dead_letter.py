# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Decode dead-letter triage metadata from a RabbitMQ message.

When a message is dead-lettered, RabbitMQ records *why* and *how many times*
it was redelivered in the message's headers — not in the JSON body. This
module provides a single pure function, :func:`decode_dead_lettered_message`,
that reads those headers (plus the JSON body) off an aio-pika incoming message
and returns a :class:`DecodedDeadLetter` with everything the DLQ browse
consumer and the servicer response-builder need. Keeping the decode pure
(no I/O, no broker calls) keeps those callers thin and unit-testable.

The ``x-death`` header
----------------------
``message.headers['x-death']`` is a list of tables (dicts), one per
origin-queue/reason pair the message has passed through. The most-recent
entry — the one describing the dead-lettering we care about — is
``x-death[0]``. Each table carries:

- ``reason``       — the AMQP dead-letter reason (see mapping below).
- ``queue``        — the origin queue name, e.g.
                     ``event-queue:<queue_id>``.
- ``count``        — times this (queue, reason) pair was recorded.
- ``time``         — when the dead-lettering occurred (a ``datetime``).
- ``exchange``     — the origin exchange.
- ``routing-keys`` — the original routing keys (a list of strings).

The quorum-queue total delivery-attempt count is the *separate* top-level
``x-delivery-count`` header, which is preferred over ``x-death[0]['count']``
when present.

AMQP reason -> proto ``Reason`` enum mapping
--------------------------------------------
The proto ``DeadLetteredEvent.Reason`` enum is spelled ``REASON_DELIVERY_LIMIT``
/ ``REASON_EXPIRED`` / ``REASON_MAXLEN`` (with ``REASON_UNSPECIFIED`` as the
default). The broker reports lowercase AMQP reasons, which map as follows:

- ``delivery_limit`` -> ``REASON_DELIVERY_LIMIT``
- ``rejected``       -> ``REASON_DELIVERY_LIMIT`` (quorum queues report the
                        delivery-limit / poison case as ``rejected``)
- ``expired``        -> ``REASON_EXPIRED``
- ``maxlen``         -> ``REASON_MAXLEN``
- anything else / missing ``x-death`` -> ``REASON_UNSPECIFIED``

This module returns the proto enum *name* as a string so it stays free of a
protobuf import; callers resolve the name against the generated enum.

Defensive decoding
-------------------
A message with a missing or empty ``x-death``, a missing ``x-delivery-count``,
or a malformed timestamp must not crash the stream: such cases are logged at
debug level and fall back to sensible defaults. This function never raises for
a header-shape problem. A malformed JSON *body* may still raise, matching the
existing parse behavior in ``RabbitMQBase._parse_and_authorize``.
"""

import datetime
import json
import logging
from dataclasses import dataclass, field
from typing import Optional

from app.lib.models.event_message import EventMessage

logger = logging.getLogger(__name__)

# The proto Reason enum default, used when x-death is absent or carries an
# unrecognized reason.
REASON_UNSPECIFIED = "REASON_UNSPECIFIED"

# Maps the lowercase AMQP dead-letter reason reported by the broker in
# x-death[0]['reason'] to the proto DeadLetteredEvent.Reason enum name.
# Quorum queues surface the delivery-limit / poison case as "rejected", so
# both "rejected" and "delivery_limit" map to REASON_DELIVERY_LIMIT.
AMQP_REASON_TO_PROTO_REASON = {
    "delivery_limit": "REASON_DELIVERY_LIMIT",
    "rejected": "REASON_DELIVERY_LIMIT",
    "expired": "REASON_EXPIRED",
    "maxlen": "REASON_MAXLEN",
}

# Exceptions that datetime.fromtimestamp can raise for a malformed epoch
# x-death time. Kept as a module-level tuple so the except clause references it
# by name: Black 25 strips the parens from `except (A, B, C):`, and the
# resulting parenless form fails to parse on Python 3.13 (see issues ticket
# b.8qg). Black cannot strip parens it did not add, so the constant form
# survives lint and parses on 3.13.
_DECODE_TIME_EXC = (ValueError, OverflowError, OSError)


@dataclass
class DecodedDeadLetter:
    """
    Structured triage metadata decoded from a dead-lettered message.

    Header-derived fields:

    :param reason: The proto ``DeadLetteredEvent.Reason`` enum *name*
        (e.g. ``"REASON_DELIVERY_LIMIT"``), or ``"REASON_UNSPECIFIED"`` when the
        message has no ``x-death`` or an unrecognized reason.
    :param origin_queue: The origin queue name from ``x-death[0]['queue']``,
        or ``""`` when unavailable.
    :param delivery_count: The broker's delivery-attempt count, preferring the
        top-level ``x-delivery-count`` header and falling back to
        ``x-death[0]['count']``; ``0`` when neither is present.
    :param dead_lettered_at: The dead-letter timestamp from
        ``x-death[0]['time']`` as a ``datetime``, or ``None`` when missing or
        malformed.
    :param original_routing_keys: The original routing keys from
        ``x-death[0]['routing-keys']``; an empty list when unavailable.

    Body-derived fields (decoded from the JSON body via :class:`EventMessage`):

    :param event_type: The event type from the body, or ``"unknown"``.
    :param resource_id: The resource id from the body, or ``""``.
    :param principal_identity: The principal identity from the body, or ``""``.
    :param message: The full decoded JSON body (the event payload).
    :param delivery_tag: The session-scoped delivery tag assigned by the browse
        consumer's :class:`AckTracker` (the handle the servicer uses for
        REPLAY / DISCARD / LEAVE). The pure decode helper does not know the tag,
        so it defaults to ``None`` and is set post-construction by
        ``DeadLetterConsumer._browse_with_client_acks``.
    """

    reason: str
    origin_queue: str
    delivery_count: int
    dead_lettered_at: Optional[datetime.datetime]
    original_routing_keys: list[str]
    event_type: str
    resource_id: str
    principal_identity: str
    message: dict = field(default_factory=dict)
    delivery_tag: Optional[int] = None


def _map_reason(amqp_reason: object) -> str:
    """Map an AMQP x-death reason to a proto Reason enum name."""
    if isinstance(amqp_reason, str):
        return AMQP_REASON_TO_PROTO_REASON.get(amqp_reason, REASON_UNSPECIFIED)
    return REASON_UNSPECIFIED


def _first_x_death_entry(headers: dict) -> dict:
    """
    Return the most-recent x-death table, or an empty dict.

    Never raises for a missing/empty/malformed x-death header.
    """
    x_death = headers.get("x-death")
    if not isinstance(x_death, list) or not x_death:
        logger.debug("Dead-lettered message has no x-death header")
        return {}
    entry = x_death[0]
    if not isinstance(entry, dict):
        logger.debug("x-death[0] is not a table: %r", entry)
        return {}
    return entry


def _decode_delivery_count(headers: dict, x_death_entry: dict) -> int:
    """
    Resolve the delivery-attempt count.

    Prefers the top-level ``x-delivery-count`` (the quorum-queue total) and
    falls back to ``x-death[0]['count']``; ``0`` when neither is usable.
    """
    for candidate in (headers.get("x-delivery-count"), x_death_entry.get("count")):
        if isinstance(candidate, bool):
            continue
        if isinstance(candidate, int):
            return candidate
        if isinstance(candidate, str):
            try:
                return int(candidate)
            except ValueError:
                logger.debug("Non-integer delivery count header: %r", candidate)
    return 0


def _decode_dead_lettered_at(
    x_death_entry: dict,
) -> Optional[datetime.datetime]:
    """
    Normalize ``x-death[0]['time']`` to a ``datetime``.

    aio-pika decodes the AMQP timestamp field to a ``datetime``; integer epoch
    seconds are also accepted defensively. Returns ``None`` for a missing or
    malformed value (logged at debug).
    """
    raw_time = x_death_entry.get("time")
    if raw_time is None:
        return None
    if isinstance(raw_time, datetime.datetime):
        return raw_time
    if isinstance(raw_time, bool):
        logger.debug("x-death[0]['time'] is a bool, ignoring")
        return None
    if isinstance(raw_time, (int, float)):
        try:
            return datetime.datetime.fromtimestamp(raw_time, tz=datetime.timezone.utc)
        except _DECODE_TIME_EXC:
            logger.debug("Malformed epoch x-death time: %r", raw_time)
            return None
    logger.debug("Unrecognized x-death time type: %r", raw_time)
    return None


def _decode_routing_keys(x_death_entry: dict) -> list[str]:
    """Return ``x-death[0]['routing-keys']`` as a list of strings."""
    keys = x_death_entry.get("routing-keys")
    if not isinstance(keys, list):
        return []
    return [str(key) for key in keys]


def decode_dead_lettered_message(message) -> DecodedDeadLetter:
    """
    Decode triage metadata and body fields from a dead-lettered message.

    :param message: An aio-pika incoming message (anything exposing
        ``.headers`` and ``.body``).
    :returns: A :class:`DecodedDeadLetter` with header- and body-derived
        fields.
    :raises: Only the JSON decode of a malformed body may raise (matching
        ``RabbitMQBase._parse_and_authorize``); header-shape problems never
        raise and instead fall back to defaults.
    """
    headers = message.headers if isinstance(message.headers, dict) else {}

    x_death_entry = _first_x_death_entry(headers)
    reason = _map_reason(x_death_entry.get("reason"))
    origin_queue = x_death_entry.get("queue") or ""
    delivery_count = _decode_delivery_count(headers, x_death_entry)
    dead_lettered_at = _decode_dead_lettered_at(x_death_entry)
    original_routing_keys = _decode_routing_keys(x_death_entry)

    # Reuse the existing JSON-body parse path rather than duplicating it. A
    # malformed body raises here, matching _parse_and_authorize.
    event_message = EventMessage(json.loads(message.body.decode("utf-8")))
    body = event_message.message

    return DecodedDeadLetter(
        reason=reason,
        origin_queue=str(origin_queue),
        delivery_count=delivery_count,
        dead_lettered_at=dead_lettered_at,
        original_routing_keys=original_routing_keys,
        event_type=event_message.event_type,
        resource_id=body.get("resource_id") or "",
        principal_identity=body.get("principal_identity") or "",
        message=body,
    )
