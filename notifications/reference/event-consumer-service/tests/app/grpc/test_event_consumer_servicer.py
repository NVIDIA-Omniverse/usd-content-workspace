# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import inspect
import json
import logging
import re

import aio_pika
import grpc
import pytest
from unittest.mock import AsyncMock, MagicMock

from app.config import settings
from app.grpc import event_consumer_servicer as servicer_module
from app.grpc.event_consumer_servicer import (
    EventConsumerServicer,
)
from app.lib.models.event_message import EventMessage
from app.lib.observability.consumer_metrics import consumer_metrics
from app.lib.permissions import permissions
from app.lib.permissions.storage_permissions import (
    cache_writer as cache_writer_module,
    storage_permissions,
)
from app.lib.queue_names import redact_reconnect_token
from app.lib.rabbitmq import (
    DurableQueueConsumer,
    NonDurableQueueConsumer,
    capacity_guard,
)
from app.lib.rabbitmq.ack_tracker import AckTracker
from app.lib.rabbitmq.dead_letter_consumer import (
    ActionResult,
    ActionStatus,
    DeadLetterConsumer,
)
from app.lib.rabbitmq.rabbitmq_base import (
    ConsumerCancelledError,
    RabbitMQBase,
    rabbitmq_connector,
)
from notification_common.errors.error_details import ErrorDetails
from notification_common.errors.errors import (
    AuthenticationError,
    BadRequestError,
    ChannelNotAvailableError,
    QueueNotFoundError,
    UnprocessableError,
)
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ConsumeDurableEventsRequest,
    ConsumeDurableEventsWithAcksRequest,
    ConsumeNonDurableEventsRequest,
    CreateDurableQueueRequest,
    DeleteDurableQueueRequest,
    FilterGroup as ProtoFilterGroup,
    ManageDeadLetteredEventsRequest,
    ManageDeadLetteredEventsResponse,
    ResourceFilter as ProtoResourceFilter,
    UpdateDurableQueueRequest,
)
from tests.fixtures.cache_scope import TEST_PRINCIPAL_REF
from tests.fixtures.consumer_liveness import (
    CONSUMER_LOSS_TIMEOUT_S,
    IDLE_POLL_NEVER_FIRES_SECONDS,
    PUMP_CONSUMER_TAG,
    PumpedQueue,
)
from tests.fixtures.dead_channel import UNRELATED_FAILURES, dead_channel_failures
from tests.fixtures.own_operation_dead_channel import (
    DURABLE_QUEUE_ID as OWN_OPERATION_QUEUE_ID,
    RECONNECT_GUIDANCE,
    OwnOperationDeadChannelRig,
)
from tests.fixtures.queue_id_bounds import (
    MALFORMED_QUEUE_IDS,
    MALFORMED_QUEUE_ID_IDS,
    MALFORMED_RECONNECT_TOKENS,
    MALFORMED_RECONNECT_TOKEN_IDS,
    MAX_LENGTH_RECONNECT_TOKEN,
    MAX_QUEUE_ID_LENGTH,
    OVER_LONG_RECONNECT_TOKEN,
)
from tests.fixtures.reconnect_token import RECONNECT_TOKEN

_OUTCOME = ManageDeadLetteredEventsResponse.ActionOutcome.Outcome
_ACTION = ManageDeadLetteredEventsRequest.Action


@pytest.fixture
def servicer():
    return EventConsumerServicer()


@pytest.fixture
def mock_context():
    ctx = MagicMock()
    ctx.peer.return_value = "ipv4:10.0.0.1:5000"
    ctx.invocation_metadata.return_value = []
    # ``grpc.aio``'s send_initial_metadata is a coroutine, so the realistic
    # drift a servicer would introduce is ``await
    # context.send_initial_metadata(...)``. A bare MagicMock attribute is not
    # awaitable, which would turn that drift into a TypeError inside the RPC
    # rather than a failed "was it sent?" assertion.
    ctx.send_initial_metadata = AsyncMock()
    return ctx


class TestIsProxied:
    """
    _is_proxied classifies a stream on exactly one bit: the X-Via-Proxy
    request stamp (SDD SR-3.7). Stamped means the stream arrived through
    the per-request load-balanced proxy path, so a retry can land on
    another pod -- re-routable, sheddable below the memory cap. Unstamped
    means the stream is pinned to this pod and only the cap may refuse it.
    """

    def test_returns_false_when_no_metadata(self, servicer, mock_context):
        # The fixture's default: no headers at all. A stream that carries
        # no stamp is pinned.
        result = servicer._is_proxied(mock_context)

        assert result is False

    def test_returns_true_when_proxy_header_present(self, servicer, mock_context):
        mock_context.invocation_metadata.return_value = [
            ("x-via-proxy", "true"),
        ]

        result = servicer._is_proxied(mock_context)

        assert result is True

    def test_returns_true_when_proxy_header_among_others(self, servicer, mock_context):
        # The stamp is found regardless of its position among real headers.
        mock_context.invocation_metadata.return_value = [
            ("authorization", "Bearer token"),
            ("x-via-proxy", "true"),
            ("user-agent", "grpc-python"),
        ]

        result = servicer._is_proxied(mock_context)

        assert result is True

    def test_returns_false_when_other_headers_present(self, servicer, mock_context):
        mock_context.invocation_metadata.return_value = [
            ("authorization", "Bearer token"),
            ("user-agent", "grpc-python"),
        ]

        result = servicer._is_proxied(mock_context)

        assert result is False

    def test_treats_empty_proxy_header_value_as_absent(self, servicer, mock_context):
        # An empty stamp value classifies as pinned. _is_proxied
        # truthiness-tests metadata.get(_PROXY_HEADER), so "" reads the
        # same as a missing header: an X-Via-Proxy header sent with an
        # empty value carries no admission-class signal and must not flip
        # the stream to re-routable. Anything other than a non-empty stamp
        # keeps the stream pinned to this pod, admitted up to the cap.
        mock_context.invocation_metadata.return_value = [
            ("x-via-proxy", ""),
        ]

        result = servicer._is_proxied(mock_context)

        assert result is False

    def test_honors_self_applied_stamp_from_direct_client(self, servicer, mock_context):
        # Per SR-3.7 audit addition (a) the stamp is trusted as sent: this
        # is a direct (unproxied) client that supplied the header itself,
        # and it is still classified re-routable. That is deliberate, not
        # a gap -- self-classifying only costs the client its own stream
        # (it may be shed early and its retry lands back on this pod), so
        # no anti-spoof hardening is warranted in the data path. The
        # warning is an operator-doc obligation carried by Task 6. This
        # test exists so a later "hardening" change cannot land silently
        # as a bug fix.
        mock_context.peer.return_value = "ipv4:10.0.0.99:5000"
        mock_context.invocation_metadata.return_value = [
            ("user-agent", "grpc-python"),
            ("x-via-proxy", "true"),
        ]

        result = servicer._is_proxied(mock_context)

        assert result is True


class TestPeerIdentityHelperNameContract:
    """
    Name contract in the style of
    ``tests/app/config/test_settings.py``'s
    ``TestMemorySafeStreamCapNameContract``: the SR-3.7 admission bit
    survives permanently while the peer-identity resolver is deleted
    (SDD SR-4.2). Guards both directions -- a resurrection of
    ``_get_peer_id`` and an over-broad deletion sweep that would take
    ``_is_proxied`` / ``_PROXY_HEADER`` (and with them two-tier
    admission) along with it.
    """

    def test_is_proxied_and_proxy_header_survive(self):
        assert hasattr(EventConsumerServicer, "_is_proxied")
        assert EventConsumerServicer._PROXY_HEADER == "x-via-proxy"

    def test_get_peer_id_is_gone(self):
        assert not hasattr(EventConsumerServicer, "_get_peer_id")


class TestGetNonDurableConsumerNoPeerArgument:
    """
    The servicer no longer resolves peer identity when building the
    consumer (SDD SR-4.2): ``_get_non_durable_consumer`` calls
    ``get_instance`` with exactly the reconnect token plus the two
    filter-group arguments and no fourth positional / ``peer_id``
    keyword. This is the pin that fails loudly if a rebase resurrects
    the parameter and re-threads it through the servicer.
    """

    @pytest.mark.asyncio
    async def test_get_instance_called_without_peer_argument(self, servicer, mocker):
        get_instance = mocker.patch.object(NonDurableQueueConsumer, "get_instance")
        mocker.patch.object(
            EventConsumerServicer,
            "_filter_grps_from_request",
            return_value=(["initial"], ["previous"]),
        )
        first_request = MagicMock()
        first_request.reconnect_token = "token-abc"

        await servicer._get_non_durable_consumer(first_request)

        get_instance.assert_called_once_with("token-abc", ["initial"], ["previous"])
        _, kwargs = get_instance.call_args
        assert "peer_id" not in kwargs


async def _empty_async_iterator():
    """An async iterator that yields nothing, simulating a client that
    opens a bidirectional stream but sends zero messages."""
    return
    yield  # pragma: no cover – makes this a valid async generator


class TestConsumeDurableEventsWithAcksEmptyStream:

    @pytest.mark.asyncio
    async def test_returns_cleanly_when_client_sends_no_messages(
        self, servicer, mock_context, mocker
    ):
        # ConsumeDurableEventsWithAcks admits on the durable pool
        # (raise_if_at_capacity(durable_pool=True)), which is the same
        # entry point the non-durable path uses with the default
        # argument -- so one patch target covers both. The autouse
        # before_each already neutralizes this method; re-patching here
        # is redundant but kept deliberately, to name the admission call
        # this test has to get past before it can reach the empty-stream
        # behavior it actually asserts. patch.object on the singleton
        # (not the string form) so a rename fails at the patch site.
        mocker.patch.object(capacity_guard, "raise_if_at_capacity")

        responses = [
            r
            async for r in servicer.ConsumeDurableEventsWithAcks(
                _empty_async_iterator(), mock_context
            )
        ]

        assert responses == []


_DEAD_CHANNEL_QUEUE_ID = "queue-with-a-dead-channel"


async def _idle_event_generator():
    """
    The event side of the merge, ending immediately without yielding.

    The channel-death arms below need the settle side to be the source of
    the exception, so the event side contributes nothing. That pins the
    client-facing classification and the end state of the tracker mark, but
    NOT the race the mark exists for: this generator has finished before
    the settle fails, so there is no pending event read for the merge to
    cancel and no consume-loop cleanup to read the mark. That race -- a
    settle failing while the real consume loop waits on its next delivery --
    is pinned by ``TestASettleSideChannelDeathNarrowsTheEventSideCleanup``.
    """
    return
    yield  # pragma: no cover - makes this a valid async generator


def _settle_failure_rig(mocker, failure, verb="nack"):
    """
    Patch ``DurableQueueConsumer.consume_with_acks`` so settling delivery
    tag 1 fails with ``failure``.

    The REAL ``AckTracker`` is used, holding one aio-pika message whose
    ``ack`` / ``nack`` raises: that is where a dead channel actually
    surfaces, and it keeps the tracker's own pop-then-settle ordering in
    the path under test. Passing ``failure=None`` gives the healthy control
    arm, where the settle succeeds and the stream ends normally.
    """
    tracker = AckTracker()
    message = AsyncMock(spec=aio_pika.abc.AbstractIncomingMessage)
    message.delivery_tag = 1
    if failure is not None:
        getattr(message, verb).side_effect = failure
    tracker.track(message)

    async def _consume_with_acks(_queue_id):
        return _idle_event_generator(), tracker

    mocker.patch.object(
        DurableQueueConsumer, "consume_with_acks", side_effect=_consume_with_acks
    )
    return tracker


class _RecordingGenerator:
    """
    A consume generator that yields nothing and records being closed.

    A plain async generator cannot carry a spy -- generator objects take no
    attribute assignment, so ``aclose`` cannot be wrapped -- and one that
    never started never sees GeneratorExit either. This stands in for the
    consume generator where the assertion is about the servicer's
    ``finally``, not about the loop's own behaviour.
    """

    def __init__(self):
        self.closed = False

    def __aiter__(self):
        return self

    async def __anext__(self):
        raise StopAsyncIteration

    async def aclose(self):
        self.closed = True


async def _settle_requests(verb, tag=1):
    """A client stream that opens the queue then settles one tag."""
    yield ConsumeDurableEventsWithAcksRequest(queue_id=_DEAD_CHANNEL_QUEUE_ID)
    if verb == "nack":
        yield ConsumeDurableEventsWithAcksRequest(nack_delivery_tags=[tag])
    else:
        yield ConsumeDurableEventsWithAcksRequest(ack_delivery_tags=[tag])


# The broker spelling of a durable queue: the `event-queue` prefix joined
# to the client's queue_id, with nothing derived from the caller in
# between. What the mapped error must carry, because that is the spelling
# the broker-side log lines around the incident use.
_BROKER_QUEUE_NAME_PATTERN = r"queue_name: event-queue:" + _DEAD_CHANNEL_QUEUE_ID


class TestDeadChannelMapped:
    """
    The shared primitive the three bidi streams wrap their merge in,
    exercised directly.

    The RPC-level classes below pin it end to end on each surface; these
    pin the helper's own contract -- the client-facing raise, and only for
    the dead-channel classes -- so a change to it fails here, at the one
    place it is written, rather than only in whichever RPC happens to cover
    it. It is classification only: the cleanup's dead-channel verdict comes
    from the failing broker call itself (``AckTracker`` marks itself at a
    failing settle), never from this wrapper.
    """

    @pytest.mark.asyncio
    @dead_channel_failures
    async def test_a_dead_channel_becomes_consumer_cancelled(self, make_failure):
        with pytest.raises(ConsumerCancelledError, match="Reconnect to resume"):
            async with EventConsumerServicer._dead_channel_mapped("event-queue:q"):
                raise make_failure()

    @pytest.mark.asyncio
    @dead_channel_failures
    async def test_the_mapped_error_records_the_channel_as_closed(self, make_failure):
        with pytest.raises(ConsumerCancelledError) as exc_info:
            async with EventConsumerServicer._dead_channel_mapped("q"):
                raise make_failure()

        assert exc_info.value.channel_closed is True

    @pytest.mark.asyncio
    async def test_the_mapped_error_is_chained_to_the_dead_channel_error(self):
        failure = aio_pika.exceptions.ChannelInvalidStateError()

        with pytest.raises(ConsumerCancelledError) as exc_info:
            async with EventConsumerServicer._dead_channel_mapped("q"):
                raise failure

        assert exc_info.value.__cause__ is failure

    @pytest.mark.asyncio
    async def test_the_error_carries_the_queue_name_it_was_given(self):
        with pytest.raises(ConsumerCancelledError, match="queue_name: the-broker-name"):
            async with EventConsumerServicer._dead_channel_mapped("the-broker-name"):
                raise aio_pika.exceptions.ChannelInvalidStateError()

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "make_failure",
        [lambda: ValueError("a genuine internal bug"), *UNRELATED_FAILURES.values()],
        ids=["value_error", *UNRELATED_FAILURES],
    )
    async def test_an_unrelated_exception_is_not_mapped(self, make_failure):
        # The builtin ConnectionError arm matters on its own: aiormq's
        # closed-connection family subclasses it, so a catch widened to the
        # base class would map every unrelated socket failure as a dead
        # channel.
        failure = make_failure()

        with pytest.raises(type(failure)) as exc_info:
            async with EventConsumerServicer._dead_channel_mapped("q"):
                raise failure

        assert exc_info.value is failure

    @pytest.mark.asyncio
    async def test_a_consumer_cancelled_error_passes_through_as_itself(self):
        # The consume loops already end a stream with ConsumerCancelledError
        # when one of their own broker operations finds the channel dead,
        # and the liveness poll does when it finds the consumer gone. That
        # error carries the loop's own verdict, so the wrapper must not
        # replace it.
        failure = ConsumerCancelledError("q", channel_closed=False)

        with pytest.raises(ConsumerCancelledError) as exc_info:
            async with EventConsumerServicer._dead_channel_mapped("q"):
                raise failure

        assert exc_info.value is failure


class TestConsumeDurableEventsWithAcksChannelDeath:
    """
    A settle that fails because the channel is dead ends the stream with
    the classified ``ConsumerCancelledError``, not a bare INTERNAL.

    The dead-channel classes (``DEAD_CHANNEL_EXC``) are the ONLY ones
    mapped: an unrelated exception is a genuine internal bug and must still
    surface as itself, otherwise every server-side defect on this RPC would
    be reported to clients as "reconnect and resume" and quietly retried
    forever.
    """

    @staticmethod
    async def _drive(servicer, context, verb):
        return [
            r
            async for r in servicer.ConsumeDurableEventsWithAcks(
                _settle_requests(verb), context
            )
        ]

    @pytest.mark.asyncio
    @dead_channel_failures
    async def test_nack_on_a_dead_channel_ends_the_stream_as_consumer_cancelled(
        self, servicer, mock_context, mocker, make_failure
    ):
        _settle_failure_rig(mocker, make_failure())

        with pytest.raises(ConsumerCancelledError, match="Reconnect to resume"):
            await self._drive(servicer, mock_context, "nack")

    @pytest.mark.asyncio
    async def test_nack_on_a_dead_channel_records_the_channel_as_closed(
        self, servicer, mock_context, mocker
    ):
        # channel_closed=True is a statement of fact about the incarnation
        # that carried this session's delivery tags: every mapped class means
        # it is gone. It is the client-facing half of the classification --
        # what suppresses the doomed cleanup nacks is the tracker mark, not
        # this flag (that mechanism is pinned separately below).
        _settle_failure_rig(mocker, aio_pika.exceptions.ChannelInvalidStateError())

        with pytest.raises(ConsumerCancelledError) as exc_info:
            await self._drive(servicer, mock_context, "nack")

        assert exc_info.value.channel_closed is True

    @pytest.mark.asyncio
    @pytest.mark.parametrize("verb", ["ack", "nack"], ids=["ack", "nack"])
    async def test_a_dead_channel_settle_marks_the_session_tracker(
        self, servicer, mock_context, mocker, verb
    ):
        # The end state of the mark that narrows the consume loop's cleanup.
        # The raise above is client-facing only: it never reaches the consume
        # generator, whose cleanup reads the verdict off this tracker. The
        # servicer's mapping does not mark it; the tracker marks itself at
        # the failing settle, so this pins only that the tracker's own mark
        # lands on this RPC's path. That it lands before the cleanup reads it
        # is pinned by ``TestASettleSideChannelDeathNarrowsTheEventSideCleanup``,
        # whose consume loop is still waiting when the settle fails.
        tracker = _settle_failure_rig(
            mocker, aio_pika.exceptions.ChannelInvalidStateError(), verb=verb
        )

        with pytest.raises(ConsumerCancelledError):
            await self._drive(servicer, mock_context, verb)

        assert tracker.channel_dead is True

    @pytest.mark.asyncio
    async def test_an_unrelated_settle_failure_leaves_the_tracker_unmarked(
        self, servicer, mock_context, mocker
    ):
        # The other half of "only the dead-channel classes are mapped": a
        # genuine internal bug must not also suppress the cleanup nacks,
        # because on that path the channel is still alive and those
        # deliveries are still ours to hand back.
        tracker = _settle_failure_rig(mocker, ValueError("a genuine internal bug"))

        with pytest.raises(ValueError):
            await self._drive(servicer, mock_context, "nack")

        assert tracker.channel_dead is False

    @pytest.mark.asyncio
    async def test_a_healthy_stream_leaves_the_tracker_unmarked(
        self, servicer, mock_context, mocker
    ):
        # The control for every mark assertion above: a settle that succeeds
        # leaves the verdict alone, so the mark tracks the failure rather
        # than merely happening on every stream teardown.
        tracker = _settle_failure_rig(mocker, None)

        await self._drive(servicer, mock_context, "nack")

        assert tracker.channel_dead is False

    @pytest.mark.asyncio
    async def test_ack_on_a_dead_channel_ends_the_stream_as_consumer_cancelled(
        self, servicer, mock_context, mocker
    ):
        # The catch sits above the whole merge, so both verbs are covered by
        # one mapping. Pinned for the ack side too: a future refactor that
        # narrowed the catch to the nack loop would pass every nack test
        # above while silently regressing the ack side.
        _settle_failure_rig(
            mocker,
            aio_pika.exceptions.ChannelClosed(406, "PRECONDITION_FAILED"),
            verb="ack",
        )

        with pytest.raises(ConsumerCancelledError, match="Reconnect to resume"):
            await self._drive(servicer, mock_context, "ack")

    @pytest.mark.asyncio
    async def test_unrelated_settle_failure_is_not_mapped(
        self, servicer, mock_context, mocker
    ):
        _settle_failure_rig(mocker, ValueError("a genuine internal bug"))

        with pytest.raises(ValueError, match="a genuine internal bug"):
            await self._drive(servicer, mock_context, "nack")

    @pytest.mark.asyncio
    async def test_the_error_names_the_broker_queue(
        self, servicer, mock_context, mocker
    ):
        # The name goes to the log-only ``internal_message`` so the incident
        # correlates with the broker-side log lines around it, which carry
        # the broker spelling. The client still gets the generic guidance.
        _settle_failure_rig(mocker, aio_pika.exceptions.ChannelInvalidStateError())

        with pytest.raises(ConsumerCancelledError, match=_BROKER_QUEUE_NAME_PATTERN):
            await self._drive(servicer, mock_context, "nack")

    @pytest.mark.asyncio
    async def test_a_queue_name_resolution_failure_still_closes_the_generator(
        self, servicer, mock_context, mocker
    ):
        # The broker name is resolved INSIDE the try whose finally closes the
        # consume generator. Hoisting that line above the try would look
        # harmless -- it only feeds an error message -- but would leak the
        # generator's pool channel on every failure to resolve.
        recording = _RecordingGenerator()

        async def _consume_with_acks(_queue_id):
            return recording, AckTracker()

        mocker.patch.object(
            DurableQueueConsumer, "consume_with_acks", side_effect=_consume_with_acks
        )
        mocker.patch.object(
            DurableQueueConsumer,
            "_queue_name",
            side_effect=ValueError("cannot resolve the queue name"),
        )

        with pytest.raises(ValueError, match="cannot resolve the queue name"):
            await self._drive(servicer, mock_context, "nack")

        assert recording.closed is True

    @pytest.mark.asyncio
    async def test_the_error_does_not_name_the_client_queue_id_alone(
        self, servicer, mock_context, mocker
    ):
        # The client's queue_id is not a broker queue name: nothing in the
        # broker's logs is spelled that way, so an error carrying it bare
        # would be uncorrelatable. Pinned separately because the pattern
        # above still matches a string that merely CONTAINS the queue_id.
        _settle_failure_rig(mocker, aio_pika.exceptions.ChannelInvalidStateError())

        with pytest.raises(ConsumerCancelledError) as exc_info:
            await self._drive(servicer, mock_context, "nack")

        assert f"queue_name: {_DEAD_CHANNEL_QUEUE_ID}" not in str(exc_info.value)


