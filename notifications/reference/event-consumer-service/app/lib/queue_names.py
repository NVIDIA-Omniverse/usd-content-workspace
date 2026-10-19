# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
The broker queue-name shape, and the redactions that make a name exportable.

A durable queue's id is a plain identifier: the client chooses it (or the
server mints a uuid4 for it), it authorizes nothing on its own, and an
operator needs to read it verbatim to correlate a log line with a queue in
the broker. It is logged in full.

A NON-durable queue's id is a different thing wearing the same name. It IS
the ``reconnect_token``: the client is handed it in the stream's first
response and presents it to reattach. It is not a credential -- presenting it
requires a valid bearer token and every delivered event is authorized per
principal by the permissions policy -- but it is the queue's identifier, so an
authorized principal that obtains it could attach to the same stream and take
a share of its events, and the two API surfaces say so. An identifier that is
copied into logs, spans, and metric attributes is handed to everything
downstream of them -- log aggregation, trace storage, the metrics backend, and
everyone with read access to any of those -- so it is kept out of all three as
defense in depth. Those three sinks are closed:
every id this service writes down goes through this module first, the
uvicorn access line is rewritten on its way out (see
:mod:`app.lib.observability.access_log_redaction`), and the shared REST
tracing middleware in ``notification_common`` records a request's URL with
its query string dropped, so the ``?reconnect_token=`` form never reaches a
span either.

Both kinds of id end up inside a broker queue name, so the name inherits the
same split. This module is where that split is decided, once:

* :func:`redact_reconnect_token` turns a token into a short, stable,
  non-reversible reference -- the ``sha256-<12 hex>`` value written down as
  ``queue_ref``.
* :func:`queue_ref_for_name` gets that same reference out of a broker queue
  name, for the callers that hold a name rather than an id.
* :func:`redact_queue_name` rewrites a whole broker queue name in place,
  keeping the surrounding shape, for the callers that are logging a name as
  a name. Durable names are left untouched.

One more identifier gets the same treatment here, for the same reason. A
non-durable stream's storage-permissions cache is namespaced by the queue AND
by the principal presenting the reconnect token, so that a second principal
attaching to the same queue builds its own authorization state instead of
inheriting the first's. A principal id is an identity, so it is not written
into a Redis key or an in-process registry key verbatim either:
:func:`redact_principal_id` renders it with the same construction the token
uses, and :data:`NO_PRINCIPAL` stands in when authentication is disabled and
there is no principal to name. Neither value is exported -- see
:mod:`app.lib.permissions.storage_permissions.cache_scope`, which pairs them
with a queue name and keeps the two roles apart.

Two properties the callers rely on:

* **One rendering.** Every ``queue_ref`` in this service -- span attribute,
  metric attribute, or log field -- is the bare ``sha256-<12 hex>`` produced
  by :func:`redact_reconnect_token`, so the three join by exact match. The
  reference is wrapped back up in the ``event-queue:...:nd`` shape only
  where the thing being reported is a queue *name*, by
  :func:`redact_queue_name` here. Either way the reference inside is the
  same value.
* **Applied once, to raw names only.** Every caller passes the broker's own
  name, never a previously redacted one: the id component of a non-durable
  name is ALWAYS hashed, so feeding a redacted name back in would produce
  the digest of a digest, correlating with nothing. That is a contract the
  call sites keep, and today they do -- the liveness and cancel paths redact
  ``queue.name`` once, ``ConsumerCancelledError`` redacts its raw argument
  once inside itself, ``QueueNotFoundError`` re-renders nothing, and the
  lookup path redacts the name it just built from the client's id. Hashing
  unconditionally is what makes that contract safe to state so plainly:
  there is one rule, keyed on the name's shape, and no path where a
  reference-shaped value the client chose is passed through verbatim into a
  log line, span, or metric attribute.

