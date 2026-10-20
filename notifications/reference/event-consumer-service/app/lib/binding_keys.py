# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from notification_common.rabbitmq import routing

from app.lib.models.filter_group import FilterGroup
from app.lib.models.filter_type import FilterType
from app.lib.models.resource_filter import ResourceFilter


def binding_keys_from_filters(filter_groups: list[FilterGroup]) -> list[str]:
    keys = []

    for filter_group in filter_groups or []:
        keys.extend(_keys_for_group(filter_group))

    return keys


def _keys_for_group(filter_group: FilterGroup) -> list[str]:
    event_type = filter_group.event_type
    resource_filters = filter_group.filters

    if not resource_filters:
        # if there are no filters, then only messages published without
        # a resource id will be consumed
        return [_binding_key_for_no_resource_id(event_type)]

    binding_keys = []

    for resource_filter in resource_filters:
        is_blank = resource_filter.resource_id == ""

        # if there's no resource_id and the type is STARTS_WITH_LAZY
        # then in addition to the normal binding key that would
        # be created ("{event-type}.*"), we also need a binding key that
        # will get events with no resource_id
        if is_blank and resource_filter.filter_type == FilterType.STARTS_WITH_LAZY:
            binding_keys.append(_binding_key_for_no_resource_id(event_type))

        # add the normal binding keys
        binding_keys.append(_binding_key(event_type, resource_filter))

    return binding_keys


def _binding_key_for_no_resource_id(event_type: str) -> str:
    # noinspection PyTypeChecker
    return routing.routing_key(event_type, None)


def _binding_key(event_type: str, resource_filter: ResourceFilter):
    """
    Create the binding key based on the event_type and resource_filter.
    If there's no resource filter, then the client wants all events
    for the event_type, so use "" as the resource_id.

    LAZY filters use a separate encoder (``lazy_filter_routing_key``)
    that folds the resource_id tail without keeping the last segment
    separate, so the ``.*`` we append below lines up with the slot the
    publisher fills with its "last" hash. EQ and GREEDY use the same
    encoder as the publisher.
    """
    resource_id = resource_filter.resource_id
    filter_type = resource_filter.filter_type

    if filter_type == FilterType.STARTS_WITH_LAZY:
        # should match the resource_id and the next word before the next "/"
        prefix = routing.lazy_filter_routing_key(event_type, resource_id)
        return f"{prefix}.*"

    key = routing.routing_key(event_type, resource_id)

    if filter_type == FilterType.STARTS_WITH_GREEDY:
        # should match the resource_id and anything following it
        key = f"{key}.#"

    return key
