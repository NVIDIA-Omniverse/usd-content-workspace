# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging
from contextlib import aclosing, asynccontextmanager
from typing import AsyncIterator, AsyncGenerator, List

import grpc
from google.protobuf.timestamp_pb2 import Timestamp

from notification_common.observability import tracing as common_tracing

from app.config import settings
from app.grpc.grpc_helpers import (
    convert_to_event_proto,
    describe_non_durable_request,
    validate_consume_durable_request,
    convert_to_models,
)
from app.lib.auth import authenticate
from app.lib.models.event_message import EventMessage
from app.lib.models.filter_group import FilterGroup
from app.lib.permissions import permissions
from app.lib.rabbitmq import (
    NonDurableQueueConsumer,
    DurableQueueConsumer,
    capacity_guard,
)
from app.lib.rabbitmq.ack_tracker import DEAD_CHANNEL_EXC, AckTracker
from app.lib.rabbitmq.rabbitmq_base import (
    ConsumerCancelledError,
    validate_queue_id,
    validate_reconnect_token,
)
from app.lib.rabbitmq.dead_letter import DecodedDeadLetter
from app.lib.rabbitmq.dead_letter_consumer import (
    ActionStatus,
    DeadLetterConsumer,
    ReplayStatus,
)
from app.lib.observability.consumer_metrics import consumer_metrics
from app.lib.observability import tracing
from app.lib.streams import merge
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ConsumeDurableEventsRequest,
    ConsumeDurableEventsResponse,
    ConsumeDurableEventsWithAcksRequest,
    ConsumeDurableEventsWithAcksResponse,
    ConsumeNonDurableEventsRequest,
    ConsumeNonDurableEventsResponse,
    CreateDurableQueueRequest,
    CreateDurableQueueResponse,
    DeleteDurableQueueRequest,
    DeleteDurableQueueResponse,
    FilterUpdateAck,
    ManageDeadLetteredEventsRequest,
    ManageDeadLetteredEventsResponse,
    UpdateDurableQueueRequest,
    UpdateDurableQueueResponse,
)
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2_grpc import (
    EventConsumerServiceServicer,
)

logger = logging.getLogger(__name__)


