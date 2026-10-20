# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from notification_common.errors.errors import UnprocessableError
from notification_common.rabbitmq.routing import (
    max_safe_greedy_depth,
    starts_with_greedy_path_depth,
)
from pydantic import ConfigDict, model_validator, Field

from app.lib.models.filter_type import FilterType
from app.lib.models.schema_model import SchemaModel


class ResourceFilter(SchemaModel):
    """
    Filters events based on their resource id as defined by the
    publishing service.
    """

    filter_type: FilterType = Field(
        description=(
            "Specifies the type of filter to apply. "
            "How much of the resource_id needs to match the resource "
            "in the message for the filter to be successful.\n\n"
            "- unspecified: The default, unspecified filter type. "
            "This will default to eq behavior.\n"
            "- eq: Performs an exact match on the resource_id.\n"
            '- starts_with_lazy: Performs a "starts with" match '
            '(lazy). For example, a filter for "folder_1" would match '
            '"folder_1/file.txt" but not "folder_10/file.txt".\n'
            '- starts_with_greedy: Performs a "starts with" match '
            '(greedy). For example, a filter for "folder_1" would '
            'match "folder_1/sub/file.txt".'
        ),
    )
    resource_id: str = Field(
        max_length=4096,
        description=(
            "The path to filter the field by."
            "</br></br>"
            "A resource is a path like string with words separated "
            "by slashes. For example, if two messages were published for the "
            "following resources:"
            "</br>"
            "<b>folder_1/folder_2/folder_3/astronaut.usd</b> "
            "and <b>folder_1/astronaut.usd</b>, then"
            "</br>"
            "1) A filter with a resource_id of <b>folder_1</b> and "
            "<b>filter_type = starts_with_greedy</b> would match both messages."
            "</br>"
            "2) A filter with a resource_id of <b>folder_1</b> and a "
            "<b>filter_type = starts_with_lazy</b> would only match the second message."
            "</br></br>"
            'If <b>resource_id = ""</b>, then there are three possibilities '
            "depending on the value of filter_type:"
            "</br>"
            "1) starts_with_greedy: consume all messages of the "
            "event_type with or without a resource_id, "
            "regardless of what the value for resource_id in the "
            "message is; "
            "</br>"
            "2) starts_with_lazy: consume any message that doesn't have "
            'a resource_id or has a resource_id="", '
            "AND any message with a resource_id like 'something' or "
            "'something/', but not 'something/something-else'; "
            "</br>"
            "3) eq: generates an error response."
        ),
    )

    # make model hashable so we can compare them to each other
    # in the FilterGroup._groups_in_first_list_but_not_second function
    model_config = ConfigDict(frozen=True)

    @model_validator(mode="after")
    def validate_filter_type_eq_has_resource_id(self):
        if not self.filter_type == FilterType.EQ:
            return self

        if not self.resource_id:
            raise UnprocessableError("Resource id is required when filter type is EQ")

        return self

    @model_validator(mode="after")
    def validate_starts_with_greedy_depth(self):
        if self.filter_type != FilterType.STARTS_WITH_GREEDY:
            return self

        depth = starts_with_greedy_path_depth(self.resource_id)
        max_depth = max_safe_greedy_depth()
        if depth > max_depth:
            raise UnprocessableError(
                f"starts_with_greedy filter resource_id has {depth} path "
                f"segments, but the maximum supported is {max_depth}. "
                "Routing keys fold path segments past this boundary into "
                "a single hash, which prevents '#' bindings from matching "
                "deeper events. Use a shorter resource_id or restructure "
                "the publishing path."
            )

        return self
