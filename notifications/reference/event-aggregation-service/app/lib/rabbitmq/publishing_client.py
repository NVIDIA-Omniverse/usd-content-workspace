# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from app.lib.permissions.permissions import raise_if_cant_publish
from notification_common.rabbitmq.omni_event import OmniEvent
from notification_common.rabbitmq.publisher import Publisher, PublishingResult


class PublishingClient:
    """
    Subclass of Publisher just used to add permissions checks
    """

    def __init__(self, publisher: Publisher) -> None:
        super().__init__()
        self.publisher = publisher

    async def batch_publish(
        self,
        events: list[OmniEvent],
    ) -> list[PublishingResult]:
        """Check permissions before publishing"""
        event_types = {event.event_type for event in events}
        await raise_if_cant_publish(event_types)

        return await self.publisher.batch_publish(events)

    async def publish(self, event: OmniEvent) -> PublishingResult:
        """Check permissions before publishing"""
        await raise_if_cant_publish({event.event_type})

        return await self.publisher.publish(event)
