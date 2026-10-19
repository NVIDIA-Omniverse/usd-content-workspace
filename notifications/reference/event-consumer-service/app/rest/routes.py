# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""FastAPI routes for the EventConsumer service."""

import logging
from typing import Optional, List

from fastapi import APIRouter, Request, Query, Depends, Header, Response
from fastapi.security import HTTPBearer
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector
from notification_common.rest import NotificationRestServer
from notification_common.rest.models.health_response import HealthResponse
from sse_starlette import sse

from app.config import settings
from app.lib.auth import authenticate
from app.lib.models.filter_group import FilterGroup
from app.lib.rabbitmq import (
    DurableQueueConsumer,
    rabbitmq_connector,
    capacity_guard,
)
from app.lib.rabbitmq.rabbitmq_base import (
    QUEUE_ID_PATTERN,
    RECONNECT_TOKEN_MAX_LENGTH,
    RECONNECT_TOKEN_PATTERN,
)
from app.rest.handlers import non_durable_events_stream, durable_events_stream
from app.rest.models.channel_pool_metrics import ChannelPoolMetricsResponse
from app.rest.models.durable_queue_models import (
    CreateDurableQueueRequest,
    CreateDurableQueueResponse,
    DeleteDurableQueueResponse,
    UpdateDurableQueueRequest,
    UpdateDurableQueueResponse,
)
from app.rest.models.sse_events import ConsumeEvent

# Security dependency for protected endpoints
http_bearer = HTTPBearer(auto_error=False)

logger = logging.getLogger(__name__)

router = APIRouter()

# Get standard error responses from base class
STANDARD_ERROR_RESPONSES = NotificationRestServer.get_standard_error_responses()


def _check_non_durable_capacity():
    """FastAPI dependency that rejects requests at non-durable capacity."""
    # REST/SSE streams are unconditionally re-routable (SDD SR-3.7): every
    # request is routed afresh, whether it arrives through the proxy or the
    # readiness-honoring in-cluster Service, and an SSE reconnect re-routes by
    # construction. Deliberately do NOT consult the X-Via-Proxy stamp the gRPC
    # path uses -- the chart stamps only the ConsumeNonDurableEvents route, so
    # testing for it here would misclassify every REST stream as pinned.
    capacity_guard.raise_if_at_capacity(re_routable=True)


def _check_durable_capacity():
    """FastAPI dependency that rejects requests at durable capacity."""
    capacity_guard.raise_if_at_capacity(durable_pool=True)


def _validation_error_response(description: str) -> dict:
    """
    Build a 422 response definition that names the field that failed.

    It replaces the generic "Validation Error" response FastAPI would
    generate, while carrying the same HTTPValidationError body that generic
    response would have carried, so the spec still tells a generated client
    what the 422 payload looks like.

    :param description: The field-naming summary shown in the spec.

    :return: A responses-dict fragment ready to splat into a route.
    """
    return {
        422: {
            "description": description,
            "content": {
                "application/json": {
                    "schema": {"$ref": "#/components/schemas/HTTPValidationError"}
                }
            },
        }
    }


# Shared response definition for a queue_id that fails QUEUE_ID_PATTERN, used
# by every endpoint that accepts one.
INVALID_QUEUE_ID_RESPONSE = _validation_error_response(
    "Invalid queue_id (failed pattern validation)"
)

# Shared response definition for a reconnect_token that fails
# RECONNECT_TOKEN_PATTERN, used by the non-durable streaming endpoint. The
# token becomes the broker's queue name, so the pattern is what keeps that
# name legal -- both its length and its alphabet.
INVALID_RECONNECT_TOKEN_RESPONSE = _validation_error_response(
    "Invalid reconnect_token (failed pattern validation)"
)

# Shared response definition for channel pool exhaustion
CHANNEL_POOL_EXHAUSTED_RESPONSE = {
    503: {
        "description": (
            "Service Unavailable - The server's channel pool is "
            "exhausted. This is a temporary condition. Clients "
            "should retry the request with exponential backoff "
            "until the error no longer occurs."
        )
    }
}

