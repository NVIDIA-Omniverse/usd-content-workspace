# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Handler for durable event streaming."""

import logging
from typing import AsyncGenerator

from fastapi import Request
from notification_common.observability import tracing as common_tracing
from sse_starlette import sse

from app.lib.rabbitmq import DurableQueueConsumer
from app.lib.observability.sse_metrics import sse_metrics
from app.rest.models.connection_metadata import ConnectionMetadata
from app.lib.observability import tracing
from app.rest.handlers.event_stream_common import event_stream_generator

logger = logging.getLogger(__name__)


async def durable_events_stream(
    request: Request,
    queue_id: str,
) -> sse.EventSourceResponse:
    """
    Handle streaming events from a durable queue.

    Uses server-side delayed acks: messages are acked to RabbitMQ
    automatically after a short delay, giving the TCP stack time
    to deliver them to the client. Client-side acks are only
    available via gRPC (ConsumeDurableEventsWithAcks).

    This validates the queue exists before starting the stream,
    so authentication and queue-not-found errors are returned
    as HTTP errors rather than SSE error events.
    """
    from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase

    async with RabbitMQBase.connector.get_channel(durable_pool=True) as channel:
        await DurableQueueConsumer.get_queue(channel, queue_id)

    common_tracing.add_current_span_attributes(
        queue_id=queue_id,
    )

    connection_metadata = ConnectionMetadata(
        endpoint=request.url.path,
    )

    sse_metrics.start_connection(connection_metadata)

    generator = DurableQueueConsumer.consume(queue_id)

    return sse.EventSourceResponse(
        _durable_event_generator(
            request,
            generator,
            queue_id,
            connection_metadata,
        ),
        ping=10,
    )


@tracing.trace_sse_event_stream(
    stream_name="sse_durable_event_stream",
    attributes={"streaming_type": "server_sent_events_durable"},
)
async def _durable_event_generator(
    request: Request,
    event_source,
    queue_id: str,
    connection_metadata: ConnectionMetadata,
) -> AsyncGenerator[dict, None]:
    async for event in event_stream_generator(
        request=request,
        event_source=event_source,
        queue_id=queue_id,
        connection_metadata=connection_metadata,
        stream_type="SSE durable",
    ):
        yield event