_DLQ_DEAD_CHANNEL_TAG = 1


def _dlq_settle_failure_rig(mocker, failure, verb="ack"):
    """
    Patch ``DeadLetterConsumer.browse`` so settling the browsed DLQ copy
    fails with ``failure``.

    The REAL ``AckTracker`` holds one aio-pika message whose ``ack`` (the
    DISCARD verb) or ``nack`` (LEAVE) raises: that is where a dead channel
    actually surfaces, and the production ``discard`` / ``leave`` handlers
    run unchanged over it. Passing ``failure=None`` gives the healthy
    control arm.
    """
    tracker = AckTracker()
    message = AsyncMock(spec=aio_pika.abc.AbstractIncomingMessage)
    message.delivery_tag = _DLQ_DEAD_CHANNEL_TAG
    if failure is not None:
        getattr(message, verb).side_effect = failure
    tracker.track(message, origin_queue="event-queue:queue-1")

    async def _browse():
        return _idle_event_generator(), tracker, MagicMock()

    mocker.patch.object(DeadLetterConsumer, "browse", side_effect=_browse)
    return tracker


async def _dlq_action_requests(action):
    """A client stream that sends one DLQ action for the browsed copy."""
    yield ManageDeadLetteredEventsRequest(
        actions=[
            ManageDeadLetteredEventsRequest.ActionEntry(
                delivery_tag=_DLQ_DEAD_CHANNEL_TAG, action=action
            )
        ]
    )


class TestManageDeadLetteredEventsChannelDeath:
    """
    The DLQ browse gets the SAME classification as the durable
    consume-with-acks stream, because it has the same shape: a settle loop
    merged with an event-producing generator, where a DISCARD / LEAVE that
    lands after the broker killed the channel fails ahead of the browse
    side's next idle liveness poll.

    The browse holds its OWN tracker (one per session, from
    ``DeadLetterConsumer.browse``), and that is the one the browse
    generator's cleanup reads -- so marking anything else would leave the
    cleanup nacking DLQ copies the broker already requeued.
    """

    @staticmethod
    async def _drive(servicer, context, action):
        return [
            r
            async for r in servicer.ManageDeadLetteredEvents(
                _dlq_action_requests(action), context
            )
        ]

    @pytest.mark.asyncio
    @dead_channel_failures
    async def test_discard_on_a_dead_channel_ends_the_stream_as_consumer_cancelled(
        self, servicer, mock_context, mocker, make_failure
    ):
        # DISCARD acks the DLQ copy, so it is the ack-side verb here.
        _dlq_settle_failure_rig(mocker, make_failure(), verb="ack")

        with pytest.raises(ConsumerCancelledError, match="Reconnect to resume"):
            await self._drive(servicer, mock_context, _ACTION.ACTION_DISCARD)

    @pytest.mark.asyncio
    async def test_leave_on_a_dead_channel_ends_the_stream_as_consumer_cancelled(
        self, servicer, mock_context, mocker
    ):
        # LEAVE nack-requeues the DLQ copy, the other settle verb. Covered
        # separately because a catch narrowed to one handler would pass every
        # DISCARD case above while regressing this one.
        _dlq_settle_failure_rig(
            mocker, aio_pika.exceptions.ChannelInvalidStateError(), verb="nack"
        )

        with pytest.raises(ConsumerCancelledError, match="Reconnect to resume"):
            await self._drive(servicer, mock_context, _ACTION.ACTION_LEAVE)

    @pytest.mark.asyncio
    async def test_a_dead_channel_action_records_the_channel_as_closed(
        self, servicer, mock_context, mocker
    ):
        _dlq_settle_failure_rig(mocker, aio_pika.exceptions.ChannelClosed(406, "gone"))

        with pytest.raises(ConsumerCancelledError) as exc_info:
            await self._drive(servicer, mock_context, _ACTION.ACTION_DISCARD)

        assert exc_info.value.channel_closed is True

    @pytest.mark.asyncio
    async def test_a_dead_channel_action_marks_the_browse_tracker(
        self, servicer, mock_context, mocker
    ):
        tracker = _dlq_settle_failure_rig(
            mocker, aio_pika.exceptions.ChannelInvalidStateError()
        )

        with pytest.raises(ConsumerCancelledError):
            await self._drive(servicer, mock_context, _ACTION.ACTION_DISCARD)

        assert tracker.channel_dead is True

    @pytest.mark.asyncio
    async def test_the_error_names_the_dead_letter_queue(
        self, servicer, mock_context, mocker
    ):
        # The browse session's queue is the configured DLQ, not any durable
        # queue, so that is the name an operator triaging the incident needs
        # to see in the log-only internal message.
        _dlq_settle_failure_rig(mocker, aio_pika.exceptions.ChannelInvalidStateError())

        with pytest.raises(ConsumerCancelledError) as exc_info:
            await self._drive(servicer, mock_context, _ACTION.ACTION_DISCARD)

        assert f"queue_name: {settings.settings.dead_letter_queue_name}" in str(
            exc_info.value
        )

    @pytest.mark.asyncio
    async def test_an_unrelated_action_failure_is_not_mapped(
        self, servicer, mock_context, mocker
    ):
        # A genuine internal bug in the DLQ path must still surface as
        # itself, or every server-side defect on this RPC would be reported
        # to operators as "reconnect and resume".
        _dlq_settle_failure_rig(mocker, ValueError("a genuine internal bug"))

        with pytest.raises(ValueError, match="a genuine internal bug"):
            await self._drive(servicer, mock_context, _ACTION.ACTION_DISCARD)

    @pytest.mark.asyncio
    async def test_an_unrelated_action_failure_leaves_the_tracker_unmarked(
        self, servicer, mock_context, mocker
    ):
        tracker = _dlq_settle_failure_rig(mocker, ValueError("a genuine internal bug"))

        with pytest.raises(ValueError):
            await self._drive(servicer, mock_context, _ACTION.ACTION_DISCARD)

        assert tracker.channel_dead is False

    @pytest.mark.asyncio
    async def test_a_healthy_action_leaves_the_tracker_unmarked(
        self, servicer, mock_context, mocker
    ):
        # The control arm: a DISCARD that settles cleanly reports its outcome
        # and leaves the browse cleanup on its ordinary path.
        tracker = _dlq_settle_failure_rig(mocker, None)

        responses = await self._drive(servicer, mock_context, _ACTION.ACTION_DISCARD)

        assert responses[0].action_outcome.outcome == _OUTCOME.OUTCOME_DISCARDED
        assert tracker.channel_dead is False


# A body both real consume loops turn into a response: the durable loop parses
# it into an EventMessage, the browse loop decodes it as a dead-letter, and
# both build an Event proto from it, which needs an event_type.
_PENDING_READ_EVENT_BODY = json.dumps(
    {
        "event_type": "asset.updated",
        "resource_id": "resource-1",
        "principal_identity": "user-1",
        "message": {"key": "value"},
    }
).encode("utf-8")

# The delivery the client settles, and the two that stay in flight. The
# in-flight ones are what the event side's cleanup would nack on a healthy
# channel, so an assertion about them is vacuous without them.
_SETTLED_TAG = 1
_IN_FLIGHT_TAGS = (2, 3)


def _pending_read_headers():
    """A fresh x-death header set the browse loop decodes an origin queue from."""
    return {
        "x-death": [
            {
                "queue": "event-queue:origin",
                "reason": "rejected",
                "count": 1,
                "time": 1700000000,
                "exchange": "events",
                "routing-keys": ["events.foo"],
            }
        ],
        "x-delivery-count": 1,
    }


class _PendingReadProbe:
    """
    Observes whether the consume loop is waiting inside ``_next_message``.

    It wraps the loop's pump-queue getter -- the ``messages_queue.get()``
    that ``_next_message`` awaits under the liveness-poll timeout -- so the
    rig can SEE the event read is pending when the settle fails, rather than
    inferring it from a sleep. ``parked`` is set while a get is outstanding
    and cleared when it returns or is cancelled. A delivery already buffered
    takes ``_next_message``'s fast path and never reaches the getter.
    """

    def __init__(self):
        self.pending = False
        self.parked = asyncio.Event()
        self._instrumented = None

    def instrument(self, messages_queue):
        if self._instrumented is messages_queue:
            return
        self._instrumented = messages_queue
        real_get = messages_queue.get

        async def _get():
            self.pending = True
            self.parked.set()
            try:
                return await real_get()
            finally:
                self.pending = False
                self.parked.clear()

        messages_queue.get = _get


async def _within_deadline(awaitable, what):
    """Await ``awaitable``, failing by name rather than hanging the suite."""
    try:
        return await asyncio.wait_for(awaitable, timeout=CONSUMER_LOSS_TIMEOUT_S)
    except asyncio.TimeoutError:
        pytest.fail(f"{what} within {CONSUMER_LOSS_TIMEOUT_S}s")


def _channel_is_closed():
    return aio_pika.exceptions.ChannelInvalidStateError("channel is closed")


def _origin_queue_gone():
    return aio_pika.exceptions.DeliveryError(None, None)


def _genuine_internal_bug():
    return ValueError("a genuine internal bug")


# What each settle surface sends, which verb on the delivery it reaches, and
# what REPLAY's republish raises first (a factory, or None for a confirmed
# publish). The three REPLAY arms are its three settles: the
# ack-on-confirm, the origin-gone LEAVE, and the publish-failed LEAVE, whose
# publish error REPLAY swallows before the nack.
_SETTLE_SURFACES = {
    "ack": ("durable", "ack", None),
    "nack": ("durable", "nack", None),
    "discard": ("dlq", "ack", None),
    "leave": ("dlq", "nack", None),
    "replay_confirmed": ("dlq", "ack", None),
    "replay_origin_gone": ("dlq", "nack", _origin_queue_gone),
    "replay_publish_failed": ("dlq", "nack", _channel_is_closed),
}

_DLQ_ACTIONS = {
    "discard": _ACTION.ACTION_DISCARD,
    "leave": _ACTION.ACTION_LEAVE,
    "replay_confirmed": _ACTION.ACTION_REPLAY,
    "replay_origin_gone": _ACTION.ACTION_REPLAY,
    "replay_publish_failed": _ACTION.ACTION_REPLAY,
}

_ALL_SETTLE_SURFACES = pytest.mark.parametrize("surface", list(_SETTLE_SURFACES))


class _PendingReadRig:
    """
    A settle failing while the REAL consume loop waits on its next delivery.

    Runs the real servicer RPC (``ConsumeDurableEventsWithAcks`` or
    ``ManageDeadLetteredEvents``), the real ``consume_with_acks`` / ``browse``
    generator and its consume loop, the real ``merge`` and the real
    ``AckTracker``. Only the broker edge is doubled: the pool channel, the
    queue (:class:`PumpedQueue`) and the deliveries.

    The sequence is the one the channel-dead mark exists for. Three
    deliveries reach the client, so three are in flight; the loop goes back
    to waiting on the empty pump queue, which :class:`_PendingReadProbe`
    confirms; and only then does the client settle delivery 1, whose broker
    call raises what ``make_failure`` builds. The merge then cancels the
    loop's pending read, which runs the loop's ``finally`` and schedules its
    cleanup, before the error reaches anything above the merge.

    The cleanup outcome is captured when the pool channel comes back, which
    is the cleanup's last step. Deliveries buffered but not yet read cannot
    exist here -- the read is pending only because the pump queue is empty --
    so the buffered half of the cleanup is observed as its call to drain the
    pump queue (``_nack_prefetched_messages``, wrapped so it still runs)
    rather than as a nack on any delivery.
    """

    def __init__(self, mocker, surface, make_failure):
        self.kind, self.verb, make_publish_failure = _SETTLE_SURFACES[surface]
        self.surface = surface
        self.failure = make_failure()
        queue_name = (
            f"event-queue:{_DEAD_CHANNEL_QUEUE_ID}"
            if self.kind == "durable"
            else settings.settings.dead_letter_queue_name
        )
        self.queue = PumpedQueue(queue_name)
        self.probe = _PendingReadProbe()
        self.deliveries = {}
        self.read_pending_at_failure = []
        self.responses = []
        self.client_error = None
        self._events_received = asyncio.Event()
        self._settle_go = asyncio.Event()
        self._released = asyncio.Event()

        channel = MagicMock()
        channel.set_qos = AsyncMock()
        publish_failure = make_publish_failure() if make_publish_failure else None
        channel.default_exchange.publish = AsyncMock(side_effect=publish_failure)
        mocker.patch.object(
            rabbitmq_connector,
            "get_channel_direct",
            new_callable=AsyncMock,
            return_value=channel,
        )

        async def _release(_channel):
            self._released.set()

        self.return_channel = mocker.patch.object(
            rabbitmq_connector,
            "return_channel",
            new_callable=AsyncMock,
            side_effect=_release,
        )
        mocker.patch.object(
            DurableQueueConsumer,
            "get_queue",
            new_callable=AsyncMock,
            return_value=self.queue,
        )
        mocker.patch.object(
            DeadLetterConsumer,
            "_get_dead_letter_queue",
            new_callable=AsyncMock,
            return_value=self.queue,
        )
        self.nack_prefetched = mocker.patch.object(
            RabbitMQBase,
            "_nack_prefetched_messages",
            new_callable=AsyncMock,
            wraps=RabbitMQBase._nack_prefetched_messages,
        )
        # The idle liveness poll must not fire: the point is that the settle
        # side detects the death before the event side's poll does.
        mocker.patch.object(
            RabbitMQBase,
            "_liveness_poll_timeout",
            return_value=IDLE_POLL_NEVER_FIRES_SECONDS,
        )

        next_message = RabbitMQBase.__dict__["_next_message"].__func__
        probe = self.probe

        async def _observed_next_message(cls, messages_queue, *args, **kwargs):
            probe.instrument(messages_queue)
            return await next_message(cls, messages_queue, *args, **kwargs)

        mocker.patch.object(
            RabbitMQBase, "_next_message", classmethod(_observed_next_message)
        )

    @property
    def in_flight(self):
        return [self.deliveries[tag] for tag in _IN_FLIGHT_TAGS]

    def _delivery(self, tag):
        message = AsyncMock(spec=aio_pika.abc.AbstractIncomingMessage)
        message.delivery_tag = tag
        message.body = _PENDING_READ_EVENT_BODY
        message.headers = _pending_read_headers()
        if tag == _SETTLED_TAG:

            def _settle_fails(*_args, **_kwargs):
                self.read_pending_at_failure.append(self.probe.pending)
                raise self.failure

            getattr(message, self.verb).side_effect = _settle_fails
        self.deliveries[tag] = message
        return message

    async def _durable_requests(self):
        yield ConsumeDurableEventsWithAcksRequest(queue_id=_DEAD_CHANNEL_QUEUE_ID)
        await self._settle_go.wait()
        if self.verb == "ack":
            yield ConsumeDurableEventsWithAcksRequest(ack_delivery_tags=[_SETTLED_TAG])
        else:
            yield ConsumeDurableEventsWithAcksRequest(nack_delivery_tags=[_SETTLED_TAG])
        # The client keeps its stream open; only the failure ends it.
        await asyncio.Event().wait()

    async def _dlq_requests(self):
        await self._settle_go.wait()
        yield ManageDeadLetteredEventsRequest(
            actions=[
                ManageDeadLetteredEventsRequest.ActionEntry(
                    delivery_tag=_SETTLED_TAG, action=_DLQ_ACTIONS[self.surface]
                )
            ]
        )
        await asyncio.Event().wait()

    async def _client(self, servicer, context):
        if self.kind == "durable":
            stream = servicer.ConsumeDurableEventsWithAcks(
                self._durable_requests(), context
            )
        else:
            stream = servicer.ManageDeadLetteredEvents(self._dlq_requests(), context)
        try:
            async for response in stream:
                self.responses.append(response)
                if len(self.responses) == 1 + len(_IN_FLIGHT_TAGS):
                    self._events_received.set()
        except Exception as exc:
            self.client_error = exc

    async def run(self, servicer, context):
        client = asyncio.ensure_future(self._client(servicer, context))
        await _within_deadline(
            self.queue.consuming.wait(), "the consume loop did not start consuming"
        )
        for tag in (_SETTLED_TAG, *_IN_FLIGHT_TAGS):
            await self.queue.deliver(self._delivery(tag))
        await _within_deadline(
            self._events_received.wait(),
            "the client did not receive every delivery",
        )
        await _within_deadline(
            self.probe.parked.wait(),
            "the consume loop did not go back to waiting on its next delivery",
        )
        self._settle_go.set()
        await _within_deadline(client, "the client stream did not end")
        await _within_deadline(
            self._released.wait(),
            "the consume loop's cleanup did not release the pool channel",
        )
        return self


@pytest.fixture
def run_pending_read_rig(servicer, mock_context, mocker):
    async def _run(surface, make_failure):
        rig = _PendingReadRig(mocker, surface, make_failure)
        return await rig.run(servicer, mock_context)

    return _run


class TestASettleSideChannelDeathNarrowsTheEventSideCleanup:
    """
    A settle that finds the channel dead narrows the cleanup of a consume
    loop that is still waiting on its next delivery (issue b.xr7).

    This is the order production reaches on the flagship ack-timeout kill:
    the client settles while the stream is idle, so the settle fails first
    and the consume loop is parked in ``_next_message``. The merge tears the
    loop down by cancelling that pending read, and the loop's cleanup is
    scheduled before the error reaches the servicer's
    ``_dead_channel_mapped``. The cleanup reads the verdict off the session
    tracker, so it narrows only if the tracker was marked at the failing
    settle itself. Unmarked, it takes the healthy-channel path: it sends a
    ``basic.cancel`` for a consumer the broker already dropped, nacks the
    in-flight deliveries and drains the buffered ones, whose tags all died
    with the channel.

    Nothing else marks the tracker -- ``_dead_channel_mapped`` only
    classifies the error for the client -- so the no-cancel, no-in-flight-
    nack and no-drain assertions each fail without the mark at the settle.
    ``AckTracker.nack_all`` re-reads the flag itself, but the drain of
    buffered deliveries runs on the cleanup's single read.
    """

    _make_failure = staticmethod(_channel_is_closed)

    @pytest.mark.asyncio
    @_ALL_SETTLE_SURFACES
    async def test_the_event_read_is_pending_when_the_settle_fails(
        self, run_pending_read_rig, surface
    ):
        # The precondition every other assertion here depends on. Without a
        # pending read there is nothing for the merge to cancel, so the
        # consume loop's cleanup would not race the error to the mark at all.
        rig = await run_pending_read_rig(surface, self._make_failure)

        assert rig.read_pending_at_failure == [True]

    @pytest.mark.asyncio
    @_ALL_SETTLE_SURFACES
    async def test_the_cleanup_sends_no_consumer_cancel(
        self, run_pending_read_rig, surface
    ):
        rig = await run_pending_read_rig(surface, self._make_failure)

        rig.queue.cancel.assert_not_awaited()

    @pytest.mark.asyncio
    @_ALL_SETTLE_SURFACES
    async def test_the_cleanup_nacks_no_in_flight_delivery(
        self, run_pending_read_rig, surface
    ):
        # The broker requeued these when it closed the channel, and their
        # delivery tags died with it.
        rig = await run_pending_read_rig(surface, self._make_failure)

        assert [message.nack.await_count for message in rig.in_flight] == [0, 0]

    @pytest.mark.asyncio
    @_ALL_SETTLE_SURFACES
    async def test_the_cleanup_drains_no_prefetched_delivery(
        self, run_pending_read_rig, surface
    ):
        # Asserted on the call, not on a delivery: the pump queue is empty
        # whenever the read is pending. Unlike nack_all, the drain has no
        # check of its own, so only the cleanup's read of the verdict skips
        # it.
        rig = await run_pending_read_rig(surface, self._make_failure)

        rig.nack_prefetched.assert_not_awaited()

    @pytest.mark.asyncio
    @_ALL_SETTLE_SURFACES
    async def test_the_pool_channel_is_released_exactly_once(
        self, run_pending_read_rig, surface
    ):
        # The narrowing must not also skip on_cleanup_complete, or every
        # incident would leak a pool channel.
        rig = await run_pending_read_rig(surface, self._make_failure)

        rig.return_channel.assert_awaited_once()

    @pytest.mark.asyncio
    @_ALL_SETTLE_SURFACES
    async def test_the_client_receives_consumer_cancelled(
        self, run_pending_read_rig, surface
    ):
        rig = await run_pending_read_rig(surface, self._make_failure)

        assert isinstance(rig.client_error, ConsumerCancelledError)