# Additional note for streaming endpoints about authorization errors
STREAMING_AUTH_NOTE = (
    "\n\n**Note on Authorization During Streaming:**\n"
    "Authorization errors (401 or 403) can occur at any time during "
    "the streaming session, not just at the initial connection. For example, "
    "if an authentication token expires while the stream is active, the server "
    "will terminate the connection with an authorization error. Clients must "
    "handle this by reconnecting to the same queue with refreshed credentials. "
    "For non-durable queues, use the reconnect_token or Last-Event-ID to "
    "reconnect to the same queue. For durable queues, simply reconnect using "
    "the same queue_id."
)


async def _parse_new_filter_groups(
    filter_groups: Optional[str] = Query(
        default=None,
        max_length=65536,
        description="""URL-encoded JSON string containing an array of \
FilterGroup objects.
Each FilterGroup defines filters to apply to events based on the \
event type and optionally the resource id of the messages.
If omitted, all events will be received.

If using a reconnect_token to reconnect, then this field should be omitted unless
trying to change the filters from previous values.

**NOTE**: If you are using this request to change which
      filters are being used, you must also include the previous filter groups in
      the "previous_filter_groups" field.

**Example 1: Basic filter with a single resource filter**
```json
[
  {
    "event_type": "my.event.type",
    "filters": [
      {
        "filter_type": "starts_with_greedy",
        "resource_id": "folder/"
      }
    ]
  }
]
```

**Example 2: Filter with multiple resource filters (OR logic)**
```json
[
  {
    "event_type": "my.event.type",
    "filters": [
      {
        "filter_type": "starts_with_lazy",
        "resource_id": "folder1/"
      },
      {
        "filter_type": "eq",
        "resource_id": "specific/path"
      }
    ]
  }
]
```

**Example 3: Multiple event types with different filters**
```json
[
  {
    "event_type": "storage.created",
    "filters": [
      {
        "filter_type": "starts_with_greedy",
        "resource_id": "uploads/"
      }
    ]
  },
  {
    "event_type": "storage.deleted",
    "filters": [
      {
        "filter_type": "starts_with_lazy",
        "resource_id": "temp/"
      }
    ]
  }
]
```

See the **FilterGroup**, **ResourceFilter**, and **FilterType** \
schemas in the Components section below for the complete structure \
and field descriptions.""",
    ),
) -> Optional[List[FilterGroup]]:
    return await _parse_filter_groups(filter_groups)


async def _parse_prev_filter_groups(
    previous_filter_groups: Optional[str] = Query(
        default=None,
        max_length=65536,
        description="""Optional. Similar to filter_groups, \
a URL-encoded JSON string containing \
an array of FilterGroup objects.

If this request is being used to update which filters are being used,
in addition to specifying the new filter groups with the
"filter_groups" field, this field must be included and should contain
the previous filter groups.

See the **FilterGroup**, **ResourceFilter**, and **FilterType** \
schemas in the Components section below for the complete structure \
and field descriptions.""",
    ),
) -> Optional[List[FilterGroup]]:
    return await _parse_filter_groups(previous_filter_groups)


async def _parse_filter_groups(filter_groups) -> Optional[List[FilterGroup]]:
    """
    Parse the filter_groups query parameter into FilterGroup objects.

    :param filter_groups: JSON string representation of filter groups
    :return: FilterGroup: A list of FilterGroup objects or None if
                          no filter_groups provided
    """
    if not filter_groups:
        return None

    return FilterGroup.from_json_string(filter_groups)


