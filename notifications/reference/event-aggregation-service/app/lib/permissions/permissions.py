# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from notification_common.config.common_settings import common_settings
from notification_common.permissions import permissions_client
from notification_common.permissions.models import Resource

PUBLISH_ACTION = "publish-event"


async def raise_if_cant_publish(event_types: set[str]) -> None:
    if not common_settings.permissions_enabled:
        return

    for event_type in event_types:
        resource = Resource(type="EventType", id=event_type)

        if not await permissions_client.can(PUBLISH_ACTION, resource=resource):
            raise PermissionError(f"Not allowed to publish events of type {event_type}")
