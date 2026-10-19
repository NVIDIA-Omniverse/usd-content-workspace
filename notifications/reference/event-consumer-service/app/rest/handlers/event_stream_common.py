# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Common event streaming logic shared by durable and non-durable handlers."""

import logging
from typing import AsyncGenerator, Callable, Awaitable

from fastapi import Request
from notification_common.errors.error_details import ErrorDetails

from app.lib.models.event_message import EventMessage
from app.lib.observability.consumer_metrics import consumer_metrics
from app.lib.observability.sse_metrics import sse_metrics
from app.rest.models.connection_metadata import ConnectionMetadata
from app.rest.models.sse_events import ConsumeEvent, ErrorEvent, ConnectedEvent

logger = logging.getLogger(__name__)


async def event_stream_generator(
    request: Request,
    event_source: AsyncGenerator[EventMessage, None],
    queue_id: str,
    connection_metadata: ConnectionMetadata,
    stream_type: str = "SSE",
    cleanup_fn: Callable[[], Awaitable[None]] | None = None,
) -> AsyncGenerator[dict, None]:
    """
    Generate SSE events from an event source.

    Args:
        request: The FastAPI request object
        event_source: An async generator that yields EventMessage objects
        queue_id: The ID of the queue being consumed. It is sent to the
            client as the SSE ``id:`` field of the ``connected`` event and
            is used for nothing else -- on a non-durable stream that value
            IS the reconnect token, so it must reach the client verbatim
            and must not be logged or stamped on a span here.
        connection_metadata: Metadata about the connection for metrics
        stream_type: A description of the stream type for logging
        cleanup_fn: An optional async cleanup function to call in finally block
    """
    is_error_disconnect = False

    try:
        # Send initial connection established event
        yield ConnectedEvent(id=queue_id).to_dict()

        sse_metrics.record_event_sent(connection_metadata, "connected")

        async for event_msg in event_source:
            if await request.is_disconnected():
                break

            try:
                event_data = dict(event_msg.message)

                yield ConsumeEvent(data=event_data).to_dict()

                sse_metrics.record_event_sent(connection_metadata, event_msg.event_type)
                consumer_metrics.record_event_processing_success(event_msg)

            except Exception as e:
                consumer_metrics.record_event_processing_failure(event_msg)
                # We should not get here since we control the publishing.
                # However, we do not want such an error to make it back
                # to the client.
                # Eventually, we should have an alarm mechanism for this.
                logger.error(
                    f"Error validating event data for " f"{event_msg.event_type}: {e}"
                )
                continue

    except Exception as e:
        logger.exception(
            f"Exception in {stream_type} stream for "
            f"{request.method} {request.url.path}: {e}",
        )

        yield ErrorEvent(data=ErrorDetails(e).to_model()).to_dict()
        sse_metrics.record_event_sent(connection_metadata, "error")

        is_error_disconnect = True

        # When there are errors, we want to terminate the generator
        # because we're no longer able to process messages
        return
    finally:
        if cleanup_fn is not None:
            await cleanup_fn()

        sse_metrics.end_connection(
            metadata=connection_metadata,
            is_error_disconnect=is_error_disconnect,
        )