@router.get(
    "/events/stream",
    description=(
        "Starts a streaming session to receive events using a "
        "non-durable queue. This means that only events that occur "
        "while connected will be consumed." + STREAMING_AUTH_NOTE
    ),
    response_model=None,
    response_class=sse.EventSourceResponse,
    dependencies=[
        Depends(http_bearer),
        Depends(_check_non_durable_capacity),
        Depends(authenticate),
    ],
    responses={
        200: {
            "description": "Server-Sent Events stream",
            "content": {
                "text/event-stream": {"schema": ConsumeEvent.simplified_schema()}
            },
        },
        400: {"description": "Invalid filter_groups parameter"},
        **INVALID_RECONNECT_TOKEN_RESPONSE,
        **STANDARD_ERROR_RESPONSES,
        **CHANNEL_POOL_EXHAUSTED_RESPONSE,
    },
)
async def stream_events(
    request: Request,
    filter_groups: Optional[List[FilterGroup]] = Depends(_parse_new_filter_groups),
    previous_filter_groups: Optional[List[FilterGroup]] = Depends(
        _parse_prev_filter_groups
    ),
    reconnect_token: str | None = Query(
        None,
        max_length=RECONNECT_TOKEN_MAX_LENGTH,
        pattern=RECONNECT_TOKEN_PATTERN,
        deprecated=True,
        description="Token for reconnecting to the same queue after disconnect. "
        "This is sent by the client when it is doing a manual reconnection, "
        "e.g. after a 401 error.\n\n"
        "**Deprecated: send the token in the `Last-Event-ID` request header "
        "instead.** This parameter still works and will keep working. The "
        "header carries the same token and supports the same operations, "
        "including a filter update.\n\n"
        "Also can be used to consume from the same queue in parallel by multiple "
        "simultaneous clients\n\n"
        "Pass back a token the server issued, verbatim. A token that is not 1 to "
        f"{RECONNECT_TOKEN_MAX_LENGTH} printable ASCII characters is rejected "
        "with a 422; the tokens this service issues are uuid4s, well inside "
        "that. A token of legal shape that names no queue is a 404, not a "
        "422.\n\n"
        "NOTE: the queues have a TTL that is configurable and has a 20 second "
        "default.",
    ),
    # Annotated as a plain `str` (not `str | None`) on purpose: the api-governance
    # OWASP ruleset (owasp:api1:2023-no-numeric-ids) matches every parameter
    # whose name ends in "-id" and requires its schema to carry an explicit
    # non-integer `type`; a nullable annotation renders as `anyOf` with no
    # top-level `type` and fails that check. The None default keeps it optional.
    last_event_id: str = Header(
        None,
        # The header carries the same value as the reconnect_token query
        # parameter, so it takes the same constraint -- otherwise it would be
        # an unvalidated way in to the same broker queue name.
        max_length=RECONNECT_TOKEN_MAX_LENGTH,
        pattern=RECONNECT_TOKEN_PATTERN,
        description="Reconnect token for resuming the same non-durable queue "
        "after a disconnect. Browsers send it automatically from the SSE event "
        "id during automatic reconnection, in which case the connection "
        "resumes with the filters it already had.\n\n"
        "To update the filters at the same time, send both `filter_groups` "
        "and `previous_filter_groups` alongside this header. Sending "
        "`previous_filter_groups` is what makes a request a filter update; "
        "without it, any `filter_groups` on the request are ignored and the "
        "stream resumes as-is.",
        # Published in the schema on purpose: the deprecation notice on the
        # query parameter points clients here, so the replacement carrier and
        # its constraints must be discoverable from the API document.
    ),
) -> sse.EventSourceResponse:
    """
    Stream events in real-time using Server-Sent Events (SSE).

    Starts a streaming session to receive events using a non-durable queue.
    This means that only events that occur while connected will be consumed.

    :param request: The incoming request
    :param filter_groups: Optional list of FilterGroup objects for event filtering.
    :param previous_filter_groups: Optional list of the prev FilterGroup objects
    before updating the event filtering.
    :param reconnect_token: Reconnect token from query parameter.
    :param last_event_id: Reconnect token from Last-Event-ID header.

    :return: EventSourceResponse: An SSE response streaming events
    """
    if last_event_id and not previous_filter_groups:
        # A browser's automatic SSE reconnect replays the original request
        # URL. For a stream opened as a fresh connect that URL carries the
        # filter_groups the stream was opened with and no
        # previous_filter_groups -- that is a plain reconnect, and passing
        # the filters on would fail as an incomplete filter update.
        # `previous_filter_groups` is what makes a request an update, so a
        # client that sends one has asked for an update deliberately and
        # gets one, whichever carrier brought the token. That is also the
        # one exception to the replay described above: an EventSource
        # opened on a filter-update URL replays `previous_filter_groups`
        # too, so its automatic reconnects re-run that update rather than
        # resuming as-is. Re-running is idempotent -- the same before/after
        # pair yields the same bindings to add and remove, and the queue is
        # already in that state.
        filter_groups = None

    return await non_durable_events_stream(
        request,
        filter_groups,
        previous_filter_groups,
        last_event_id or reconnect_token,
    )


