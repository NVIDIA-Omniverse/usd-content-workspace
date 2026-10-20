# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import random
import string

from notification_common.storage.event_types import STORAGE_CREATED

from app.lib.models.filter_group import FilterGroup
from app.lib.models.filter_type import FilterType
from app.lib.models.resource_filter import ResourceFilter


def _path_part():
    return "".join(random.choices(string.ascii_letters, k=3))


def filter_group(
    event_type=STORAGE_CREATED,
    filter_count=1,
    filter_type=FilterType.STARTS_WITH_LAZY,
    resource_id=None,
):
    if not resource_id:
        resource_id = f"/{_path_part()}/{_path_part()}/{_path_part()}/"

    filters = [
        ResourceFilter(
            filter_type=filter_type,
            resource_id=resource_id,
        )
        for _ in range(filter_count)
    ]

    return FilterGroup(event_type=event_type, filters=tuple(filters))