class TestAnUnrelatedSettleFailureKeepsTheEventSideCleanupWhole:
    """
    The control arm for
    :class:`TestASettleSideChannelDeathNarrowsTheEventSideCleanup`: the same
    rig, with a settle failure that says nothing about the channel.

    The channel is still alive here, so the cleanup owes the broker the
    cancel, the nacks and the drain of the buffered deliveries. These pin
    that the rig's cleanup is live -- that the assertions above observe a
    narrowing, not a cleanup that never ran or a rig that cannot see a
    cancel, a nack or a drain.
    """

    _make_failure = staticmethod(_genuine_internal_bug)

    @pytest.mark.asyncio
    @_ALL_SETTLE_SURFACES
    async def test_the_event_read_is_pending_when_the_settle_fails(
        self, run_pending_read_rig, surface
    ):
        rig = await run_pending_read_rig(surface, self._make_failure)

        assert rig.read_pending_at_failure == [True]

    @pytest.mark.asyncio
    @_ALL_SETTLE_SURFACES
    async def test_the_cleanup_cancels_the_consumer(
        self, run_pending_read_rig, surface
    ):
        rig = await run_pending_read_rig(surface, self._make_failure)

        rig.queue.cancel.assert_awaited_once_with(PUMP_CONSUMER_TAG)

    @pytest.mark.asyncio
    @_ALL_SETTLE_SURFACES
    async def test_the_cleanup_nacks_every_in_flight_delivery(
        self, run_pending_read_rig, surface
    ):
        rig = await run_pending_read_rig(surface, self._make_failure)

        assert [message.nack.await_count for message in rig.in_flight] == [1, 1]

    @pytest.mark.asyncio
    @_ALL_SETTLE_SURFACES
    async def test_the_cleanup_drains_the_prefetched_deliveries(
        self, run_pending_read_rig, surface
    ):
        rig = await run_pending_read_rig(surface, self._make_failure)

        rig.nack_prefetched.assert_awaited_once()

    @pytest.mark.asyncio
    @_ALL_SETTLE_SURFACES
    async def test_the_pool_channel_is_released_exactly_once(
        self, run_pending_read_rig, surface
    ):
        rig = await run_pending_read_rig(surface, self._make_failure)

        rig.return_channel.assert_awaited_once()

    @pytest.mark.asyncio
    @_ALL_SETTLE_SURFACES
    async def test_the_client_receives_the_failure_unmapped(
        self, run_pending_read_rig, surface
    ):
        rig = await run_pending_read_rig(surface, self._make_failure)

        assert rig.client_error is rig.failure


# The non-durable queue the own-operation rig consumes from. Its broker
# name embeds it, as production's does.
_OWN_OPERATION_NON_DURABLE_QUEUE_ID = "non-durable-whose-channel-dies"


async def _durable_acks_stream_requests():
    """Open a durable-with-acks stream and keep it open, settling nothing."""
    yield ConsumeDurableEventsWithAcksRequest(queue_id=OWN_OPERATION_QUEUE_ID)
    await asyncio.Event().wait()


async def _non_durable_stream_requests():
    """Open a non-durable stream and keep it open, updating nothing."""
    yield ConsumeNonDurableEventsRequest(
        filter_groups=[_proto_filter_group("asset.updated")]
    )
    await asyncio.Event().wait()


def _open_consume_durable_events_stream(servicer, context, _rig, _mocker):
    return servicer.ConsumeDurableEvents(
        ConsumeDurableEventsRequest(queue_id=OWN_OPERATION_QUEUE_ID), context
    )


def _open_consume_durable_events_with_acks_stream(servicer, context, _rig, _mocker):
    return servicer.ConsumeDurableEventsWithAcks(
        _durable_acks_stream_requests(), context
    )


def _open_consume_non_durable_events_stream(servicer, context, rig, mocker):
    # A REAL consumer over the rig's queue, so its consume() runs the real
    # delayed-ack loop and its cleanup_channel() the real release.
    consumer = NonDurableQueueConsumer(
        rig.queue, _OWN_OPERATION_NON_DURABLE_QUEUE_ID, MagicMock()
    )
    mocker.patch.object(NonDurableQueueConsumer, "get_instance", return_value=consumer)
    return servicer.ConsumeNonDurableEvents(_non_durable_stream_requests(), context)


# Each consume RPC, with the broker name of the queue its stream reads.
_OWN_OPERATION_SURFACES = {
    "ConsumeDurableEvents": (
        _open_consume_durable_events_stream,
        f"event-queue:{OWN_OPERATION_QUEUE_ID}",
    ),
    "ConsumeDurableEventsWithAcks": (
        _open_consume_durable_events_with_acks_stream,
        f"event-queue:{OWN_OPERATION_QUEUE_ID}",
    ),
    "ConsumeNonDurableEvents": (
        _open_consume_non_durable_events_stream,
        f"event-queue:{_OWN_OPERATION_NON_DURABLE_QUEUE_ID}:nd",
    ),
}


async def _error_the_client_receives(stream):
    """
    Read ``stream`` to its end and return the error that ended it, or None.

    Captured rather than raised so the rig can still wait for the consume
    loop's cleanup before the test asserts on it.
    """
    try:
        async for _response in stream:
            pass
    except Exception as exc:  # what the exception interceptor would send
        return exc
    return None


@pytest.fixture(params=list(_OWN_OPERATION_SURFACES))
def run_own_operation_rig(request, servicer, mock_context, mocker):
    async def _run():
        open_stream, queue_name = _OWN_OPERATION_SURFACES[request.param]
        rig = OwnOperationDeadChannelRig(mocker, queue_name)
        stream = open_stream(servicer, mock_context, rig, mocker)
        return await rig.run(_error_the_client_receives(stream))

    return _run


class TestAConsumeLoopOwnOperationOnADeadChannelThroughTheServicer:
    """
    A consume loop whose own skip-ack finds the channel dead ends every
    consume RPC with the reconnect guidance and a narrowed cleanup
    (issue b.7yv).

    The rig (``tests/fixtures/own_operation_dead_channel.py``) runs the real
    RPC, consumer and consume loop: two deliveries reach the client and stay
    outstanding, then the loop acks a delivery the authorization check
    denies, against a channel the broker has just killed, with a fourth
    buffered behind it. The cleanup must not send a ``basic.cancel`` for a
    consumer the broker already dropped, nor nack deliveries it already
    requeued, and must still release the channel.

    The no-cancel assertion is what tells the fix apart on all three RPCs.
    The client-text assertion does so only on ``ConsumeDurableEvents``: on
    the two merge-wrapped streams the servicer's ``_dead_channel_mapped``
    already turned the raw channel error into the same text.
    """

    @pytest.mark.asyncio
    async def test_the_client_receives_the_reconnect_guidance(
        self, run_own_operation_rig
    ):
        result = await run_own_operation_rig()

        assert ErrorDetails(result.seen).message == RECONNECT_GUIDANCE

    @pytest.mark.asyncio
    async def test_no_consumer_cancel_is_sent(self, run_own_operation_rig):
        result = await run_own_operation_rig()

        result.cancel.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_no_outstanding_delivery_is_nacked(self, run_own_operation_rig):
        result = await run_own_operation_rig()

        assert result.outstanding_nacks == [0, 0]

    @pytest.mark.asyncio
    async def test_no_buffered_delivery_is_nacked(self, run_own_operation_rig):
        result = await run_own_operation_rig()

        assert result.buffered_nacks == 0

    @pytest.mark.asyncio
    async def test_the_pool_channel_is_released_exactly_once(
        self, run_own_operation_rig
    ):
        result = await run_own_operation_rig()

        result.return_channel.assert_awaited_once()


class TestConsumeNonDurableEventsEmptyStream:

    @pytest.mark.asyncio
    async def test_returns_cleanly_when_client_sends_no_messages(
        self, servicer, mock_context, mocker
    ):
        # ConsumeNonDurableEvents admits on the non-durable pool
        # (raise_if_at_capacity() with the default durable_pool=False).
        # Same note as the durable case above: redundant with the
        # autouse before_each, kept to name the admission call this
        # test must clear.
        mocker.patch.object(capacity_guard, "raise_if_at_capacity")

        responses = [
            r
            async for r in servicer.ConsumeNonDurableEvents(
                _empty_async_iterator(), mock_context
            )
        ]

        assert responses == []


class TestConsumeNonDurableEventsPassesClassificationToGuard:
    """
    The classification actually reaches admission.

    _is_proxied could be perfectly correct and never consulted, which
    would silently disable the whole two-tier admission scheme without
    failing any test in TestIsProxied. These tests drive the real RPC and
    assert on the arguments of the patched guard call at
    ``event_consumer_servicer.py`` -- the same
    ``assert_called_once_with`` pattern the integration suite uses in
    ``tests/integration_tests/grpc/test_capacity_guard.py``. The empty
    request iterator is enough: admission is the first statement in the
    method, ahead of the first ``anext``.
    """

    @staticmethod
    async def _drive(servicer, context):
        return [
            r
            async for r in servicer.ConsumeNonDurableEvents(
                _empty_async_iterator(), context
            )
        ]

    @pytest.mark.asyncio
    async def test_stamped_request_admits_as_re_routable(
        self, servicer, mock_context, mocker
    ):
        guard = mocker.patch.object(capacity_guard, "raise_if_at_capacity")
        mock_context.invocation_metadata.return_value = [
            ("x-via-proxy", "true"),
        ]

        await self._drive(servicer, mock_context)

        guard.assert_called_once_with(re_routable=True)

    @pytest.mark.asyncio
    async def test_unstamped_request_admits_as_pinned(
        self, servicer, mock_context, mocker
    ):
        guard = mocker.patch.object(capacity_guard, "raise_if_at_capacity")

        await self._drive(servicer, mock_context)

        guard.assert_called_once_with(re_routable=False)

    @pytest.mark.asyncio
    async def test_stamped_and_unstamped_requests_differ_in_classification(
        self, servicer, mock_context, mocker
    ):
        # Both directions in one test, so the pin survives even if the two
        # tests above are read as asserting an unchanging constant: the
        # argument has to track the request, not just happen to match.
        guard = mocker.patch.object(capacity_guard, "raise_if_at_capacity")

        mock_context.invocation_metadata.return_value = [
            ("x-via-proxy", "true"),
        ]
        await self._drive(servicer, mock_context)

        mock_context.invocation_metadata.return_value = []
        await self._drive(servicer, mock_context)

        assert [call.kwargs["re_routable"] for call in guard.call_args_list] == [
            True,
            False,
        ]


_NON_DURABLE_QUEUE_ID = "non-durable-with-a-dead-channel"

# The broker spelling of a non-durable queue: the durable spelling plus
# the `:nd` suffix _queue_name appends. This is what the servicer hands to
# ConsumerCancelledError -- which redacts it, because the id inside it is
# the client's reconnect token.
_NON_DURABLE_BROKER_QUEUE_NAME = f"event-queue:{_NON_DURABLE_QUEUE_ID}:nd"


async def _non_durable_events(failure, events=()):
    """
    The event side of the non-durable merge.

    With ``failure=None`` and no ``events`` it ends immediately without
    yielding, so the filter-update side is the source of the exception --
    the real race the mapping exists for, where a rebind fails on the dead
    channel while the event side is still idle, before its next liveness
    poll. With a ``failure`` it raises instead, standing in for the
    liveness poll winning that race and producing its own typed error.

    ``events`` are ``EventMessage`` objects yielded first, in order, for
    the tests that need a real event response off this half of the merge.
    """
    for event in events:
        yield event

    if failure is not None:
        raise failure


def _filter_update_rig(
    mocker,
    update_failure=None,
    event_failure=None,
    update_result=True,
    connect_time_update_changed=None,
    events=(),
):
    """
    Patch ``NonDurableQueueConsumer.get_instance`` so the stream runs
    against a consumer whose ``update_queue`` fails with
    ``update_failure`` and whose ``consume`` fails with ``event_failure``.

    A REAL ``NonDurableQueueConsumer`` is used so ``consumer.queue.name``
    -- the broker name the servicer reads for the error -- is reached the
    way production reaches it, and so ``cleanup_channel`` stays the real
    method (a no-op here: the suite's connector holds no pools). Only the
    three broker-touching members are replaced. Passing neither failure
    gives the healthy control arm.

    ``update_result`` is what the stand-in ``update_queue`` returns when it
    does not fail -- the "did this update change the queue's bindings?"
    verdict the servicer turns into ``filter_update_ack.unchanged``. It is
    ignored when ``update_failure`` is given, because a ``side_effect``
    that raises never reaches the return value. Pass a list to give a
    different verdict to each successive update.

    ``connect_time_update_changed`` is what ``get_instance`` would have
    recorded for the connect the stream arrived with, and has to be paired
    with the request stream the test drives, because production derives it
    from that request's shape:

    * ``True`` -- a fresh connect (``get_instance`` created the queue and
      bound the set the request named), or a reconnect whose connect-time
      update rebound the queue.
    * ``False`` -- a reconnect whose connect-time update named two equal
      sets, so no bind or unbind call was issued.
    * ``None`` -- a token-only reattach, which named no set at all. This is
      the DEFAULT because every stream in the mid-stream class below opens
      that way, which is what leaves each acknowledgement it collects
      attributable to a mid-stream update. A test that drives a fresh
      connect and reads the opening response must pass ``True``, or the
      servicer emits no opening acknowledgement and the assertion reads
      proto3 defaults off an unset submessage. Tests that assert only on an
      exception never observe the value.

    ``events`` are handed to ``_non_durable_events``.
    """
    queue = MagicMock(spec=aio_pika.abc.AbstractQueue)
    queue.name = _NON_DURABLE_BROKER_QUEUE_NAME
    consumer = NonDurableQueueConsumer(queue, _NON_DURABLE_QUEUE_ID, MagicMock())
    consumer.connect_time_update_changed = connect_time_update_changed

    mocker.patch.object(
        consumer,
        "consume",
        side_effect=lambda: _non_durable_events(event_failure, events),
    )
    if update_failure is not None:
        update_side_effect = update_failure
    elif isinstance(update_result, list):
        update_side_effect = update_result
    else:
        update_side_effect = None

    update_queue = mocker.patch.object(
        consumer,
        "update_queue",
        side_effect=update_side_effect,
        return_value=update_result,
    )
    cleanup_channel = mocker.patch.object(
        consumer, "cleanup_channel", side_effect=consumer.cleanup_channel
    )
    mocker.patch.object(NonDurableQueueConsumer, "get_instance", return_value=consumer)
    return update_queue, cleanup_channel


#: The resource id every filter group built below carries -- a client's
#: folder path, which is exactly what the acknowledgement log line must not
#: write down. Shared by the factory and the log-hygiene assertion that
#: looks for it, so the assertion cannot go on passing against a literal the
#: factory no longer produces.
_FILTER_RESOURCE_ID = "/a/b/c/"


def _proto_filter_group(event_type):
    return ProtoFilterGroup(
        event_type=event_type,
        filters=[
            ProtoResourceFilter(
                filter_type=ProtoResourceFilter.FilterType.FILTER_TYPE_EQ,
                resource_id=_FILTER_RESOURCE_ID,
            ),
        ],
    )


async def _filter_update_requests():
    """
    A client stream that opens a non-durable stream then updates its
    filters.

    The second request CHANGES the filters, so the rebind under test is
    really issued. ``_listen_for_filter_updates`` forwards a request whose
    new groups equal its previous ones too -- the "nothing to do" verdict
    belongs to ``update_queue`` -- but such a request reaches the real
    ``update_queue`` only to be turned away by it, which is not the path
    these tests drive.
    """
    yield ConsumeNonDurableEventsRequest(filter_groups=[_proto_filter_group("e1")])
    yield ConsumeNonDurableEventsRequest(
        filter_groups=[_proto_filter_group("e2")],
        previous_filter_groups=[_proto_filter_group("e1")],
    )


class TestConsumeNonDurableEventsChannelDeath:
    """
    The non-durable stream gets the SAME classification as the two ack
    streams, because it has the same shape: an event-producing generator
    merged with a loop that issues client-driven broker operations. Here
    the operation is the filter update's rebind, which binds and unbinds
    against the consumer's channel -- so a filter update racing the
    broker's cancellation of that channel fails inside the merge, ahead of
    the event side's next idle liveness poll.

    This stream settles nothing on the client's behalf, so it has no
    ``AckTracker``, and a rebind that fails on the dead channel records no
    cleanup verdict: the wrapper is classification only, as it is on every
    surface. The typed errors the liveness path already produces must
    survive the wrapper unchanged -- reclassifying them would turn a 404
    "your queue is gone" into "reconnect and resume", sending the client
    back for a queue that no longer exists.
    """

    @staticmethod
    async def _drive(servicer, context):
        return [
            r
            async for r in servicer.ConsumeNonDurableEvents(
                _filter_update_requests(), context
            )
        ]

    @pytest.mark.asyncio
    @dead_channel_failures
    async def test_a_filter_update_on_a_dead_channel_ends_as_consumer_cancelled(
        self, servicer, mock_context, mocker, make_failure
    ):
        _filter_update_rig(mocker, update_failure=make_failure())

        with pytest.raises(ConsumerCancelledError, match="Reconnect to resume"):
            await self._drive(servicer, mock_context)

    @pytest.mark.asyncio
    async def test_a_filter_update_on_a_dead_channel_records_the_channel_as_closed(
        self, servicer, mock_context, mocker
    ):
        _filter_update_rig(
            mocker, update_failure=aio_pika.exceptions.ChannelInvalidStateError()
        )

        with pytest.raises(ConsumerCancelledError) as exc_info:
            await self._drive(servicer, mock_context)

        assert exc_info.value.channel_closed is True

    @pytest.mark.asyncio
    async def test_the_mapped_error_is_chained_to_the_rebind_failure(
        self, servicer, mock_context, mocker
    ):
        # Chaining the original channel error as __cause__ is what shows the
        # except clause ran all the way through -- an error raised inside it
        # would surface instead of the classified error, as a bare INTERNAL
        # -- and it keeps the broker's own reason in the logged traceback.
        failure = aio_pika.exceptions.ChannelInvalidStateError()
        _filter_update_rig(mocker, update_failure=failure)

        with pytest.raises(ConsumerCancelledError) as exc_info:
            await self._drive(servicer, mock_context)

        assert exc_info.value.__cause__ is failure

    @pytest.mark.asyncio
    async def test_the_error_names_the_broker_queue_in_redacted_form(
        self, servicer, mock_context, mocker
    ):
        # The name goes to the log-only ``internal_message``, and a
        # non-durable broker name embeds the stream's reconnect token, so
        # what is written down is the redacted spelling: the surrounding
        # shape -- prefix and `:nd` suffix -- intact, with the id replaced
        # by a stable reference an operator can still correlate on. The
        # client still gets the generic guidance.
        _filter_update_rig(
            mocker, update_failure=aio_pika.exceptions.ChannelInvalidStateError()
        )

        with pytest.raises(ConsumerCancelledError) as exc_info:
            await self._drive(servicer, mock_context)

        assert re.search(
            r"queue_name: event-queue:sha256-[0-9a-f]{12}:nd", str(exc_info.value)
        )

    @pytest.mark.asyncio
    async def test_the_error_does_not_name_the_reconnect_token(
        self, servicer, mock_context, mocker
    ):
        # The queue_id IS the client's reconnect token -- the queue's
        # identifier -- and this message is re-logged by the exception
        # interceptor on every stream the broker cancels. Pinned separately
        # because the shape assertion above still passes for a message that
        # ALSO carries the raw token somewhere else in it.
        _filter_update_rig(
            mocker, update_failure=aio_pika.exceptions.ChannelInvalidStateError()
        )

        with pytest.raises(ConsumerCancelledError) as exc_info:
            await self._drive(servicer, mock_context)

        assert _NON_DURABLE_QUEUE_ID not in str(exc_info.value)

    @pytest.mark.asyncio
    async def test_an_unrelated_filter_update_failure_is_not_mapped(
        self, servicer, mock_context, mocker
    ):
        # The other half of "only the dead-channel classes are mapped": a
        # genuine internal bug must still surface as itself, or every
        # server-side defect on this RPC would be reported to clients as
        # "reconnect and resume" and quietly retried forever.
        _filter_update_rig(mocker, update_failure=ValueError("a genuine internal bug"))

        with pytest.raises(ValueError, match="a genuine internal bug"):
            await self._drive(servicer, mock_context)

    @pytest.mark.asyncio
    async def test_a_dead_channel_from_the_event_side_is_also_mapped(
        self, servicer, mock_context, mocker
    ):
        # The wrapper sits above the WHOLE merge, not around the
        # update_queue call: a raw channel error surfacing from the event
        # side means the same thing and gets the same classification.
        # Narrowing the wrapper to the rebind alone would pass every
        # filter-update case above while leaving this one a bare INTERNAL.
        _filter_update_rig(
            mocker, event_failure=aio_pika.exceptions.ChannelInvalidStateError()
        )

        with pytest.raises(ConsumerCancelledError, match="Reconnect to resume"):
            await self._drive(servicer, mock_context)

    @pytest.mark.asyncio
    async def test_a_queue_not_found_from_the_event_side_is_not_reclassified(
        self, servicer, mock_context, mocker
    ):
        # The wrapper now spans the whole merge, so it also sits over the
        # event side -- whose liveness poll raises its OWN typed errors.
        # QueueNotFoundError means the queue is gone for good (404); mapped
        # into ConsumerCancelledError it would become INTERNAL "reconnect
        # to resume", sending the client back for a queue that will never
        # return.
        _filter_update_rig(
            mocker, event_failure=QueueNotFoundError(queue_name="gone-queue")
        )

        with pytest.raises(QueueNotFoundError):
            await self._drive(servicer, mock_context)

    @pytest.mark.asyncio
    async def test_a_consumer_cancelled_from_the_event_side_keeps_its_verdict(
        self, servicer, mock_context, mocker
    ):
        # The liveness path also raises ConsumerCancelledError directly, and
        # it decides channel_closed from what it observed at the moment of
        # the loss. Re-wrapping it here would overwrite a False verdict with
        # the wrapper's unconditional True, and the consume loop's cleanup
        # reads exactly that flag to choose its teardown path.
        _filter_update_rig(
            mocker,
            event_failure=ConsumerCancelledError(
                _NON_DURABLE_BROKER_QUEUE_NAME, channel_closed=False
            ),
        )

        with pytest.raises(ConsumerCancelledError) as exc_info:
            await self._drive(servicer, mock_context)

        assert exc_info.value.channel_closed is False

    @pytest.mark.asyncio
    async def test_a_dead_channel_still_releases_the_consumer_channel(
        self, servicer, mock_context, mocker
    ):
        # The mapping is raised from INSIDE the try whose finally returns
        # the channel to the pool. Hoisting the wrapper above that try would
        # look harmless -- it only reclassifies an error -- but would leak
        # one pool channel per dead-channel incident, on the path where the
        # pool is already under pressure.
        _, cleanup_channel = _filter_update_rig(
            mocker, update_failure=aio_pika.exceptions.ChannelInvalidStateError()
        )

        with pytest.raises(ConsumerCancelledError):
            await self._drive(servicer, mock_context)

        cleanup_channel.assert_awaited_once()

    @pytest.mark.asyncio
    async def test_a_healthy_filter_update_completes_the_stream(
        self, servicer, mock_context, mocker
    ):
        # The control arm for every case above: with a live channel the
        # rebind is issued and the stream ends normally, having sent the
        # opening reconnect-token response and the acknowledgement of the
        # one update. Without it, a wrapper that swallowed the whole merge
        # would pass every raises-based test here.
        #
        # This stream opens with a fresh connect, so the verdict is the one
        # ``get_instance`` records for one. That puts the opening
        # acknowledgement ON the opening response rather than on one of its
        # own, which is why the count below is still two.
        update_queue, _ = _filter_update_rig(mocker, connect_time_update_changed=True)

        responses = await self._drive(servicer, mock_context)

        assert update_queue.await_count == 1
        assert responses[0].reconnect_token == _NON_DURABLE_QUEUE_ID
        assert len(responses) == 2


