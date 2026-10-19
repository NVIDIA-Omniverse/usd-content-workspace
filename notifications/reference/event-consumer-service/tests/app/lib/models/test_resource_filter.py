# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from notification_common.config.common_settings import common_settings
from notification_common.errors.errors import UnprocessableError
from pydantic import ValidationError

from app.lib.models.filter_type import FilterType
from app.lib.models.resource_filter import ResourceFilter


def test_parses_filter_type_from_string():
    filter_data = {"filter_type": "eq", "resource_id": "test-resource"}
    flt = ResourceFilter.model_validate(filter_data)
    assert flt.filter_type == FilterType.EQ


def test_parses_filter_type_from_uppercase_string_for_backward_compatibility():
    """Test backward compatibility with old uppercase enum values."""
    filter_data = {"filter_type": "EQ", "resource_id": "test-resource"}
    flt = ResourceFilter.model_validate(filter_data)
    assert flt.filter_type == FilterType.EQ
    # Verify it serializes to the new lowercase format
    assert flt.model_dump(mode="json")["filter_type"] == "eq"


def test_parses_all_filter_types_from_uppercase_for_backward_compatibility():
    """Test backward compatibility for all uppercase enum values."""
    test_cases = [
        ("UNSPECIFIED", FilterType.UNSPECIFIED, "unspecified"),
        ("EQ", FilterType.EQ, "eq"),
        ("STARTS_WITH_LAZY", FilterType.STARTS_WITH_LAZY, "starts_with_lazy"),
        (
            "STARTS_WITH_GREEDY",
            FilterType.STARTS_WITH_GREEDY,
            "starts_with_greedy",
        ),
    ]

    for uppercase_value, expected_enum, expected_lowercase in test_cases:
        # Skip UNSPECIFIED with empty resource_id due to validation rules
        resource_id = "test-resource" if uppercase_value != "UNSPECIFIED" else ""
        if uppercase_value == "EQ":
            resource_id = "test-resource"  # EQ requires non-empty resource_id

        filter_data = {"filter_type": uppercase_value, "resource_id": resource_id}
        flt = ResourceFilter.model_validate(filter_data)
        assert flt.filter_type == expected_enum
        # Verify it serializes to the new lowercase format
        assert flt.model_dump(mode="json")["filter_type"] == expected_lowercase


def test_parses_filter_type_from_int():
    filter_data = {"filter_type": 1, "resource_id": "test-resource"}
    flt = ResourceFilter.model_validate(filter_data)
    assert flt.filter_type == FilterType.EQ


def test_raises_value_error_for_invalid_filter_type_string():
    with pytest.raises(ValidationError):
        filter_data = {"filter_type": "INVALID", "resource_id": "test-resource"}
        ResourceFilter.model_validate(filter_data)


def test_raises_error_for_filter_type_eq_and_empty_resource_id():
    ResourceFilter(filter_type=FilterType.EQ, resource_id="something")

    with pytest.raises(
        UnprocessableError, match="Resource id is required when filter type is EQ"
    ):
        ResourceFilter(filter_type=FilterType.EQ, resource_id="")


class TestStartsWithGreedyDepthValidation:
    @pytest.fixture
    def small_max_depth(self, mocker):
        # shrink the routing cap so we can exercise the boundary without
        # building a 37-segment path. With max_prefix_depth=4 the safe
        # greedy depth is 2 (max_prefix_depth - 2).
        mocker.patch.object(common_settings, "routing_max_prefix_depth", 4)

    def test_accepts_greedy_filter_at_exactly_the_cap(self, small_max_depth):
        ResourceFilter(filter_type=FilterType.STARTS_WITH_GREEDY, resource_id="a/b")

    def test_accepts_greedy_filter_below_the_cap(self, small_max_depth):
        ResourceFilter(filter_type=FilterType.STARTS_WITH_GREEDY, resource_id="a")

    def test_accepts_greedy_filter_with_empty_resource_id(self, small_max_depth):
        ResourceFilter(filter_type=FilterType.STARTS_WITH_GREEDY, resource_id="")

    def test_rejects_greedy_filter_one_segment_past_the_cap(self, small_max_depth):
        with pytest.raises(
            UnprocessableError,
            match=(
                "starts_with_greedy filter resource_id has 3 path segments, "
                "but the maximum supported is 2"
            ),
        ):
            ResourceFilter(
                filter_type=FilterType.STARTS_WITH_GREEDY, resource_id="a/b/c"
            )

    def test_rejects_greedy_filter_far_past_the_cap(self, small_max_depth):
        with pytest.raises(
            UnprocessableError,
            match="starts_with_greedy filter resource_id has 10 path segments",
        ):
            ResourceFilter(
                filter_type=FilterType.STARTS_WITH_GREEDY,
                resource_id="/".join(f"p{i}" for i in range(10)),
            )

    def test_error_message_explains_the_underlying_cause(self, small_max_depth):
        with pytest.raises(UnprocessableError) as exc_info:
            ResourceFilter(
                filter_type=FilterType.STARTS_WITH_GREEDY, resource_id="a/b/c/d"
            )
        message = str(exc_info.value)
        assert "fold" in message
        assert "'#' bindings" in message
        assert "deeper events" in message

    def test_lazy_filter_past_the_cap_is_unaffected(self, small_max_depth):
        # LAZY works at any depth under the new routing encoding;
        # the GREEDY-specific validator should not touch it.
        ResourceFilter(
            filter_type=FilterType.STARTS_WITH_LAZY,
            resource_id="a/b/c/d/e/f/g",
        )

    def test_eq_filter_past_the_greedy_cap_is_unaffected(self, small_max_depth):
        # EQ exact-match works at any depth (publisher and consumer
        # encode the path identically), so it is not capped.
        ResourceFilter(filter_type=FilterType.EQ, resource_id="a/b/c/d/e/f/g")


def test_extends_schema_model():
    """Verify that ResourceFilter extends SchemaModel."""
    from app.lib.models.schema_model import SchemaModel

    assert issubclass(ResourceFilter, SchemaModel)
    assert hasattr(ResourceFilter, "simplified_schema")
    assert hasattr(ResourceFilter, "add_to_openapi_schema")
