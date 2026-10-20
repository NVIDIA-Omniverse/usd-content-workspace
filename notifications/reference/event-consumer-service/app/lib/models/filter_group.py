# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import logging
from typing import List

from notification_common.errors.errors import UnprocessableError
from notification_common.storage.event_types import (
    STORAGE_DIR_CREATED,
    STORAGE_DIR_DELETED,
    is_storage_event,
)
from pydantic import (
    TypeAdapter,
    Field,
    model_validator,
    ConfigDict,
)

from app.lib.models.filter_type import FilterType
from app.lib.models.resource_filter import ResourceFilter
from app.lib.models.schema_model import SchemaModel

logger = logging.getLogger(__name__)


class FilterGroup(SchemaModel):
    event_type: str = Field(
        ...,
        min_length=1,
        max_length=1024,
        description=(
            "Event type for this filter. Other services that publish "
            "events will define unique event types."
        ),
    )

    # if there are no filters, then only messages published without
    # a resource id will be consumed
    filters: tuple[ResourceFilter, ...] = Field(
        max_length=100,
        description=(
            "Optional: the filters for filtering on resource id "
            "(filters use OR logic). If no resource filters are supplied "
            "then messages will only be filtered by event_type."
        ),
    )

    # make model hashable so we can compare them to each other
    # in the _groups_in_first_list_but_not_second function below
    model_config = ConfigDict(frozen=True)

    def is_storage_dir_event_type(self) -> bool:
        return self.event_type in [STORAGE_DIR_CREATED, STORAGE_DIR_DELETED]

    def is_storage_event_type(self) -> bool:
        return is_storage_event(self.event_type)

    @staticmethod
    def added_groups(
        old_groups: list[FilterGroup], new_groups: list[FilterGroup]
    ) -> list[FilterGroup]:
        """
        Based on two sets of groups, new and old,
        return the set of groups that are in the new group,
        and not in the old group.
        """
        return FilterGroup._groups_in_first_list_but_not_second(new_groups, old_groups)

    @staticmethod
    def removed_groups(
        old_groups: list[FilterGroup], new_groups: list[FilterGroup]
    ) -> list[FilterGroup]:
        """
        Based on two sets of groups, new and old,
        return the set of groups that are in the old group,
        and not in the new group.
        """
        return FilterGroup._groups_in_first_list_but_not_second(old_groups, new_groups)

    @model_validator(mode="after")
    def validate_storage_events_require_resource_filters(self):
        """
        If it is for storage events,
        then raise an error if there are no resource filters
        """
        if not self.is_storage_event_type():
            return self

        if not self.filters:
            raise UnprocessableError(
                "Resource filters are required when consuming storage events"
            )

        return self

    @model_validator(mode="after")
    def validate_dir_events_not_allowed_with_greedy_filter_type(self):
        """
        If the filter type is STARTS_WITH_GREEDY, then disallow directory events.
        """
        if not self.is_storage_dir_event_type():
            return self

        for resource_filter in self.filters:
            if resource_filter.filter_type == FilterType.STARTS_WITH_GREEDY:
                raise UnprocessableError(
                    "FilterType.STARTS_WITH_GREEDY is not allowed for "
                    "storage directory events"
                )

        return self

    @model_validator(mode="after")
    def validate_storage_events_resources_end_in_slash_and_dont_use_filter_type_eq(
        self,
    ):
        """
        The supported use case for storage events is only listening for
        updates to the contents of a folder.
        So make sure that any storage event resource IDs end in a slash
        and any storage event filter types are not "EQ".
        """
        if not self.is_storage_event_type():
            return self

        for resource_filter in self.filters:
            if resource_filter.filter_type == FilterType.EQ:
                # we only support looking for events in a directory,
                # so FilterType.EQ not allowed
                raise UnprocessableError(
                    "filter_type of 'EQ' not allowed for storage events"
                )

            if not resource_filter.resource_id.endswith("/"):
                # we only support looking for events in a directory,
                # so resource_id must end in slash
                raise UnprocessableError(
                    "When consuming storage events only folders values are "
                    "supported, and the resource_id value must end with a '/'"
                )

        return self

    @classmethod
    def from_json_string(cls, json_str: str) -> List[FilterGroup]:
        adapter = TypeAdapter(List[FilterGroup])
        return adapter.validate_json(json_str)

    @staticmethod
    def _groups_in_first_list_but_not_second(
        first_groups: list[FilterGroup], second_groups: list[FilterGroup]
    ) -> list[FilterGroup]:
        """
        Based on two sets of groups, first and second,
        return the set of groups that are in the first group,
        and not in the second group.
        """
        first_groups = set(first_groups)
        second_groups = set(second_groups)

        return list((first_groups | second_groups) - second_groups)