#: The response field carrying a filter-update acknowledgement. Every
#: assertion below reaches it through ``HasField``, never by truthiness: a
#: ``FilterUpdateAck`` whose echoed set happens to be empty is
#: indistinguishable from an absent one otherwise, and "no acknowledgement"
#: and "an acknowledgement of nothing" are different claims about the queue.
_ACK_FIELD = "filter_update_ack"

#: The prefix of the INFO line the servicer writes when it acknowledges an
#: update. Named so the log assertions can tell it from the two "received a
#: request" lines the same stream emits.
_ACK_LOG_PREFIX = "filter update acknowledged:"


def _acknowledgements(responses):
    """Every response on a stream that acknowledges a filter update."""
    return [r for r in responses if r.HasField(_ACK_FIELD)]


async def _fresh_connect_request():
    """
    A client that opens a brand new stream naming a filter set, with no
    token, and sends nothing more.

    ``get_instance`` created the queue and bound that set by the time the
    stream yields, so the opening response is the only acknowledgement
    this client ever gets for it.
    """
    yield ConsumeNonDurableEventsRequest(filter_groups=[_proto_filter_group("e1")])


async def _reconnect_with_update_request(new_event_type):
    """
    A client that reattaches with its token AND names a connect-time
    filter update: both the set it wants and the set it believes the queue
    already holds.

    Naming both sets is what makes this shape admissible --
    ``get_instance`` refuses a token plus ``filter_groups`` alone (see
    ``TestConsumeNonDurableEventsHalfSpecifiedOpeningUpdate``). It applies
    the update before the stream yields anything, so the opening response
    is the only acknowledgement this client ever gets for it.

    ``new_event_type`` is the set the client asks for; the set it already
    has is always ``e1``, so passing ``"e1"`` makes this the reconnect
    that changes nothing.
    """
    yield ConsumeNonDurableEventsRequest(
        reconnect_token=RECONNECT_TOKEN,
        filter_groups=[_proto_filter_group(new_event_type)],
        previous_filter_groups=[_proto_filter_group("e1")],
    )


async def _token_only_request():
    """
    A client that reattaches with its token alone, naming no filter set.

    The service keeps no filter state and cannot read a queue's bindings
    back from the broker, so there is no set it could echo to this client
    -- which is why this stream is the one every "no acknowledgement" case
    is driven with, and why the mid-stream cases below open with it: it
    leaves the acknowledgements on the stream to the updates alone.
    """
    yield ConsumeNonDurableEventsRequest(reconnect_token=RECONNECT_TOKEN)


async def _token_only_then_update_requests():
    """A token-only reattach followed by one filter change."""
    yield ConsumeNonDurableEventsRequest(reconnect_token=RECONNECT_TOKEN)
    yield ConsumeNonDurableEventsRequest(
        filter_groups=[_proto_filter_group("e2")],
        previous_filter_groups=[_proto_filter_group("e1")],
    )


async def _token_only_then_equal_set_requests():
    """
    A token-only reattach followed by a request that re-sends the set the
    client already has.

    This is the request the listener used to drop before the servicer could
    see it. It changes no bindings, and is acknowledged anyway so the
    client learns the queue is at the set it named.
    """
    yield ConsumeNonDurableEventsRequest(reconnect_token=RECONNECT_TOKEN)
    yield ConsumeNonDurableEventsRequest(
        filter_groups=[_proto_filter_group("e1")],
        previous_filter_groups=[_proto_filter_group("e1")],
    )


async def _token_only_then_empty_request():
    """
    A token-only reattach followed by a request naming neither a new set
    nor a previous one.

    It is not an update at all: there is nothing to apply and nothing to
    echo, and an acknowledgement carrying an empty set would assert that
    the queue holds no bindings.
    """
    yield ConsumeNonDurableEventsRequest(reconnect_token=RECONNECT_TOKEN)
    yield ConsumeNonDurableEventsRequest()


async def _two_filter_update_requests():
    """A token-only reattach followed by two filter changes, e2 then e3."""
    yield ConsumeNonDurableEventsRequest(reconnect_token=RECONNECT_TOKEN)
    yield ConsumeNonDurableEventsRequest(
        filter_groups=[_proto_filter_group("e2")],
        previous_filter_groups=[_proto_filter_group("e1")],
    )
    yield ConsumeNonDurableEventsRequest(
        filter_groups=[_proto_filter_group("e3")],
        previous_filter_groups=[_proto_filter_group("e2")],
    )


async def _two_updates_naming_one_set_requests():
    """
    Two updates whose new sets are identical and whose previous sets are
    not -- the second is the no-op the first left the queue at.

    Their echoed sets are therefore identical, so the only thing that can
    tell one acknowledgement from the other is the verdict ``update_queue``
    returned for it. That is what makes this stream, and not
    ``_two_filter_update_requests``, able to catch acknowledgements
    reordered against the updates that produced them.
    """
    yield ConsumeNonDurableEventsRequest(reconnect_token=RECONNECT_TOKEN)
    yield ConsumeNonDurableEventsRequest(
        filter_groups=[_proto_filter_group("e2")],
        previous_filter_groups=[_proto_filter_group("e1")],
    )
    yield ConsumeNonDurableEventsRequest(
        filter_groups=[_proto_filter_group("e2")],
        previous_filter_groups=[_proto_filter_group("e2")],
    )


async def _drive_non_durable_stream(servicer, context, requests):
    """Drain a non-durable stream driven by ``requests``; return every
    response."""
    return [r async for r in servicer.ConsumeNonDurableEvents(requests, context)]


class TestConsumeNonDurableEventsOpeningAcknowledgement:
    """
    Issue b.yvu: the stream's first response tells the client two things
    besides its reconnect token.

    The capability flag says that THIS server acknowledges filter updates.
    A client reads it as soon as the stream opens and uses it to choose
    between the token-resume recipe and opening a fresh queue, which is
    what lets one client build talk to a fleet mid-upgrade.

    The opening acknowledgement says that the set the opening request named
    has been applied. It is the only acknowledgement that set ever gets:
    ``get_instance`` applies a connect-time update before the stream yields
    anything, so it never passes through the mid-stream loop below.

    Three connect shapes reach that yield, one request builder each:

    * ``_fresh_connect_request`` -- no token, names a set. Acknowledged,
      and never as unchanged: the queue was just created and bound.
    * ``_reconnect_with_update_request`` -- a token plus BOTH sets.
      Acknowledged, as unchanged or not according to whether the two sets
      it named differ.
    * ``_token_only_request`` -- a token alone. Not acknowledged at all,
      because the service keeps no filter state and cannot report a set it
      was not told.

    The fourth shape -- a token plus a new set without its predecessor --
    never reaches that yield at all: the half-specified-opening-update class
    below pins ``get_instance`` refusing it.
    """

    @pytest.mark.asyncio
    async def test_the_opening_response_advertises_the_capability(
        self, servicer, mock_context, mocker
    ):
        _filter_update_rig(mocker, connect_time_update_changed=True)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _fresh_connect_request()
        )

        assert responses[0].supports_filter_update_acks is True

    @pytest.mark.asyncio
    async def test_the_opening_response_acknowledges_the_first_requests_set(
        self, servicer, mock_context, mocker
    ):
        # A fresh connect: ``get_instance`` created the queue and bound the
        # set this request named, and records that as the connect-time
        # verdict.
        _filter_update_rig(mocker, connect_time_update_changed=True)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _fresh_connect_request()
        )

        assert responses[0].HasField(_ACK_FIELD)

    @pytest.mark.asyncio
    async def test_the_opening_acknowledgement_echoes_the_groups_verbatim(
        self, servicer, mock_context, mocker
    ):
        # A verbatim echo of the request's own proto groups, not a
        # round trip through the app models: the service cannot read the
        # broker's bindings back, so what it reports is what the client
        # said, and a converted copy would risk differing from it.
        _filter_update_rig(mocker, connect_time_update_changed=True)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _fresh_connect_request()
        )

        assert list(responses[0].filter_update_ack.applied_filter_groups) == [
            _proto_filter_group("e1")
        ]

    @pytest.mark.asyncio
    async def test_a_fresh_connect_is_not_acknowledged_as_unchanged(
        self, servicer, mock_context, mocker
    ):
        # A fresh connect established the set it named, so ``get_instance``
        # recorded True. ``unchanged`` is a claim that a rebind was
        # unnecessary, and telling a client its brand new queue was left
        # unchanged would invite it to skip the bindings it just asked for.
        #
        # Read through ``_acknowledgements`` rather than off
        # ``responses[0]`` directly: an unset ``filter_update_ack``
        # sub-message reads ``unchanged`` as proto3's False, so a servicer
        # that emitted no opening acknowledgement at all would satisfy this
        # assertion. Indexing the acknowledgements makes that regression an
        # error here instead of a pass.
        _filter_update_rig(mocker, connect_time_update_changed=True)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _fresh_connect_request()
        )

        assert _acknowledgements(responses)[0].filter_update_ack.unchanged is False

    @pytest.mark.asyncio
    async def test_a_connect_time_update_that_changed_nothing_is_marked_unchanged(
        self, servicer, mock_context, mocker
    ):
        # The reconnect where the client re-sent the set it already had --
        # the same set on both sides of the request, which is why the
        # verdict paired with it is the False ``get_instance`` records when
        # it issues no bind or unbind call. The verdict is read back from
        # what ``update_queue`` reported, so the "filters are unchanged"
        # predicate stays in the one place its docstring says it lives.
        #
        # The rebound arm of this shape is not repeated here: its opening
        # response is indistinguishable from the fresh connect's above, and
        # the two arms are told apart over the wire by
        # ``tests/integration_tests/grpc/test_consume_non_durable_events.py::
        # test_the_opening_acknowledgement_reports_the_connect_time_verdict``.
        _filter_update_rig(mocker, connect_time_update_changed=False)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _reconnect_with_update_request("e1")
        )

        assert responses[0].filter_update_ack.unchanged is True

    @pytest.mark.asyncio
    async def test_a_token_only_reattach_carries_no_acknowledgement(
        self, servicer, mock_context, mocker
    ):
        # An echo here would be a claim the service is not entitled to
        # make: it was told no set, and AMQP offers no way to read the
        # queue's bindings back. The verdict is spelled out rather than left
        # to the rig's default, because it is the whole subject of the test:
        # None is what ``get_instance`` records for a reattach that named no
        # set.
        _filter_update_rig(mocker, connect_time_update_changed=None)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _token_only_request()
        )

        assert not responses[0].HasField(_ACK_FIELD)

    @pytest.mark.asyncio
    async def test_a_token_only_reattach_still_advertises_the_capability(
        self, servicer, mock_context, mocker
    ):
        # The flag describes the server, not the response. Dropping it
        # alongside the acknowledgement would tell exactly the clients that
        # resumed by token -- the ones the recipe is for -- that this
        # server does not support it.
        _filter_update_rig(mocker, connect_time_update_changed=None)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _token_only_request()
        )

        assert responses[0].supports_filter_update_acks is True


async def _half_specified_opening_update_request():
    """
    A reconnecting client whose opening request names a new filter set but
    not the previous one it is replacing.

    This is the shape that sits between the two acknowledged ones: it is
    neither the token-only reattach (which names nothing) nor a complete
    connect-time update (which names both sets). ``get_instance`` rejects
    it, so it never reaches the stream's first yield.
    """
    yield ConsumeNonDurableEventsRequest(
        reconnect_token=RECONNECT_TOKEN,
        filter_groups=[_proto_filter_group("e2")],
    )


class TestConsumeNonDurableEventsHalfSpecifiedOpeningUpdate:
    """
    Issue b.yvu: an opening request naming a new set without its predecessor
    is refused, not acknowledged.

    The three connect shapes above all rest on this: the
    servicer gates its opening acknowledgement on the recorded verdict being
    non-None, and reads ``first_request.filter_groups`` as the set to echo.
    Were this shape admitted, it would arrive at that yield as a token-only
    reattach whose verdict is None but whose request DOES name filter groups
    -- so a servicer that ever gated on the request's shape instead would
    echo a set that no bind call was made for.

    Driven against the REAL ``get_instance`` -- no rig -- because the guard
    being pinned is its own, and the refusal must come from the code the
    servicer actually calls. The consumer-level tests pin the same guard at
    the source; this is what says the gRPC surface surfaces it rather than
    swallowing it into a stream that opens anyway. The mid-stream half of
    this shape is pinned over the wire by
    ``tests/integration_tests/grpc/test_consume_non_durable_events.py::
    test_validates_filter_groups_on_subsequent_requests``.
    """

    @pytest.mark.asyncio
    async def test_the_stream_ends_as_invalid_argument(self, servicer, mock_context):
        with pytest.raises(BadRequestError) as exc_info:
            await _drive_non_durable_stream(
                servicer, mock_context, _half_specified_opening_update_request()
            )

        assert (
            ErrorDetails(exc_info.value).grpc_code == grpc.StatusCode.INVALID_ARGUMENT
        )

    @pytest.mark.asyncio
    async def test_no_response_reaches_the_client(self, servicer, mock_context):
        # Including the opening one: the reconnect token and the capability
        # flag ride a response that is only sent once the consumer exists,
        # so a client that was refused is never handed a token for a stream
        # it does not have.
        responses = []

        with pytest.raises(BadRequestError):
            async for response in servicer.ConsumeNonDurableEvents(
                _half_specified_opening_update_request(), mock_context
            ):
                responses.append(response)

        assert responses == []


