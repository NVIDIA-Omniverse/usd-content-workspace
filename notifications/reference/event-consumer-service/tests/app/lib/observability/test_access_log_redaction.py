# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Unit tests for ``app.lib.observability.access_log_redaction``.

Logging is normally not the contract, but here it IS: this module exists
for no other reason than to keep a non-durable queue's identifier out of
one specific log stream, as defense in depth. The REST stream endpoint
accepts its reconnect token as a ``?reconnect_token=`` query parameter,
uvicorn's access logger writes the request line WITH its raw query string,
and every request carrying the query form would otherwise put the token
verbatim into this service's stdout and from there into whatever
aggregates it.

So the assertions below are about emitted text, deliberately. They cover
the three layers in turn: the substitution itself, the ``logging.Filter``
that applies it to a record uvicorn's way (a format string whose ARGUMENT
carries the path), and the installation that puts that filter on the
access logger before uvicorn can log anything.
"""

import logging
import uuid

import pytest

from app.lib.observability.access_log_redaction import (
    AccessLogRedactionFilter,
    FILTER_GROUPS_PLACEHOLDER,
    install_access_log_redaction,
    redact_filter_groups,
    redact_reconnect_tokens,
)
from app.lib.queue_names import redact_reconnect_token
from tests.fixtures.reconnect_token import RECONNECT_TOKEN as TOKEN

#: The reference the token must be replaced by. It is the SAME value every
#: other observability surface in the service reports for that stream, which
#: is what lets an operator correlate an access line with the stream's spans,
#: metrics, and log lines without being handed the token itself.
TOKEN_REF = redact_reconnect_token(TOKEN)

#: Uvicorn's access format string. It is a format string whose interesting
#: part rides in the ARGUMENTS -- the path with its query string is the
#: second positional -- which is why the filter has to rewrite `record.args`
#: and not just `record.msg`.
UVICORN_ACCESS_MSG = '%s - "%s %s HTTP/%s" %d'

#: The service's own access logger name, the default this module installs on.
UVICORN_ACCESS_LOGGER = "uvicorn.access"


def make_uvicorn_record(path: str) -> logging.LogRecord:
    """
    Build a record shaped exactly like the one uvicorn's access logger
    emits, with ``path`` in the positional argument that carries the path
    and query string.
    """
    return logging.LogRecord(
        name=UVICORN_ACCESS_LOGGER,
        level=logging.INFO,
        pathname=__file__,
        lineno=1,
        msg=UVICORN_ACCESS_MSG,
        args=("127.0.0.1:54321", "GET", path, "1.1", 200),
        exc_info=None,
    )


class TestRedactReconnectTokens:

    def test_replaces_the_token_with_its_reference(self):
        assert redact_reconnect_tokens(f"?reconnect_token={TOKEN}") == (
            f"?reconnect_token={TOKEN_REF}"
        )

    def test_the_result_does_not_contain_the_token(self):
        assert TOKEN not in redact_reconnect_tokens(f"?reconnect_token={TOKEN}")

    def test_keeps_the_parameter_name(self):
        # The line has to stay readable as a request line: only the value
        # is the queue identifier.
        assert "reconnect_token=" in redact_reconnect_tokens(
            f"?reconnect_token={TOKEN}"
        )

    def test_redacts_every_occurrence(self):
        text = f"?reconnect_token={TOKEN}&x=1&reconnect_token={TOKEN}"

        assert TOKEN not in redact_reconnect_tokens(text)

    def test_leaves_a_following_parameter_intact(self):
        # `&` terminates the value; swallowing what follows would eat the
        # rest of the request line.
        text = f"?reconnect_token={TOKEN}&page=2"

        assert redact_reconnect_tokens(text).endswith("&page=2")

    def test_leaves_a_preceding_parameter_intact(self):
        text = f"?page=2&reconnect_token={TOKEN}"

        assert redact_reconnect_tokens(text).startswith("?page=2&")

    def test_stops_at_the_closing_quote_of_a_request_line(self):
        # Uvicorn's format closes the request line with `"`, which is one of
        # the terminators the pattern recognises.
        text = f'"GET /events?reconnect_token={TOKEN} HTTP/1.1"'

        assert redact_reconnect_tokens(text).endswith(' HTTP/1.1"')

    def test_stops_at_a_fragment(self):
        text = f"/events?reconnect_token={TOKEN}#section"

        assert redact_reconnect_tokens(text).endswith("#section")

    def test_decodes_a_percent_encoded_value_before_hashing(self):
        # The reference has to be the one derived from the token the app
        # actually saw, or the access line correlates with nothing.
        encoded = "a%2Fb%2Bc"

        assert redact_reconnect_tokens(f"?reconnect_token={encoded}") == (
            f"?reconnect_token={redact_reconnect_token('a/b+c')}"
        )

    def test_leaves_an_empty_value_alone(self):
        # There is no token to redact, and rendering the digest of the empty
        # string would invent a reference to a queue that does not exist.
        assert redact_reconnect_tokens("?reconnect_token=") == "?reconnect_token="

    def test_leaves_an_empty_value_alone_when_another_parameter_follows(self):
        text = "?reconnect_token=&page=2"

        assert redact_reconnect_tokens(text) == text

    @pytest.mark.parametrize(
        "parameter",
        ["Reconnect_Token", "RECONNECT_TOKEN"],
        ids=["mixed-case", "upper-case"],
    )
    def test_matches_the_parameter_case_insensitively(self, parameter):
        # A mis-cased parameter is not read as a token by the app, but it
        # costs nothing to decline to log it either.
        assert TOKEN not in redact_reconnect_tokens(f"?{parameter}={TOKEN}")

    def test_redacts_a_percent_encoded_parameter_name(self):
        # Starlette decodes query keys, so `reconnect%5Ftoken=` reaches the
        # handler as `reconnect_token`.
        assert TOKEN not in redact_reconnect_tokens(f"?reconnect%5Ftoken={TOKEN}")

    def test_returns_text_without_the_parameter_unchanged(self):
        text = '127.0.0.1:0 - "GET /v1/events?page=2 HTTP/1.1" 200'

        assert redact_reconnect_tokens(text) == text


class TestAccessLogRedactionFilter:

    @pytest.fixture
    def log_filter(self):
        return AccessLogRedactionFilter()

    def test_returns_true_so_the_record_is_still_emitted(self, log_filter):
        # This filter exists to edit, not to drop: rejecting the record
        # would silently delete the access log rather than redact it.
        assert log_filter.filter(make_uvicorn_record("/v1/events")) is True

    def test_redacts_the_token_in_a_positional_argument(self, log_filter):
        record = make_uvicorn_record(f"/v1/events?reconnect_token={TOKEN}")

        log_filter.filter(record)

        assert record.args[2] == f"/v1/events?reconnect_token={TOKEN_REF}"

    def test_the_formatted_uvicorn_record_does_not_contain_the_token(self, log_filter):
        # The whole point: redacting only `msg` would redact nothing at all,
        # because uvicorn puts the query string in an argument.
        record = make_uvicorn_record(f"/v1/events?reconnect_token={TOKEN}")

        log_filter.filter(record)

        assert TOKEN not in record.getMessage()

    def test_leaves_the_other_positional_arguments_untouched(self, log_filter):
        record = make_uvicorn_record(f"/v1/events?reconnect_token={TOKEN}")

        log_filter.filter(record)

        assert record.args[0] == "127.0.0.1:54321"

    def test_leaves_non_string_arguments_untouched(self, log_filter):
        # The status code is an int; the rewrite must not stringify it, or
        # the `%d` in uvicorn's format string stops formatting.
        record = make_uvicorn_record(f"/v1/events?reconnect_token={TOKEN}")

        log_filter.filter(record)

        assert record.args[4] == 200

    def test_redacts_the_token_in_the_message_itself(self, log_filter):
        # Not uvicorn's shape, but a library that pre-formats its message
        # would otherwise slip through.
        record = make_uvicorn_record("/v1/events")
        record.msg = f"GET /v1/events?reconnect_token={TOKEN}"
        record.args = None

        log_filter.filter(record)

        assert record.msg == f"GET /v1/events?reconnect_token={TOKEN_REF}"

    def test_redacts_the_token_in_a_mapping_argument(self, log_filter):
        # `logging` keeps a lone mapping as the args itself, for
        # `%(name)s`-style formatting.
        record = make_uvicorn_record("/v1/events")
        record.msg = "%(path)s"
        record.args = {"path": f"/v1/events?reconnect_token={TOKEN}"}

        log_filter.filter(record)

        assert record.args["path"] == f"/v1/events?reconnect_token={TOKEN_REF}"

    def test_leaves_a_record_without_a_token_unchanged(self, log_filter):
        record = make_uvicorn_record("/v1/events?page=2")
        original = record.getMessage()

        log_filter.filter(record)

        assert record.getMessage() == original


#: A filter-group value as a browser or ``urllib`` sends it: URL-encoded JSON
#: whose resource id is a client folder path.
SECRET_PATH = "/secret-tenant/private/folder/"
ENCODED_FILTER_GROUPS = (
    "%5B%7B%22event_type%22%3A%22STORAGE_CREATED%22%2C%22filters%22%3A%5B%7B"
    "%22filter_type%22%3A%22EQ%22%2C%22resource_id%22%3A%22"
    "%2Fsecret-tenant%2Fprivate%2Ffolder%2F%22%7D%5D%7D%5D"
)
#: The same value sent unencoded, which a curl reproduction can do.
RAW_FILTER_GROUPS = (
    '[{"event_type":"STORAGE_CREATED","filters":[{"filter_type":"EQ",'
    f'"resource_id":"{SECRET_PATH}"}}]}}]'
)


class TestRedactFilterGroups:
    def test_replaces_an_encoded_value_with_the_placeholder(self):
        text = f"/v1/events/stream?filter_groups={ENCODED_FILTER_GROUPS}"

        assert redact_filter_groups(text) == (
            f"/v1/events/stream?filter_groups={FILTER_GROUPS_PLACEHOLDER}"
        )

    def test_replaces_a_previous_filter_groups_value(self):
        text = f"/v1/events/stream?previous_filter_groups={ENCODED_FILTER_GROUPS}"

        assert redact_filter_groups(text) == (
            f"/v1/events/stream?previous_filter_groups={FILTER_GROUPS_PLACEHOLDER}"
        )

    def test_an_unencoded_value_does_not_survive(self):
        text = f'"GET /v1/events/stream?filter_groups={RAW_FILTER_GROUPS} HTTP/1.1"'

        assert SECRET_PATH not in redact_filter_groups(text)

    def test_leaves_a_following_parameter_intact(self):
        text = f"/v1/events/stream?filter_groups={ENCODED_FILTER_GROUPS}&page=2"

        assert redact_filter_groups(text).endswith("&page=2")

    def test_leaves_an_empty_value_alone(self):
        text = "/v1/events/stream?filter_groups=&page=2"

        assert redact_filter_groups(text) == text

    def test_redacts_a_percent_encoded_parameter_name(self):
        # Starlette decodes query keys, so `filter%5Fgroups=` reaches the
        # handler as `filter_groups`.
        text = f"/v1/events/stream?filter%5Fgroups={ENCODED_FILTER_GROUPS}"

        assert "secret-tenant" not in redact_filter_groups(text)

    def test_keeps_the_end_of_the_request_line(self):
        text = (
            f'"GET /v1/events/stream?filter_groups={ENCODED_FILTER_GROUPS}' ' HTTP/1.1"'
        )

        assert redact_filter_groups(text).endswith(' HTTP/1.1"')

    def test_leaves_a_parameter_that_only_ends_with_the_name_alone(self):
        text = f"/v1/events/stream?myfilter_groups={ENCODED_FILTER_GROUPS}"

        assert redact_filter_groups(text) == text


class TestFilterGroupsInTheAccessLine:
    def test_the_formatted_uvicorn_record_does_not_name_the_path(self):
        record = make_uvicorn_record(
            f"/v1/events/stream?filter_groups={ENCODED_FILTER_GROUPS}"
            f"&previous_filter_groups={ENCODED_FILTER_GROUPS}"
            f"&reconnect_token={TOKEN}"
        )

        AccessLogRedactionFilter().filter(record)

        assert "secret-tenant" not in record.getMessage()

    def test_the_token_is_still_redacted_alongside_the_groups(self):
        record = make_uvicorn_record(
            f"/v1/events/stream?filter_groups={ENCODED_FILTER_GROUPS}"
            f"&reconnect_token={TOKEN}"
        )

        AccessLogRedactionFilter().filter(record)

        assert record.args[2].endswith(f"reconnect_token={TOKEN_REF}")


@pytest.fixture
def access_logger():
    """
    A logger standing in for ``uvicorn.access``, uniquely named so parallel
    xdist workers cannot install onto each other's, and stripped of the
    filters this module adds on teardown.

    The real access logger is deliberately NOT used here: installation is
    process-global and permanent, so a test that installed onto it would
    leak into every later test in the same worker.
    """
    logger = logging.getLogger(f"test-access-log-{uuid.uuid4()}")
    logger.setLevel(logging.INFO)
    try:
        yield logger
    finally:
        for existing in list(logger.filters):
            logger.removeFilter(existing)


def installed_filters(logger: logging.Logger) -> list:
    return [f for f in logger.filters if isinstance(f, AccessLogRedactionFilter)]


class TestInstallAccessLogRedaction:

    def test_attaches_the_filter_to_the_named_logger(self, access_logger):
        install_access_log_redaction(access_logger.name)

        assert len(installed_filters(access_logger)) == 1

    def test_a_second_call_does_not_stack_a_second_filter(self, access_logger):
        # Construction imports this module and a test may install again;
        # stacking would redact an already-redacted line, hashing the
        # reference into something that correlates with nothing.
        install_access_log_redaction(access_logger.name)
        install_access_log_redaction(access_logger.name)

        assert len(installed_filters(access_logger)) == 1

    def test_a_record_logged_through_the_logger_is_emitted_redacted(
        self, access_logger, caplog
    ):
        install_access_log_redaction(access_logger.name)

        with caplog.at_level(logging.INFO, logger=access_logger.name):
            access_logger.info(
                UVICORN_ACCESS_MSG,
                "127.0.0.1:54321",
                "GET",
                f"/v1/events?reconnect_token={TOKEN}",
                "1.1",
                200,
            )

        assert TOKEN_REF in caplog.text

    def test_a_record_logged_through_the_logger_does_not_carry_the_token(
        self, access_logger, caplog
    ):
        install_access_log_redaction(access_logger.name)

        with caplog.at_level(logging.INFO, logger=access_logger.name):
            access_logger.info(
                UVICORN_ACCESS_MSG,
                "127.0.0.1:54321",
                "GET",
                f"/v1/events?reconnect_token={TOKEN}",
                "1.1",
                200,
            )

        assert TOKEN not in caplog.text

    def test_an_uninstalled_logger_emits_the_token(self, access_logger, caplog):
        # Guards the two guards above: without the filter the token DOES
        # reach the log, so they are testing the install and not the
        # capture harness.
        with caplog.at_level(logging.INFO, logger=access_logger.name):
            access_logger.info(
                UVICORN_ACCESS_MSG,
                "127.0.0.1:54321",
                "GET",
                f"/v1/events?reconnect_token={TOKEN}",
                "1.1",
                200,
            )

        assert TOKEN in caplog.text


class TestRestServerInstallsTheFilter:
    """
    Installing on import of ``app.rest.rest_server`` is what makes the
    redaction unconditional: that module is imported to obtain the app, so
    the filter is in place before uvicorn is handed the app and long before
    it can log a request. Nothing else in the service installs it, so if
    that import-time call were dropped the access log would silently go
    back to writing tokens.
    """

    def test_the_access_logger_carries_the_filter_after_the_import(self):
        import app.rest.rest_server  # noqa: F401

        assert installed_filters(logging.getLogger(UVICORN_ACCESS_LOGGER))
