# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import re
from unittest.mock import call

import pytest

from notification_common.config.common_settings import common_settings
from notification_common.errors.errors import BadRequestError
from notification_common.rabbitmq import routing

# noinspection PyProtectedMember
from notification_common.rabbitmq.routing import (
    routing_key,
    lazy_filter_routing_key,
    max_safe_greedy_depth,
    starts_with_greedy_path_depth,
    _split_ignore_double_slash,
    hash_string,
    _encode_resource_id,
    _encode_event_type,
)


@pytest.fixture(autouse=True)
def clear_caches():
    hash_string.cache_clear()


class TestRoutingKey:
    def test_raises_error_with_missing_event_type(self):
        error_string = "event_type must not be empty"

        with pytest.raises(BadRequestError, match=error_string):
            routing_key("", "resource_id")

    def test_raises_error_when_event_type_contains_slash(self):
        error_string = re.escape("event_type must not contain '/'")
        with pytest.raises(BadRequestError, match=error_string):
            routing_key("event/type", "resource_id")

    def test_encodes_resource_id(self, mocker):
        spy = mocker.patch.object(routing, "_encode_resource_id")
        routing_key("event_type", "resource-id")
        spy.assert_called_once_with("resource-id", lazy_filter=False)

    def test_encodes_event_type(self, mocker):
        spy = mocker.patch.object(routing, "_encode_event_type")
        routing_key("event-type", "resource-id")
        spy.assert_called_once_with("event-type")

    def test_resource_ids_at_default_prefix_depth_dont_fold(self):
        # at exactly routing_max_prefix_depth (37) segments,
        # the tail-fold doesn't kick in
        resource_id = "/".join([f"part{i:02d}" for i in range(37)])
        result = routing_key("event_type", resource_id)
        # 37 hashed segments + 1 event_type hash + dot separators
        # => 37 dots between the event_type hash and the last segment
        assert result.count(".") == 37

    def test_resource_ids_past_default_prefix_depth_get_tail_folded(self):
        # 100 segments collapses to exactly routing_max_prefix_depth (37)
        # via tail-fold, so the routing key stays under the 255-byte limit
        very_deep_resource_id = "/".join([f"part{i:02d}" for i in range(100)])
        result = routing_key("event_type", very_deep_resource_id)
        assert result.count(".") == 37
        assert len(result.encode("utf-8")) <= 255

    def test_raises_clear_error_when_event_type_has_too_many_parts(self):
        # at default cap of 5, a 6-part event_type fails up front with
        # a dedicated message rather than the generic 255-byte error
        error_string = "event_type has 6 parts, maximum is 5"
        with pytest.raises(BadRequestError, match=error_string):
            routing_key("a.b.c.d.e.f", "resource")

    def test_event_type_cap_fires_regardless_of_resource_id_depth(self):
        # the over-long event_type error is independent of resource_id —
        # even a trivial resource_id triggers it, so publishers can
        # rely on event_type validation as the only failure mode
        with pytest.raises(BadRequestError, match="event_type has 7 parts"):
            routing_key("a.b.c.d.e.f.g", "x")

    def test_valid_event_type_with_deep_resource_id_never_exceeds_budget(self):
        # publishers using a max-allowed event_type (5 parts) and an
        # arbitrarily deep resource_id should always produce a routing key
        # that fits the 255-byte RabbitMQ limit
        max_event_type = ".".join(["part"] * 5)
        deep_resource_id = "/".join([f"p{i}" for i in range(1000)])
        result = routing_key(max_event_type, deep_resource_id)
        assert len(result.encode("utf-8")) <= 255

    def test_arbitrarily_deep_resource_ids_stay_under_255_bytes(self):
        # demonstrates support for very deep paths that previously errored
        absurdly_deep_resource_id = "/".join([f"p{i}" for i in range(10000)])
        result = routing_key("event_type", absurdly_deep_resource_id)
        assert len(result.encode("utf-8")) <= 255

    def test_raises_error_only_if_encoded_routing_key_still_exceeds_budget(
        self, mocker
    ):
        # if a service misconfigures settings so the encoded key exceeds
        # 255 bytes, we still raise — the existing safety check remains
        mocker.patch.object(common_settings, "routing_hash_length", 64)
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 50)
        too_long_resource_id = "/".join([f"part{i:02d}" for i in range(50)])

        error_string = "The routing_key must not exceed 255 bytes. Routing key: *"
        with pytest.raises(BadRequestError, match=error_string):
            routing_key("event_type", too_long_resource_id)

    def test_concatenates_event_type_and_resource_id_with_dot(self):
        assert routing_key("e.t", "b") == "e1671.e358e.92eb5"

    def test_handles_empty_resource_id(self):
        result = routing_key("ab", "")
        assert result == "187ef"