@router.get(
    "/events/stream-durable",
    description=(
        "Starts a streaming session to receive events using a durable queue. "
        "This means any events that occurred since the queue was created "
        "with a call to the CreateDurableQueue endpoint will be received, "
        "even if the events occur while the client is disconnected."
        "\n\n**Message Acknowledgment:**\n"
        "Messages delivered over this SSE stream are acknowledged to the "
        "broker automatically by the server after a short delay. Clients "
        "do not need to (and cannot) acknowledge individual messages.\n\n"
        "There is no negative acknowledgment (nack) here either, so a "
        "message this stream delivers cannot be returned to the queue for "
        "redelivery.\n\n"
        "If your application requires explicit client-side acknowledgment "
        "of each message (e.g. to guarantee at-least-once processing), or "
        "needs to negatively acknowledge a message it failed to process so "
        "the broker redelivers it, use the gRPC "
        "`ConsumeDurableEventsWithAcks` RPC instead. Its request carries "
        "both `ack_delivery_tags` and `nack_delivery_tags`. That RPC uses "
        "a bidirectional stream, which keeps the acknowledgment on the same "
        "connection and works correctly across multi-pod deployments - a "
        "delivery tag is scoped to the broker channel that issued it, so a "
        "separate REST call could not settle it reliably."
        "\n\n**Queue Lifetime:**\n"
        "A durable queue that has gone without an attached consumer for the "
        "broker's configured idle window (24 hours by default; "
        "operator-tunable) is deleted by the broker along with any messages "
        "it still holds, and this endpoint then returns 404. Only an "
        "attached consumer keeps a queue alive - publishing to it and "
        "re-declaring it do not. A client recovers by calling the "
        "CreateDurableQueue endpoint again with the SAME queue_id and the "
        "SAME filter groups (the bindings are rebuilt from the filters), so "
        "it must persist both." + STREAMING_AUTH_NOTE
    ),
    response_model=None,
    response_class=sse.EventSourceResponse,
    dependencies=[
        Depends(http_bearer),
        Depends(_check_durable_capacity),
        Depends(authenticate),
    ],
    responses={
        200: {
            "description": "Server-Sent Events stream",
            "content": {
                "text/event-stream": {"schema": ConsumeEvent.simplified_schema()}
            },
        },
        **INVALID_QUEUE_ID_RESPONSE,
        404: {
            "description": (
                "Queue not found - the queue_id is unknown, or the queue was "
                "auto-deleted by the broker after going without an attached "
                "consumer for the idle window. Recover by calling "
                "CreateDurableQueue again with the same queue_id and filter "
                "groups."
            )
        },
        **STANDARD_ERROR_RESPONSES,
        **CHANNEL_POOL_EXHAUSTED_RESPONSE,
    },
)
async def stream_durable_events(
    request: Request,
    queue_id: str = Query(
        ...,
        pattern=QUEUE_ID_PATTERN,
        max_length=200,
        description=(
            "The queue_id which was created with a call to CreateDurableQueue, "
            "and identifies the queue used for consuming durable events. "
            "It must match the shared character and length constraints (1-200 "
            "characters, each one of A-Z, a-z, 0-9, '.', '_', or '-').\n\n"
            "queue_ids share one deployment-wide namespace, so this must be the "
            "id of a queue that was created by (or for) your client; see the "
            "queue_id field of `POST /queues/durable`."
        ),
    ),
) -> sse.EventSourceResponse:
    """
    Stream events in real-time using Server-Sent Events (SSE) from a durable queue.

    Starts a streaming session to receive events using a durable queue.
    This means any events that occurred since the queue was created
    with a call to the CreateDurableQueue endpoint will be received,
    even if the events occur while the client is disconnected.

    A durable queue that has gone without an attached consumer for the
    broker's configured idle window (24 hours by default; operator-tunable)
    is deleted by the broker along with any messages it still holds, and
    this endpoint then returns 404. Only an attached consumer keeps a queue
    alive - publishing to it and re-declaring it do not. A client recovers
    by calling CreateDurableQueue again with the same queue_id and the same
    filter groups, because the bindings are rebuilt from the filters.

    :param request: The incoming request
    :param queue_id: The queue ID created from a prior CreateDurableQueue call

    :return: EventSourceResponse: An SSE response streaming events
    """
    return await durable_events_stream(request, queue_id)