class EventConsumerServicer(EventConsumerServiceServicer):

    @staticmethod
    @asynccontextmanager
    async def _dead_channel_mapped(queue_name: str):
        """
        Classify a dead channel raised out of a broker-op-and-events merge.

        All three bidi streams here have the same shape: an
        event-producing generator merged with a loop that issues
        client-driven operations against the broker. That operation --
        settling a delivery tag on the two ack surfaces, rebinding the
        queue on the non-durable filter-update surface -- fails inside the
        merge when the broker has killed the consumer's channel, and it
        fails ahead of the event side, which only notices the loss on its
        next idle liveness poll or its next own broker operation. So the
        merged side wins the race. This wrapper turns that raw
        dead-channel error into ``ConsumerCancelledError`` for the CLIENT, so
        the stream ends with "Internal Server Error: The broker cancelled
        this stream's consumer. Reconnect to resume consuming." instead of
        a bare INTERNAL "Internal Server Error" -- same prefix, but with
        the actionable tail that says what to do about it.
        ``channel_closed=True`` is a statement of fact: every mapped class
        means the channel incarnation the consumer was started on is gone.

        It is classification only. The raise never reaches the consume
        generator, so it gives the generator's cleanup nothing; where the
        cleanup has a dead-channel verdict, it comes from the failing
        broker call itself. On the two ack surfaces, a client settle that
        fails on a dead channel marks the stream's tracker (see
        ``AckTracker.ack`` / ``AckTracker.nack``) before ``merge`` tears
        the event side down. A broker operation of the consume loop's own
        that fails that way records ``consumer_lost_exc`` inside the
        generator before its cleanup is scheduled, and ends the stream
        with ``ConsumerCancelledError`` already, so it passes through here
        unmapped. A filter-update rebind that fails on the non-durable
        surface records no verdict for the cleanup.

        ONLY the classes in ``DEAD_CHANNEL_EXC`` (the dead-channel and
        closed-connection families) are mapped, so a genuine internal bug
        still surfaces as itself.

        :param queue_name: The BROKER queue name, for the error's log-only
            ``internal_message``. Pass it raw: ``ConsumerCancelledError``
            redacts it, so a durable name still correlates with the
            broker-side log lines around it by carrying the same spelling,
            while a non-durable name does not repeat the reconnect token it
            embeds.
        """
        try:
            yield
        except DEAD_CHANNEL_EXC as exc:
            raise ConsumerCancelledError(queue_name, channel_closed=True) from exc

    @tracing.trace_grpc_event_stream(
        stream_name="grpc.consume_durable_events", attributes={"stream_type": "durable"}
    )
    async def ConsumeDurableEvents(
        self,
        request: ConsumeDurableEventsRequest,
        context: grpc.aio.ServicerContext,
    ):
        # This check must stay the first statement in the method: this RPC's
        # HTTPProxy route retries resource-exhausted (SDD SR-3.7), so the
        # refusal has to leave as a trailers-only response, and nothing above
        # it may commit a response. "First statement in the method" is the
        # requirement -- the shared AuthInterceptor's token resolution runs
        # above the handler and refuses an unresolvable token UNAUTHENTICATED
        # before the guard is reached, which is intended. See
        # ConsumeNonDurableEvents for the full rationale.
        capacity_guard.raise_if_at_capacity(durable_pool=True)

        await authenticate()

        # Validation comes before the log line and the span attribute, not
        # after them. The queue id is client-controlled and unbounded until
        # it is checked, so logging it first would let a single rejected
        # request write an arbitrarily large log line -- and let a caller
        # choose the text that appears in it.
        await validate_consume_durable_request(request, context)

        logger.info("Received consume durable request for queue %s", request.queue_id)

        common_tracing.add_current_span_attributes(queue_id=request.queue_id)

        async for event_msg in DurableQueueConsumer.consume(request.queue_id):
            response = self._build_event_response(
                event_msg, ConsumeDurableEventsResponse()
            )
            yield response

    @tracing.trace_grpc_event_stream(
        stream_name="grpc.consume_durable_events_with_acks",
        attributes={"stream_type": "durable_with_acks"},
    )
    async def ConsumeDurableEventsWithAcks(
        self,
        request_iterator: AsyncIterator[ConsumeDurableEventsWithAcksRequest],
        context: grpc.aio.ServicerContext,
    ):
        # This check must stay the first statement in the method: this RPC's
        # HTTPProxy route retries resource-exhausted (SDD SR-3.7), so the
        # refusal has to leave as a trailers-only response, and nothing above
        # it may commit a response or read the client's request stream.
        # "First statement in the method" is the requirement -- the shared
        # AuthInterceptor's token resolution runs above the handler and
        # refuses an unresolvable token UNAUTHENTICATED before the guard is
        # reached, which is intended. See ConsumeNonDurableEvents for the full
        # rationale.
        capacity_guard.raise_if_at_capacity(durable_pool=True)

        await authenticate()

        try:
            first_request = await anext(request_iterator)
        except StopAsyncIteration:
            return

        # Validated before it is logged or stamped on the span, for the
        # reason given in ConsumeDurableEvents.
        await validate_consume_durable_request(first_request, context)

        logger.info(
            "Received consume durable with acks request for queue %s",
            first_request.queue_id,
        )

        common_tracing.add_current_span_attributes(
            queue_id=first_request.queue_id,
        )

        generator, ack_tracker = await DurableQueueConsumer.consume_with_acks(
            first_request.queue_id
        )

        try:
            # The broker name is resolved here rather than inside the
            # mapping below: it is the same cheap derivation from the queue
            # id the consume above just made, and keeping the await out of
            # the exception path means a failure to resolve it can never
            # displace the error being classified. It sits inside the try
            # so that failure still closes the generator.
            queue_name = await DurableQueueConsumer._queue_name(first_request.queue_id)

            # Both settle verbs (ack and nack) are covered symmetrically
            # because the wrapper sits above the whole merge.
            async with self._dead_channel_mapped(queue_name):
                async with aclosing(
                    merge(
                        self._stream_events_from_durable_queue_with_acks(generator),
                        self._listen_for_durable_acks(request_iterator, ack_tracker),
                    )
                ) as streamer:
                    async for stream_response in streamer:
                        if isinstance(
                            stream_response, ConsumeDurableEventsWithAcksResponse
                        ):
                            yield stream_response
        finally:
            await generator.aclose()

    @tracing.trace_grpc_event_stream(
        stream_name="grpc.consume_non_durable_events",
        attributes={"stream_type": "non_durable"},
    )
    async def ConsumeNonDurableEvents(
        self,
        request_iterator: AsyncIterator[ConsumeNonDurableEventsRequest],
        context: grpc.aio.ServicerContext,
    ):
        # This check must stay the first statement in the method, ahead of
        # initial metadata and the first yield. A shed re-routable stream is
        # re-routed by the proxy's retry policy (retryOn resource-exhausted,
        # SDD SR-3.7), and Envoy decides whether a gRPC response is retryable
        # from the response headers alone -- so the refusal has to arrive as a
        # trailers-only response. Moving the check below anything that commits
        # a response silently breaks re-routing. The wire-level frame
        # assertions in
        # tests/integration_tests/grpc/test_capacity_refusal_is_trailers_only.py
        # catch a response committed ahead of the refusal on this RPC, and the
        # streaming-refusal pins in
        # TestRefusedStreamsCommitNothingBeforeTheRefusal, in
        # tests/app/grpc/test_event_consumer_servicer.py, catch the same drift
        # on every streaming RPC, including the durable methods, where a guard
        # moved below the existing yield site is instead caught by the
        # admission-ordering pins in
        # TestAdmissionPrecedesAuthenticationOnEveryGuardedRpc in that same
        # file. Those pins catch a different drift: the guard falling below
        # authenticate() on every guarded RPC.
        #
        # What this requirement is NOT: a claim that the guard is the first
        # thing the pod does. The shared AuthInterceptor resolves the caller's
        # bearer token above this handler, so a request whose token does not
        # resolve is refused UNAUTHENTICATED before the guard is ever
        # consulted, and the proxy's retryOn resource-exhausted policy does
        # not re-route it. That is intended: such a request fails identically
        # on every pod, so retrying it would spend fleet retry budget during a
        # saturation incident to reach the same refusal, and an unauthenticated
        # caller no longer learns that the pod is saturated. The requirement
        # above is a statement about the servicer method, and it holds
        # verbatim for every caller whose token resolves. Residual cost, known
        # and accepted: under a principal_source of opaque or userInfoService
        # the interceptor's resolution is a network round trip an over-capacity
        # pod pays before refusing, repeated on each retry; under jwt it is a
        # local signature verification against a process-cached JWKS, refetched
        # only when auth_context_ttl lapses or the token's kid is unknown; under
        # the opt-in jwtUnverified it is a local payload decode.
        capacity_guard.raise_if_at_capacity(re_routable=self._is_proxied(context))

        await authenticate()

        try:
            first_request = await anext(request_iterator)
        except StopAsyncIteration:
            return

        # The token becomes the non-durable queue's broker name, so its
        # length and alphabet are constrained here, at the boundary, rather
        # than several layers down where the failure is an AMQP
        # frame-building error.
        #
        # Guarded on non-empty because proto3 has no absent string: a first
        # request that carries no token at all -- the ordinary fresh-stream
        # case, which the `is_reconnect` span attribute below reads off the
        # same emptiness -- arrives here as "", which is not a token to
        # validate against a pattern that requires one. The REST
        # halves do not need the guard: an omitted query parameter or header
        # is None there, and an explicitly empty one is a supplied value that
        # names no queue.
        if first_request.reconnect_token:
            validate_reconnect_token(first_request.reconnect_token)

        consumer = await self._get_non_durable_consumer(first_request)

        try:
            # `consumer.queue_id` IS this stream's reconnect token, so the
            # span carries the redacted reference instead. It is stable, so
            # the stream is still followable across pods and across the
            # log lines that name the same reference.
            common_tracing.add_current_span_attributes(
                queue_ref=consumer.queue_ref,
                is_reconnect=first_request.reconnect_token != "",
            )

            # Send an initial response with the reconnect_token.
            #
            # The capability flag rides every response on this stream, not
            # just this one: proto3 has no absent bool, so a flag set here
            # alone would have this server telling the client
            # `supports_filter_update_acks=false` on every event response
            # that follows. It is a statement about the server, so it is
            # true wherever it is read.
            #
            # What this connect did with the filter set the opening request
            # named is `get_instance`'s verdict, recorded on the consumer
            # rather than re-derived here from the request's shape. It is a
            # statement about the request received, not about the queue's
            # resulting state. It is three-valued, and both tests below read
            # one of the three:
            #
            # `is not None` gates the acknowledgement, because the two
            # non-None values are exactly the connects whose request named a
            # filter set -- a fresh connect, and a reconnect whose
            # connect-time update ran. That is the only way a client that
            # reconnected with a token AND an update learns the server
            # processed the update, since it is applied before this stream
            # yields anything. None is the reconnect that named no set on
            # this connect -- no filter groups and no previous filter groups
            # -- so the server was told nothing to echo and acknowledges
            # nothing. The echo below is never empty: a fresh connect must
            # name `filter_groups`, because `get_instance` rejects a
            # request carrying neither `filter_groups` nor `queue_id`, and
            # a reconnect that names one of `filter_groups` and
            # `previous_filter_groups` names both, because `get_instance`
            # rejects either half on its own.
            #
            # `is False` -- never `not ...`, which without the gate above
            # would read the token-only reconnect's None as unchanged -- sets
            # `unchanged` only for the connect whose update named two equal
            # sets as sent, so no bind or unbind call was issued.
            initial_ack = None
            if consumer.connect_time_update_changed is not None:
                initial_ack = FilterUpdateAck(
                    applied_filter_groups=first_request.filter_groups,
                    unchanged=consumer.connect_time_update_changed is False,
                )

            yield ConsumeNonDurableEventsResponse(
                reconnect_token=consumer.queue_id,
                supports_filter_update_acks=True,
                filter_update_ack=initial_ack,
            )

            # Same merge-of-broker-ops shape as the two ack streams, so it
            # gets the same classification: `update_queue` binds and
            # unbinds against the consumer's channel, so a filter update
            # racing the broker's cancellation of that channel fails here,
            # ahead of the event side's liveness poll. The filter-update
            # acknowledgement is yielded from inside this wrapper, for the
            # reason given at the yield.
            async with self._dead_channel_mapped(consumer.queue.name):
                # Combine the two async streams
                async with aclosing(
                    merge(
                        self._stream_events_from_non_durable_queue(consumer),
                        self._listen_for_filter_updates(request_iterator),
                    )
                ) as streamer:
                    async for stream_response in streamer:
                        if isinstance(stream_response, ConsumeNonDurableEventsResponse):
                            # It is from the stream consuming messages so send it out.
                            yield stream_response
                        else:
                            (
                                new_f_grps,
                                prev_f_grps,
                            ) = await self._filter_grps_from_request(stream_response)

                            # It is from the stream listening for new requests to
                            # update the filters.
                            newly_received_filters = new_f_grps

                            changed = await consumer.update_queue(
                                newly_received_filters, prev_f_grps
                            )

                            # The acknowledgement is yielded here, after the
                            # await, and never from
                            # `_listen_for_filter_updates`: `merge` forwards
                            # from each source as soon as an item is
                            # available and gives no ordering between them, so
                            # an acknowledgement yielded by the listener could
                            # reach the client before the bind and unbind
                            # calls the update required have returned. It
                            # also stays inside `_dead_channel_mapped`, so a
                            # channel the broker killed while the update was
                            # applied still ends the stream as a cancelled
                            # consumer rather than acknowledging an update
                            # that did not land. A rejected update raises out
                            # of `update_queue` above and is never
                            # acknowledged.
                            ack = FilterUpdateAck(
                                applied_filter_groups=stream_response.filter_groups,
                                unchanged=not changed,
                            )
                            yield ConsumeNonDurableEventsResponse(
                                reconnect_token=consumer.queue_id,
                                supports_filter_update_acks=True,
                                filter_update_ack=ack,
                            )

                            # Counts only: the filter groups name resources and
                            # `queue_id` is the reconnect token, so neither goes
                            # into a log line. See
                            # `describe_non_durable_request`.
                            logger.info(
                                "filter update acknowledged: queue_ref=%s "
                                "filter_groups=%d unchanged=%s",
                                consumer.queue_ref,
                                len(stream_response.filter_groups),
                                not changed,
                            )
        finally:
            await consumer.cleanup_channel()

    @tracing.trace_grpc_event_stream(
        stream_name="grpc.manage_dead_lettered_events",
        attributes={"stream_type": "dead_letter"},
    )
    async def ManageDeadLetteredEvents(
        self,
        request_iterator: AsyncIterator[ManageDeadLetteredEventsRequest],
        context: grpc.aio.ServicerContext,
    ):
        # Dead-letter browse borrows a channel from the durable pool. This
        # check must stay the first statement in the method: this RPC's
        # HTTPProxy route retries resource-exhausted (SDD SR-3.7), so the
        # refusal has to leave as a trailers-only response, and nothing above
        # it may commit a response. Reading nothing from the client before
        # refusing is also what guarantees the request bytes Envoy replays
        # were never consumed by the refusing pod, so a REPLAY or DISCARD
        # action cannot be acted on twice. "First statement in the method" is
        # the requirement -- the shared AuthInterceptor's token resolution
        # runs above the handler and refuses an unresolvable token
        # UNAUTHENTICATED before the guard is reached, which is intended. See
        # ConsumeNonDurableEvents for the full rationale.
        capacity_guard.raise_if_at_capacity(durable_pool=True)

        await authenticate()

        await permissions.raise_if_cant_inspect_and_replay_all_dlq()

        logger.info("Received manage dead-lettered events request")

        generator, ack_tracker, channel = await DeadLetterConsumer.browse()

        try:
            # Same merge-of-settles shape as the durable consume-with-acks
            # stream, so it gets the same classification: a REPLAY /
            # DISCARD / LEAVE that settles against a channel the broker
            # has killed fails ahead of the browse side's liveness poll.
            # That failing settle marks the browse's OWN tracker (one per
            # browse session, from DeadLetterConsumer.browse), which is
            # the one the browse generator's cleanup reads.
            async with self._dead_channel_mapped(
                settings.settings.dead_letter_queue_name
            ):
                async with aclosing(
                    merge(
                        self._stream_dead_letters(generator),
                        self._listen_for_dead_letter_actions(
                            request_iterator, channel, ack_tracker
                        ),
                    )
                ) as streamer:
                    async for stream_response in streamer:
                        yield stream_response
        finally:
            await generator.aclose()

    async def CreateDurableQueue(
        self, request: CreateDurableQueueRequest, context: grpc.aio.ServicerContext
    ):
        capacity_guard.raise_if_at_capacity(durable_pool=True)

        await authenticate()

        # A proto3 scalar string defaults to "" when unset, so an empty
        # queue_id means the client did not supply one. Validate any
        # supplied id before touching the broker, mirroring the
        # empty-filters validation-before-broker precedent -- and before
        # the request reaches the log line below, for the reason given in
        # ConsumeDurableEvents.
        queue_id = request.queue_id or None
        if queue_id is not None:
            validate_queue_id(queue_id)

        logger.info(
            "Received create durable queue request for queue %s with "
            "%d filter group(s)",
            queue_id,
            len(request.filter_groups),
        )
        # get the filter groups for the request
        filter_groups = convert_to_models(request.filter_groups)  # type: ignore

        queue_id, created = await DurableQueueConsumer.create_queue(
            filter_groups, queue_id=queue_id
        )

        return CreateDurableQueueResponse(queue_id=queue_id, created=created)

    async def DeleteDurableQueue(
        self, request: DeleteDurableQueueRequest, _context: grpc.aio.ServicerContext
    ):
        capacity_guard.raise_if_at_capacity(durable_pool=True)

        await authenticate()

        # Validated before it is logged, for the reason given in
        # ConsumeDurableEvents.
        validate_queue_id(request.queue_id)

        logger.info(
            "Received delete durable queue request for queue %s", request.queue_id
        )

        await DurableQueueConsumer.delete_queue(request.queue_id)
        return DeleteDurableQueueResponse()

    async def UpdateDurableQueue(
        self, request: UpdateDurableQueueRequest, context: grpc.aio.ServicerContext
    ):
        capacity_guard.raise_if_at_capacity(durable_pool=True)

        await authenticate()

        # Validated before it is logged, for the reason given in
        # ConsumeDurableEvents.
        validate_queue_id(request.queue_id)

        logger.info(
            "Received update durable queue request for queue %s: "
            "%d current filter group(s), %d new filter group(s)",
            request.queue_id,
            len(request.current_filter_groups),
            len(request.new_filter_groups),
        )

        current_filter_groups = convert_to_models(request.current_filter_groups)
        new_filter_groups = convert_to_models(request.new_filter_groups)

        await DurableQueueConsumer.update_queue(
            request.queue_id, current_filter_groups, new_filter_groups
        )

        return UpdateDurableQueueResponse()

    # SR-3.7 admission-class stamp. The chart-side X-Via-Proxy injection is
    # retained permanently (b.jfs is closed as superseded).
    _PROXY_HEADER = "x-via-proxy"

    @staticmethod
    def _is_proxied(context: grpc.aio.ServicerContext) -> bool:
        """
        Return True if a retry of this stream can land on another pod.

        The X-Via-Proxy stamp means the request arrived through the
        per-request load-balanced proxy path, so the client's retry is
        routed afresh and this stream is re-routable: it is shed at the
        shed threshold, below the memory cap (SDD SR-3.7). An unstamped
        request reached this pod directly, is pinned to it for the life
        of the stream, and is therefore admitted all the way up to the
        cap.

        The stamp is trusted as sent. A direct client that supplies the
        header self-classifies as re-routable and may be shed early;
        that only costs it its own stream, so no anti-spoof hardening is
        warranted here. Operator docs carry the warning.
        """
        metadata = dict(context.invocation_metadata())
        return bool(metadata.get(EventConsumerServicer._PROXY_HEADER))

    @classmethod
    async def _get_non_durable_consumer(
        cls,
        first_request: ConsumeNonDurableEventsRequest,
    ) -> NonDurableQueueConsumer:
        # Summarised rather than interpolated: the request carries the
        # client's reconnect_token, and rendering the message would print
        # it. See `describe_non_durable_request`.
        logger.info(
            "Received first consume non-durable request from stream: %s",
            describe_non_durable_request(first_request),
        )

        initial_filter_grps, prev_filter_grps = await cls._filter_grps_from_request(
            first_request
        )

        return await NonDurableQueueConsumer.get_instance(
            first_request.reconnect_token,
            initial_filter_grps,
            prev_filter_grps,
        )

    @staticmethod
    async def _stream_events_from_non_durable_queue(
        consumer: NonDurableQueueConsumer,
    ) -> AsyncGenerator[ConsumeNonDurableEventsResponse, None]:

        async for event_msg in consumer.consume():
            response = EventConsumerServicer._build_event_response(
                event_msg,
                ConsumeNonDurableEventsResponse(
                    reconnect_token=consumer.queue_id,
                    supports_filter_update_acks=True,
                ),
            )

            yield response  # type: ignore

    @classmethod
    async def _listen_for_filter_updates(
        cls, request_iterator: AsyncIterator[ConsumeNonDurableEventsRequest]
    ) -> AsyncGenerator[ConsumeNonDurableEventsRequest, None]:

        async for request in request_iterator:
            # Summarised for the same reason as the first request: every
            # message on this stream repeats the client's reconnect_token.
            logger.info(
                "Received consume non-durable request for filter update: %s",
                describe_non_durable_request(request),
            )

            # A request that names no set at all is not an update and is
            # dropped here, unacknowledged: there is nothing to echo, and an
            # empty echo would report that the client asked for no filter
            # groups at all.
            # Every request that names a set is forwarded, including one
            # whose two sets are equal -- the "filters are unchanged, do
            # nothing" predicate lives in `update_queue` and only there, and
            # the caller needs its verdict to acknowledge the no-op. The
            # conversion to filter-group models happens once, in the merged
            # loop, where the update is applied.
            if not request.filter_groups and not request.previous_filter_groups:
                continue

            yield request

    @staticmethod
    async def _stream_events_from_durable_queue_with_acks(
        generator: AsyncGenerator[EventMessage, None],
    ) -> AsyncGenerator[ConsumeDurableEventsWithAcksResponse, None]:
        async for event_msg in generator:
            response = EventConsumerServicer._build_event_response(
                event_msg,
                ConsumeDurableEventsWithAcksResponse(
                    delivery_tag=event_msg.delivery_tag
                ),
            )
            yield response

    @staticmethod
    async def _listen_for_durable_acks(
        request_iterator: AsyncIterator[ConsumeDurableEventsWithAcksRequest],
        ack_tracker: AckTracker,
    ) -> AsyncGenerator[None, None]:
        """
        Settle client-supplied delivery tags: ack first, then nack.

        The two loops run in that order per request, which is what gives the
        documented precedence for a tag the client put in BOTH fields: the
        ack settles and removes it, so the nack that follows finds nothing
        and is skipped. Nacks always requeue -- ack already covers "drop
        and forget", so requeue=False has no caller here -- and the broker
        redelivers a requeued message immediately, possibly on this same
        stream.

        An unknown or already-settled tag raises KeyError out of the
        tracker and is warned about and skipped, on both verbs: one stale
        tag from a client that is behind must not take the stream down.
        KeyError alone is caught here; a dead channel propagates, with the
        tracker already marked by the failing settle, and is mapped to the
        classified ``ConsumerCancelledError`` by the caller.
        """
        async for request in request_iterator:
            for tag in request.ack_delivery_tags:
                try:
                    await ack_tracker.ack(tag)
                except KeyError:
                    logger.warning("Received ack for unknown delivery_tag=%s", tag)

            for tag in request.nack_delivery_tags:
                try:
                    await ack_tracker.nack(tag, requeue=True)
                except KeyError:
                    logger.warning("Received nack for unknown delivery_tag=%s", tag)
                else:
                    consumer_metrics.record_client_nack()
        # This generator never yields responses — it only settles delivery
        # tags. The empty yield makes it an async generator, so it is an
        # async iterator `merge` can drive.
        return  # pragma: no cover
        yield  # pragma: no cover

    _Outcome = ManageDeadLetteredEventsResponse.ActionOutcome.Outcome

    # Maps a structured REPLAY result status to the proto ActionOutcome.Outcome.
    _REPLAY_STATUS_TO_OUTCOME = {
        ReplayStatus.REPLAYED: _Outcome.OUTCOME_REPLAYED,
        ReplayStatus.ORIGIN_QUEUE_DELETED: _Outcome.OUTCOME_ORIGIN_QUEUE_DELETED,
        ReplayStatus.REPLAY_FAILED: _Outcome.OUTCOME_REPLAY_FAILED,
    }

    # Maps a non-replay (DISCARD / LEAVE) action status to the proto
    # ActionOutcome.Outcome. UNKNOWN_TAG is filtered by the early return in
    # _dispatch_dead_letter_action and never reaches this map, so an unmapped
    # future ActionStatus raises KeyError loudly rather than mapping silently.
    _ACTION_STATUS_TO_OUTCOME = {
        ActionStatus.DISCARDED: _Outcome.OUTCOME_DISCARDED,
        ActionStatus.LEFT: _Outcome.OUTCOME_LEFT,
    }

    @staticmethod
    async def _stream_dead_letters(
        generator: AsyncGenerator[DecodedDeadLetter, None],
    ) -> AsyncGenerator[ManageDeadLetteredEventsResponse, None]:
        """
        Build the dead_lettered_event-arm responses from the browse generator.

        Each decoded dead-letter becomes a ``ManageDeadLetteredEventsResponse``
        whose ``dead_lettered_event`` arm carries the decoded reason, origin
        queue, delivery count, dead-letter timestamp, resource id, the session
        delivery tag, and the embedded ``Event`` (event_type /
        principal_identity / payload) built via ``convert_to_event_proto``.
        """
        DeadLetteredEvent = ManageDeadLetteredEventsResponse.DeadLetteredEvent
        async for record in generator:
            event_proto = convert_to_event_proto(record.message)

            dead_lettered_event = DeadLetteredEvent(
                reason=DeadLetteredEvent.Reason.Value(record.reason),
                origin_queue=record.origin_queue,
                delivery_count=record.delivery_count,
                event=event_proto,
                resource_id=record.resource_id,
                delivery_tag=record.delivery_tag,
            )

            if record.dead_lettered_at is not None:
                timestamp = Timestamp()
                timestamp.FromDatetime(record.dead_lettered_at)
                dead_lettered_event.dead_lettered_at.CopyFrom(timestamp)

            yield ManageDeadLetteredEventsResponse(
                dead_lettered_event=dead_lettered_event
            )

    @classmethod
    async def _listen_for_dead_letter_actions(
        cls,
        request_iterator: AsyncIterator[ManageDeadLetteredEventsRequest],
        channel,
        ack_tracker: AckTracker,
    ) -> AsyncGenerator[ManageDeadLetteredEventsResponse, None]:
        """
        Dispatch per-tag REPLAY / DISCARD / LEAVE actions and report outcomes.

        Mirrors ``_listen_for_durable_acks`` in shape, but YIELDS an
        ``action_outcome``-arm ``ManageDeadLetteredEventsResponse`` for every
        action so the operator observes each result, correlated by
        ``delivery_tag`` (SR-3.4). REPLAY republishes on the SAME
        publisher-confirm browse ``channel`` so the DLQ copy is acked only on
        publish-confirm; an ORIGIN_QUEUE_DELETED or REPLAY_FAILED result leaves
        the DLQ copy in place (no loss). An UNKNOWN_TAG result is skipped
        without crashing the stream.
        """
        Action = ManageDeadLetteredEventsRequest.Action
        async for request in request_iterator:
            for entry in request.actions:
                outcome = await cls._dispatch_dead_letter_action(
                    entry, channel, ack_tracker, Action
                )
                if outcome is not None:
                    yield outcome

    @classmethod
    async def _dispatch_dead_letter_action(
        cls,
        entry: ManageDeadLetteredEventsRequest.ActionEntry,
        channel,
        ack_tracker: AckTracker,
        Action,
    ) -> ManageDeadLetteredEventsResponse | None:
        """
        Dispatch one action entry to its handler and map it to an outcome.

        Returns the ``action_outcome``-arm response to stream back, or ``None``
        for an UNKNOWN_TAG result (skipped without crashing the stream).
        """
        tag = entry.delivery_tag

        if entry.action == Action.ACTION_REPLAY:
            origin_queue = cls._origin_queue_for_tag(ack_tracker, tag)
            if origin_queue is None:
                logger.warning(
                    "REPLAY for unknown delivery_tag=%s (already actioned?)", tag
                )
                return None
            result = await DeadLetterConsumer.replay(
                channel, ack_tracker, tag, origin_queue
            )
            return cls._outcome_response(
                tag, cls._REPLAY_STATUS_TO_OUTCOME[result.status], result.detail
            )

        if entry.action == Action.ACTION_DISCARD:
            result = await DeadLetterConsumer.discard(ack_tracker, tag)
        elif entry.action == Action.ACTION_LEAVE:
            result = await DeadLetterConsumer.leave(ack_tracker, tag)
        else:
            logger.warning(
                "Received unspecified dead-letter action=%s for delivery_tag=%s",
                entry.action,
                tag,
            )
            return None

        if result.status == ActionStatus.UNKNOWN_TAG:
            return None

        outcome = cls._ACTION_STATUS_TO_OUTCOME[result.status]
        return cls._outcome_response(tag, outcome, result.detail)

    @staticmethod
    def _origin_queue_for_tag(ack_tracker: AckTracker, delivery_tag: int) -> str | None:
        """
        Resolve the origin queue for a REPLAY from the tracker's per-tag cache.

        The origin queue MUST come from the message's ``x-death`` entry, never
        from client input (replay-target invariant in
        ``DeadLetterConsumer.replay``). The browse path decodes each message
        once and caches its ``origin_queue`` in the tracker keyed by
        ``delivery_tag``, so REPLAY recovers the same value without a second
        full decode of the body. Returns ``None`` for an unknown /
        already-actioned tag so the caller can skip without crashing the
        stream.
        """
        return ack_tracker.origin_queue(delivery_tag)

    @staticmethod
    def _outcome_response(
        delivery_tag: int,
        outcome,
        detail: str | None,
    ) -> ManageDeadLetteredEventsResponse:
        action_outcome = ManageDeadLetteredEventsResponse.ActionOutcome(
            delivery_tag=delivery_tag,
            outcome=outcome,
            detail=detail or "",
        )
        return ManageDeadLetteredEventsResponse(action_outcome=action_outcome)

    @staticmethod
    async def _filter_grps_from_request(request):
        new_filter_groups: List[FilterGroup] = convert_to_models(
            request.filter_groups  # type: ignore
        )

        previous_filter_groups: List[FilterGroup] = convert_to_models(
            request.previous_filter_groups  # type: ignore
        )

        return new_filter_groups, previous_filter_groups

    @staticmethod
    def _build_event_response(
        event_msg: EventMessage,
        response: (
            ConsumeDurableEventsResponse
            | ConsumeDurableEventsWithAcksResponse
            | ConsumeNonDurableEventsResponse
        ),
    ) -> (
        ConsumeDurableEventsResponse
        | ConsumeDurableEventsWithAcksResponse
        | ConsumeNonDurableEventsResponse
    ):
        """
        Builds a response from an event message, and includes metrics tracking.

        Args:
            event_msg: The event message to process
            response: The response object to populate with the event

        Returns:
            The response object with the processed event
        """
        try:
            event_proto = convert_to_event_proto(event_msg.message)
            response.events.append(event_proto)

            consumer_metrics.record_event_processing_success(event_msg)
            return response

        except Exception:
            consumer_metrics.record_event_processing_failure(event_msg)
            raise
