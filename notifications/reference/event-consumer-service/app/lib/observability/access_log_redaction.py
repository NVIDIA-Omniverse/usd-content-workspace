# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Keeps reconnect tokens and filter groups out of the service's own HTTP access
log.

The REST stream endpoint accepts its reconnect token two ways: the
``Last-Event-ID`` header (what an ``EventSource`` client sends on its own,
and what SSE reconnects use in practice) and a ``reconnect_token`` query
parameter (what a plain HTTP client, a curl reproduction, or a browser that
cannot set headers uses). The token is not a credential: presenting it
requires a valid bearer token and every delivered event is authorized per
principal by the permissions policy. It is the queue's identifier, so an
authorized principal that obtains it could attach to the same stream and take
a share of its events; it is kept out of telemetry as defense in depth. That
is also why the header form is the one to prefer, precisely because a header
never reaches an access log. The query form cannot be withdrawn: it is a
published part of the API, and clients that can only put it in a URL exist.

This service's own handlers never write a token down -- every id they report
goes through :mod:`app.lib.queue_names` first. What they do not control is
the code that sees the raw query string on their behalf, and there are two
such places. The shared REST tracing middleware in ``notification_common``
was one: it now records a request's URL with the query string dropped, so
the token never reaches a span. Uvicorn's access logger is the other, and it
is the one this module closes: it emits the request line built by
``uvicorn.protocols.utils.get_path_with_query_string``, which is the path
WITH the raw query string, so every request carrying the query form would
write the token verbatim into this service's stdout -- and from there into
whatever aggregates it.

So this module installs a filter on the ``uvicorn.access`` logger that
rewrites ``reconnect_token=<token>`` to the same redacted ``queue_ref`` every
other observability surface here reports (see :mod:`app.lib.queue_names`).
An operator reading an access line can still correlate it with that stream's
spans, metrics, and log lines; nobody reading it can replay the stream.

The same request line carries the stream's ``filter_groups`` and
``previous_filter_groups`` query parameters: URL-encoded JSON whose resource
ids are the client's folder paths. The filter replaces each of those values
with ``<redacted>``, keeping the parameter name so the line still shows which
parameters the request used.

