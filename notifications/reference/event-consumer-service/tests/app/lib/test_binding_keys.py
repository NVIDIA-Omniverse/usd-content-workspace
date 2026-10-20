# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import call

import pytest
from notification_common.rabbitmq import routing

from app.lib import binding_keys
from app.lib.models.filter_group import FilterGroup
from app.lib.models.filter_type import FilterType
from app.lib.models.resource_filter import ResourceFilter


class TestBindingKeysFromFilters:
    def test_makes_keys_from_the_filter_groups(self, mocker):
        spy = mocker.patch.object(binding_keys, "_keys_for_group")

        filter1 = ResourceFilter(filter_type=FilterType.EQ, resource_id="r1")
        filter2 = ResourceFilter(filter_type=FilterType.EQ, resource_id="r2")
        group_1 = FilterGroup(event_type="e1", filters=(filter1,))
        group_2 = FilterGroup(event_type="e2", filters=(filter2,))
        binding_keys.binding_keys_from_filters([group_1, group_2])

        spy.assert_has_calls([call(group_1), call(group_2)], any_order=True)

    def test_returns_all_keys_based_on_the_filter_groups(self, mocker):
        mocker.patch.object(
            binding_keys, "_keys_for_group", side_effect=[[1, 2], [3], [4, 5, 6]]
        )

        filter1 = ResourceFilter(filter_type=FilterType.EQ, resource_id="r1")
        filter2 = ResourceFilter(filter_type=FilterType.EQ, resource_id="r2")
        filter3 = ResourceFilter(filter_type=FilterType.EQ, resource_id="r3")

        result = binding_keys.binding_keys_from_filters(
            [
                FilterGroup(event_type="e1", filters=(filter1,)),
                FilterGroup(event_type="e2", filters=(filter2,)),
                FilterGroup(event_type="e3", filters=(filter3,)),
            ]
        )
        assert result == [1, 2, 3, 4, 5, 6]


class TestKeysForGroup:
    @pytest.fixture
    def group(self):
        filter_1 = ResourceFilter(filter_type=FilterType.EQ, resource_id="r1")
        filter_2 = ResourceFilter(filter_type=FilterType.EQ, resource_id="r2")

        return FilterGroup(
            event_type="e",
            filters=(
                filter_1,
                filter_2,
            ),
        )

    class TestWithNoResourceFilters:
        def test_returns_routing_key_for_just_event_type(self):
            group = FilterGroup(event_type="e", filters=())
            assert binding_keys._keys_for_group(group) == ["e1671"]

    class TestWithBlankResourceIdInFilter:
        def test_returns_two_keys(self):
            resource_filter = ResourceFilter(
                filter_type=FilterType.STARTS_WITH_LAZY, resource_id=""
            )
            group = FilterGroup(event_type="e", filters=(resource_filter,))

            assert binding_keys._keys_for_group(group) == ["e1671", "e1671.*"]

    def test_makes_one_key_for_each_resource_filter(self, group, mocker):
        spy = mocker.patch.object(binding_keys, "_binding_key")

        binding_keys._keys_for_group(group)

        spy.assert_has_calls([call("e", group.filters[0]), call("e", group.filters[1])])

    def test_returns_all_keys(self, group, mocker):
        mocker.patch.object(binding_keys, "_binding_key", side_effect=[1, 2])
        assert binding_keys._keys_for_group(group) == [1, 2]


class TestBindingKeyForNoResourceId:
    def test_creates_binding_key_with_no_resource_id(self, mocker):
        spy = mocker.spy(binding_keys.routing, "routing_key")
        binding_keys._binding_key_for_no_resource_id("test-type")
        spy.assert_called_once_with("test-type", None)


class TestBindingKey:
    @pytest.fixture(autouse=True)
    def mock_routing_keys(self, mocker):
        mocker.patch.object(routing, "routing_key", return_value="key")
        mocker.patch.object(routing, "lazy_filter_routing_key", return_value="lazy")

    def test_uses_routing_key_as_the_base_for_eq(self, mocker):
        spy = mocker.patch.object(routing, "routing_key")
        res_filter = ResourceFilter(filter_type=FilterType.EQ, resource_id="id")
        binding_keys._binding_key("e", res_filter)

        spy.assert_called_once_with("e", "id")

    def test_adds_hash_when_type_is_greedy(self):
        res_filter = ResourceFilter(
            filter_type=FilterType.STARTS_WITH_GREEDY, resource_id="id"
        )
        assert binding_keys._binding_key("e", res_filter) == "key.#"

    def test_lazy_uses_lazy_filter_routing_key_as_the_base(self, mocker):
        spy = mocker.patch.object(routing, "lazy_filter_routing_key")
        res_filter = ResourceFilter(
            filter_type=FilterType.STARTS_WITH_LAZY, resource_id="id"
        )
        binding_keys._binding_key("e", res_filter)

        spy.assert_called_once_with("e", "id")

    def test_adds_star_when_type_is_lazy(self):
        res_filter = ResourceFilter(
            filter_type=FilterType.STARTS_WITH_LAZY, resource_id="id"
        )
        # uses the LAZY encoder, not the publisher encoder, so the
        # appended ".*" lines up with the publisher's separate "last"
        # segment on deep paths.
        assert binding_keys._binding_key("e", res_filter) == "lazy.*"

    def test_just_returns_routing_key_when_type_is_eq(self):
        res_filter = ResourceFilter(filter_type=FilterType.EQ, resource_id="id")
        assert binding_keys._binding_key("e", res_filter) == "key"