@router.post(
    "/queues/durable",
    description=(
        "Creates a durable queue which will immediately begin receiving "
        "and queuing up messages for later consumption. A durable queue "
        "exists until it is explicitly deleted with a call to the "
        "DeleteDurableQueue endpoint, or until it has gone without an "
        "attached consumer for the broker's configured idle window (24 "
        "hours by default; operator-tunable), at which point the broker "
        "deletes it along with any messages it still holds. Only an "
        "attached consumer keeps a queue alive - publishing to it and "
        "re-declaring it do not. Consuming from a queue that has been "
        "reaped this way returns 404; a client recovers by calling this "
        "endpoint again with the same queue_id, so it must persist BOTH "
        "the queue_id AND the filter groups the queue was created with "
        "(the bindings are rebuilt from the filters). A config object "
        "will be returned that includes an ID that must be reused as an "
        "argument in all future ConsumeDurableEvents calls."
        "\n\n**Queue ID Uniqueness:**\n"
        "A client-supplied queue_id must be unique across the whole "
        "deployment. Queue ids share a single namespace, and this endpoint "
        "is ensure-exists: if a queue with the supplied id already exists "
        "- regardless of who created it or which filter groups are passed "
        "- the call returns created=false along with that existing queue, "
        "still bound to its existing filters. Two callers that pick the "
        "same id therefore share one queue: each receives the other's "
        "events, and either can delete it. Use a UUID, or a namespaced "
        "convention such as `<service-name>-<purpose>`; never a bare "
        "generic word like `events`."
        "\n\n**Authorization:**\n"
        "Per-queue isolation is not enforced. The three durable-queue "
        "permissions are deployment-wide rather than per-queue: a principal "
        "granted `consume-durable-queues` can consume from any durable queue "
        "whose id it knows, one granted `delete-durable-queues` can delete "
        "any of them, and `create-durable-queues` authorizes both creating a "
        "queue and updating the filters of any existing one. Operators should "
        "therefore grant these permissions deliberately - to service accounts "
        "rather than to broad user groups."
    ),
    response_model=CreateDurableQueueResponse,
    dependencies=[
        Depends(http_bearer),
        Depends(_check_durable_capacity),
        Depends(authenticate),
    ],
    responses={
        200: {
            "description": (
                "Queue created (no queue_id supplied), or an existing queue "
                "with the supplied queue_id was returned unchanged "
                "(created=false)."
            )
        },
        201: {
            "description": (
                "A new durable queue was created with the supplied queue_id "
                "(created=true)."
            )
        },
        400: {"description": "Invalid request"},
        **INVALID_QUEUE_ID_RESPONSE,
        **STANDARD_ERROR_RESPONSES,
        **CHANNEL_POOL_EXHAUSTED_RESPONSE,
    },
)
async def create_durable_queue(
    body: CreateDurableQueueRequest,
    response: Response,
) -> CreateDurableQueueResponse:
    """
    Create a durable queue for consuming events.

    Creates a durable queue which will immediately begin receiving
    and queuing up messages for later consumption. A durable queue exists
    until it is explicitly deleted, or until it has gone without an
    attached consumer for the broker's configured idle window (24 hours by
    default; operator-tunable), at which point the broker deletes it along
    with any messages it still holds. Only an attached consumer keeps a
    queue alive - publishing to it and re-declaring it do not. Consuming
    from a queue reaped this way returns 404, and the client recovers by
    calling this endpoint again with the same queue_id -- so a client must
    persist BOTH the queue_id AND the filter groups, because the bindings
    are rebuilt from the filters.

    When building a service that uses the Notification APIs,
    this will typically be called outside the code base
    of that service, and then the "queue_id" value
    retrieved will be used as configuration for that service.

    This is not required for non-durable events,
    e.g., when ConsumeNonDurableEvents is called the client only
    receives events that occur while the client is connected.

    An optional client-supplied ``queue_id`` may be provided in the request
    body to make queue creation idempotent (ensure-exists). When ``queue_id``
    is omitted the call returns ``200`` and a server-minted id. When a
    ``queue_id`` is supplied, a newly created queue returns ``201`` with
    ``created=true`` and an already-existing queue returns ``200`` with
    ``created=false``.

    A client-supplied ``queue_id`` must be unique across the whole
    deployment. Queue ids share a single namespace, so an id that is already
    in use attaches this call to the existing queue -- regardless of who
    created it or which filter groups are passed -- and that queue is
    returned with its existing filters. Two callers that pick the same id
    share one queue: each receives the other's events, and either can delete
    it. Use a UUID, or a namespaced convention such as
    ``<service-name>-<purpose>``; never a bare generic word.

    Per-queue isolation is not enforced. The three durable-queue permissions
    are deployment-wide rather than per-queue: a principal granted
    ``consume-durable-queues`` can consume from any durable queue whose id it
    knows, one granted ``delete-durable-queues`` can delete any of them, and
    ``create-durable-queues`` authorizes both creating a queue and updating
    the filters of any existing one. Operators should grant these permissions
    deliberately -- to service accounts rather than to broad user groups.

    :param body: The request containing filter groups and an optional queue_id
    :param response: The injected response used to set the status code

    :return: CreateDurableQueueResponse: The queue ID and whether it was newly
        created
    """
    queue_id, created = await DurableQueueConsumer.create_queue(
        body.filter_groups, queue_id=body.queue_id
    )
    if body.queue_id is not None and created:
        response.status_code = 201
    return CreateDurableQueueResponse(queue_id=queue_id, created=created)


