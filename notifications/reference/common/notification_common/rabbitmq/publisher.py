# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import aio_pika
from pydantic import BaseModel, Field
from pydantic_settings import SettingsConfigDict

from notification_common.errors.errors import RabbitmqShutdownError
from notification_common.observability import tracing
from notification_common.rabbitmq import routing
from notification_common.rabbitmq.omni_event import OmniEvent
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

logger = logging.getLogger(__name__)


class PublishingResult(BaseModel):
    event: OmniEvent
    success: bool
    reason: str | None = None

    model_config = SettingsConfigDict(arbitrary_types_allowed=True)


class SuccessfulPublishingResult(PublishingResult):
    success: bool = Field(default=True)


class FailedPublishingResult(PublishingResult):
    success: bool = Field(default=False)


class Publisher(RabbitMQConnector):
    """
    Class for rabbitmq connection and channel pools.

    This module creates an instance, and it is that instance that should be
    imported in other modules.
    """

    async def publish(self, event: OmniEvent) -> PublishingResult:
        """
        Publish events to RabbitMQ using the "events" exchange that
        is declared when the channel is created.

        Raises RabbitmqShutdownError if the service is shutting down.

        :param event: OmniEvent to publish.
        """
        try:
            if self.closing:
                raise RabbitmqShutdownError()

            async with self.get_channel() as channel:
                if self.closing:
                    raise RabbitmqShutdownError()

                exchange = await self.get_exchange(channel)

                with tracing.trace_operation("publish_event"):
                    # try to publish the event
                    await exchange.publish(
                        message=self._message(event),
                        routing_key=routing.routing_key(
                            event.event_type, event.resource_id
                        ),
                    )
                    tracing.add_current_span_event("event_published", event=str(event))

            if logger.isEnabledFor(logging.DEBUG):
                logger.debug(f"Published event: {event}")

            return SuccessfulPublishingResult(event=event)

        except Exception as e:
            error_message = f"Failed to publish event: {type(e).__name__} - {e}"
            logger.exception(error_message)
            return FailedPublishingResult(event=event, reason=error_message)

    async def batch_publish(
        self,
        events: list[OmniEvent],
    ) -> list[PublishingResult]:
        """
        Publishes a list of events to RabbitMQ sequentially using a single channel.

        :param events: List of OmniEvents to publish
        :return: A list of PublishingResult indicating whether publish was successful
                 for each event
        """
        if not events:
            return []

        try:
            if self.closing:
                return [
                    FailedPublishingResult(
                        event=event, reason="Service is shutting down"
                    )
                    for event in events
                ]

            async with self.get_channel() as channel:
                if self.closing:
                    return [
                        FailedPublishingResult(
                            event=event, reason="Service is shutting down"
                        )
                        for event in events
                    ]

                exchange = await self.get_exchange(channel)
                results = []

                for event in events:
                    try:
                        with tracing.trace_operation("publish_event"):
                            await exchange.publish(
                                message=self._message(event),
                                routing_key=routing.routing_key(
                                    event.event_type, event.resource_id
                                ),
                            )
                            tracing.add_current_span_event(
                                "event_published", event=str(event)
                            )

                        if logger.isEnabledFor(logging.DEBUG):
                            logger.debug(f"Published event: {event}")

                        results.append(SuccessfulPublishingResult(event=event))

                    except Exception as e:
                        error_message = (
                            f"Failed to publish event: {type(e).__name__} - {e}"
                        )
                        logger.exception(error_message)
                        results.append(
                            FailedPublishingResult(event=event, reason=error_message)
                        )

                return results

        except Exception as e:
            # If we can't get a channel or there's a general failure,
            # mark all events as failed
            error_message = (
                f"Failed to get channel for batch publish: {type(e).__name__} - {e}"
            )
            logger.exception(error_message)
            return [
                FailedPublishingResult(event=event, reason=error_message)
                for event in events
            ]

    @staticmethod
    def _message(event: OmniEvent) -> aio_pika.Message:
        return aio_pika.Message(
            event.encoded_for_publish(),
            delivery_mode=aio_pika.DeliveryMode.PERSISTENT,
        )
