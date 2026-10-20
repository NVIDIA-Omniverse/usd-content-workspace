# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Unit tests for ``decode_dead_lettered_message``.

Covers the happy-path x-death / header / body decode plus the defensive
fallbacks. The module's contract is that a header-shape problem on a
dead-lettered message must never crash the stream: a missing/empty ``x-death``,
an absent delivery count, a malformed timestamp, a non-list ``routing-keys``, or
a JSON body missing the optional triage fields must each fall back to a sensible
default rather than raise.
"""

import datetime

from app.lib.rabbitmq.dead_letter import decode_dead_lettered_message


class TestDecodeDeadLetteredMessage:
    """Tests for the pure x-death / header / body decode."""

    def test_decodes_reason_to_proto_enum_name(self, dead_lettered_message):
        decoded = decode_dead_lettered_message(dead_lettered_message)

        # The default fixture reports "rejected", which quorum queues use for
        # the delivery-limit / poison case -> DELIVERY_LIMIT.
        assert decoded.reason == "REASON_DELIVERY_LIMIT"

    def test_decodes_origin_queue(self, dead_lettered_message):
        decoded = decode_dead_lettered_message(dead_lettered_message)

        assert decoded.origin_queue == "origin-queue"

    def test_prefers_top_level_delivery_count(self, make_dead_lettered_message):
        message = make_dead_lettered_message(delivery_count=7, count=2)

        decoded = decode_dead_lettered_message(message)

        assert decoded.delivery_count == 7

    def test_decodes_dead_lettered_at_timestamp(self, dead_lettered_message):
        decoded = decode_dead_lettered_message(dead_lettered_message)

        assert decoded.dead_lettered_at == datetime.datetime.fromtimestamp(
            1700000000, tz=datetime.timezone.utc
        )

    def test_decodes_original_routing_keys(self, dead_lettered_message):
        decoded = decode_dead_lettered_message(dead_lettered_message)

        assert decoded.original_routing_keys == ["events.foo"]

    def test_decodes_event_type_from_body(self, dead_lettered_message):
        decoded = decode_dead_lettered_message(dead_lettered_message)

        assert decoded.event_type == "asset.updated"

    def test_decodes_resource_id_from_body(self, dead_lettered_message):
        decoded = decode_dead_lettered_message(dead_lettered_message)

        assert decoded.resource_id == "resource-123"

    def test_decodes_principal_identity_from_body(self, dead_lettered_message):
        decoded = decode_dead_lettered_message(dead_lettered_message)

        assert decoded.principal_identity == "user-abc"

    def test_decodes_full_body_payload(self, dead_lettered_message):
        decoded = decode_dead_lettered_message(dead_lettered_message)

        assert decoded.message["payload"] == {"key": "value"}


class TestDecodeMultiEntryXDeath:
    """The most-recent (first) x-death entry must be selected."""

    def test_selects_most_recent_reason(self, multi_entry_dead_lettered_message):
        decoded = decode_dead_lettered_message(multi_entry_dead_lettered_message)

        assert decoded.reason == "REASON_MAXLEN"

    def test_selects_most_recent_origin_queue(self, multi_entry_dead_lettered_message):
        decoded = decode_dead_lettered_message(multi_entry_dead_lettered_message)

        assert decoded.origin_queue == "newest-queue"

    def test_selects_most_recent_routing_keys(self, multi_entry_dead_lettered_message):
        decoded = decode_dead_lettered_message(multi_entry_dead_lettered_message)

        assert decoded.original_routing_keys == ["events.bar"]

    def test_selects_most_recent_timestamp(self, multi_entry_dead_lettered_message):
        decoded = decode_dead_lettered_message(multi_entry_dead_lettered_message)

        assert decoded.dead_lettered_at == datetime.datetime.fromtimestamp(
            1700000500, tz=datetime.timezone.utc
        )


class TestDecodeWithoutXDeath:
    """A message with no ``x-death`` header falls back to defaults."""

    def test_reason_is_unspecified(self, make_dead_lettered_message):
        message = make_dead_lettered_message()
        message.headers = {}

        decoded = decode_dead_lettered_message(message)

        assert decoded.reason == "REASON_UNSPECIFIED"

    def test_origin_queue_is_empty(self, make_dead_lettered_message):
        message = make_dead_lettered_message()
        message.headers = {}

        decoded = decode_dead_lettered_message(message)

        assert decoded.origin_queue == ""


class TestDecodeMissingDeliveryCount:
    """Absent ``x-delivery-count`` and ``count`` -> ``delivery_count == 0``."""

    def test_delivery_count_defaults_to_zero(self, make_dead_lettered_message):
        message = make_dead_lettered_message(delivery_count=None, count=None)

        decoded = decode_dead_lettered_message(message)

        assert decoded.delivery_count == 0


class TestDecodeMalformedTimestamp:
    """A missing or non-numeric ``time`` -> ``dead_lettered_at is None``."""

    def test_missing_time_yields_none(self, make_dead_lettered_message):
        message = make_dead_lettered_message(time=None)

        decoded = decode_dead_lettered_message(message)

        assert decoded.dead_lettered_at is None

    def test_non_numeric_time_yields_none(self, make_dead_lettered_message):
        message = make_dead_lettered_message(time="not-a-timestamp")

        decoded = decode_dead_lettered_message(message)

        assert decoded.dead_lettered_at is None


class TestDecodeNonListRoutingKeys:
    """A non-list ``routing-keys`` -> empty list."""

    def test_routing_keys_default_to_empty_list(self, make_dead_lettered_message):
        message = make_dead_lettered_message(routing_keys="events.foo")

        decoded = decode_dead_lettered_message(message)

        assert decoded.original_routing_keys == []


class TestDecodeBodyMissingFields:
    """A JSON body missing the optional triage fields -> empty strings."""

    def test_missing_resource_id_yields_empty_string(self, make_dead_lettered_message):
        body = {"event_type": "asset.updated", "principal_identity": "user-abc"}
        message = make_dead_lettered_message(body=body)

        decoded = decode_dead_lettered_message(message)

        assert decoded.resource_id == ""

    def test_missing_principal_identity_yields_empty_string(
        self, make_dead_lettered_message
    ):
        body = {"event_type": "asset.updated", "resource_id": "resource-123"}
        message = make_dead_lettered_message(body=body)

        decoded = decode_dead_lettered_message(message)

        assert decoded.principal_identity == ""


class TestDecodeNonTableXDeathEntry:
    """An ``x-death[0]`` that is not a dict falls back to defaults."""

    def test_reason_is_unspecified(self, make_dead_lettered_message):
        message = make_dead_lettered_message()
        message.headers = {"x-death": ["not-a-table"]}

        decoded = decode_dead_lettered_message(message)

        assert decoded.reason == "REASON_UNSPECIFIED"

    def test_origin_queue_is_empty(self, make_dead_lettered_message):
        message = make_dead_lettered_message()
        message.headers = {"x-death": ["not-a-table"]}

        decoded = decode_dead_lettered_message(message)

        assert decoded.origin_queue == ""


class TestDecodeDeliveryCountHeaderShapes:
    """Defensive resolution of the delivery-count headers."""

    def test_bool_top_level_count_is_skipped(self, make_dead_lettered_message):
        # A bool is an int subclass, but must not be read as a count: the
        # x-death count is used instead.
        message = make_dead_lettered_message(count=4)
        message.headers["x-delivery-count"] = True

        decoded = decode_dead_lettered_message(message)

        assert decoded.delivery_count == 4

    def test_string_top_level_count_is_parsed(self, make_dead_lettered_message):
        message = make_dead_lettered_message()
        message.headers["x-delivery-count"] = "9"

        decoded = decode_dead_lettered_message(message)

        assert decoded.delivery_count == 9

    def test_non_integer_string_count_falls_back_to_zero(
        self, make_dead_lettered_message
    ):
        # Both candidates are non-integer strings, so neither resolves and the
        # count falls back to 0.
        message = make_dead_lettered_message()
        message.headers["x-delivery-count"] = "not-a-number"
        message.headers["x-death"][0]["count"] = "also-bad"

        decoded = decode_dead_lettered_message(message)

        assert decoded.delivery_count == 0


class TestDecodeDeadLetteredAtShapes:
    """Defensive normalization of ``x-death[0]['time']``."""

    def test_datetime_time_is_returned_as_is(self, make_dead_lettered_message):
        stamp = datetime.datetime(2024, 1, 2, tzinfo=datetime.timezone.utc)
        message = make_dead_lettered_message(time=stamp)

        decoded = decode_dead_lettered_message(message)

        assert decoded.dead_lettered_at == stamp

    def test_bool_time_yields_none(self, make_dead_lettered_message):
        # A bool is an int subclass; it must not be read as an epoch.
        message = make_dead_lettered_message(time=True)

        decoded = decode_dead_lettered_message(message)

        assert decoded.dead_lettered_at is None

    def test_out_of_range_epoch_time_yields_none(self, make_dead_lettered_message):
        # An epoch so large that fromtimestamp raises OverflowError/OSError
        # must fall back to None rather than crash the stream.
        message = make_dead_lettered_message(time=10**20)

        decoded = decode_dead_lettered_message(message)

        assert decoded.dead_lettered_at is None