@router.delete(
    "/queues/durable",
    description=(
        "Deletes a durable consumer queue. This is called if a durable consumer "
        "queue was created and used, and it is no longer required. "
        "This typically only happens if the consumer client is being taken "
        "down permanently."
    ),
    response_model=DeleteDurableQueueResponse,
    dependencies=[
        Depends(http_bearer),
        Depends(_check_durable_capacity),
        Depends(authenticate),
    ],
    responses={
        200: {"description": "Queue deleted successfully"},
        400: {"description": "Invalid request"},
        404: {"description": "Queue not found"},
        **INVALID_QUEUE_ID_RESPONSE,
        **STANDARD_ERROR_RESPONSES,
        **CHANNEL_POOL_EXHAUSTED_RESPONSE,
    },
)
async def delete_durable_queue(
    queue_id: str = Query(
        ...,
        pattern=QUEUE_ID_PATTERN,
        max_length=200,
        description=(
            "The queue_id identifies the durable queue to delete. "
            "It was created with a call to the CreateDurableQueue endpoint. "
            "It must match the shared character and length constraints (1-200 "
            "characters, each one of A-Z, a-z, 0-9, '.', '_', or '-').\n\n"
            "queue_ids share one deployment-wide namespace, so this must be the "
            "id of a queue that was created by (or for) your client; see the "
            "queue_id field of `POST /queues/durable`."
        ),
    ),
) -> DeleteDurableQueueResponse:
    """
    Delete a durable queue.

    Deletes a durable consumer queue. This is called if a durable consumer
    queue was created and used, and it is no longer required.
    This typically only happens if the consumer client is being taken
    down permanently.

    :param queue_id: The queue ID to delete

    :return: DeleteDurableQueueResponse: Success response
    """
    await DurableQueueConsumer.delete_queue(queue_id)
    return DeleteDurableQueueResponse()


@router.patch(
    "/queues/durable",
    description=(
        "Updates the filtering on an existing durable queue that was "
        "created using CreateDurableQueue. The filters will be updated "
        "without losing messages. This may be preferable to deleting "
        "and re-creating the durable queue, which may result in lost messages."
    ),
    response_model=UpdateDurableQueueResponse,
    dependencies=[
        Depends(http_bearer),
        Depends(_check_durable_capacity),
        Depends(authenticate),
    ],
    responses={
        200: {"description": "Queue updated successfully"},
        400: {"description": "Invalid request"},
        404: {"description": "Queue not found"},
        **INVALID_QUEUE_ID_RESPONSE,
        **STANDARD_ERROR_RESPONSES,
        **CHANNEL_POOL_EXHAUSTED_RESPONSE,
    },
)
async def update_durable_queue(
    body: UpdateDurableQueueRequest,
) -> UpdateDurableQueueResponse:
    """
    Update filters on a durable queue.

    Updates the filtering on an existing durable queue
    created using CreateDurableQueue. The filters will be updated
    without losing messages. This may be preferable to deleting
    and re-creating the durable queue, which may result in lost messages.

    :param body: The request containing queue ID and filter updates

    :return: UpdateDurableQueueResponse: Success response
    """
    await DurableQueueConsumer.update_queue(
        body.queue_id, body.current_filter_groups, body.new_filter_groups
    )
    return UpdateDurableQueueResponse()