It deliberately holds nothing but the naming shape and stdlib imports, so
that every layer that emits observability -- the AMQP consumers, the gRPC
servicer, the REST handlers, the uvicorn access-log filter, and the
storage-permissions cache, which keys its Redis entries on the non-durable
broker name -- can import it without importing any of the others.
"""

import hashlib

# The prefix every event queue's broker name starts with. It is also the
# selector of the `event-queue-limits` broker policy the chart applies
# (`^event-queue:`), which a chart-render test pins to this constant, so it
# is a naming contract rather than a formatting detail.
QUEUE_NAME_PREFIX = "event-queue"

# Appended to a non-durable queue's broker name to distinguish it from a
# durable queue that happens to carry the same id. It also makes a
# non-durable name recognisable from the name alone, which is what lets
# `redact_queue_name` decide whether a name needs redacting without being
# told which kind of queue it came from.
NON_DURABLE_SUFFIX = ":nd"

# The rendered form of a redacted token. It names the construction so an
# operator reading a log line knows the value is a reference and not an id
# to be pasted into a request, and it contains no `:` so a redacted name
# still splits on `:` exactly like a real one.
_REDACTED_PREFIX = "sha256-"

# How much of the digest is kept. The truncation, NOT the hash, is what
# makes the result non-reversible: 48 bits leaves roughly 2^80 tokens
# mapping onto every rendered value, so no amount of precomputation
# recovers the token from the reference. Twelve hex characters were chosen
# over eight because eight is 32 bits, where two live streams out of ~65k
# collide by birthday -- close enough to a busy deployment's concurrent
# stream count to make a shared reference an operator would misread.
# Twelve puts that boundary at ~16M concurrent streams.
_REDACTED_DIGEST_CHARS = 12

# Stands in for a principal id when authentication is disabled and there is
# none to name. It is a single shared value on purpose: with authentication
# off every stream on a queue is the same anonymous caller, so they share one
# cache scope, exactly as they shared one before the scope existed. It also
# reads as an explanation rather than as an empty field that looks like a bug,
# which is why the durable queue's create log line reports it too.
NO_PRINCIPAL = "authn-disabled"


def redact_reconnect_token(reconnect_token: str) -> str:
    """
    Render a reconnect token as a reference that is safe to export.

    Correlation is the whole point: the same token always renders to the
    same value, on every pod and across restarts, so an operator can still
    follow one non-durable stream through logs, spans, and metrics. That is
    why the digest is unsalted -- a per-process salt would make the
    reference useless for exactly the cross-pod correlation it exists for,
    and buys nothing, since the truncation above is what makes the value
    non-reversible.

    This always hashes, including a value that already looks like a
    reference, and so do :func:`redact_queue_name` and
    :func:`queue_ref_for_name`. It is the entry point for a token that came
    straight off the wire -- the ``reconnect_token`` query parameter, the
    ``Last-Event-ID`` header -- and those are logged and traced before
    anything has decided whether they name a real queue. Short-circuiting
    would let a client that sent ``sha256-<12 hex>`` as its "token" choose
    the exact ``queue_ref`` its request is recorded under, and so file its
    request line under another stream's reference. Hashing it costs nothing:
    a real token is a uuid4 and can never take that shape, so nothing
    legitimate is double-hashed by this.

    :param reconnect_token: The token (equivalently, the non-durable queue
        id) to render.

    :return: A short, stable, non-reversible reference to that token.
    """
    digest = hashlib.sha256(reconnect_token.encode("utf-8")).hexdigest()
    return f"{_REDACTED_PREFIX}{digest[:_REDACTED_DIGEST_CHARS]}"


def redact_principal_id(principal: str | None) -> str:
    """
    Render a principal id as a reference that is safe to store.

    Used to namespace a non-durable stream's storage-permissions cache by
    the principal presenting the reconnect token. It is carried inside a
    :class:`~app.lib.permissions.storage_permissions.cache_scope.CacheScope`
    key, which reaches four sinks and no others: the Redis keys, the
    in-process CacheWriter registry, the ``_active_queue_id_var``
    ContextVar that binds a request to its writer, and the metrics module's
    ``_last_queue_depth`` depth-baseline dict. All four are internal; the
    reference reaches no log line, no span, and no exported metric
    attribute -- there is no operator question that a principal component
    in an exported reference answers, and every exported ``queue_ref``
    stays queue-only so logs, spans, and metrics keep joining by exact
    match.

    The construction is :func:`redact_reconnect_token`'s. What it buys is a
    namespace separator, not a protection: a stable, recomputable value
    that keeps the raw identity out of the four sinks above. That is why it
    is unsalted -- every pod must derive the same key for the same
    principal for a reconnect on another pod to hydrate the state it left
    behind, and a per-process salt would break exactly that. Truncating
    bounds the key length and puts a birthday collision boundary at roughly
    16M live scopes, the same boundary :func:`redact_reconnect_token` chose.

    The truncation is NOT a defense against a reader who can already list
    Redis keys recovering a low-entropy principal id (an email address, a
    short user id) from a candidate list: against such a list a 48-bit
    prefix identifies the principal as surely as a full digest would. Nor
    does it need to be. That reader is already inside the trust boundary,
    where the service writes the principal id verbatim into INFO log lines:
    ``durable_queue_consumer``'s create and attach lines both log
    ``principal=%s``.

    :param principal: The principal id, or ``None`` when authentication is
        disabled and there is no principal to name.

    :return: A short, stable, non-reversible reference to that principal, or
        :data:`NO_PRINCIPAL`.
    """
    if principal is None:
        return NO_PRINCIPAL

    digest = hashlib.sha256(principal.encode("utf-8")).hexdigest()
    return f"{_REDACTED_PREFIX}{digest[:_REDACTED_DIGEST_CHARS]}"


def _non_durable_queue_id(queue_name: str) -> str | None:
    """
    Return the id component of a non-durable broker queue name, or ``None``
    when ``queue_name`` is not one.

    A name is non-durable only if it has the full ``event-queue:<id>:nd``
    shape *with* an id between the two. The emptiness check is not
    theoretical: ``event-queue:nd`` is a legal DURABLE name, because ``nd``
    is a legal queue id, and it starts and ends exactly like a non-durable
    one. Without the check its "id" is the empty slice and the name would
    render as the digest of the empty string -- a durable queue an operator
    could no longer find in the broker. Nothing else can reach the check,
    since an id may not contain ``:``.

    :param queue_name: The broker queue name to inspect.

    :return: The id component, or ``None`` if this is not a non-durable name.
    """
    prefix = f"{QUEUE_NAME_PREFIX}:"
    if not queue_name.startswith(prefix) or not queue_name.endswith(NON_DURABLE_SUFFIX):
        return None

    start = len(prefix)
    end = len(queue_name) - len(NON_DURABLE_SUFFIX)
    if end <= start:
        return None
    return queue_name[start:end]


def queue_ref_for_name(queue_name: str) -> str:
    """
    Render the ``queue_ref`` for a broker queue name.

    This is the same value :func:`redact_reconnect_token` produces for that
    queue's id -- the bare ``sha256-<12 hex>`` reference, with no surrounding
    name -- for the benefit of callers that hold a broker name rather than an
    id. Those are the storage-permissions layers, which reach it through
    :attr:`~app.lib.permissions.storage_permissions.cache_scope.CacheScope.queue_ref`:
    they address Redis and the CacheWriter registry by a scope key that
    carries the raw non-durable name, and only ever emit this reference.
    Using this rather than :func:`redact_queue_name` is what keeps their
    metric attributes and log fields joinable, by exact match, against the
    ``queue_ref`` span attribute the same stream carries.

    Pass a broker queue NAME, never a scope key. A scope key does not end in
    ``:nd``, so this function would not recognise it and would return it --
    reconnect token and all -- verbatim. ``CacheScope`` exists so that no
    caller holds a value where the two could be confused.

    A name with no embedded token -- a durable queue's, the dead-letter
    queue's, anything this module does not recognise -- is its own reference
    and is returned unchanged, exactly as :func:`redact_queue_name` returns
    it unchanged.

    The id component of a non-durable name is ALWAYS hashed, with no
    pass-through for one that already looks like a reference. Callers pass
    the broker's own name, never a name this module has already rendered;
    see the module docstring.

    :param queue_name: The broker queue name to render a reference for.

    :return: The bare reference for a non-durable name; the name itself
        otherwise.
    """
    queue_id = _non_durable_queue_id(queue_name)
    if queue_id is None:
        return queue_name
    return redact_reconnect_token(queue_id)


def redact_queue_name(queue_name: str) -> str:
    """
    Render a broker queue name in a form that is safe to export.

    A non-durable name embeds a reconnect token, so its id component is
    replaced by :func:`redact_reconnect_token` while the surrounding shape
    is kept intact -- ``event-queue:sha256-0123456789ab:nd`` still reads as
    the queue name it is. Use this where a queue *name* is being reported;
    where a reference is being reported, use :func:`queue_ref_for_name`.

    Every other name, including every durable queue name and the dead-letter
    queue's, is returned unchanged: those carry no reconnect token, and an
    operator needs them verbatim to find the queue in the broker.

    Anything that does not have the non-durable shape is passed through
    rather than guessed at, so a name this module does not recognise is
    never silently mangled into something that matches nothing. That
    includes ``event-queue:nd``, which is a legal durable name -- see
    :func:`_non_durable_queue_id`.

    The decision is made on SHAPE, and the id it finds is ALWAYS hashed --
    there is no pass-through for an id that already looks like a reference,
    so the rendering is the same whether the name describes a queue the
    broker handed back or one a lookup has only just built from a client's
    id. What that costs is idempotence: a name that has already been through
    here must not come back, or its reference becomes the digest of a
    digest. No caller does that; see the module docstring for the contract
    and why it is the safer of the two.

    :param queue_name: The broker queue name to render.

    :return: The name with any embedded reconnect token redacted.
    """
    queue_id = _non_durable_queue_id(queue_name)
    if queue_id is None:
        return queue_name

    redacted = redact_reconnect_token(queue_id)
    return f"{QUEUE_NAME_PREFIX}:{redacted}{NON_DURABLE_SUFFIX}"
