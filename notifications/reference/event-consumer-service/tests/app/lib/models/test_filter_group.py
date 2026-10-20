# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from notification_common.errors.error_details import ErrorDetails
from notification_common.errors.errors import UnprocessableError
from notification_common.storage.event_types import (
    STORAGE_DIR_CREATED,
    STORAGE_DIR_DELETED,
    STORAGE_CREATED,
    STORAGE_EVENTS,
)
from pydantic import ValidationError

from app.lib.models.filter_group import FilterGroup
from app.lib.models.filter_type import FilterType
from app.lib.models.resource_filter import ResourceFilter
from tests.fixtures.mock_filter_group import filter_group


@pytest.fixture
def valid_resource_filter():
    return ResourceFilter(filter_type=FilterType.EQ, resource_id="test-resource")


@pytest.fixture
def valid_filter_group(valid_resource_filter):
    return FilterGroup(event_type="test-event", filters=(valid_resource_filter,))


def test_raises_value_error_for_empty_event_type(valid_resource_filter):
    with pytest.raises(ValueError, match="String should have at least 1 character"):
        FilterGroup(event_type="", filters=(valid_resource_filter,))


@pytest.mark.parametrize(
    "filter_type_value",
    [
        "eq",  # Lowercase (new format)
        "EQ",  # Uppercase (backward compatibility)
    ],
)
def test_parses_filter_type_from_json_string(filter_type_value):
    """Test parsing filter groups with both lowercase and uppercase filter types."""
    json_str = f"""[
        {{
            "event_type": "test-event",
            "filters": [
                {{
                    "filter_type": "{filter_type_value}",
                    "resource_id": "test-resource"
                }}
            ]
        }}
    ]"""

    result = FilterGroup.from_json_string(json_str)

    assert len(result) == 1
    assert result[0].event_type == "test-event"
    assert len(result[0].filters) == 1
    assert result[0].filters[0].filter_type == FilterType.EQ
    assert result[0].filters[0].resource_id == "test-resource"


def test_raises_value_error_for_invalid_json_string():
    with pytest.raises(ValidationError) as e:
        FilterGroup.from_json_string("invalid json")

    expected_error_message = "Invalid JSON: expected value at line 1 column 1"

    # noinspection PyUnresolvedReferences
    assert ErrorDetails._pydantic_friendly_error(e.value) == expected_error_message


def test_raises_value_error_for_invalid_model():
    json_str = """[
        {
            "filters": []
        }
    ]"""

    with pytest.raises(ValueError):
        FilterGroup.from_json_string(json_str)


class TestAddedGroups:
    def test_returns_groups_that_are_in_new_list_and_not_in_old_list(self, mocker):
        old_groups = [filter_group()]
        new_groups = [filter_group()]
        spy = mocker.patch.object(
            FilterGroup, "_groups_in_first_list_but_not_second", return_value="results"
        )

        assert FilterGroup.added_groups(old_groups, new_groups) == "results"

        spy.assert_called_once_with(new_groups, old_groups)


class TestRemovedGroups:
    def test_returns_groups_that_are_in_old_list_and_not_in_new_list(self, mocker):
        old_groups = [filter_group()]
        new_groups = [filter_group()]
        spy = mocker.patch.object(
            FilterGroup, "_groups_in_first_list_but_not_second", return_value="results"
        )

        assert FilterGroup.removed_groups(old_groups, new_groups) == "results"

        spy.assert_called_once_with(old_groups, new_groups)


class TestValidateStorageEventsRequireResourceFilters:
    @pytest.mark.parametrize("event_type", STORAGE_EVENTS)
    def test_raises_error_for_storage_events_with_no_resource_filter(self, event_type):
        error_msg = "Resource filters are required when consuming storage events"

        with pytest.raises(UnprocessableError, match=error_msg):
            FilterGroup(event_type=event_type, filters=())

    def test_doesnt_raise_error_for_non_storage_events_with_no_resource_filter(self):
        FilterGroup(event_type="test-type", filters=())


class TestValidateDirEventsNotAllowedWithGreedyFilterType:
    class TestWithFilterTypeGreedy:
        @pytest.mark.parametrize(
            "event_type", [STORAGE_DIR_CREATED, STORAGE_DIR_DELETED]
        )
        def test_raises_value_error_for_directory_events(self, event_type):
            greedy_filter = ResourceFilter(
                filter_type=FilterType.STARTS_WITH_GREEDY, resource_id="test-resource"
            )
            error_msg = (
                "FilterType.STARTS_WITH_GREEDY is not allowed for "
                "storage directory events"
            )

            with pytest.raises(UnprocessableError, match=error_msg):
                FilterGroup(event_type=event_type, filters=(greedy_filter,))

        def test_doesnt_raise_value_error_for_other_event_types(
            self, valid_resource_filter
        ):
            FilterGroup(event_type="event_type", filters=(valid_resource_filter,))

    class TestWithFilterTypeLazy:
        @pytest.mark.parametrize(
            "event_type", [STORAGE_DIR_CREATED, STORAGE_DIR_DELETED]
        )
        def test_doesnt_raise_value_error_for_directory_events(self, event_type):
            lazy_filter = ResourceFilter(
                filter_type=FilterType.STARTS_WITH_LAZY, resource_id="test-resource/"
            )

            FilterGroup(event_type=event_type, filters=(lazy_filter,))


class TestValidateStorageEventGroups:
    class TestWithStorageEventType:
        def test_raises_error_with_filter_type_eq(self):
            with pytest.raises(UnprocessableError, match="'EQ' not allowed"):
                filter_group(event_type=STORAGE_CREATED, filter_type=FilterType.EQ)

        def test_raises_when_resource_id_doesnt_end_in_slash(self):
            rf = ResourceFilter(
                resource_id="test-resource", filter_type=FilterType.STARTS_WITH_LAZY
            )

            with pytest.raises(
                UnprocessableError,
                match="When consuming storage events only "
                "folders values are supported",
            ):
                FilterGroup(event_type=STORAGE_CREATED, filters=(rf,))


class TestGroupsInFirstListButNotSecond:
    def test_returns_only_groups_that_are_in_the_first_list_and_not_in_the_second_list(
        self,
    ):
        only_in_first = filter_group()
        in_both = filter_group()
        only_in_second = filter_group()

        first_group = [only_in_first, in_both]
        second_group = [in_both, only_in_second]

        assert FilterGroup._groups_in_first_list_but_not_second(
            first_group, second_group
        ) == [only_in_first]


def test_extends_schema_model():
    """Verify that FilterGroup extends SchemaModel."""
    from app.lib.models.schema_model import SchemaModel

    assert issubclass(FilterGroup, SchemaModel)
    assert hasattr(FilterGroup, "simplified_schema")
    assert hasattr(FilterGroup, "add_to_openapi_schema")