class TestConsumeNonDurableEventsFilterUpdateAcknowledgement:
    """
    Issue b.yvu: every mid-stream filter update the service processes is
    acknowledged, after the bind and unbind calls it required have
    returned.

    This is what restores a client's knowledge of its queue's bindings.
    Without it the only set a client can be sure of is the one it opened
    with, so a connection that drops with an update in flight leaves it
    unable to name a ``previous_filter_groups`` that is certainly right --
    and a wrong guess does not fail, it silently leaves a resource
    unbound.

    Each stream here opens with a token-only reattach, which carries no
    acknowledgement of its own, so every acknowledgement collected belongs
    to a mid-stream update.
    """

    @pytest.mark.asyncio
    async def test_an_applied_update_is_acknowledged(
        self, servicer, mock_context, mocker
    ):
        _filter_update_rig(mocker)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _token_only_then_update_requests()
        )

        assert len(_acknowledgements(responses)) == 1

    @pytest.mark.asyncio
    async def test_the_acknowledgement_echoes_the_requests_filter_groups(
        self, servicer, mock_context, mocker
    ):
        _filter_update_rig(mocker)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _token_only_then_update_requests()
        )

        ack = _acknowledgements(responses)[0].filter_update_ack
        assert list(ack.applied_filter_groups) == [_proto_filter_group("e2")]

    @pytest.mark.asyncio
    async def test_the_acknowledgement_carries_no_events(
        self, servicer, mock_context, mocker
    ):
        # Clients today infer "this is not an event message" from an empty
        # events list, and an old client must be able to ignore an
        # acknowledgement entirely.
        _filter_update_rig(mocker)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _token_only_then_update_requests()
        )

        assert list(_acknowledgements(responses)[0].events) == []

    @pytest.mark.asyncio
    async def test_the_acknowledgement_carries_the_reconnect_token(
        self, servicer, mock_context, mocker
    ):
        _filter_update_rig(mocker)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _token_only_then_update_requests()
        )

        assert _acknowledgements(responses)[0].reconnect_token == _NON_DURABLE_QUEUE_ID

    @pytest.mark.asyncio
    async def test_the_acknowledgement_advertises_the_capability(
        self, servicer, mock_context, mocker
    ):
        _filter_update_rig(mocker)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _token_only_then_update_requests()
        )

        assert _acknowledgements(responses)[0].supports_filter_update_acks is True

    @pytest.mark.asyncio
    async def test_an_update_that_rebound_the_queue_is_not_marked_unchanged(
        self, servicer, mock_context, mocker
    ):
        _filter_update_rig(mocker, update_result=True)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _token_only_then_update_requests()
        )

        assert _acknowledgements(responses)[0].filter_update_ack.unchanged is False

    @pytest.mark.asyncio
    async def test_an_equal_set_update_reaches_update_queue(
        self, servicer, mock_context, mocker
    ):
        # The listener used to drop a request whose two sets were equal, so
        # the servicer never saw it and could not acknowledge it. The drop
        # now belongs to ``update_queue`` alone, whose verdict the
        # acknowledgement reports.
        update_queue, _ = _filter_update_rig(mocker, update_result=False)

        await _drive_non_durable_stream(
            servicer, mock_context, _token_only_then_equal_set_requests()
        )

        assert update_queue.await_count == 1

    @pytest.mark.asyncio
    async def test_an_equal_set_update_is_acknowledged(
        self, servicer, mock_context, mocker
    ):
        _filter_update_rig(mocker, update_result=False)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _token_only_then_equal_set_requests()
        )

        assert len(_acknowledgements(responses)) == 1

    @pytest.mark.asyncio
    async def test_an_equal_set_update_is_marked_unchanged(
        self, servicer, mock_context, mocker
    ):
        # The point of acknowledging a no-op: a client replaying an update
        # it never saw confirmed has to be told that the queue was already
        # there, or it cannot tell a replayed no-op from a lost update.
        _filter_update_rig(mocker, update_result=False)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _token_only_then_equal_set_requests()
        )

        assert _acknowledgements(responses)[0].filter_update_ack.unchanged is True

    @pytest.mark.asyncio
    async def test_a_request_naming_no_set_is_not_acknowledged(
        self, servicer, mock_context, mocker
    ):
        _filter_update_rig(mocker)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _token_only_then_empty_request()
        )

        assert _acknowledgements(responses) == []

    @pytest.mark.asyncio
    async def test_a_request_naming_no_set_never_reaches_update_queue(
        self, servicer, mock_context, mocker
    ):
        # Still dropped by the listener, and dropped there rather than in
        # ``update_queue``: it names no sets at all, so there is nothing for
        # the rebind to compare.
        update_queue, _ = _filter_update_rig(mocker)

        await _drive_non_durable_stream(
            servicer, mock_context, _token_only_then_empty_request()
        )

        update_queue.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_a_rejected_update_is_not_acknowledged(
        self, servicer, mock_context, mocker
    ):
        # The acknowledgement is yielded after ``update_queue`` returns, so
        # an update that raises out of it cannot be acknowledged. Hoisting
        # the yield above the await would tell the client a rejected update
        # had been applied, on the one path where the stream then dies.
        _filter_update_rig(mocker, update_failure=BadRequestError("no such filter"))
        responses = []

        with pytest.raises(BadRequestError):
            async for response in servicer.ConsumeNonDurableEvents(
                _token_only_then_update_requests(), mock_context
            ):
                responses.append(response)

        assert _acknowledgements(responses) == []

    @pytest.mark.asyncio
    async def test_a_rejected_update_still_ends_the_stream_as_invalid_argument(
        self, servicer, mock_context, mocker
    ):
        _filter_update_rig(mocker, update_failure=BadRequestError("no such filter"))

        with pytest.raises(BadRequestError) as exc_info:
            await _drive_non_durable_stream(
                servicer, mock_context, _token_only_then_update_requests()
            )

        assert (
            ErrorDetails(exc_info.value).grpc_code == grpc.StatusCode.INVALID_ARGUMENT
        )

    @pytest.mark.asyncio
    async def test_the_rebind_is_awaited_before_the_acknowledgement_is_yielded(
        self, servicer, mock_context, mocker
    ):
        # The acknowledgement's whole meaning is that the bind and unbind
        # calls have returned. ``merge`` forwards from each source as soon as
        # an item is available and orders neither against the other,
        # so an acknowledgement yielded from ``_listen_for_filter_updates``
        # would routinely reach the client ahead of the rebind it claims to
        # report -- and be believed. The ``sleep(0)`` inside the rebind
        # hands the loop a scheduling point, so a servicer that yielded
        # first is caught having done so rather than winning by luck.
        observed = []
        update_queue, _ = _filter_update_rig(mocker)

        async def _record_rebind(*_args, **_kwargs):
            await asyncio.sleep(0)
            observed.append("rebind")
            return True

        update_queue.side_effect = _record_rebind

        async for response in servicer.ConsumeNonDurableEvents(
            _token_only_then_update_requests(), mock_context
        ):
            if response.HasField(_ACK_FIELD):
                observed.append("acknowledgement")

        assert observed == ["rebind", "acknowledgement"]

    @pytest.mark.asyncio
    async def test_acknowledgements_arrive_in_the_order_the_updates_were_sent(
        self, servicer, mock_context, mocker
    ):
        # The RPC comment promises this ordering, and the replay recipe
        # rests on it: a client takes the last acknowledgement it received
        # as the queue's current set and replays only what followed it. The
        # merged loop processes one update at a time, which is what makes
        # the promise true.
        _filter_update_rig(mocker)

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _two_filter_update_requests()
        )

        assert [
            list(r.filter_update_ack.applied_filter_groups)
            for r in _acknowledgements(responses)
        ] == [[_proto_filter_group("e2")], [_proto_filter_group("e3")]]

    @pytest.mark.asyncio
    async def test_two_updates_naming_one_set_are_acknowledged_in_send_order(
        self, servicer, mock_context, mocker
    ):
        # The echo cannot tell these two acknowledgements apart, so the
        # verdicts do: the first update rebinds, the second is the no-op it
        # left the queue at. Reversed, coalesced, or matched to the wrong
        # request, the verdicts come back the other way round -- which the
        # echo-ordering test above would not see.
        _filter_update_rig(mocker, update_result=[True, False])

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _two_updates_naming_one_set_requests()
        )

        assert [
            r.filter_update_ack.unchanged for r in _acknowledgements(responses)
        ] == [False, True]

    @pytest.mark.asyncio
    async def test_an_event_response_advertises_the_capability(
        self, servicer, mock_context, mocker, mock_record_success
    ):
        # proto3 has no absent bool, so a flag set on the opening response
        # alone has this server telling the client
        # ``supports_filter_update_acks=false`` on every event response that
        # follows -- and a client that read the flag off any message but the
        # first would conclude the capability was gone.
        _filter_update_rig(mocker, events=[EventMessage({"event_type": "e1"})])

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _token_only_request()
        )

        event_responses = [r for r in responses if r.events]
        assert event_responses[0].supports_filter_update_acks is True

    @pytest.mark.asyncio
    async def test_an_event_response_carries_no_acknowledgement(
        self, servicer, mock_context, mocker, mock_record_success
    ):
        # The acknowledgement is present only on a response that
        # acknowledges something, so a client can partition the stream by
        # that field alone.
        _filter_update_rig(mocker, events=[EventMessage({"event_type": "e1"})])

        responses = await _drive_non_durable_stream(
            servicer, mock_context, _token_only_request()
        )

        event_responses = [r for r in responses if r.events]
        assert not event_responses[0].HasField(_ACK_FIELD)


class TestFilterUpdateAcknowledgementLogging:
    """
    The line written when an update is acknowledged carries counts and the
    redacted queue reference, and nothing else.

    A client can drive this line arbitrarily often -- one per filter change
    for the life of the stream -- so anything sensitive in it is written
    down repeatedly. The filter groups name the client's resources and
    ``queue_id`` IS the reconnect token, so neither appears; what remains
    has to still be enough for an operator to follow one stream's filter
    changes through the logs.
    """

    @pytest.fixture
    def acknowledgement_records(self, servicer, mock_context, mocker, caplog):
        async def _run(**rig_kwargs):
            _filter_update_rig(mocker, **rig_kwargs)

            with caplog.at_level(logging.INFO):
                await _drive_non_durable_stream(
                    servicer, mock_context, _token_only_then_update_requests()
                )

            return [
                r for r in caplog.records if r.getMessage().startswith(_ACK_LOG_PREFIX)
            ]

        return _run

    @pytest.mark.asyncio
    async def test_the_line_is_emitted(self, acknowledgement_records):
        # Guards every assertion below: a stream that never acknowledged
        # anything would make them all vacuous.
        records = await acknowledgement_records()

        assert records != []

    @pytest.mark.asyncio
    async def test_the_line_names_the_redacted_queue_reference(
        self, acknowledgement_records
    ):
        records = await acknowledgement_records()

        expected = f"queue_ref={redact_reconnect_token(_NON_DURABLE_QUEUE_ID)}"
        assert all(expected in r.getMessage() for r in records)

    @pytest.mark.asyncio
    async def test_the_line_does_not_name_the_reconnect_token(
        self, acknowledgement_records
    ):
        # ``record.__dict__`` covers the rendered message, the interpolation
        # args, and anything passed through ``extra`` -- all three reach the
        # log aggregator.
        records = await acknowledgement_records()

        assert [r for r in records if _NON_DURABLE_QUEUE_ID in str(r.__dict__)] == []

    @pytest.mark.asyncio
    async def test_the_line_counts_the_filter_groups_rather_than_naming_them(
        self, acknowledgement_records
    ):
        # The groups carry resource ids -- the client's folder paths. A
        # count answers the operator's question ("did this stream's filters
        # churn?") without putting them in the log.
        records = await acknowledgement_records()

        assert "filter_groups=1" in records[0].getMessage()

    @pytest.mark.asyncio
    async def test_the_line_does_not_name_a_resource(self, acknowledgement_records):
        # The failure mode: interpolating the groups themselves renders
        # protobuf text format, which prints every resource id in them.
        records = await acknowledgement_records()

        assert all(_FILTER_RESOURCE_ID not in r.getMessage() for r in records)

    @pytest.mark.asyncio
    async def test_the_line_reports_an_applied_update_as_changed(
        self, acknowledgement_records
    ):
        records = await acknowledgement_records(update_result=True)

        assert "unchanged=False" in records[0].getMessage()

    @pytest.mark.asyncio
    async def test_the_line_reports_a_no_op_update_as_unchanged(
        self, acknowledgement_records
    ):
        records = await acknowledgement_records(update_result=False)

        assert "unchanged=True" in records[0].getMessage()


_MID_STREAM_QUEUE_ID = "mid-stream-filter-update"

# The broker spelling `get_instance` derives the stream's cache scope from:
# it uses `queue.name`, not the queue id.
_MID_STREAM_BROKER_QUEUE_NAME = f"event-queue:{_MID_STREAM_QUEUE_ID}:nd"

# What the writer is actually registered under and the ContextVar is bound
# to: the scope key, which pairs that broker name with the presenting
# principal's reference (the autouse `mock_principal_id` fixture's).
_MID_STREAM_SCOPE_KEY = f"{_MID_STREAM_BROKER_QUEUE_NAME}:{TEST_PRINCIPAL_REF}"

# The failure the raising arm injects: an ordinary server-side bug, which
# `_dead_channel_mapped` deliberately leaves unmapped, so the stream dies
# with this exact error rather than a reclassified one.
_MID_STREAM_UPDATE_BUG = "a genuine bug inside the mid-stream update"


class _MidStreamUpdateCapture:
    """
    What the mid-stream ``update_queue`` saw when it ran.

    ``active`` is what a storage-permissions mutator would have found by
    calling ``active_writer()`` from inside that call; ``consumers_own`` is
    the writer the stream's consumer holds, so the two can be compared for
    identity rather than merely for presence.

    Both start as ``None``, which is also what they read if the update
    never ran at all -- so every assertion made against them has to carry
    an ``is not None`` of its own to mean anything.
    """

    def __init__(self):
        self.active = None
        self.consumers_own = None


def _mid_stream_update_queue(capture, failure=None):
    """
    Build the stand-in for the mid-stream ``update_queue``.

    It records into ``capture`` what a storage-permissions mutator running
    inside the update would have found, then raises ``failure`` if one was
    given -- the error arm, where the binding still has to be released on
    the way out.
    """

    async def _update_queue(consumer, *_args, **_kwargs):
        capture.active = cache_writer_module.active_writer()
        capture.consumers_own = consumer._cache_writer
        if failure is not None:
            raise failure

    return _update_queue


class TestConsumeNonDurableEventsMidStreamFilterUpdateWriterBinding:
    """
    Bee b.xbb regression, second path: the MID-STREAM filter update.

    ``get_instance`` binds the CacheWriter ContextVar before its own
    ``update_queue`` call (pinned in
    ``TestGetInstanceFilterUpdateLifecycle``), but a client can also push
    new filters onto an already-open stream, and that update is issued from
    the servicer's stream body -- outside ``get_instance`` entirely. Nothing
    binds the ContextVar there. It works only because a ``ContextVar`` set
    inside an async-generator body during one ``__anext__`` is still bound
    at a later one: the bind made when the first request opened the stream
    survives the ``trace_grpc_event_stream`` wrapper generator, and is still
    live when the second request drives ``update_queue`` from the servicer's
    main generator body.

    Caveat: ``_drive`` below iterates the servicer generator with a direct
    ``async for`` inside the test coroutine, so what is pinned is the
    property under a single-task driver -- the same shape as every other
    stream test in this file -- not under grpc.aio's own driver.

    That is an implementation detail of the language, not a contract the
    code states, so it is pinned here -- at the surface where it actually
    holds. If it stops holding, ``active_writer()`` returns ``None`` inside
    the update and every write-through op the update's mutations would have
    emitted is silently dropped, which is exactly the failure b.xbb
    reported on the other path.

    The real ``get_instance``/``cleanup_channel`` lifecycle runs here (real
    ``CacheWriter``, real ``register_writer``, real ``set_active_queue_id``)
    -- only the broker and the storage API are mocked. A rig that bound the
    ContextVar itself would pin the test's own setup instead of the
    production sequence.
    """

    @pytest.fixture(autouse=True)
    def unbound_active_queue_id(self):
        # Explicit save/restore of the module-level ContextVar. The stream
        # binds it from inside an async-generator body, and an async
        # generator gets no context copy of its own -- which is the very
        # property under test -- so the bind is visible to this test's
        # context too, and would otherwise outlive the test.
        var = cache_writer_module._active_queue_id_var
        token = var.set(None)
        yield
        var.reset(token)

    @pytest.fixture
    def redis_enabled(self, mocker):
        # active_writer() short-circuits to None when write-through is off,
        # so the whole property is unobservable without this.
        mocker.patch.object(settings.settings, "redis_enabled", True)

    @pytest.fixture
    def mid_stream_update(self, mocker, redis_enabled):
        """
        Mock the broker and storage API, then capture what
        ``active_writer()`` returns from inside the mid-stream
        ``update_queue``.
        """
        queue = MagicMock(spec=aio_pika.abc.AbstractQueue)
        queue.name = _MID_STREAM_BROKER_QUEUE_NAME

        mocker.patch.object(
            NonDurableQueueConsumer,
            "_create_queue",
            return_value=(_MID_STREAM_QUEUE_ID, queue),
        )
        mocker.patch.object(
            rabbitmq_connector, "get_channel_direct", return_value=MagicMock()
        )
        mocker.patch.object(rabbitmq_connector, "return_channel")
        mocker.patch.object(storage_permissions, "initialize")
        mocker.patch.object(
            NonDurableQueueConsumer,
            "consume",
            side_effect=lambda: _non_durable_events(None),
        )

        capture = _MidStreamUpdateCapture()

        mocker.patch.object(
            NonDurableQueueConsumer,
            "update_queue",
            _mid_stream_update_queue(capture),
        )

        yield capture

        # Defensive: a cleanup regression must fail the assertion below,
        # not poison every later test in this worker with a live registry
        # entry. Runs after the test body, so it cannot mask the failure.
        # Unregisters by the WRITER the stream actually held -- the registry
        # takes the writer, and removes the entry only if that writer still
        # owns it.
        if capture.consumers_own is not None:
            cache_writer_module.unregister_writer(capture.consumers_own)

    @pytest.fixture
    def failing_mid_stream_update(self, mocker, mid_stream_update):
        """
        The same rig, with the mid-stream update raising after it records
        what it saw.

        Re-patching the class attribute overrides the patch
        ``mid_stream_update`` installed; ``mocker`` unwinds both in reverse
        order at teardown.
        """
        mocker.patch.object(
            NonDurableQueueConsumer,
            "update_queue",
            _mid_stream_update_queue(
                mid_stream_update, failure=ValueError(_MID_STREAM_UPDATE_BUG)
            ),
        )
        return mid_stream_update

    @staticmethod
    async def _drive(servicer, context):
        return [
            r
            async for r in servicer.ConsumeNonDurableEvents(
                _filter_update_requests(), context
            )
        ]

    @pytest.mark.asyncio
    async def test_the_mid_stream_update_binds_the_streams_own_writer(
        self, servicer, mock_context, mid_stream_update
    ):
        # Chained on purpose. `active is consumers_own` alone is a
        # tautology -- both fields start as None and stay None if the
        # update never fires, so a stream that dropped the update entirely
        # would pass it green. The `is not None` tail makes the identity
        # claim carry the "it fired at all" claim with it. And presence
        # alone would not be enough either: a writer left registered by
        # some other stream is also non-None, and ops enqueued on it land
        # under the wrong queue's Redis keys.
        await self._drive(servicer, mock_context)

        assert mid_stream_update.active is mid_stream_update.consumers_own is not None

    @pytest.mark.asyncio
    async def test_the_active_queue_id_is_unbound_when_the_stream_ends(
        self, servicer, mock_context, mid_stream_update
    ):
        # The flip side of the bind surviving the whole stream: because the
        # generator shares its caller's context, a missed teardown leaks the
        # binding out of the RPC rather than dying with it.
        #
        # Read the ContextVar itself rather than active_writer(), which
        # also consults the registry and so returns None when EITHER half
        # of cleanup_channel ran -- a regression that dropped only
        # reset_active_queue_id would pass through it unnoticed.
        await self._drive(servicer, mock_context)

        # Guard, not the subject: a stream that never bound anything would
        # satisfy the release assertion for the wrong reason.
        assert mid_stream_update.active is mid_stream_update.consumers_own is not None
        assert cache_writer_module._active_queue_id_var.get() is None

    @pytest.mark.asyncio
    async def test_the_registry_entry_is_dropped_when_the_stream_ends(
        self, servicer, mock_context, mid_stream_update
    ):
        # The other half of cleanup_channel's teardown, pinned separately
        # so neither half can stand in for the other. The registry is a
        # process-wide dict keyed by the cache-scope key, so an entry left
        # behind outlives the RPC: a later stream from the same principal
        # on the same queue would find this stream's drained, stopped
        # writer.
        await self._drive(servicer, mock_context)

        assert mid_stream_update.active is mid_stream_update.consumers_own is not None
        assert cache_writer_module.get_writer_for_queue(_MID_STREAM_SCOPE_KEY) is None

    @pytest.mark.asyncio
    async def test_the_active_queue_id_is_unbound_when_the_update_raises(
        self, servicer, mock_context, failing_mid_stream_update
    ):
        # The release lives in the servicer's finally, so an update that
        # dies mid-stream must still leave the ContextVar clean. Teardown
        # moved onto the success path would pass every test above and leak
        # here -- and this leak escapes the RPC, since the stream generator
        # shares its caller's context.
        with pytest.raises(ValueError, match=_MID_STREAM_UPDATE_BUG):
            await self._drive(servicer, mock_context)

        assert (
            failing_mid_stream_update.active
            is failing_mid_stream_update.consumers_own
            is not None
        )
        assert cache_writer_module._active_queue_id_var.get() is None


class TestOriginQueueForTag:
    """REPLAY recovers the origin queue from the tracker's per-tag cache."""

    def test_returns_cached_origin_queue(self):
        tracker = AckTracker()
        message = MagicMock()
        message.delivery_tag = 7
        tracker.track(message, origin_queue="origin-queue")

        result = EventConsumerServicer._origin_queue_for_tag(tracker, 7)

        assert result == "origin-queue"

    def test_returns_none_for_unknown_tag(self):
        tracker = AckTracker()

        result = EventConsumerServicer._origin_queue_for_tag(tracker, 999)

        assert result is None

    def test_does_not_redecode_the_message(self, mocker):
        # The re-decode is gone: _origin_queue_for_tag must read the cache, not
        # call decode_dead_lettered_message. The symbol is no longer imported in
        # the servicer module, so patch it on its defining module and assert it
        # is never invoked.
        decode = mocker.patch(
            "app.lib.rabbitmq.dead_letter.decode_dead_lettered_message"
        )
        tracker = AckTracker()
        message = MagicMock()
        message.delivery_tag = 7
        tracker.track(message, origin_queue="origin-queue")

        EventConsumerServicer._origin_queue_for_tag(tracker, 7)

        decode.assert_not_called()


def _action_entry(
    delivery_tag: int, action
) -> "ManageDeadLetteredEventsRequest.ActionEntry":
    return ManageDeadLetteredEventsRequest.ActionEntry(
        delivery_tag=delivery_tag, action=action
    )


class TestDispatchDeadLetterActionOutcomeMap:
    """The ActionStatus -> Outcome map is explicit and total."""

    @pytest.mark.asyncio
    async def test_discarded_maps_to_discarded_outcome(self, mocker):
        mocker.patch.object(
            DeadLetterConsumer,
            "discard",
            return_value=ActionResult(status=ActionStatus.DISCARDED, delivery_tag=3),
        )
        entry = _action_entry(3, _ACTION.ACTION_DISCARD)

        response = await EventConsumerServicer._dispatch_dead_letter_action(
            entry, MagicMock(), AckTracker(), _ACTION
        )

        assert response.action_outcome.outcome == _OUTCOME.OUTCOME_DISCARDED

    @pytest.mark.asyncio
    async def test_left_maps_to_left_outcome(self, mocker):
        mocker.patch.object(
            DeadLetterConsumer,
            "leave",
            return_value=ActionResult(status=ActionStatus.LEFT, delivery_tag=4),
        )
        entry = _action_entry(4, _ACTION.ACTION_LEAVE)

        response = await EventConsumerServicer._dispatch_dead_letter_action(
            entry, MagicMock(), AckTracker(), _ACTION
        )

        assert response.action_outcome.outcome == _OUTCOME.OUTCOME_LEFT

    @pytest.mark.asyncio
    async def test_unknown_tag_yields_no_response(self, mocker):
        mocker.patch.object(
            DeadLetterConsumer,
            "discard",
            return_value=ActionResult(
                status=ActionStatus.UNKNOWN_TAG, delivery_tag=5, detail="gone"
            ),
        )
        entry = _action_entry(5, _ACTION.ACTION_DISCARD)

        response = await EventConsumerServicer._dispatch_dead_letter_action(
            entry, MagicMock(), AckTracker(), _ACTION
        )

        assert response is None

    @pytest.mark.asyncio
    async def test_unmapped_status_raises_key_error(self, mocker):
        # Regression guard: a future ActionStatus member reaching the map must
        # fail loudly (KeyError) rather than silently mapping to LEFT, which is
        # what the old bare-else did. Simulate a handler returning a status that
        # is not in _ACTION_STATUS_TO_OUTCOME and is not UNKNOWN_TAG.
        unmapped = MagicMock()  # stands in for a future, unmapped ActionStatus
        mocker.patch.object(
            DeadLetterConsumer,
            "discard",
            return_value=ActionResult(status=unmapped, delivery_tag=6),
        )
        entry = _action_entry(6, _ACTION.ACTION_DISCARD)

        with pytest.raises(KeyError):
            await EventConsumerServicer._dispatch_dead_letter_action(
                entry, MagicMock(), AckTracker(), _ACTION
            )


