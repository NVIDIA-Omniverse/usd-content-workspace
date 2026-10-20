# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Unit tests for ``app.lib.queue_names``.

This module decides, once, which broker queue names are safe to write down
and which embed a reconnect token -- the identifier of a non-durable queue,
kept out of telemetry as defense in depth. Every observability site in the
service -- log lines, span attributes, metric attributes -- depends on that
decision being right, so the two properties it rests on are pinned here
directly: a redacted reference never contains the value it stands for, and a
durable name is never mangled.
"""

import re

import pytest

from app.lib import queue_names
from app.lib.queue_names import (
    NON_DURABLE_SUFFIX,
    NO_PRINCIPAL,
    QUEUE_NAME_PREFIX,
    queue_ref_for_name,
    redact_principal_id,
    redact_queue_name,
    redact_reconnect_token,
)
from app.lib.rabbitmq import rabbitmq_base
from tests.fixtures.reconnect_token import RECONNECT_TOKEN as TOKEN

#: What a rendered reference looks like: the construction is named so an
#: operator knows the value is not an id to paste into a request, and the
#: digest is truncated to 12 hex characters.
REDACTED_PATTERN = r"^sha256-[0-9a-f]{12}$"

#: The dead-letter queue's name, which carries no reconnect token and which an
#: operator needs verbatim to find the queue in the broker.
DEAD_LETTER_QUEUE_NAME = "dead-letter-queue"

#: A value that already has the rendered-reference shape. It is what a
#: client would have to send to try to choose the reference its own request
#: is filed under, and it is also what a name that has already been through
#: this module carries.
REFERENCE_SHAPED = "sha256-0123456789ab"

#: Two names that LOOK non-durable and are not: `nd` is a legal durable
#: queue id, so `event-queue:nd` is a legal durable name that starts and
#: ends exactly like a non-durable one, and `event-queue::nd` has the full
#: shape with an empty id where the token would be. Redacting either would
#: render the digest of the empty string -- a queue an operator could no
#: longer find in the broker.
DURABLE_NAMES_SHAPED_LIKE_NON_DURABLE = [
    f"{QUEUE_NAME_PREFIX}{NON_DURABLE_SUFFIX}",
    f"{QUEUE_NAME_PREFIX}:{NON_DURABLE_SUFFIX}",
]

#: A principal id shaped like the real thing, and a second, different one.
#: Spelled unmistakably so a leak into a key or a log line reads as a leak.
PRINCIPAL = "principal-do-not-store-verbatim@example.com"
OTHER_PRINCIPAL = "other-principal-do-not-store-verbatim@example.com"


class TestRedactReconnectToken:

    def test_renders_the_documented_shape(self):
        assert re.fullmatch(REDACTED_PATTERN, redact_reconnect_token(TOKEN))

    def test_does_not_contain_the_token(self):
        assert TOKEN not in redact_reconnect_token(TOKEN)

    def test_is_stable_for_the_same_token(self):
        # Correlation is the whole point: the same stream must render to the
        # same reference on every pod and across restarts, which is why the
        # digest is unsalted.
        assert redact_reconnect_token(TOKEN) == redact_reconnect_token(TOKEN)

    def test_differs_for_a_different_token(self):
        assert redact_reconnect_token(TOKEN) != redact_reconnect_token(f"{TOKEN}-2")

    def test_renders_the_documented_shape_for_the_empty_token(self):
        # Total on strings: this takes raw client input, so a caller that
        # sends an empty token must get an ordinary reference back rather
        # than an exception out of a log line. (What it must NOT do is
        # stand in for a MISSING id -- callers that have none report
        # `unknown` instead, so that one constant digest does not read like
        # a real stream's reference. See `NonDurableQueueConsumer.__del__`.)
        assert re.fullmatch(REDACTED_PATTERN, redact_reconnect_token(""))

    def test_renders_the_documented_shape_for_a_non_ascii_token(self):
        # The digest is taken over UTF-8 bytes, so a token outside ASCII
        # must not raise on the way in.
        assert re.fullmatch(REDACTED_PATTERN, redact_reconnect_token("tökén-π"))

    def test_contains_no_colon(self):
        # A redacted name still has to split on `:` exactly like a real one.
        assert ":" not in redact_reconnect_token(TOKEN)

    def test_hashes_a_token_that_already_looks_like_a_reference(self):
        # This is the entry point for raw client input. Passing a
        # reference-shaped value through would let a client that sent
        # `sha256-<12 hex>` as its "token" choose the `queue_ref` its own
        # request is logged and traced under, and so file it under another
        # stream's reference. Every entry point in this module hashes
        # unconditionally for the same reason -- there is no short-circuit
        # anywhere, on the token path or the name path.
        assert redact_reconnect_token(REFERENCE_SHAPED) != REFERENCE_SHAPED

    def test_still_renders_the_documented_shape_for_a_reference_shaped_token(self):
        # Hashing it must not produce something that reads as anything
        # other than an ordinary reference.
        assert re.fullmatch(REDACTED_PATTERN, redact_reconnect_token(REFERENCE_SHAPED))


class TestRedactPrincipalId:
    """
    The reference that namespaces a storage-permissions cache by the
    principal presenting the request.

    What it buys is a namespace separator, not a protection: it has to be
    stable and recomputable across pods so a reconnect elsewhere hydrates
    the state its own principal left behind, and it has to keep the raw
    identity out of the four internal sinks the key reaches (Redis keys,
    the writer registry, the ContextVar, the metrics depth baseline).
    """

    def test_renders_the_documented_shape(self):
        assert re.fullmatch(REDACTED_PATTERN, redact_principal_id(PRINCIPAL))

    def test_does_not_contain_the_principal(self):
        assert PRINCIPAL not in redact_principal_id(PRINCIPAL)

    def test_is_stable_for_the_same_principal(self):
        # Unsalted on purpose: every pod must derive the same key for the
        # same principal, or a reconnect on another pod hydrates nothing.
        assert redact_principal_id(PRINCIPAL) == redact_principal_id(PRINCIPAL)

    def test_differs_for_a_different_principal(self):
        # This is the separation the whole design rests on: two principals
        # on one queue must not land in one namespace.
        assert redact_principal_id(PRINCIPAL) != redact_principal_id(OTHER_PRINCIPAL)

    def test_renders_the_same_reference_as_the_token_redaction_construction(self):
        # Same construction, so the two references are indistinguishable in
        # shape and a key built from both reads uniformly.
        assert redact_principal_id(PRINCIPAL) == redact_reconnect_token(PRINCIPAL)

    def test_contains_no_colon(self):
        # A scope key joins its two halves with `:`, so a reference that
        # contained one would make the key ambiguous.
        assert ":" not in redact_principal_id(PRINCIPAL)

    def test_renders_the_documented_shape_for_a_non_ascii_principal(self):
        assert re.fullmatch(REDACTED_PATTERN, redact_principal_id("prïncipäl-π"))

    def test_renders_the_documented_shape_for_the_empty_principal(self):
        assert re.fullmatch(REDACTED_PATTERN, redact_principal_id(""))

    def test_returns_the_no_principal_constant_for_none(self):
        # Authentication disabled: there is no principal to name, and every
        # stream on the queue is the same anonymous caller, so they share
        # one scope exactly as they did before the scope existed.
        assert redact_principal_id(None) == NO_PRINCIPAL

    def test_the_empty_principal_is_not_the_no_principal_constant(self):
        # An empty string is a principal the caller supplied; None is the
        # absence of one. Collapsing them would put an authenticated
        # caller into the authentication-disabled scope.
        assert redact_principal_id("") != NO_PRINCIPAL


class TestNoPrincipalConstant:

    def test_is_the_authn_disabled_sentinel(self):
        assert NO_PRINCIPAL == "authn-disabled"

    def test_reads_as_an_explanation_rather_than_a_reference(self):
        # It is reported in the durable consumer's create log line, where an
        # empty field would read like a bug and a `sha256-...` reference
        # would read like a real principal.
        assert not re.fullmatch(REDACTED_PATTERN, NO_PRINCIPAL)


class TestRedactQueueName:

    @pytest.fixture
    def non_durable_name(self):
        return f"{QUEUE_NAME_PREFIX}:{TOKEN}{NON_DURABLE_SUFFIX}"

    @pytest.fixture
    def durable_name(self):
        return f"{QUEUE_NAME_PREFIX}:my-app-queue"

    def test_redacts_a_non_durable_name(self, non_durable_name):
        assert re.fullmatch(
            rf"{QUEUE_NAME_PREFIX}:sha256-[0-9a-f]{{12}}{NON_DURABLE_SUFFIX}",
            redact_queue_name(non_durable_name),
        )

    def test_a_redacted_non_durable_name_does_not_contain_the_token(
        self, non_durable_name
    ):
        assert TOKEN not in redact_queue_name(non_durable_name)

    def test_keeps_the_non_durable_suffix(self, non_durable_name):
        # The suffix is what makes a name recognisable as non-durable from
        # the name alone, including after redaction.
        assert redact_queue_name(non_durable_name).endswith(NON_DURABLE_SUFFIX)

    def test_uses_the_same_reference_as_the_token_redaction(self, non_durable_name):
        # A log line naming the queue and a span attribute naming the token
        # have to join on the same value.
        assert redact_reconnect_token(TOKEN) in redact_queue_name(non_durable_name)

    def test_leaves_a_durable_name_unchanged(self, durable_name):
        # A durable id authorizes nothing, and an operator needs it verbatim
        # to correlate a log line with a queue in the broker.
        assert redact_queue_name(durable_name) == durable_name

    def test_leaves_the_dead_letter_queue_name_unchanged(self):
        assert redact_queue_name(DEAD_LETTER_QUEUE_NAME) == DEAD_LETTER_QUEUE_NAME

    @pytest.mark.parametrize(
        "name",
        [
            "",
            "some-other-queue",
            f"other-prefix:{TOKEN}{NON_DURABLE_SUFFIX}",
            f"{QUEUE_NAME_PREFIX}:{TOKEN}",
        ],
        ids=["empty", "no-prefix", "wrong-prefix", "no-suffix"],
    )
    def test_passes_an_unrecognised_shape_through(self, name):
        # A name this module does not recognise is never guessed at: mangling
        # it would produce something that matches nothing in the broker.
        assert redact_queue_name(name) == name

    @pytest.mark.parametrize("name", DURABLE_NAMES_SHAPED_LIKE_NON_DURABLE)
    def test_passes_a_durable_name_shaped_like_a_non_durable_one_through(self, name):
        # `nd` is a legal durable queue id, so these are real durable names
        # that begin and end exactly like a non-durable one. Redacting the
        # empty slice between the two would render the digest of the empty
        # string and lose the queue.
        assert redact_queue_name(name) == name

    def test_re_redacting_a_name_does_not_yield_the_same_reference(
        self, non_durable_name
    ):
        # NOT idempotent, deliberately: the id is always hashed, so feeding
        # a name back through produces the digest of a digest. This is the
        # cost of deciding on shape alone, and it is safe only because
        # every caller passes the broker's own name -- see the module
        # docstring's "applied once, to raw names only" contract. Pinned so
        # a future caller that re-renders an already-redacted name has to
        # come here and read why it must not.
        once = redact_queue_name(non_durable_name)

        assert redact_queue_name(once) != once

    def test_re_redacting_a_name_still_yields_the_documented_shape(
        self, non_durable_name
    ):
        # The failure mode of breaking that contract is a reference that
        # correlates with nothing -- not a mangled name or an exception.
        once = redact_queue_name(non_durable_name)

        assert re.fullmatch(
            rf"{QUEUE_NAME_PREFIX}:sha256-[0-9a-f]{{12}}{NON_DURABLE_SUFFIX}",
            redact_queue_name(once),
        )


class TestQueueRefForName:
    """
    The reference for a caller that holds a broker NAME rather than an id.

    Its whole reason to exist is that the value it returns is the BARE
    reference -- byte-identical to what ``redact_reconnect_token`` produces
    for the same queue's id -- so that the storage-permissions layers, which
    key Redis and the writer registry on the raw name, emit metric
    attributes and log fields that join a stream's spans by exact match.
    """

    @pytest.fixture
    def non_durable_name(self):
        return f"{QUEUE_NAME_PREFIX}:{TOKEN}{NON_DURABLE_SUFFIX}"

    @pytest.fixture
    def durable_name(self):
        return f"{QUEUE_NAME_PREFIX}:my-app-queue"

    def test_renders_the_same_reference_as_the_token_redaction(self, non_durable_name):
        assert queue_ref_for_name(non_durable_name) == redact_reconnect_token(TOKEN)

    def test_renders_the_bare_reference_with_no_surrounding_name(
        self, non_durable_name
    ):
        # Wrapping it back into `event-queue:...:nd` here is what would
        # break the exact-match join against the span attribute.
        assert re.fullmatch(REDACTED_PATTERN, queue_ref_for_name(non_durable_name))

    def test_does_not_contain_the_token(self, non_durable_name):
        assert TOKEN not in queue_ref_for_name(non_durable_name)

    def test_leaves_a_durable_name_unchanged(self, durable_name):
        # A durable name carries no reconnect token and is its own reference.
        assert queue_ref_for_name(durable_name) == durable_name

    def test_leaves_the_dead_letter_queue_name_unchanged(self):
        assert queue_ref_for_name(DEAD_LETTER_QUEUE_NAME) == DEAD_LETTER_QUEUE_NAME

    @pytest.mark.parametrize(
        "name",
        [
            "",
            "some-other-queue",
            f"other-prefix:{TOKEN}{NON_DURABLE_SUFFIX}",
            f"{QUEUE_NAME_PREFIX}:{TOKEN}",
        ],
        ids=["empty", "no-prefix", "wrong-prefix", "no-suffix"],
    )
    def test_passes_an_unrecognised_shape_through(self, name):
        assert queue_ref_for_name(name) == name

    @pytest.mark.parametrize("name", DURABLE_NAMES_SHAPED_LIKE_NON_DURABLE)
    def test_passes_a_durable_name_shaped_like_a_non_durable_one_through(self, name):
        assert queue_ref_for_name(name) == name

    def test_an_already_redacted_name_yields_a_different_reference(
        self, non_durable_name
    ):
        # The same "raw names only" contract `redact_queue_name` keeps: the
        # id is always hashed, so a name that has been through the
        # redaction once no longer resolves to its own stream's reference.
        # Both entry points must be fed the broker's own name.
        redacted = redact_queue_name(non_durable_name)

        assert queue_ref_for_name(redacted) != queue_ref_for_name(non_durable_name)

    def test_a_reference_shaped_id_is_hashed_rather_than_passed_through(self):
        # The property that makes deciding on shape safe on the lookup
        # path, where the id is a token straight off the wire: a client
        # presenting `sha256-<12 hex>` does not get to pick the reference
        # its own request is filed under.
        name = f"{QUEUE_NAME_PREFIX}:{REFERENCE_SHAPED}{NON_DURABLE_SUFFIX}"

        assert queue_ref_for_name(name) != REFERENCE_SHAPED


class TestNamingConstants:
    """
    The prefix and suffix are naming contracts, not formatting details: the
    chart's ``event-queue-limits`` broker policy selects on ``^event-queue:``
    and a chart-render test pins that against this constant, and the suffix
    is what lets ``redact_queue_name`` tell the two kinds of name apart.
    """

    def test_queue_name_prefix_value(self):
        assert QUEUE_NAME_PREFIX == "event-queue"

    def test_non_durable_suffix_value(self):
        assert NON_DURABLE_SUFFIX == ":nd"

    def test_rabbitmq_base_reuses_the_shared_prefix(self):
        # The chart-render test resolves the policy selector through
        # rabbitmq_base; re-declaring the prefix there would let the two
        # drift apart silently.
        assert rabbitmq_base.QUEUE_NAME_PREFIX is queue_names.QUEUE_NAME_PREFIX
