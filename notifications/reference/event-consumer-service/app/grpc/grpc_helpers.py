# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import grpc
from google.protobuf.internal.containers import RepeatedCompositeFieldContainer

from app.lib.models.filter_group import FilterGroup
from app.lib.models.filter_type import FilterType
from app.lib.models.resource_filter import ResourceFilter
from app.lib.queue_names import redact_reconnect_token
from app.lib.rabbitmq.rabbitmq_base import validate_queue_id
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ConsumeDurableEventsRequest,
    ConsumeDurableEventsWithAcksRequest,
    ConsumeNonDurableEventsRequest,
    Event,
)
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    FilterGroup as ProtoFilterGroup,
)

logger = logging.getLogger(__name__)


async def validate_consume_durable_request(
    request: ConsumeDurableEventsRequest | ConsumeDurableEventsWithAcksRequest,
    context: grpc.aio.ServicerContext,
):
    """
    Reject a consume request whose queue_id is missing or malformed.

    The missing case keeps its own abort so the caller still gets the
    field-name message it has always got. Everything else is deferred to the
    shared ``validate_queue_id``, which is the same check the create surface
    applies and which fails the same way: ``UnprocessableError``, rendered as
    INVALID_ARGUMENT by the exception interceptor. Both outcomes are the same
    status code, so a client sees one error class for a bad id whichever
    surface it used.
    """
    if not request.queue_id:
        await context.abort(
            grpc.StatusCode.INVALID_ARGUMENT,
            "Queue ID is missing from consume durable request",
        )
        # `abort` raises in a real servicer context, so this return only
        # matters under a test double that does not -- there it keeps the
        # missing case reporting the missing-field message rather than
        # falling through to the pattern check below.
        return

    validate_queue_id(request.queue_id)


def describe_non_durable_request(request: ConsumeNonDurableEventsRequest) -> str:
    """
    Summarise a non-durable consume request for a log line.

    The request cannot be logged as itself. Its ``reconnect_token`` field is
    the queue's identifier -- not a credential, but an authorized principal
    that obtains it could attach to the same stream and take a share of its
    events, so it is kept out of telemetry as defense in depth -- and
    protobuf's text format, which is what an interpolated
    message renders as, prints every populated field, including that one. So
    the log line is built from derived facts instead: whether this is a
    reconnect, which reference it is a reconnect to, and how many filter
    groups came with it. That is what the operator questions these lines get
    read for actually need, and none of it names a queue a reader could
    attach to.

    The reference is present only on a reconnect, because only then is
    there a queue to refer to; a fresh connect has not been assigned one
    yet, and the stream's first response is where its reference is
    reported.

    :param request: The request to summarise.

    :return: A single-line, space-separated summary safe to log.
    """
    is_reconnect = bool(request.reconnect_token)

    fields = [f"is_reconnect={is_reconnect}"]
    if is_reconnect:
        fields.append(f"queue_ref={redact_reconnect_token(request.reconnect_token)}")
    fields.append(f"filter_groups={len(request.filter_groups)}")
    fields.append(f"previous_filter_groups={len(request.previous_filter_groups)}")

    return " ".join(fields)


def convert_to_models(
    proto_filter_groups: RepeatedCompositeFieldContainer[ProtoFilterGroup],
) -> list[FilterGroup]:
    proto_filter_groups = list(proto_filter_groups)

    return [
        FilterGroup(
            event_type=fg.event_type,
            filters=tuple(
                [
                    ResourceFilter(
                        filter_type=FilterType(f.filter_type),
                        resource_id=f.resource_id,
                    )
                    for f in fg.filters
                ]
            ),
        )
        for fg in list(proto_filter_groups)
    ]


def convert_to_event_proto(event_dict: dict) -> Event:
    """
    Convert dictionary to an Event proto message.
    """
    event = Event()
    event.event_type = event_dict["event_type"]
    event.principal_identity = event_dict.get("principal_identity", "")

    if event_dict.get("occurred_at"):
        event.occurred_at.FromJsonString(event_dict["occurred_at"])

    if event_dict.get("published_at"):
        event.published_at.FromJsonString(event_dict["published_at"])

    if event_dict.get("message"):
        for key, value in event_dict["message"].items():
            event.message[key] = value

    return event