@router.get(
    "/health",
    description="Health check endpoint for the API",
    response_model=HealthResponse,
)
async def health_check() -> HealthResponse:
    """Health check endpoint."""
    return HealthResponse()


@router.get(
    "/metrics/channel-pool",
    description=(
        "Returns the current channel pool utilization metrics. "
        "This endpoint is used by the autoscaler to make scaling decisions "
        "based on the channel pool capacity."
    ),
    response_model=ChannelPoolMetricsResponse,
)
async def channel_pool_metrics() -> ChannelPoolMetricsResponse:
    """
    Get non-durable channel pool utilization metrics.

    Reports the autoscaler's capacity signal in STREAM (client) slots, not
    channels: utilization percentage, streams in use, and effectively usable
    stream capacity. Counting streams keeps the signal truthful at any packing
    density -- once a channel carries more than one client, channel counts
    understate the real load.

    The signal keys on the non-durable pool alone. The durable pool has its
    own capacity budget, so durable checkouts must not move the non-durable
    scaling signal. The stream capacity reported here is the pool's
    max_pool_size * channel_client_limit, which is the SOFT threshold: the
    scaling and steering signal, which refuses nothing on its own. The pod's
    actual admission ceiling is the memory-safe stream cap, and that cap is
    deliberately not reported here -- adding pods relieves stream pressure,
    which is why streams are the autoscaler's input, while broker memory is
    fleet-wide and no number of extra pods relieves it.
    """
    by_pool = rabbitmq_connector.pool_stats_by_pool()

    # Unlike the capacity guard and the readiness servicer, this surface has
    # no fail-closed branch to fall into: a 500 here would drop the pod out
    # of the autoscaler's view entirely, so every read below is defensive and
    # degrades to zeros rather than raising. ChannelPoolMetricsResponse imposes
    # no magnitude ceiling on either count, so no pool size or memory cap an
    # operator can configure turns this surface non-200. The bounds that do
    # survive on the model -- ge=0 on both counts, plus le=100 on
    # utilization_percent -- are unreachable only because the clamps below
    # (max(0, ...) on each count and min(100, max(0, ...)) on the percentage)
    # guarantee it; dropping a clamp would put a ValidationError back in this
    # handler's path. In particular, the in-use count exceeding the reported
    # capacity is normal two-tier operation while the pod serves in the
    # overflow band, and is reported as-is.
    stats = by_pool.get(RabbitMQConnector.NON_DURABLE_POOL_NAME) or {}

    # Clamp to non-negative values defensively. In rare race conditions,
    # the pool can report negative client counts (e.g., double-return of a channel).
    streams_in_use = max(0, stats.get("total_client_count", 0))
    stream_capacity = max(1, stats.get("max_client_capacity", 1))

    # Subtract the readiness reserve so the autoscaler sees only effectively
    # usable capacity. The reserve is a count of non-durable stream slots,
    # subtracted from that pool's stream capacity. Subtracting it keeps the
    # autoscaler's view conservative, so the autoscaler reacts before the soft
    # threshold is reached. Slots within the reserve do still serve clients:
    # going unready removes the pod from the readiness-gated main Service
    # only, while the -envoy Service (publishNotReadyAddresses: true) keeps it
    # in Envoy's EDS cluster, so the pod keeps admitting -- pinned streams up
    # to the memory-safe stream cap, and re-routable ones up to the shed
    # threshold.
    reserve = settings.settings.readiness_min_available_channels
    effective_stream_capacity = max(0, stream_capacity - reserve)
    utilization_divisor = max(1, effective_stream_capacity)

    utilization_percent = min(
        100,
        max(0, (streams_in_use * 100) // utilization_divisor),
    )

    return ChannelPoolMetricsResponse(
        utilization_percent=utilization_percent,
        channels_in_use=streams_in_use,
        channels_total=effective_stream_capacity,
    )