class TestDispatchDeadLetterActionReplayUnknownTag:
    """REPLAY for a tag with no cached origin queue is skipped (no response)."""

    @pytest.mark.asyncio
    async def test_replay_unknown_tag_returns_none(self, mocker):
        # The tracker has nothing cached for this tag, so _origin_queue_for_tag
        # returns None and the REPLAY is skipped without calling replay().
        replay = mocker.patch.object(DeadLetterConsumer, "replay")
        entry = _action_entry(99, _ACTION.ACTION_REPLAY)

        response = await EventConsumerServicer._dispatch_dead_letter_action(
            entry, MagicMock(), AckTracker(), _ACTION
        )

        assert response is None
        replay.assert_not_called()


class TestDispatchDeadLetterActionUnspecified:
    """An unspecified / unknown action is logged and skipped (no response)."""

    @pytest.mark.asyncio
    async def test_unspecified_action_returns_none(self, mocker):
        # ACTION_UNSPECIFIED (the proto default, value 0) hits neither REPLAY,
        # DISCARD, nor LEAVE: the else branch logs and returns None.
        discard = mocker.patch.object(DeadLetterConsumer, "discard")
        leave = mocker.patch.object(DeadLetterConsumer, "leave")
        entry = _action_entry(7, _ACTION.ACTION_UNSPECIFIED)

        response = await EventConsumerServicer._dispatch_dead_letter_action(
            entry, MagicMock(), AckTracker(), _ACTION
        )

        assert response is None
        discard.assert_not_called()
        leave.assert_not_called()


def _tracked_message(delivery_tag, settle_log):
    """
    An aio-pika incoming message whose settles are recorded in ``settle_log``.

    The log is shared across every message in a test, so the ORDER of settles
    across different messages is observable -- which is what pins "acks are
    processed before nacks" rather than merely "both happened".
    """
    message = AsyncMock(spec=aio_pika.abc.AbstractIncomingMessage)
    message.delivery_tag = delivery_tag

    async def _record_ack():
        settle_log.append(("ack", delivery_tag))

    async def _record_nack(requeue=True):
        settle_log.append(("nack", delivery_tag, requeue))

    message.ack = AsyncMock(side_effect=_record_ack)
    message.nack = AsyncMock(side_effect=_record_nack)
    return message


class TestListenForDurableAcks:
    """
    _listen_for_durable_acks settles each requested tag and tolerates
    unknown ones.

    Driven against the REAL ``AckTracker`` rather than a mock: the
    unknown-tag contract these tests turn on is the tracker's own KeyError,
    and a mock tracker would let the listener "pass" against a KeyError the
    production tracker does not actually raise. The observable is the
    aio-pika message's ``ack`` / ``nack`` spy.
    """

    @staticmethod
    async def _request_iter(*requests):
        for request in requests:
            yield request

    @classmethod
    async def _drive(cls, tracker, *requests):
        async for _ in EventConsumerServicer._listen_for_durable_acks(
            cls._request_iter(*requests), tracker
        ):
            pass

    @pytest.mark.asyncio
    async def test_acks_each_requested_tag(self):
        tracker = AckTracker()
        settle_log = []
        tracker.track(_tracked_message(1, settle_log))
        tracker.track(_tracked_message(2, settle_log))

        await self._drive(
            tracker, ConsumeDurableEventsWithAcksRequest(ack_delivery_tags=[1, 2])
        )

        assert settle_log == [("ack", 1), ("ack", 2)]

    @pytest.mark.asyncio
    async def test_logs_warning_on_unknown_tag(self, mocker):
        # An ack for an unknown delivery tag raises KeyError inside the
        # tracker; the listener swallows it and logs a warning so one stale
        # tag does not crash the stream.
        mock_logger = mocker.patch.object(servicer_module, "logger")

        await self._drive(
            AckTracker(), ConsumeDurableEventsWithAcksRequest(ack_delivery_tags=[42])
        )

        mock_logger.warning.assert_called_once_with(
            "Received ack for unknown delivery_tag=%s", 42
        )

    @pytest.mark.asyncio
    async def test_nacks_each_requested_tag_with_requeue(self):
        # requeue=True is the whole point of the verb: a nacked message goes
        # back on its queue for redelivery. requeue=False would silently drop
        # it, which is what an ack is for.
        tracker = AckTracker()
        settle_log = []
        message_one = _tracked_message(1, settle_log)
        message_two = _tracked_message(2, settle_log)
        tracker.track(message_one)
        tracker.track(message_two)

        await self._drive(
            tracker, ConsumeDurableEventsWithAcksRequest(nack_delivery_tags=[1, 2])
        )

        message_one.nack.assert_awaited_once_with(requeue=True)
        message_two.nack.assert_awaited_once_with(requeue=True)

    @pytest.mark.asyncio
    async def test_acks_are_settled_before_nacks(self):
        # The documented precedence for a tag in both fields is a consequence
        # of this ordering, so the ordering itself is the contract. The
        # request lists the nack tag first, so an implementation that simply
        # walked the request's fields in declaration order would fail here.
        tracker = AckTracker()
        settle_log = []
        tracker.track(_tracked_message(1, settle_log))
        tracker.track(_tracked_message(2, settle_log))

        await self._drive(
            tracker,
            ConsumeDurableEventsWithAcksRequest(
                nack_delivery_tags=[1], ack_delivery_tags=[2]
            ),
        )

        assert settle_log == [("ack", 2), ("nack", 1, True)]

    @pytest.mark.asyncio
    async def test_tag_in_both_fields_is_acked_not_nacked(self):
        tracker = AckTracker()
        settle_log = []
        message = _tracked_message(1, settle_log)
        tracker.track(message)

        await self._drive(
            tracker,
            ConsumeDurableEventsWithAcksRequest(
                ack_delivery_tags=[1], nack_delivery_tags=[1]
            ),
        )

        message.ack.assert_awaited_once()
        message.nack.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_tag_in_both_fields_still_attempts_the_nack(self, mocker):
        # The precedence must come from the ack having already removed the
        # tag, not from the listener skipping the nack loop: the nack IS
        # attempted, hits the tracker's unknown-tag KeyError, and is
        # swallowed. Asserting only "the message was not nacked" would pass
        # for an implementation that dropped nack_delivery_tags entirely.
        tracker = AckTracker()
        settle_log = []
        tracker.track(_tracked_message(1, settle_log))
        spy = mocker.patch.object(tracker, "nack", wraps=tracker.nack)

        await self._drive(
            tracker,
            ConsumeDurableEventsWithAcksRequest(
                ack_delivery_tags=[1], nack_delivery_tags=[1]
            ),
        )

        spy.assert_awaited_once_with(1, requeue=True)

    @pytest.mark.asyncio
    async def test_logs_warning_on_unknown_nack_tag(self, mocker):
        mock_logger = mocker.patch.object(servicer_module, "logger")

        await self._drive(
            AckTracker(), ConsumeDurableEventsWithAcksRequest(nack_delivery_tags=[42])
        )

        mock_logger.warning.assert_called_once_with(
            "Received nack for unknown delivery_tag=%s", 42
        )

    @pytest.mark.asyncio
    async def test_unknown_nack_tag_does_not_stop_later_settles(self):
        # One stale tag from a client that is behind must not take the stream
        # down: the tags after it in the same request, and the requests after
        # it in the stream, are still settled.
        tracker = AckTracker()
        settle_log = []
        tracker.track(_tracked_message(1, settle_log))
        tracker.track(_tracked_message(2, settle_log))

        await self._drive(
            tracker,
            ConsumeDurableEventsWithAcksRequest(nack_delivery_tags=[999, 1]),
            ConsumeDurableEventsWithAcksRequest(nack_delivery_tags=[2]),
        )

        assert settle_log == [("nack", 1, True), ("nack", 2, True)]

    @pytest.mark.asyncio
    async def test_records_a_client_nack_metric_per_settled_tag(self, mocker):
        tracker = AckTracker()
        settle_log = []
        tracker.track(_tracked_message(1, settle_log))
        tracker.track(_tracked_message(2, settle_log))
        record = mocker.patch.object(consumer_metrics, "record_client_nack")

        await self._drive(
            tracker, ConsumeDurableEventsWithAcksRequest(nack_delivery_tags=[1, 2])
        )

        assert record.call_count == 2

    @pytest.mark.asyncio
    async def test_does_not_record_a_client_nack_metric_for_unknown_tag(self, mocker):
        # The counter has to mean "a message went back on a queue". Counting
        # the KeyError path would inflate it with a client's stale tags and
        # break its use as a dead-lettering leading indicator.
        record = mocker.patch.object(consumer_metrics, "record_client_nack")

        await self._drive(
            AckTracker(), ConsumeDurableEventsWithAcksRequest(nack_delivery_tags=[42])
        )

        record.assert_not_called()

    @pytest.mark.asyncio
    async def test_does_not_record_a_client_nack_metric_when_the_settle_fails(
        self, mocker
    ):
        # The counter has to mean "a message went back on a queue". A nack
        # that reached the broker and was refused -- the dead-channel case
        # that ends the stream -- put nothing back, so counting it would
        # inflate the dead-lettering leading indicator with exactly the
        # incidents where no message moved.
        tracker = AckTracker()
        message = AsyncMock(spec=aio_pika.abc.AbstractIncomingMessage)
        message.delivery_tag = 1
        message.nack.side_effect = aio_pika.exceptions.ChannelInvalidStateError()
        tracker.track(message)
        record = mocker.patch.object(consumer_metrics, "record_client_nack")

        with pytest.raises(aio_pika.exceptions.ChannelInvalidStateError):
            await self._drive(
                tracker, ConsumeDurableEventsWithAcksRequest(nack_delivery_tags=[1])
            )

        record.assert_not_called()

    @pytest.mark.asyncio
    async def test_does_not_record_a_client_nack_metric_for_an_ack(self, mocker):
        tracker = AckTracker()
        settle_log = []
        tracker.track(_tracked_message(1, settle_log))
        record = mocker.patch.object(consumer_metrics, "record_client_nack")

        await self._drive(
            tracker, ConsumeDurableEventsWithAcksRequest(ack_delivery_tags=[1])
        )

        record.assert_not_called()


class TestBuildEventResponseFailure:
    """_build_event_response records a failure metric and re-raises on error."""

    def test_records_failure_and_reraises(self, mocker, mock_record_failure):
        # convert_to_event_proto raising (e.g. a malformed event body) must
        # record the failure metric and propagate the exception.
        mocker.patch.object(
            EventConsumerServicer,
            "_build_event_response",
            wraps=EventConsumerServicer._build_event_response,
        )
        mocker.patch(
            "app.grpc.event_consumer_servicer.convert_to_event_proto",
            side_effect=ValueError("bad event"),
        )
        event_msg = EventMessage({"event_type": "x"})

        with pytest.raises(ValueError, match="bad event"):
            EventConsumerServicer._build_event_response(event_msg, MagicMock())

        mock_record_failure.assert_called_once_with(event_msg)


# Every authenticated RPC on this service, each paired with two callables:
#
#   ``install_downstream(mocker)`` stubs everything the RPC does AFTER the
#   authentication step and returns the one stub that stands for "the RPC
#   did its work" -- the broker-touching call the caller came for.
#
#   ``drive(servicer, context)`` runs the RPC to its normal outcome.
#
# The split is what makes the ordering assertion real. A driver that merely
# ran to completion would keep passing with ``await authenticate()`` moved
# to the bottom of the method, because nothing would record that the work
# had already happened. Asserting the returned stub was never called pins
# authentication ahead of the work uniformly, for every RPC in the table.
#
# The five drivers whose RPC takes a client-supplied queue_id accept one, so
# the same table serves the queue-id validation tests below.

#: An id that passes QUEUE_ID_PATTERN, so nothing but the step under test
#: can stop a driver short.
_VALID_QUEUE_ID = "q"


def _install_consume_durable_events(mocker):
    # ``validate_consume_durable_request`` is deliberately left real: the
    # driver passes a well-formed queue_id, so the only thing that can stop
    # the RPC short of ``consume`` is the authentication step under test.
    return mocker.patch.object(
        DurableQueueConsumer,
        "consume",
        side_effect=lambda _qid: _idle_event_generator(),
    )


async def _drive_consume_durable_events(servicer, context, queue_id=_VALID_QUEUE_ID):
    return [
        r
        async for r in servicer.ConsumeDurableEvents(
            ConsumeDurableEventsRequest(queue_id=queue_id), context
        )
    ]


async def _open_durable_acks_stream(queue_id):
    """A client stream that opens a queue and then sends nothing more."""
    yield ConsumeDurableEventsWithAcksRequest(queue_id=queue_id)


def _install_consume_durable_events_with_acks(mocker):
    async def _consume_with_acks(_queue_id):
        return _idle_event_generator(), AckTracker()

    return mocker.patch.object(
        DurableQueueConsumer, "consume_with_acks", side_effect=_consume_with_acks
    )


async def _drive_consume_durable_events_with_acks(
    servicer, context, queue_id=_VALID_QUEUE_ID
):
    # A request stream carrying one real request, not an empty one: the RPC
    # has to get past ``anext`` to reach the work whose absence is asserted.
    return [
        r
        async for r in servicer.ConsumeDurableEventsWithAcks(
            _open_durable_acks_stream(queue_id), context
        )
    ]


async def _open_non_durable_stream(reconnect_token=""):
    """A client stream that opens a stream and then sends nothing more."""
    yield ConsumeNonDurableEventsRequest(
        filter_groups=[_proto_filter_group("e1")],
        reconnect_token=reconnect_token,
    )


def _install_consume_non_durable_events(mocker):
    _filter_update_rig(mocker)

    # ``_filter_update_rig`` patched this; hand it back as the sentinel,
    # since checking out the consumer is the first thing the stream does
    # once the token is accepted.
    return NonDurableQueueConsumer.get_instance


async def _drive_consume_non_durable_events(servicer, context, reconnect_token=""):
    return [
        r
        async for r in servicer.ConsumeNonDurableEvents(
            _open_non_durable_stream(reconnect_token), context
        )
    ]


def _install_manage_dead_lettered_events(mocker):
    mocker.patch.object(permissions, "raise_if_cant_inspect_and_replay_all_dlq")

    async def _browse():
        return _idle_event_generator(), AckTracker(), MagicMock()

    return mocker.patch.object(DeadLetterConsumer, "browse", side_effect=_browse)


async def _drive_manage_dead_lettered_events(servicer, context):
    return [
        r
        async for r in servicer.ManageDeadLetteredEvents(
            _empty_async_iterator(), context
        )
    ]


def _install_create_durable_queue(mocker):
    return mocker.patch.object(
        DurableQueueConsumer, "create_queue", return_value=("q", True)
    )


async def _drive_create_durable_queue(servicer, context, queue_id=_VALID_QUEUE_ID):
    return await servicer.CreateDurableQueue(
        CreateDurableQueueRequest(queue_id=queue_id), context
    )


def _install_delete_durable_queue(mocker):
    return mocker.patch.object(DurableQueueConsumer, "delete_queue")


async def _drive_delete_durable_queue(servicer, context, queue_id=_VALID_QUEUE_ID):
    return await servicer.DeleteDurableQueue(
        DeleteDurableQueueRequest(queue_id=queue_id), context
    )


def _install_update_durable_queue(mocker):
    return mocker.patch.object(DurableQueueConsumer, "update_queue")


async def _drive_update_durable_queue(servicer, context, queue_id=_VALID_QUEUE_ID):
    return await servicer.UpdateDurableQueue(
        UpdateDurableQueueRequest(queue_id=queue_id), context
    )


_RPCS = {
    "ConsumeDurableEvents": (
        _install_consume_durable_events,
        _drive_consume_durable_events,
    ),
    "ConsumeDurableEventsWithAcks": (
        _install_consume_durable_events_with_acks,
        _drive_consume_durable_events_with_acks,
    ),
    "ConsumeNonDurableEvents": (
        _install_consume_non_durable_events,
        _drive_consume_non_durable_events,
    ),
    "ManageDeadLetteredEvents": (
        _install_manage_dead_lettered_events,
        _drive_manage_dead_lettered_events,
    ),
    "CreateDurableQueue": (
        _install_create_durable_queue,
        _drive_create_durable_queue,
    ),
    "DeleteDurableQueue": (
        _install_delete_durable_queue,
        _drive_delete_durable_queue,
    ),
    "UpdateDurableQueue": (
        _install_update_durable_queue,
        _drive_update_durable_queue,
    ),
}

AUTHENTICATED_RPCS = [pytest.param(rpc, id=name) for name, rpc in _RPCS.items()]

# Every RPC that takes a client-supplied queue_id. The two that do not are
# ConsumeNonDurableEvents (its id is the reconnect token, bounded by its own
# validator) and ManageDeadLetteredEvents (the DLQ name is a server setting).
# Listed by name rather than filtered, so adding a queue_id to a new RPC is a
# deliberate edit here and not a silent omission.
QUEUE_ID_VALIDATING_RPCS = [
    pytest.param(_RPCS[name], id=name)
    for name in (
        "ConsumeDurableEvents",
        "ConsumeDurableEventsWithAcks",
        "CreateDurableQueue",
        "DeleteDurableQueue",
        "UpdateDurableQueue",
    )
]

# The subset of the above that stamps the queue_id on the current span. The
# three CRUD RPCs never call ``add_current_span_attributes`` at all, so a
# "never reaches a span attribute" assertion passes trivially for them; only
# these two can show that validation is what keeps a malformed id off the
# span. Listed by name for the same reason as the table above.
SPAN_STAMPING_QUEUE_ID_RPCS = [
    pytest.param(_RPCS[name], id=name)
    for name in ("ConsumeDurableEvents", "ConsumeDurableEventsWithAcks")
]


@pytest.fixture
def patched_authenticate(mocker):
    """
    Replace the entry-point authentication step for one test.

    Patched on ``event_consumer_servicer`` -- the module that imported the
    name -- so a rename of the import fails at the patch site instead of
    leaving a test that silently asserts nothing.
    """

    def _patch(**kwargs):
        return mocker.patch.object(servicer_module, "authenticate", **kwargs)

    return _patch


class TestAuthenticatedRpcs:
    """
    Every RPC validates the caller's token before doing any work.

    Token validation used to ride along on durable queue naming, which
    resolved the caller's principal. Naming no longer touches an identity,
    so each entry point calls ``authenticate`` on purpose. Without it a
    deployment running with authentication on but the permissions service
    off would serve unauthenticated requests, and nothing else on the
    request path would notice.
    """

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", AUTHENTICATED_RPCS)
    async def test_awaits_authenticate(
        self, rpc, servicer, mock_context, mocker, patched_authenticate
    ):
        install_downstream, drive_rpc = rpc
        install_downstream(mocker)
        authenticate = patched_authenticate()

        await drive_rpc(servicer, mock_context)

        # assert_awaited, not assert_called: an un-awaited coroutine would
        # validate nothing while still recording the call.
        authenticate.assert_awaited_once_with()

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", AUTHENTICATED_RPCS)
    async def test_a_rejected_token_ends_the_rpc(
        self, rpc, servicer, mock_context, mocker, patched_authenticate
    ):
        install_downstream, drive_rpc = rpc
        install_downstream(mocker)
        patched_authenticate(side_effect=AuthenticationError("Token expired"))

        with pytest.raises(AuthenticationError, match="Token expired"):
            await drive_rpc(servicer, mock_context)

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", AUTHENTICATED_RPCS)
    async def test_a_rejected_token_runs_no_downstream_work(
        self, rpc, servicer, mock_context, mocker, patched_authenticate
    ):
        """
        The ordering pin: the token is checked BEFORE the RPC does the
        thing the caller asked for.

        Without this, ``await authenticate()`` could sit at the bottom of
        every one of these methods -- the queue would already be deleted,
        the stream already opened -- and the test above would still pass,
        because the error still reaches the client either way.
        """
        install_downstream, drive_rpc = rpc
        downstream = install_downstream(mocker)
        patched_authenticate(side_effect=AuthenticationError("Token expired"))

        with pytest.raises(AuthenticationError):
            await drive_rpc(servicer, mock_context)

        downstream.assert_not_called()

    @pytest.mark.asyncio
    async def test_a_rejected_token_maps_to_unauthenticated(
        self, servicer, mock_context, mocker, patched_authenticate
    ):
        # The status the client actually receives. ``ExceptionInterceptor``
        # aborts with ``ErrorDetails(error).grpc_code``, so the error raised
        # out of the RPC is fed through that same mapping here rather than
        # re-stating 401 -> UNAUTHENTICATED as a constant this file invents.
        _install_delete_durable_queue(mocker)
        patched_authenticate(side_effect=AuthenticationError("Token expired"))

        with pytest.raises(AuthenticationError) as exc_info:
            await _drive_delete_durable_queue(servicer, mock_context)

        assert ErrorDetails(exc_info.value).grpc_code == grpc.StatusCode.UNAUTHENTICATED