Filtering rather than ``access_log=False`` is deliberate: the access log is
worth keeping. Filtering rather than a URL-rewriting middleware is likewise
deliberate -- uvicorn logs from the ASGI scope, which no middleware can edit
on the way back out, and the request's own handlers need the real token.
"""

import logging
import re
import urllib.parse
from collections.abc import Mapping

from app.lib.queue_names import redact_reconnect_token

# One query parameter: a name and its value, each after the ``?`` or ``&``
# that starts it, the value ending at the next ``&`` or at whitespace
# (uvicorn's request line puts a space before ``HTTP/``). Names are compared
# after percent-decoding and lower-casing: the app decodes them too, so
# ``filter%5Fgroups=`` reaches the handler as ``filter_groups``, and a
# mis-cased name costs nothing to decline to log. Every other parameter is
# left exactly as logged.
_PARAM_RE = re.compile(r"(?<=[?&])([^=&\s]+)=([^&\s]*)")

_TOKEN_PARAM_NAMES = frozenset({"reconnect_token"})
_FILTER_GROUPS_PARAM_NAMES = frozenset({"filter_groups", "previous_filter_groups"})

# Where a token value ends inside the captured value: a fragment, or the
# quote that closes a request line logged as one string. A token is a uuid4,
# so none of these can be part of it.
_TOKEN_VALUE_END_RE = re.compile(r"[#\"']")

#: What a filter-group value is replaced with.
FILTER_GROUPS_PLACEHOLDER = "<redacted>"


def _param_name(raw_name: str) -> str:
    return urllib.parse.unquote_plus(raw_name).lower()


def _redact_token_match(match: re.Match) -> str:
    """
    Replace one reconnect-token value with its redacted form.

    The value is percent-decoded before it is redacted. Tokens are uuid4s,
    so in practice nothing needs decoding, but a client is free to encode
    the parameter, and the reference has to be the one derived from the
    token the app actually saw -- otherwise the access line correlates with
    nothing.

    An empty value is left alone: there is no token to redact, and rendering
    the digest of the empty string would invent a reference to a queue that
    does not exist.
    """
    raw_name, value = match.group(1), match.group(2)
    if _param_name(raw_name) not in _TOKEN_PARAM_NAMES:
        return match.group(0)
    raw_token = _TOKEN_VALUE_END_RE.split(value, maxsplit=1)[0]
    token_end = len(raw_token)
    rest = value[token_end:]
    if not raw_token:
        return match.group(0)
    token = urllib.parse.unquote_plus(raw_token)
    return f"{raw_name}={redact_reconnect_token(token)}{rest}"


def redact_reconnect_tokens(text: str) -> str:
    """
    Return ``text`` with every reconnect-token query value redacted.

    :param text: Any text that may contain a ``reconnect_token=`` parameter.

    :return: The text with each such value replaced by its ``queue_ref``,
        or the text unchanged when there is nothing to redact.
    """
    return _PARAM_RE.sub(_redact_token_match, text)


def _redact_filter_groups_match(match: re.Match) -> str:
    raw_name, value = match.group(1), match.group(2)
    if not value or _param_name(raw_name) not in _FILTER_GROUPS_PARAM_NAMES:
        return match.group(0)
    return f"{raw_name}={FILTER_GROUPS_PLACEHOLDER}"


def redact_filter_groups(text: str) -> str:
    """
    Return ``text`` with every ``filter_groups`` / ``previous_filter_groups``
    query value replaced by :data:`FILTER_GROUPS_PLACEHOLDER`.

    The value runs to the next ``&`` or whitespace, so a quote or ``#`` inside
    an unencoded value does not end it early. An empty value is left alone:
    there is nothing in it to hide.
    """
    return _PARAM_RE.sub(_redact_filter_groups_match, text)


def redact_access_line(text: str) -> str:
    """Return ``text`` with reconnect tokens and filter groups redacted."""
    return redact_reconnect_tokens(redact_filter_groups(text))


class AccessLogRedactionFilter(logging.Filter):
    """
    Redacts reconnect tokens and filter groups from a log record in place.

    Rewrites ``record.args`` as well as ``record.msg`` because uvicorn's
    access record is a format string whose arguments carry the interesting
    part: it logs ``'%s - "%s %s HTTP/%s" %d'`` and passes the path with its
    query string as a positional argument. Redacting only the message would
    redact nothing at all.

    Never rejects a record -- this filter exists to edit, not to drop.
    """

    def filter(self, record: logging.LogRecord) -> bool:
        if isinstance(record.msg, str):
            record.msg = redact_access_line(record.msg)

        args = record.args
        if isinstance(args, tuple):
            record.args = tuple(
                redact_access_line(arg) if isinstance(arg, str) else arg for arg in args
            )
        elif isinstance(args, Mapping):
            # `logging` keeps a lone mapping argument as the args itself,
            # for `%(name)s`-style formatting, instead of wrapping it in a
            # tuple. Uvicorn does not log that way, but other libraries do.
            record.args = {
                key: redact_access_line(value) if isinstance(value, str) else value
                for key, value in args.items()
            }

        return True


def install_access_log_redaction(logger_name: str = "uvicorn.access") -> None:
    """
    Install :class:`AccessLogRedactionFilter` on the access logger.

    Idempotent: a second call is a no-op, so importing this at REST-server
    construction and again from a test does not stack filters.

    A filter attached to the logger (rather than to a handler) is applied by
    ``Logger.handle`` before the record is passed to any handler, including
    the root handler it reaches by propagation. That matters here because
    the service configures logging with ``logging.basicConfig`` and hands
    uvicorn ``log_config=None``: uvicorn installs no handler of its own and
    the access records propagate to root. It also means the filter survives
    ``configure_logging()`` whichever order the two run in -- ``basicConfig``
    only ever touches the root logger's handlers, never this logger's
    filters.

    :param logger_name: The logger to install on. Defaults to uvicorn's
        access logger; parameterised for tests.
    """
    logger = logging.getLogger(logger_name)
    for existing in logger.filters:
        if isinstance(existing, AccessLogRedactionFilter):
            return
    logger.addFilter(AccessLogRedactionFilter())
