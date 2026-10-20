# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Handler for non-durable event streaming."""

from typing import AsyncGenerator

from fastapi import Request
from notification_common.observability import tracing as common_tracing
from sse_starlette import sse

from app.lib.models.filter_group import FilterGroup
from app.lib.rabbitmq import NonDurableQueueConsumer
from app.lib.observability.sse_metrics import sse_metrics
from app.rest.models.connection_metadata import ConnectionMetadata
from app.lib.observability import tracing
from app.rest.handlers.event_stream_common import event_stream_generator


async def non_durable_events_stream(
    request: Request,
    filter_groups: list[FilterGroup] | None = None,
    previous_filter_groups: list[FilterGroup] | None = None,
    reconnect_queue_id: str | None = None,
) -> sse.EventSourceResponse:

    consumer: NonDurableQueueConsumer = await NonDurableQueueConsumer.get_instance(
        reconnect_queue_id, filter_groups, previous_filter_groups
    )

    # `consumer.queue_id` IS this stream's reconnect token -- the same value
    # the client passes back in the query string or Last-Event-ID header --
    # so the span carries the redacted reference instead.
    common_tracing.add_current_span_attributes(
        queue_ref=consumer.queue_ref,
        is_reconnect=reconnect_queue_id is not None,
    )

    connection_metadata = ConnectionMetadata(
        endpoint=request.url.path,
    )

    sse_metrics.start_connection(connection_metadata)

    return sse.EventSourceResponse(
        _event_generator(request, consumer, connection_metadata), ping=10
    )


@tracing.trace_sse_event_stream(
    stream_name="sse_event_stream",
    attributes={"streaming_type": "server_sent_events"},
)
async def _event_generator(
    request: Request,
    consumer: NonDurableQueueConsumer,
    connection_metadata: ConnectionMetadata,
) -> AsyncGenerator[dict, None]:
    async for event in event_stream_generator(
        request=request,
        event_source=consumer.consume(),
        # The raw id, deliberately. This one is not an observability value:
        # it becomes the SSE `id:` field of the stream's `connected` event,
        # which is how the client is HANDED its reconnect token and what a
        # browser sends back as `Last-Event-ID`. Redacting it would break
        # reconnection. Nothing downstream logs it.
        queue_id=consumer.queue_id,
        connection_metadata=connection_metadata,
        stream_type="SSE",
        cleanup_fn=consumer.cleanup_channel,
    ):
        yield event