def _amqp_topic_matches(binding: str, routing_key_str: str) -> bool:
    """
    Textbook AMQP topic-exchange match. ``*`` matches exactly one
    segment; ``#`` matches zero or more segments.
    """
    return _match_segments(binding.split("."), routing_key_str.split("."))


def _match_segments(binding: list[str], routing: list[str]) -> bool:
    if not binding and not routing:
        return True
    if not binding:
        return False
    if binding[0] == "#":
        if len(binding) == 1:
            return True
        for i in range(len(routing) + 1):
            if _match_segments(binding[1:], routing[i:]):
                return True
        return False
    if not routing:
        return False
    if binding[0] == "*" or binding[0] == routing[0]:
        return _match_segments(binding[1:], routing[1:])
    return False


class TestLazyFilterRoutingKey:
    def test_shallow_filter_matches_event_exactly_one_segment_below(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = lazy_filter_routing_key("evt", "a/b") + ".*"
        event = routing_key("evt", "a/b/c")
        assert _amqp_topic_matches(binding, event)

    def test_shallow_filter_does_not_match_event_two_segments_below(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = lazy_filter_routing_key("evt", "a/b") + ".*"
        event = routing_key("evt", "a/b/c/d")
        assert not _amqp_topic_matches(binding, event)

    def test_shallow_filter_does_not_match_event_arbitrarily_deeper(self, mocker):
        # this is the false-positive case under the old encoding —
        # deep events tail-folded into one segment got picked up by
        # the shallow LAZY ".*". The new encoding keeps the publisher's
        # last segment separate, so the segment count carries enough
        # depth signal that ".*" is no longer fooled.
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = lazy_filter_routing_key("evt", "a/b") + ".*"
        event = routing_key("evt", "a/b/c/d/e/f/g")
        assert not _amqp_topic_matches(binding, event)

    def test_filter_at_max_depth_matches_one_segment_below(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = lazy_filter_routing_key("evt", "a/b/c/d") + ".*"
        event = routing_key("evt", "a/b/c/d/file1.usd")
        assert _amqp_topic_matches(binding, event)

    def test_filter_past_max_depth_matches_one_segment_below(self, mocker):
        # this is the case that was completely broken under the old
        # encoding — ".*" required a segment that the publisher never
        # emitted.
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = lazy_filter_routing_key("evt", "a/b/c/d/e") + ".*"
        event = routing_key("evt", "a/b/c/d/e/file1.usd")
        assert _amqp_topic_matches(binding, event)

    def test_filter_past_max_depth_does_not_match_exact_filter_path(self, mocker):
        # LAZY semantically requires one extra segment past the filter,
        # so the binding must NOT match the filter path itself.
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = lazy_filter_routing_key("evt", "a/b/c/d/e") + ".*"
        event = routing_key("evt", "a/b/c/d/e")
        assert not _amqp_topic_matches(binding, event)

    def test_filter_past_max_depth_does_not_match_two_levels_below(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = lazy_filter_routing_key("evt", "a/b/c/d/e") + ".*"
        event = routing_key("evt", "a/b/c/d/e/sub/file1.usd")
        assert not _amqp_topic_matches(binding, event)

    def test_filter_past_max_depth_does_not_match_diverging_sibling(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = lazy_filter_routing_key("evt", "a/b/c/d/e") + ".*"
        event = routing_key("evt", "a/b/c/d/x/file1.usd")
        assert not _amqp_topic_matches(binding, event)

    def test_lazy_binding_with_appended_star_stays_under_255_bytes(self):
        # arbitrarily deep filter path with the max-allowed event_type
        # plus the caller's ".*" suffix must fit the 255-byte cap.
        max_event_type = ".".join(["part"] * 5)
        deep_resource_id = "/".join([f"p{i}" for i in range(10000)])
        binding = lazy_filter_routing_key(max_event_type, deep_resource_id) + ".*"
        assert len(binding.encode("utf-8")) <= 255

    def test_raises_when_lazy_binding_with_star_would_exceed_255_bytes(self, mocker):
        # if settings are misconfigured so the prefix + ".*" would
        # exceed 255 bytes, we raise — the 255-byte invariant covers
        # the assembled binding, not just the prefix.
        mocker.patch.object(common_settings, "routing_hash_length", 64)
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 50)
        too_long_resource_id = "/".join([f"part{i:02d}" for i in range(50)])

        error_string = "The routing_key must not exceed 255 bytes. Routing key: *"
        with pytest.raises(BadRequestError, match=error_string):
            lazy_filter_routing_key("event_type", too_long_resource_id)


class TestEqFilterRoutingKey:
    """
    End-to-end checks that an EQ filter built via ``routing_key`` matches
    the publisher's routing_key for the same path, and only that path.
    These mirror what the consumer service does for STARTS_WITH_EQ
    bindings: just the routing_key, no wildcard suffix.
    """

    def test_shallow_filter_matches_exact_path(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = routing_key("evt", "a/b")
        event = routing_key("evt", "a/b")
        assert _amqp_topic_matches(binding, event)

    def test_shallow_filter_does_not_match_deeper_event(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = routing_key("evt", "a/b")
        event = routing_key("evt", "a/b/c")
        assert not _amqp_topic_matches(binding, event)

    def test_filter_past_fold_boundary_matches_exact_path(self, mocker):
        # Both sides go through the same head + middle-fold + last-
        # separate encoding when depth exceeds max_prefix_depth, so the
        # routing keys for identical paths still match byte-for-byte.
        # This is the case I'd previously only covered transitively
        # via publisher-shape tests + "EQ uses routing_key" wiring.
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = routing_key("evt", "a/b/c/d/e")
        event = routing_key("evt", "a/b/c/d/e")
        assert _amqp_topic_matches(binding, event)

    def test_filter_past_fold_boundary_does_not_match_diverging_deep_sibling(
        self, mocker
    ):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = routing_key("evt", "a/b/c/d/e")
        event = routing_key("evt", "a/b/c/d/x")
        assert not _amqp_topic_matches(binding, event)

    def test_filter_past_fold_boundary_does_not_match_one_deeper_event(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = routing_key("evt", "a/b/c/d/e")
        event = routing_key("evt", "a/b/c/d/e/f")
        assert not _amqp_topic_matches(binding, event)


class TestGreedyFilterRoutingKey:
    """
    End-to-end checks that a GREEDY filter (``routing_key + ".#"``)
    matches the publisher's routing_key for the filter's path and any
    deeper event under it, within the validated safe-depth range.
    """

    def test_shallow_filter_matches_exact_path(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = routing_key("evt", "a") + ".#"
        event = routing_key("evt", "a")
        assert _amqp_topic_matches(binding, event)

    def test_shallow_filter_matches_one_segment_below(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = routing_key("evt", "a") + ".#"
        event = routing_key("evt", "a/b/c")
        assert _amqp_topic_matches(binding, event)

    def test_shallow_filter_matches_arbitrarily_deep_event(self, mocker):
        # ".#" wildcard matches any number of trailing segments — the
        # core GREEDY guarantee. Even when the publisher's tail-fold
        # kicks in for very deep events, the head segments still match
        # the binding's head.
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = routing_key("evt", "a") + ".#"
        event = routing_key("evt", "a/b/c/d/e/f/g/h")
        assert _amqp_topic_matches(binding, event)

    def test_shallow_filter_does_not_match_diverging_sibling(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = routing_key("evt", "a") + ".#"
        event = routing_key("evt", "x/y/z")
        assert not _amqp_topic_matches(binding, event)

    def test_filter_at_max_safe_depth_matches_exact_path(self, mocker):
        # max_safe_greedy_depth() = max_prefix_depth - 2 = 2 here, so
        # this is the deepest GREEDY filter the validator allows.
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        assert max_safe_greedy_depth() == 2
        binding = routing_key("evt", "a/b") + ".#"
        event = routing_key("evt", "a/b")
        assert _amqp_topic_matches(binding, event)

    def test_filter_at_max_safe_depth_matches_one_segment_below(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = routing_key("evt", "a/b") + ".#"
        event = routing_key("evt", "a/b/c")
        assert _amqp_topic_matches(binding, event)

    def test_filter_at_max_safe_depth_matches_event_past_fold_boundary(self, mocker):
        # This is the important boundary test: at filter depth = max_safe,
        # the deeper event triggers the publisher's tail-fold (depth 6 > 4).
        # The shared head segments must still line up, and ".#" must still
        # match the folded middle + last suffix. If a future encoding
        # tweak desynced head boundaries between publisher and consumer,
        # this is the test that would catch it.
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        binding = routing_key("evt", "a/b") + ".#"
        event = routing_key("evt", "a/b/c/d/e/f")
        assert _amqp_topic_matches(binding, event)


class TestMaxSafeGreedyDepth:
    def test_is_two_less_than_routing_max_prefix_depth(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 10)
        assert max_safe_greedy_depth() == 8

    def test_tracks_default_setting(self):
        # the default routing_max_prefix_depth is 37, so the cap is 35
        assert max_safe_greedy_depth() == 35


class TestStartsWithGreedyPathDepth:
    def test_empty_resource_id_has_depth_zero(self):
        assert starts_with_greedy_path_depth("") == 0

    def test_counts_single_segment(self):
        assert starts_with_greedy_path_depth("folder_1") == 1

    def test_counts_multiple_segments(self):
        assert starts_with_greedy_path_depth("a/b/c/d") == 4

    def test_ignores_double_slashes_like_encoder(self):
        # "//host" is a single segment for routing purposes (matches the
        # encoder's _split_ignore_double_slash behaviour), so it counts
        # as one — not two.
        assert starts_with_greedy_path_depth("//host/a") == 2

    def test_ignores_trailing_slashes_like_encoder(self):
        assert starts_with_greedy_path_depth("a/b/c/") == 3
        assert starts_with_greedy_path_depth("a/b/c//") == 3


class TestEncodeEventType:
    def test_removes_starting_or_trailing_dots(self):
        assert _encode_event_type(".e.t.") == "e1671.e358e"

    def test_hashes_each_part_of_event_type(self, mocker):
        spy = mocker.patch.object(routing, "hash_string", return_value="h")
        _encode_event_type(".e.t.")
        spy.assert_has_calls([call("e", length=5), call("t", length=5)])

    def test_concatenates_hashed_parts_with_dot(self, mocker):
        mocker.patch.object(routing, "hash_string", side_effect=["h1", "h2"])
        assert _encode_event_type(".e.t.") == "h1.h2"

    def test_handles_case_with_no_dots(self):
        assert _encode_event_type("et") == "4de1b"

    def test_uses_configured_event_type_hash_length(self, mocker):
        mocker.patch.object(common_settings, "routing_event_type_hash_length", 3)
        # 3-char hex hashes for "e" and "t"
        assert _encode_event_type("e.t") == "e16.e35"


class TestEncodeResourceId:
    def test_replaces_single_slash_with_dot(self):
        assert _encode_resource_id("c/d/e/f") == "4a8a0.8277e.e1671.8fa14"

    def test_doesnt_replace_double_slash_with_dot(self):
        assert _encode_resource_id("//c//d") == "3b508"

    def test_doesnt_replace_starting_slashes_with_dot(self):
        assert _encode_resource_id("/c/d") == "c840e.8277e"

    def test_handles_resource_that_is_root_with_only_slash(self):
        assert _encode_resource_id("/") == ""

    def test_drops_rather_than_replace_trailing_slashes_with_dots(self):
        expected = "4a8a0.8277e"
        assert _encode_resource_id("c/d/") == expected
        assert _encode_resource_id("c/d//") == expected

    def test_hashes_any_path_part_of_resource_id_over_8_characters_long(self, mocker):
        mocker.patch.object(routing, "hash_string", return_value="h")
        assert _encode_resource_id("123456789") == "h"

    def test_uses_configured_hash_length_per_segment(self, mocker):
        mocker.patch.object(common_settings, "routing_hash_length", 4)
        # 4-char hex hashes for "a", "b", "c"
        assert _encode_resource_id("a/b/c") == "0cc1.92eb.4a8a"

    def test_folds_to_head_middle_last_when_path_exceeds_max_prefix_depth(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        mocker.patch.object(routing, "hash_string", side_effect=lambda s, length: s)
        # head_end = 2, so head = ["a", "b"], middle = "c/d", last = "e".
        # Last segment is kept separate so LAZY .* bindings can line up.
        assert _encode_resource_id("a/b/c/d/e") == "a.b.c/d.e"

    def test_does_not_fold_when_path_equals_max_prefix_depth(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 3)
        mocker.patch.object(routing, "hash_string", side_effect=lambda s, length: s)
        # 3 segments == max_depth: no fold, every segment hashed individually
        assert _encode_resource_id("a/b/c") == "a.b.c"

    def test_lazy_filter_mode_folds_tail_without_separate_last_segment(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        mocker.patch.object(routing, "hash_string", side_effect=lambda s, length: s)
        # LAZY mode does NOT keep the last segment separate — the ".*" the
        # caller appends takes that slot. head_end = 2, so head = ["a", "b"]
        # and tail = "c/d/e" (everything past head folded together).
        assert _encode_resource_id("a/b/c/d/e", lazy_filter=True) == "a.b.c/d/e"

    def test_lazy_filter_mode_skips_fold_when_path_fits_head(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        mocker.patch.object(routing, "hash_string", side_effect=lambda s, length: s)
        # filter has 2 segments, head_end = 2, so no fold — just emit
        # individually so the caller can append ".*" to match one more
        # publisher segment.
        assert _encode_resource_id("a/b", lazy_filter=True) == "a.b"

    def test_publisher_and_lazy_head_segments_match(self, mocker):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        # Publisher and LAZY encodings must share head segments byte-for-byte
        # so that a LAZY binding key (head + middle-fold + ".*") matches the
        # publisher's routing key (head + middle-fold + last) at the shared
        # head positions. This invariant is what makes LAZY work past the
        # depth limit.
        publisher = _encode_resource_id("a/b/c/d/e/f").split(".")
        lazy = _encode_resource_id("a/b/c/d/e", lazy_filter=True).split(".")
        # publisher: head(2) + middle("c/d/e") + last("f") = 4 segments
        # lazy:      head(2) + tail-fold("c/d/e")          = 3 segments
        # The first 3 publisher hashes equal the 3 LAZY hashes, and ".*"
        # appended to the LAZY key matches the publisher's "last" hash.
        assert publisher[:3] == lazy

    def test_publisher_keeps_last_segment_separate_even_at_minimum_overflow(
        self, mocker
    ):
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)
        mocker.patch.object(routing, "hash_string", side_effect=lambda s, length: s)
        # 5 segments with max_depth=4: head_end=2, head=["a","b"],
        # middle = "c" (single-segment fold), last = "d".
        # Total still max_depth segments, just restructured.
        assert _encode_resource_id("a/b/c/d") == "a.b.c.d"
        # Now bump to 5 segments — middle picks up a second element.
        assert _encode_resource_id("a/b/c/d/e") == "a.b.c/d.e"


class TestSplitIgnoreDoubleSlash:
    def test_ignores_double_slashes(self):
        assert _split_ignore_double_slash("//a//b") == ["//a//b"]

    def test_ignores_initial_slash(self):
        assert _split_ignore_double_slash("/a/b") == ["/a", "b"]

    def test_splits_on_slash(self):
        assert _split_ignore_double_slash("http://a/b/c") == ["http://a", "b", "c"]


class TestHash:
    def test_creates_md5_hash(self):
        assert hash_string("some long string") == "2fb66bbf"

    def test_creates_hashes_at_any_length(self):
        assert hash_string("shorty", 5) == "74c74"