class TestConsumeNonDurableEventsAdmissionPrecedesAuthentication:
    """
    A capacity refusal must not be turned into an authentication failure.

    ``raise_if_at_capacity`` stays the first statement in the RPC because a
    shed re-routable stream is re-routed by the proxy's retry policy
    (retryOn resource-exhausted, SDD SR-3.7), and Envoy decides
    retryability from the response headers alone -- so the refusal has to
    arrive as a trailers-only RESOURCE_EXHAUSTED. Calling ``authenticate``
    first would put its own 401 ahead of that refusal, silently breaking
    re-routing.

    First statement in the METHOD is the requirement. The shared
    ``AuthInterceptor`` resolves the token above the handler, so a caller
    whose token does not resolve is refused UNAUTHENTICATED before the
    guard is reached -- deliberately, since such a request fails
    identically on every pod. What is pinned here is the ordering for every
    caller whose token does resolve.
    """

    @staticmethod
    async def _drive(servicer, context):
        return [
            r
            async for r in servicer.ConsumeNonDurableEvents(
                _empty_async_iterator(), context
            )
        ]

    @pytest.mark.asyncio
    async def test_a_refused_stream_never_reaches_authentication(
        self, servicer, mock_context, capacity_guard_rejections, patched_authenticate
    ):
        authenticate = patched_authenticate()
        capacity_guard_rejections(non_durable_rejects=True)

        with pytest.raises(ChannelNotAvailableError):
            await self._drive(servicer, mock_context)

        authenticate.assert_not_called()

    @pytest.mark.asyncio
    async def test_an_admitted_stream_still_authenticates(
        self, servicer, mock_context, capacity_guard_rejections, patched_authenticate
    ):
        # The other half of the ordering: admission passing must not let the
        # stream skip the token check.
        authenticate = patched_authenticate()
        capacity_guard_rejections()

        await self._drive(servicer, mock_context)

        authenticate.assert_awaited_once_with()


class TestAdmissionPrecedesAuthenticationOnEveryGuardedRpc:
    """
    The same ordering, asserted across the whole surface rather than on the
    one stream it was first written for.

    Every RPC in ``_RPCS`` opens with ``raise_if_at_capacity`` -- the
    durable ones on the durable pool, the non-durable stream on its own --
    and each has to keep it AHEAD of ``authenticate``. The reason is the
    same everywhere: a saturated pod must answer a caller whose token
    resolves with the status that caller retries or re-routes on, not with
    a 401 about a credential the RPC never got as far as needing. (A caller
    whose token does not resolve is refused UNAUTHENTICATED by the shared
    interceptor above the handler, which is intended and is stated on
    ``ConsumeNonDurableEvents``.) Pinning it only on the non-durable stream
    left the other six free to drift, and the drift is invisible -- both
    orderings return an error, just a different one.
    """

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", AUTHENTICATED_RPCS)
    async def test_a_refused_call_never_reaches_authentication(
        self,
        rpc,
        servicer,
        mock_context,
        mocker,
        capacity_guard_rejections,
        patched_authenticate,
    ):
        install_downstream, drive_rpc = rpc
        install_downstream(mocker)
        authenticate = patched_authenticate()
        # Both pools refused, because which one a given RPC checks is the
        # RPC's business -- what is asserted is that whichever it checks,
        # it checks it first.
        capacity_guard_rejections(non_durable_rejects=True, durable_rejects=True)

        with pytest.raises(ChannelNotAvailableError):
            await drive_rpc(servicer, mock_context)

        authenticate.assert_not_called()

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", AUTHENTICATED_RPCS)
    async def test_a_refused_call_does_no_downstream_work(
        self, rpc, servicer, mock_context, mocker, capacity_guard_rejections
    ):
        install_downstream, drive_rpc = rpc
        downstream = install_downstream(mocker)
        capacity_guard_rejections(non_durable_rejects=True, durable_rejects=True)

        with pytest.raises(ChannelNotAvailableError):
            await drive_rpc(servicer, mock_context)

        downstream.assert_not_called()

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", AUTHENTICATED_RPCS)
    async def test_a_refused_call_maps_to_resource_exhausted(
        self, rpc, servicer, mock_context, mocker, capacity_guard_rejections
    ):
        # The status the caller actually receives, through the same
        # ``ErrorDetails`` mapping the exception interceptor aborts with.
        # It is what the proxy's retry policy keys on, so it has to be the
        # same on every guarded RPC.
        install_downstream, drive_rpc = rpc
        install_downstream(mocker)
        capacity_guard_rejections(non_durable_rejects=True, durable_rejects=True)

        with pytest.raises(ChannelNotAvailableError) as exc_info:
            await drive_rpc(servicer, mock_context)

        assert (
            ErrorDetails(exc_info.value).grpc_code == grpc.StatusCode.RESOURCE_EXHAUSTED
        )

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", AUTHENTICATED_RPCS)
    async def test_an_admitted_call_still_authenticates(
        self,
        rpc,
        servicer,
        mock_context,
        mocker,
        capacity_guard_rejections,
        patched_authenticate,
    ):
        # The other half: admission passing must not let the call skip the
        # token check, which is what "guard first" would look like if the
        # guard had simply swallowed the rest of the method.
        install_downstream, drive_rpc = rpc
        install_downstream(mocker)
        authenticate = patched_authenticate()
        capacity_guard_rejections()

        await drive_rpc(servicer, mock_context)

        authenticate.assert_awaited_once_with()


class _RecordingRequestStream:
    """
    A client request stream that counts how many times it is advanced.

    ``_empty_async_iterator`` cannot answer "did the RPC read from the
    client before it refused?" -- an untouched iterator and an exhausted one
    look identical from the outside. This one records every ``__anext__``.
    """

    def __init__(self, *requests):
        self._requests = iter(requests)
        self.reads = 0

    def __aiter__(self):
        return self

    async def __anext__(self):
        self.reads += 1

        try:
            return next(self._requests)
        except StopIteration:
            raise StopAsyncIteration from None


def _open_consume_durable_events(servicer, context):
    # The one streaming RPC whose request is unary, so there is no client
    # stream to record -- hence the None second element.
    response_stream = servicer.ConsumeDurableEvents(
        ConsumeDurableEventsRequest(queue_id=_VALID_QUEUE_ID), context
    )
    return response_stream, None


def _open_consume_durable_events_with_acks(servicer, context):
    # Seeded with a well-formed first request, so a build that read the
    # client before refusing would get a real request back and carry on --
    # the drift these tests have to catch, rather than an empty stream
    # ending the RPC for an unrelated reason.
    requests = _RecordingRequestStream(
        ConsumeDurableEventsWithAcksRequest(queue_id=_VALID_QUEUE_ID)
    )
    return servicer.ConsumeDurableEventsWithAcks(requests, context), requests


def _open_consume_non_durable_events(servicer, context):
    requests = _RecordingRequestStream(
        ConsumeNonDurableEventsRequest(filter_groups=[_proto_filter_group("e1")])
    )
    return servicer.ConsumeNonDurableEvents(requests, context), requests


def _open_manage_dead_lettered_events(servicer, context):
    # Seeded with no action: this RPC reads its client stream only from
    # inside the merge, and what is asserted is that the read never happens
    # at all, not what it would have returned.
    requests = _RecordingRequestStream()
    return servicer.ManageDeadLetteredEvents(requests, context), requests


# The four RPCs of ``_RPCS`` that answer with a stream, each paired with an
# opener that hands back its response stream UNSTARTED. Named one by one for
# the same reason as the tables above: adding a streaming RPC is a deliberate
# edit here, not a silent omission.
_STREAM_OPENERS = {
    "ConsumeDurableEvents": _open_consume_durable_events,
    "ConsumeDurableEventsWithAcks": _open_consume_durable_events_with_acks,
    "ConsumeNonDurableEvents": _open_consume_non_durable_events,
    "ManageDeadLetteredEvents": _open_manage_dead_lettered_events,
}

# Additive to ``AUTHENTICATED_RPCS``, which stays seven wide: the three CRUD
# RPCs are left out here because they answer with a single message and their
# routes do not retry resource-exhausted.
#
# The install half of each ``_RPCS`` entry is reused; the drive half is not.
# Those helpers collect inside a list comprehension, so a response yielded
# ahead of the refusal dies with the exception that ends the comprehension,
# leaving nothing to assert on.
STREAMING_RPCS = [
    pytest.param(_RPCS[name][0], _STREAM_OPENERS[name], id=name)
    for name in _STREAM_OPENERS
]

# The subset that reads a client request stream -- the only place the "read
# nothing before refusing" half of the contract is observable. Listed by name
# rather than filtered, for the reason given above, and pinned against the
# servicer's own signatures by ``test_every_request_streaming_rpc_is_pinned``.
REQUEST_STREAMING_RPCS = [
    pytest.param(_RPCS[name][0], _STREAM_OPENERS[name], id=name)
    for name in (
        "ConsumeDurableEventsWithAcks",
        "ConsumeNonDurableEvents",
        "ManageDeadLetteredEvents",
    )
]


async def _collect_until_refused(response_stream):
    """
    Iterate ``response_stream`` to its capacity refusal, keeping whatever it
    yielded on the way there.
    """
    yielded = []

    with pytest.raises(ChannelNotAvailableError):
        async for response in response_stream:
            yielded.append(response)

    return yielded


class TestRefusedStreamsCommitNothingBeforeTheRefusal:
    """
    A capacity refusal on a streaming RPC has to leave as a trailers-only
    response.

    These streams' HTTPProxy routes retry resource-exhausted (SDD SR-3.7)
    and Envoy reads retryability off the response headers alone, so a single
    response -- or one initial-metadata send -- committed above
    ``raise_if_at_capacity`` silently costs the stream its re-route. That is
    what keeps the guard the first statement in each of these methods.

    On the three durable RPCs the "yields nothing" arm pins against a
    response-committing statement introduced above the guard, not against
    the guard being relocated below the method's existing yield site: the
    reused install halves patch their downstream to a generator that yields
    nothing, so a relocated guard still collects an empty list.
    ``TestAdmissionPrecedesAuthenticationOnEveryGuardedRpc`` catches that
    form of the drift instead. ``ConsumeNonDurableEvents`` is unaffected,
    because its first yield is the reconnect token.
    """

    @pytest.mark.asyncio
    @pytest.mark.parametrize("install_downstream, open_stream", STREAMING_RPCS)
    async def test_a_refused_stream_yields_no_responses(
        self,
        install_downstream,
        open_stream,
        servicer,
        mock_context,
        mocker,
        capacity_guard_rejections,
    ):
        install_downstream(mocker)
        capacity_guard_rejections(non_durable_rejects=True, durable_rejects=True)
        response_stream, _ = open_stream(servicer, mock_context)

        yielded = await _collect_until_refused(response_stream)

        assert yielded == []

    @pytest.mark.asyncio
    @pytest.mark.parametrize("install_downstream, open_stream", STREAMING_RPCS)
    async def test_a_refused_stream_sends_no_initial_metadata(
        self,
        install_downstream,
        open_stream,
        servicer,
        mock_context,
        mocker,
        capacity_guard_rejections,
    ):
        # Initial metadata commits the very headers Envoy reads
        # retryability off, so sending it above the guard breaks the
        # re-route exactly as a yielded response would -- while leaving the
        # list the test above asserts on empty, so only this assertion can
        # catch it. Distinct from that test only because ``mock_context``
        # makes ``send_initial_metadata`` an ``AsyncMock``: an awaited drift
        # against a plain MagicMock would die as a TypeError naming neither
        # metadata nor trailers, failing both tests identically.
        install_downstream(mocker)
        capacity_guard_rejections(non_durable_rejects=True, durable_rejects=True)
        response_stream, _ = open_stream(servicer, mock_context)

        await _collect_until_refused(response_stream)

        mock_context.send_initial_metadata.assert_not_called()

    @pytest.mark.asyncio
    @pytest.mark.parametrize("install_downstream, open_stream", REQUEST_STREAMING_RPCS)
    async def test_a_refused_stream_never_reads_from_the_client(
        self,
        install_downstream,
        open_stream,
        servicer,
        mock_context,
        mocker,
        capacity_guard_rejections,
    ):
        # The replay half of the contract: the proxy re-sends the request
        # bytes to another pod, so a refusing pod that had already consumed
        # a ManageDeadLetteredEvents REPLAY or DISCARD would let that action
        # be acted on twice.
        install_downstream(mocker)
        capacity_guard_rejections(non_durable_rejects=True, durable_rejects=True)
        response_stream, requests = open_stream(servicer, mock_context)

        await _collect_until_refused(response_stream)

        assert requests.reads == 0

    @pytest.mark.asyncio
    @pytest.mark.parametrize("install_downstream, open_stream", REQUEST_STREAMING_RPCS)
    async def test_an_admitted_stream_does_read_from_the_client(
        self,
        install_downstream,
        open_stream,
        servicer,
        mock_context,
        mocker,
        capacity_guard_rejections,
    ):
        # Non-vacuity for the pin above: a recording stream that never
        # incremented would satisfy "reads nothing" while asserting nothing
        # at all.
        install_downstream(mocker)
        capacity_guard_rejections()
        response_stream, requests = open_stream(servicer, mock_context)

        async for _ in response_stream:
            pass

        assert requests.reads > 0

    def test_every_streaming_rpc_has_an_opener(self):
        # ``_STREAM_OPENERS`` is hand-listed, so a streaming RPC added to
        # ``_RPCS`` later would inherit the AUTHENTICATED_RPCS pins
        # automatically while being silently omitted from the ones in this
        # class -- quietly falsifying the "on every streaming RPC" claim the
        # docstring above makes. An async generator function is exactly what
        # a gRPC response stream is served by, so it separates the four
        # streaming RPCs from the three unary CRUD ones.
        streaming = {
            name
            for name in _RPCS
            if inspect.isasyncgenfunction(getattr(EventConsumerServicer, name))
        }

        assert streaming == set(_STREAM_OPENERS)

    def test_every_request_streaming_rpc_is_pinned(self):
        # The guard above closes only half the hole: a client-streaming RPC
        # added later would be forced into ``_STREAM_OPENERS`` by it, yet
        # could still be omitted from the hand-listed
        # ``REQUEST_STREAMING_RPCS`` and so from the two pins that are the
        # only place the "never reads from the client" half of the contract
        # is observable. ``request_iterator`` is what gRPC names the client
        # stream, and ``inspect.signature`` reads it through
        # ``trace_grpc_event_stream`` because that decorator wraps with
        # ``functools.wraps``.
        request_streaming = {
            name
            for name in _RPCS
            if "request_iterator"
            in inspect.signature(getattr(EventConsumerServicer, name)).parameters
        }

        assert request_streaming == {param.id for param in REQUEST_STREAMING_RPCS}

    @pytest.mark.asyncio
    async def test_an_admitted_stream_yields_its_first_response(
        self, servicer, mock_context, mocker, capacity_guard_rejections
    ):
        # Non-vacuity for the "yields nothing" pin: the same opener, driven
        # on the one of the four whose first response needs no broker
        # message -- the non-durable stream opens by handing the caller its
        # reconnect token -- does produce a response to collect.
        _install_consume_non_durable_events(mocker)
        capacity_guard_rejections()
        response_stream, _ = _open_consume_non_durable_events(servicer, mock_context)

        yielded = [response async for response in response_stream]

        assert len(yielded) == 1


class TestQueueIdValidatedOnEveryDurableRpc:
    """
    Every gRPC surface taking a client-supplied queue_id applies the same
    check, before it touches the broker.

    The byte-budget arithmetic behind ``QUEUE_ID_PATTERN`` only holds if
    ALL of them do: a single unvalidated surface reintroduces over-long
    broker names, which fail while the AMQP frame is being built rather
    than at the broker -- so none of the queue-facing error paths ever see
    them, and the client gets a bare ``ValueError`` about a limit it never
    chose.
    """

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", QUEUE_ID_VALIDATING_RPCS)
    @pytest.mark.parametrize(
        "queue_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_a_malformed_queue_id_is_rejected(
        self, rpc, queue_id, servicer, mock_context, mocker
    ):
        install_downstream, drive_rpc = rpc
        install_downstream(mocker)

        with pytest.raises(UnprocessableError):
            await drive_rpc(servicer, mock_context, queue_id)

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", QUEUE_ID_VALIDATING_RPCS)
    @pytest.mark.parametrize(
        "queue_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_a_malformed_queue_id_does_no_broker_work(
        self, rpc, queue_id, servicer, mock_context, mocker
    ):
        # Validation runs before a channel is checked out, so a rejected id
        # costs the durable pool nothing.
        install_downstream, drive_rpc = rpc
        downstream = install_downstream(mocker)

        with pytest.raises(UnprocessableError):
            await drive_rpc(servicer, mock_context, queue_id)

        downstream.assert_not_called()

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", QUEUE_ID_VALIDATING_RPCS)
    async def test_a_malformed_queue_id_maps_to_invalid_argument(
        self, rpc, servicer, mock_context, mocker
    ):
        # The status the client actually receives, through the same
        # ``ErrorDetails`` mapping the exception interceptor aborts with.
        # Asserted per surface because a client that switched from create
        # to update must not have to handle a second error class.
        install_downstream, drive_rpc = rpc
        install_downstream(mocker)

        with pytest.raises(UnprocessableError) as exc_info:
            await drive_rpc(servicer, mock_context, "bad:id")

        assert (
            ErrorDetails(exc_info.value).grpc_code == grpc.StatusCode.INVALID_ARGUMENT
        )

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", QUEUE_ID_VALIDATING_RPCS)
    async def test_a_well_formed_queue_id_is_accepted(
        self, rpc, servicer, mock_context, mocker
    ):
        # The other half: the check rejects what it should and nothing
        # else, so a legal id still reaches the broker.
        install_downstream, drive_rpc = rpc
        downstream = install_downstream(mocker)

        await drive_rpc(servicer, mock_context, "a" * MAX_QUEUE_ID_LENGTH)

        downstream.assert_called_once()


@pytest.fixture
def patched_validate_reconnect_token(mocker):
    """
    Replace the reconnect-token bound check for one test.

    Patched on ``event_consumer_servicer`` -- the module that imported the
    name -- so a rename of the import fails at the patch site instead of
    leaving a test that silently asserts nothing.
    """
    return mocker.patch.object(servicer_module, "validate_reconnect_token")


class TestConsumeNonDurableEventsReconnectTokenBound:
    """
    A reconnect token becomes the non-durable queue's broker name, so its
    length AND its alphabet are constrained at the boundary.

    The alphabet is deliberately wide -- printable ASCII. The token is
    opaque to the client, which only ever echoes back one the server
    minted, so a token that is merely unrecognized must keep reaching the
    lookup and failing as "queue not found" rather than being rejected here
    as malformed. What is rejected is what a broker queue name cannot carry
    at all, which would otherwise die in pamqp's frame builder as a bare
    ``ValueError`` naming neither the field nor the rule.
    """

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "token", MALFORMED_RECONNECT_TOKENS, ids=MALFORMED_RECONNECT_TOKEN_IDS
    )
    async def test_a_malformed_token_is_rejected(
        self, token, servicer, mock_context, mocker
    ):
        _install_consume_non_durable_events(mocker)

        with pytest.raises(UnprocessableError):
            await _drive_consume_non_durable_events(servicer, mock_context, token)

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "token", MALFORMED_RECONNECT_TOKENS, ids=MALFORMED_RECONNECT_TOKEN_IDS
    )
    async def test_a_malformed_token_never_reaches_the_consumer(
        self, token, servicer, mock_context, mocker
    ):
        # The point of checking here: a token the broker cannot name a
        # queue with never gets a channel checked out on its behalf.
        get_instance = _install_consume_non_durable_events(mocker)

        with pytest.raises(UnprocessableError):
            await _drive_consume_non_durable_events(servicer, mock_context, token)

        get_instance.assert_not_called()

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "token", MALFORMED_RECONNECT_TOKENS, ids=MALFORMED_RECONNECT_TOKEN_IDS
    )
    async def test_a_malformed_token_maps_to_invalid_argument(
        self, token, servicer, mock_context, mocker
    ):
        _install_consume_non_durable_events(mocker)

        with pytest.raises(UnprocessableError) as exc_info:
            await _drive_consume_non_durable_events(servicer, mock_context, token)

        assert (
            ErrorDetails(exc_info.value).grpc_code == grpc.StatusCode.INVALID_ARGUMENT
        )

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "token", MALFORMED_RECONNECT_TOKENS, ids=MALFORMED_RECONNECT_TOKEN_IDS
    )
    async def test_the_rejection_does_not_echo_the_token(
        self, token, servicer, mock_context, mocker
    ):
        # The message travels: ``ExceptionInterceptor`` re-logs it before
        # returning it, so a token echoed here reaches the log aggregator
        # on both sides of the call.
        _install_consume_non_durable_events(mocker)

        with pytest.raises(UnprocessableError) as exc_info:
            await _drive_consume_non_durable_events(servicer, mock_context, token)

        assert token not in str(exc_info.value)

    @pytest.mark.asyncio
    async def test_a_maximum_length_token_still_opens_the_stream(
        self, servicer, mock_context, mocker
    ):
        get_instance = _install_consume_non_durable_events(mocker)

        await _drive_consume_non_durable_events(
            servicer, mock_context, MAX_LENGTH_RECONNECT_TOKEN
        )

        get_instance.assert_called_once()

    @pytest.mark.asyncio
    async def test_an_unrecognised_token_of_legal_shape_still_reaches_the_lookup(
        self, servicer, mock_context, mocker
    ):
        # The whole reason the alphabet is printable ASCII rather than the
        # durable id's much narrower set: a token this service did not mint
        # but could have has to fail as NOT_FOUND from the lookup, not as a
        # 422 from this gate.
        get_instance = _install_consume_non_durable_events(mocker)

        await _drive_consume_non_durable_events(
            servicer, mock_context, "not-a-token-we-minted:~!"
        )

        get_instance.assert_called_once()


class TestConsumeNonDurableEventsFreshStreamSendsNoToken:
    """
    An absent token is not a malformed one.

    proto3 has no absent string: a first request carrying no
    ``reconnect_token`` at all -- the ordinary fresh-stream case, which is
    most of this RPC's traffic -- arrives as ``""``. The pattern requires
    at least one character, so validating unconditionally would 422 every
    new stream. The servicer therefore asks whether there IS a token before
    asking whether it is a valid one.

    The REST halves need no such guard: an omitted query parameter or
    header is ``None`` there, and an explicitly empty one is a supplied
    value that names no queue.
    """

    @pytest.mark.asyncio
    async def test_a_fresh_stream_does_not_validate_a_token(
        self, servicer, mock_context, mocker, patched_validate_reconnect_token
    ):
        _install_consume_non_durable_events(mocker)

        await _drive_consume_non_durable_events(servicer, mock_context)

        patched_validate_reconnect_token.assert_not_called()

    @pytest.mark.asyncio
    async def test_a_fresh_stream_opens(self, servicer, mock_context, mocker):
        # The consequence, with the real validator in place: dropping the
        # emptiness guard turns every new stream into an INVALID_ARGUMENT.
        get_instance = _install_consume_non_durable_events(mocker)

        await _drive_consume_non_durable_events(servicer, mock_context)

        get_instance.assert_called_once()

    @pytest.mark.asyncio
    async def test_a_reconnecting_stream_does_validate_its_token(
        self, servicer, mock_context, mocker, patched_validate_reconnect_token
    ):
        # The other half of the guard: skipping validation whenever the
        # field is falsy must not have skipped it for a real token.
        _install_consume_non_durable_events(mocker)

        await _drive_consume_non_durable_events(
            servicer, mock_context, _SECRET_RECONNECT_TOKEN
        )

        patched_validate_reconnect_token.assert_called_once_with(
            _SECRET_RECONNECT_TOKEN
        )

    @pytest.mark.asyncio
    async def test_a_refused_stream_never_validates_the_token(
        self,
        servicer,
        mock_context,
        capacity_guard_rejections,
        patched_authenticate,
        patched_validate_reconnect_token,
    ):
        # Admission stays ahead of everything, so a shed stream is refused
        # with the RESOURCE_EXHAUSTED the proxy retries on -- not with a
        # 422 about a token the pod never had capacity to serve anyway.
        patched_authenticate()
        capacity_guard_rejections(non_durable_rejects=True)

        with pytest.raises(ChannelNotAvailableError):
            await _drive_consume_non_durable_events(
                servicer, mock_context, OVER_LONG_RECONNECT_TOKEN
            )

        patched_validate_reconnect_token.assert_not_called()

    @pytest.mark.asyncio
    async def test_a_rejected_token_is_not_followed_by_field_validation(
        self,
        servicer,
        mock_context,
        patched_authenticate,
        patched_validate_reconnect_token,
    ):
        # An unauthenticated caller is told it is unauthenticated. Reporting
        # a malformed field first would tell a caller with no valid token
        # something about how the service validates its inputs.
        patched_authenticate(side_effect=AuthenticationError("Token expired"))

        with pytest.raises(AuthenticationError):
            await _drive_consume_non_durable_events(
                servicer, mock_context, OVER_LONG_RECONNECT_TOKEN
            )

        patched_validate_reconnect_token.assert_not_called()


@pytest.fixture
def patched_span_attributes(mocker):
    """
    Capture the span attributes an RPC stamps, for one test.

    Patched on the servicer module's own ``common_tracing`` alias so a
    rename of the import fails at the patch site rather than leaving a test
    that silently asserts nothing.
    """
    return mocker.patch.object(
        servicer_module.common_tracing, "add_current_span_attributes"
    )


@pytest.fixture
def patched_logger_info(mocker):
    """Capture the servicer's own INFO lines, for one test."""
    return mocker.patch.object(servicer_module.logger, "info")


class TestDurableRpcsValidateBeforeTheyObserve:
    """
    A rejected queue_id never reaches a log line or a span attribute.

    The id is client-controlled and unbounded until the check runs, so
    observing it first lets one rejected request write an arbitrarily large
    log line -- and lets the caller choose the text that appears in it. The
    assertions are made uniformly across every queue_id-taking RPC, so an
    attribute or a log line added ahead of validation on any one of them
    fails here rather than shipping.
    """

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", QUEUE_ID_VALIDATING_RPCS)
    @pytest.mark.parametrize(
        "queue_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_a_malformed_queue_id_is_never_logged(
        self, rpc, queue_id, servicer, mock_context, mocker, patched_logger_info
    ):
        install_downstream, drive_rpc = rpc
        install_downstream(mocker)

        with pytest.raises(UnprocessableError):
            await drive_rpc(servicer, mock_context, queue_id)

        patched_logger_info.assert_not_called()

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", QUEUE_ID_VALIDATING_RPCS)
    @pytest.mark.parametrize(
        "queue_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_a_malformed_queue_id_never_reaches_a_span_attribute(
        self, rpc, queue_id, servicer, mock_context, mocker, patched_span_attributes
    ):
        install_downstream, drive_rpc = rpc
        install_downstream(mocker)

        with pytest.raises(UnprocessableError):
            await drive_rpc(servicer, mock_context, queue_id)

        patched_span_attributes.assert_not_called()

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", SPAN_STAMPING_QUEUE_ID_RPCS)
    async def test_a_well_formed_queue_id_reaches_a_span_attribute(
        self, rpc, servicer, mock_context, mocker, patched_span_attributes
    ):
        # The positive control for the guard above. Only these two RPCs stamp
        # the id on the span, so without this the "never" assertion would
        # still hold with the stamping -- or the validation -- deleted.
        install_downstream, drive_rpc = rpc
        install_downstream(mocker)

        await drive_rpc(servicer, mock_context, _VALID_QUEUE_ID)

        patched_span_attributes.assert_called_once_with(queue_id=_VALID_QUEUE_ID)

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", QUEUE_ID_VALIDATING_RPCS)
    async def test_a_well_formed_queue_id_is_logged(
        self, rpc, servicer, mock_context, mocker, patched_logger_info
    ):
        # The other half: moving validation earlier must not have dropped
        # the line an operator reads these RPCs' traffic from.
        install_downstream, drive_rpc = rpc
        install_downstream(mocker)

        await drive_rpc(servicer, mock_context, _VALID_QUEUE_ID)

        patched_logger_info.assert_called_once()

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", QUEUE_ID_VALIDATING_RPCS)
    @pytest.mark.parametrize(
        "queue_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_admission_still_precedes_validation(
        self,
        rpc,
        queue_id,
        servicer,
        mock_context,
        mocker,
        capacity_guard_rejections,
    ):
        # A shed stream must be refused with the RESOURCE_EXHAUSTED the
        # proxy retries on, not with a 422 about a field the pod never had
        # capacity to serve.
        install_downstream, drive_rpc = rpc
        install_downstream(mocker)
        capacity_guard_rejections(durable_rejects=True)

        with pytest.raises(ChannelNotAvailableError):
            await drive_rpc(servicer, mock_context, queue_id)

    @pytest.mark.asyncio
    @pytest.mark.parametrize("rpc", QUEUE_ID_VALIDATING_RPCS)
    @pytest.mark.parametrize(
        "queue_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_authentication_still_precedes_validation(
        self,
        rpc,
        queue_id,
        servicer,
        mock_context,
        mocker,
        patched_authenticate,
    ):
        # An unauthenticated caller is told it is unauthenticated, rather
        # than being told how the service validates its inputs.
        install_downstream, drive_rpc = rpc
        install_downstream(mocker)
        patched_authenticate(side_effect=AuthenticationError("Token expired"))

        with pytest.raises(AuthenticationError):
            await drive_rpc(servicer, mock_context, queue_id)


#: The reconnect token a client presents to reattach to its non-durable
#: queue. It is that queue's identifier -- an *authorized* principal that
#: obtains it could attach to the same stream and take a share of its
#: events -- so every assertion below looks for it by name.
_SECRET_RECONNECT_TOKEN = RECONNECT_TOKEN


def _install_reconnecting_stream(mocker, token):
    """
    Arrange a non-durable stream reconnecting with ``token``.

    A REAL ``NonDurableQueueConsumer`` carrying that token as its queue_id,
    so ``queue_ref`` and the broker name are derived the way production
    derives them rather than being handed to the servicer pre-redacted.
    """
    queue = MagicMock(spec=aio_pika.abc.AbstractQueue)
    queue.name = f"event-queue:{token}:nd"
    consumer = NonDurableQueueConsumer(queue, token, MagicMock())

    mocker.patch.object(
        consumer, "consume", side_effect=lambda: _non_durable_events(None)
    )
    mocker.patch.object(consumer, "update_queue")
    mocker.patch.object(consumer, "cleanup_channel")
    mocker.patch.object(NonDurableQueueConsumer, "get_instance", return_value=consumer)
    return consumer


class TestConsumeNonDurableEventsSecretHygiene:
    """
    The reconnect token never becomes observability data.

    Token hygiene: the token is not a credential -- presenting it requires a
    valid bearer token and every delivered event is authorized per principal
    by the permissions policy. It is the queue's identifier, so an
    *authorized* principal that obtains it could attach to the same stream
    and take a share of its events; it is kept out of telemetry as defense
    in depth. A copy in a log line, a span attribute, or a metric attribute
    spreads it to log aggregation, trace storage, the metrics backend, and
    everyone with read access to any of them.
    """

    @pytest.mark.asyncio
    async def test_the_span_carries_the_redacted_queue_reference(
        self, servicer, mock_context, mocker, patched_span_attributes
    ):
        _install_reconnecting_stream(mocker, _SECRET_RECONNECT_TOKEN)

        await _drive_consume_non_durable_events(
            servicer, mock_context, _SECRET_RECONNECT_TOKEN
        )

        assert patched_span_attributes.call_args.kwargs[
            "queue_ref"
        ] == redact_reconnect_token(_SECRET_RECONNECT_TOKEN)

    @pytest.mark.asyncio
    async def test_the_span_does_not_carry_the_token(
        self, servicer, mock_context, mocker, patched_span_attributes
    ):
        _install_reconnecting_stream(mocker, _SECRET_RECONNECT_TOKEN)

        await _drive_consume_non_durable_events(
            servicer, mock_context, _SECRET_RECONNECT_TOKEN
        )

        assert _SECRET_RECONNECT_TOKEN not in str(patched_span_attributes.call_args)

    @pytest.mark.asyncio
    async def test_the_span_carries_no_queue_id_attribute(
        self, servicer, mock_context, mocker, patched_span_attributes
    ):
        # The durable streams keep ``queue_id`` with the real id; this one
        # must not, or a dashboard joining on that key pulls tokens into
        # view alongside ordinary queue ids.
        _install_reconnecting_stream(mocker, _SECRET_RECONNECT_TOKEN)

        await _drive_consume_non_durable_events(
            servicer, mock_context, _SECRET_RECONNECT_TOKEN
        )

        assert "queue_id" not in patched_span_attributes.call_args.kwargs

    @pytest.mark.asyncio
    async def test_the_stream_logs_something_to_inspect(
        self, servicer, mock_context, mocker, caplog
    ):
        # Guards the guard below: an empty capture would make it vacuous.
        _install_reconnecting_stream(mocker, _SECRET_RECONNECT_TOKEN)

        with caplog.at_level(logging.DEBUG):
            await _drive_consume_non_durable_events(
                servicer, mock_context, _SECRET_RECONNECT_TOKEN
            )

        assert caplog.records != []

    @pytest.mark.asyncio
    async def test_no_log_record_names_the_token(
        self, servicer, mock_context, mocker, caplog
    ):
        _install_reconnecting_stream(mocker, _SECRET_RECONNECT_TOKEN)

        with caplog.at_level(logging.DEBUG):
            await _drive_consume_non_durable_events(
                servicer, mock_context, _SECRET_RECONNECT_TOKEN
            )

        # ``record.__dict__`` covers the rendered message, the interpolation
        # args, and anything passed through ``extra`` -- all three reach the
        # log aggregator.
        leaked = [
            record
            for record in caplog.records
            if _SECRET_RECONNECT_TOKEN in str(record.__dict__)
        ]
        assert leaked == []

    @pytest.mark.asyncio
    async def test_the_log_line_names_the_redacted_queue_reference(
        self, servicer, mock_context, mocker, caplog
    ):
        # Redaction that dropped the reference entirely would be safe and
        # useless: an operator has to be able to follow one stream through
        # the logs, which is what the stable reference is for.
        _install_reconnecting_stream(mocker, _SECRET_RECONNECT_TOKEN)

        with caplog.at_level(logging.INFO):
            await _drive_consume_non_durable_events(
                servicer, mock_context, _SECRET_RECONNECT_TOKEN
            )

        expected = f"queue_ref={redact_reconnect_token(_SECRET_RECONNECT_TOKEN)}"
        assert any(expected in record.getMessage() for record in caplog.records)

    @pytest.mark.asyncio
    async def test_a_fresh_connect_logs_no_queue_reference(
        self, servicer, mock_context, mocker, caplog
    ):
        # There is no queue to refer to on a fresh connect; the stream's
        # first response is where the client is given its reference.
        _install_reconnecting_stream(mocker, "")

        with caplog.at_level(logging.INFO):
            await _drive_consume_non_durable_events(servicer, mock_context)

        assert not any("queue_ref=" in record.getMessage() for record in caplog.records)


#: The prefix of the line ``_listen_for_filter_updates`` writes for each
#: request after the first. Named so the assertions below can tell it from
#: the first-request line, which a different code path emits.
_FILTER_UPDATE_LOG_PREFIX = "Received consume non-durable request for filter update:"


async def _reconnecting_filter_update_requests(token):
    """
    A reconnecting client that opens a stream and then changes its filters.

    EVERY request on this stream repeats the reconnect_token -- that is the
    property the second ``describe_non_durable_request`` call site exists
    for. The second request CHANGES the filters, so the rebind is really
    issued and the acknowledgement line is written too; both lines are
    covered by the whole-stream assertions below.
    """
    yield ConsumeNonDurableEventsRequest(
        filter_groups=[_proto_filter_group("e1")],
        reconnect_token=token,
    )
    yield ConsumeNonDurableEventsRequest(
        filter_groups=[_proto_filter_group("e2")],
        previous_filter_groups=[_proto_filter_group("e1")],
        reconnect_token=token,
    )


class TestFilterUpdateLoggingSecretHygiene:
    """
    The token is repeated on every message of the stream, and so is the
    redaction.

    Token hygiene: the token is the queue's identifier, kept out of
    telemetry as defense in depth.

    ``_get_non_durable_consumer`` summarises the FIRST request; a long-
    lived stream's remaining requests go through
    ``_listen_for_filter_updates``, which has its own call to
    ``describe_non_durable_request``. That second site is the one a client
    can make fire arbitrarily often -- one line per filter change, for the
    life of the stream -- so a summary missed there writes the token down
    repeatedly rather than once, and the tests above, which drive a
    one-request stream, would not see it.
    """

    @staticmethod
    async def _drive(servicer, context, token):
        return [
            r
            async for r in servicer.ConsumeNonDurableEvents(
                _reconnecting_filter_update_requests(token), context
            )
        ]

    @pytest.fixture
    def filter_update_records(self, servicer, mock_context, mocker, caplog):
        async def _run():
            _install_reconnecting_stream(mocker, _SECRET_RECONNECT_TOKEN)

            with caplog.at_level(logging.DEBUG):
                await self._drive(servicer, mock_context, _SECRET_RECONNECT_TOKEN)
            return caplog.records

        return _run

    @pytest.mark.asyncio
    async def test_the_filter_update_line_is_emitted(self, filter_update_records):
        # Guards every assertion below: a stream that never reached the
        # second request would make them all vacuous.
        records = await filter_update_records()

        assert [
            r for r in records if r.getMessage().startswith(_FILTER_UPDATE_LOG_PREFIX)
        ] != []

    @pytest.mark.asyncio
    async def test_no_record_on_the_whole_stream_names_the_token(
        self, filter_update_records
    ):
        # ``record.__dict__`` covers the rendered message, the
        # interpolation args, and anything passed through ``extra`` -- all
        # three reach the log aggregator.
        records = await filter_update_records()

        leaked = [r for r in records if _SECRET_RECONNECT_TOKEN in str(r.__dict__)]
        assert leaked == []

    @pytest.mark.asyncio
    async def test_the_filter_update_line_names_the_redacted_reference(
        self, filter_update_records
    ):
        # Redaction that dropped the reference would be safe and useless:
        # a filter change has to be attributable to the stream that made
        # it.
        records = await filter_update_records()
        expected = f"queue_ref={redact_reconnect_token(_SECRET_RECONNECT_TOKEN)}"

        assert any(
            expected in r.getMessage()
            for r in records
            if r.getMessage().startswith(_FILTER_UPDATE_LOG_PREFIX)
        )

    @pytest.mark.asyncio
    async def test_the_filter_update_line_does_not_render_the_request(
        self, filter_update_records
    ):
        # The failure mode being guarded against: interpolating the
        # request itself renders protobuf text format, which prints every
        # populated field including the token. The summary's shape is what
        # tells the two apart.
        records = await filter_update_records()

        assert all(
            "reconnect_token" not in r.getMessage()
            for r in records
            if r.getMessage().startswith(_FILTER_UPDATE_LOG_PREFIX)
        )


#: The handlers' "Received ..." request lines, by RPC. Each test below drives
#: the handler with filter groups that name ``_FILTER_RESOURCE_ID`` and reads
#: only the line that handler writes.
_DURABLE_QUEUE_REQUEST_LOGS = {
    "CreateDurableQueue": (
        _install_create_durable_queue,
        lambda: CreateDurableQueueRequest(
            queue_id=_VALID_QUEUE_ID, filter_groups=[_proto_filter_group("e1")]
        ),
        "Received create durable queue request",
    ),
    "UpdateDurableQueue": (
        _install_update_durable_queue,
        lambda: UpdateDurableQueueRequest(
            queue_id=_VALID_QUEUE_ID,
            current_filter_groups=[_proto_filter_group("e1")],
            new_filter_groups=[_proto_filter_group("e2")],
        ),
        "Received update durable queue request",
    ),
}


class TestDurableQueueRequestLogging:
    """
    The create and update handlers log a summary of the request, not the
    request itself.

    Interpolating the request renders protobuf text format, which prints every
    filter group and so every resource id -- the client's folder paths -- into
    an INFO line.
    """

    @pytest.fixture(params=sorted(_DURABLE_QUEUE_REQUEST_LOGS))
    def request_records(self, request, servicer, mock_context, mocker, caplog):
        install, build_request, prefix = _DURABLE_QUEUE_REQUEST_LOGS[request.param]
        handler = getattr(servicer, request.param)

        async def _run():
            install(mocker)
            with caplog.at_level(logging.INFO, logger=servicer_module.logger.name):
                await handler(build_request(), mock_context)
            return [r for r in caplog.records if r.getMessage().startswith(prefix)]

        return _run

    @pytest.mark.asyncio
    async def test_the_line_is_emitted(self, request_records):
        # Guards the assertion below: an empty capture would make it vacuous.
        records = await request_records()

        assert records != []

    @pytest.mark.asyncio
    async def test_the_line_does_not_name_a_resource(self, request_records):
        records = await request_records()

        assert [r for r in records if _FILTER_RESOURCE_ID in str(r.__dict__)] == []

    @pytest.mark.asyncio
    async def test_the_line_names_the_queue(self, request_records):
        records = await request_records()

        assert all(f"queue {_VALID_QUEUE_ID}" in r.getMessage() for r in records)
