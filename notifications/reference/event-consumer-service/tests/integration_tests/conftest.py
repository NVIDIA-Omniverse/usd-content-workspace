# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import json
import time
from dataclasses import dataclass, field
from unittest.mock import AsyncMock, MagicMock

import aio_pika
import pytest
import pytest_asyncio
from google.protobuf import json_format
from notification_common.auth.auth_tokens import jwt_unverified_decoder
from notification_common.auth.principal_source import PrincipalSource
from notification_common.config.auth_settings import auth_settings

from app.lib.models.event_message import EventMessage
from app.lib.permissions.storage_permissions import storage_permissions
from app.lib.rabbitmq import NonDurableQueueConsumer, DurableQueueConsumer
from app.lib.rabbitmq.ack_tracker import AckTracker
from app.lib.rabbitmq.dead_letter import decode_dead_lettered_message
from app.lib.rabbitmq.dead_letter_consumer import DeadLetterConsumer
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    FilterGroup,
    ResourceFilter,
)
from tests.fixtures.expired_jwt_unverified_token import EXPIRED_TOKEN_PRINCIPAL
from tests.fixtures.reconnect_token import NON_DURABLE_QUEUE_NAME


def filter_group(event_type="e1", resource_id="r1"):
    return FilterGroup(
        event_type=event_type,
        filters=[
            ResourceFilter(
                filter_type=ResourceFilter.FilterType.FILTER_TYPE_EQ,
                resource_id=resource_id,
            ),
        ],
    )


def assert_all_messages_streamed(
    responses, non_durable=False, expected_acknowledgements=0
):
    """
    Assert that the three published events reached the client, in order,
    and that nothing else did.

    A non-durable stream carries three kinds of response: the opening one
    with the reconnect token, one per event, and one filter-update
    acknowledgement per update the client sent. Only the opening response is
    positionally fixed, because it is yielded before the merge;
    ``merge`` (``app.lib.streams``) orders the event side against the
    acknowledgement side not at all, so the events are selected by content
    rather than counted off by index.

    ``expected_acknowledgements`` is how many responses on the stream carry
    a ``filter_update_ack``, and the caller has to know it: it is one per
    mid-stream update the caller's request stream sent, PLUS one on the
    opening response itself when the first request named filter groups --
    ``get_instance`` applies that set before the stream yields anything, so
    its acknowledgement rides the opening response rather than arriving as
    one of its own. Counting them is what keeps a spurious acknowledgement
    -- a duplicate, or one for a request that named no set -- from passing
    unnoticed, which is precisely the regression the acknowledgement feature
    can introduce.

    The accounting is exact: every response after the opening one carries
    events or an acknowledgement and never both, so their two counts have to
    add back up to the number of responses that arrived. A response of any
    other kind fails that sum.
    """
    if non_durable:
        _assert_reconnect_token_response(responses[0])
        after_opening = responses[1:]
        events = [r for r in after_opening if r.events]
        acknowledgements = [r for r in after_opening if r.HasField("filter_update_ack")]

        assert len(after_opening) == len(events) + len(acknowledgements)

        opening_acknowledgements = int(responses[0].HasField("filter_update_ack"))
        assert (
            opening_acknowledgements + len(acknowledgements)
            == expected_acknowledgements
        )

        responses = events
    else:
        # A durable stream carries neither an opening response nor an
        # acknowledgement, so the accounting above is skipped entirely. Say
        # so rather than ignoring the argument: a durable call site that
        # passed a count would otherwise be told its acknowledgements were
        # verified when nothing looked at them.
        assert expected_acknowledgements == 0

    assert len(responses) == 3

    expected_response_e1 = {
        "event_type": "e1",
        "message": {"test": "message"},
        "occurred_at": "2024-12-11T16:56:41.481765Z",
        "published_at": "2024-12-11T16:56:42.481766Z",
    }
    expected_response_e2 = {
        "event_type": "e2",
        "message": {"test": "message"},
        "occurred_at": "2024-12-11T16:56:41.481765Z",
        "published_at": "2024-12-11T16:56:42.481766Z",
    }
    first_response = json_format.MessageToDict(
        responses[0], preserving_proto_field_name=True
    )
    second_response = json_format.MessageToDict(
        responses[1], preserving_proto_field_name=True
    )
    third_response = json_format.MessageToDict(
        responses[2], preserving_proto_field_name=True
    )

    # Only check the event type since timestamps might be different in CI environments
    assert (
        first_response["events"][0]["event_type"] == expected_response_e1["event_type"]
    )
    assert first_response["events"][0]["message"] == expected_response_e1["message"]

    assert (
        second_response["events"][0]["event_type"] == expected_response_e1["event_type"]
    )
    assert second_response["events"][0]["message"] == expected_response_e1["message"]

    assert (
        third_response["events"][0]["event_type"] == expected_response_e2["event_type"]
    )
    assert third_response["events"][0]["message"] == expected_response_e2["message"]


def _assert_reconnect_token_response(response_message):
    first_response = json_format.MessageToDict(
        response_message, preserving_proto_field_name=True
    )
    assert first_response["reconnect_token"] is not None
    assert first_response.get("events") is None

    # The capability flag is a statement about the server, and the first
    # response is where a client reads it -- it is what tells the client
    # whether the acknowledgement-driven resume recipe is available at all.
    assert response_message.supports_filter_update_acks is True


@pytest.fixture
def metadata():
    return [("authorization", "Bearer test-token")]


_EVENT_TYPE_E1_BODY = (
    b'{"event_type": "e1", "message": {"test": "message"}, '
    b'"occurred_at": "2024-12-11T11:56:41.481765-05:00", '
    b'"published_at": "2024-12-11T11:56:42.481766-05:00"}'
)
_EVENT_TYPE_E2_BODY = (
    b'{"event_type": "e2", "message": {"test": "message"}, '
    b'"occurred_at": "2024-12-11T11:56:41.481765-05:00", '
    b'"published_at": "2024-12-11T11:56:42.481766-05:00"}'
)

# The canned durable browse set: two e1 events then one e2, in that order.
# Module-level so the redelivering durable fixture below builds its
# deliveries from the same bodies as the default one.
_EVENT_BODIES = (_EVENT_TYPE_E1_BODY, _EVENT_TYPE_E1_BODY, _EVENT_TYPE_E2_BODY)


async def _create_raw_mock_messages_gen():
    """Generates raw AsyncMock messages with bodies and process context managers."""
    for body_content in _EVENT_BODIES:
        msg_obj = AsyncMock(spec=aio_pika.abc.AbstractIncomingMessage)
        msg_obj.body = body_content

        process_cm = AsyncMock()
        msg_obj.process = MagicMock(return_value=process_cm)

        yield msg_obj


@dataclass
class MockDurableWithAcks:
    """
    Observable hooks for the canned durable with-acks generator.

    :param messages: The raw aio-pika message mocks the generator handed to
        the stream, in delivery order, each carrying its ``ack`` / ``nack``
        spy. A test that asserts a settle APPLIED needs these: response
        counts alone pass whether the settle landed on the message or fell
        through the unknown-tag skip.
    """

    messages: list = field(default_factory=list)


@pytest_asyncio.fixture(autouse=True)
def mock_queue_iterator(mocker, mock_chan, mocked_non_durable_consumer):
    """
    Mocks message consumption and queue objects for RabbitMQ integration tests.
    This is an auto-use fixture, so it's active for all tests in its scope.

    Returns a :class:`MockDurableWithAcks` of hooks, so a test that cares
    can declare the fixture explicitly and assert on the raw messages
    behind the canned durable with-acks stream.
    """
    hooks = MockDurableWithAcks()

    mocker.patch.object(
        NonDurableQueueConsumer,
        "get_instance",
        return_value=mocked_non_durable_consumer,
    )

    # Mock get_queue to avoid the temporary channel check which can fail with
    # "is bound to a different event loop" in tests
    mocker.patch.object(RabbitMQBase, "get_queue")

    async def patched_delayed_acks(_unused_queue_obj, _unused_authz_check, **_kwargs):
        async for msg_obj in _create_raw_mock_messages_gen():
            yield EventMessage(json.loads(msg_obj.body))

    mocker.patch.object(
        RabbitMQBase,
        "_consume_with_delayed_acks",
        side_effect=patched_delayed_acks,
    )

    async def patched_consume_with_acks(
        _unused_queue_obj, _unused_authz_check, ack_tracker, **_kwargs
    ):
        tag = 0
        async for msg_obj in _create_raw_mock_messages_gen():
            tag += 1
            body = json.loads(msg_obj.body)
            ack_tracker.track(msg_obj)
            yield EventMessage(body, delivery_tag=tag)

    mocker.patch.object(
        RabbitMQBase,
        "_consume_with_client_acks",
        side_effect=patched_consume_with_acks,
    )

    async def patched_consume_with_acks_method(queue_id):
        ack_tracker = AckTracker()

        async def _gen():
            # Delivery tags come off a MONOTONIC counter, never off the
            # tracker's in-flight count. A real broker never reuses a tag
            # within a channel incarnation, and the count DROPS whenever the
            # client settles mid-stream -- so counting in flight would hand
            # the next delivery a tag the session had already issued, and a
            # settle for one message would land on another.
            delivery_tag = 0
            async for msg_obj in _create_raw_mock_messages_gen():
                delivery_tag += 1
                msg_obj.delivery_tag = delivery_tag
                tag = ack_tracker.track(msg_obj)
                hooks.messages.append(msg_obj)
                yield EventMessage(json.loads(msg_obj.body), delivery_tag=tag)

        return _gen(), ack_tracker

    mocker.patch.object(
        DurableQueueConsumer,
        "consume_with_acks",
        side_effect=patched_consume_with_acks_method,
    )

    async def patched_consume(queue_id):
        async for msg_obj in _create_raw_mock_messages_gen():
            yield EventMessage(json.loads(msg_obj.body))

    mocker.patch.object(
        DurableQueueConsumer,
        "consume",
        side_effect=patched_consume,
    )

    return hooks


# How many deliveries the redelivering durable fixture produces before its
# generator ends: the three canned events plus the one redelivery a nack
# earns. Bounding it is what makes the stream terminate on its own, so the
# test reads a complete response list instead of cancelling a live stream.
_REDELIVERY_DELIVERY_BUDGET = len(_EVENT_BODIES) + 1

# Ceiling on waiting for the client's nack to reach the fixture. The nack is
# a round trip through the real gRPC bidi stream, so it is normally
# immediate; the bound exists so a regression that never nacks fails as a
# named timeout rather than hanging the suite.
_REDELIVERY_TIMEOUT_S = 15.0


@dataclass
class MockDurableRedelivery:
    """
    Observable hooks for the redelivering durable with-acks fixture.

    :param ack_tracker: The REAL ``AckTracker`` the servicer drives, so the
        production ack / nack / unknown-tag paths run unchanged.
    :param deliveries: ``(delivery_tag, body)`` for every event the fixture
        handed to the stream, in order -- redeliveries included. A redelivered
        message appears twice, under two different tags.
    :param messages: The raw aio-pika message mocks, in delivery order. Each
        carries the ``ack`` / ``nack`` spies; ``nack(requeue=True)`` is the
        hook that actually puts the body back for redelivery.
    """

    ack_tracker: AckTracker = None
    deliveries: list = field(default_factory=list)
    messages: list = field(default_factory=list)


@pytest_asyncio.fixture
def mock_redelivering_durable_queue(mocker, mock_queue_iterator):
    """
    A durable with-acks consume generator that honours ``nack(requeue=True)``
    by redelivering the message on the SAME stream, under a new delivery tag.

    The autouse ``mock_queue_iterator`` yields a fixed three-message script
    and drops nacks on the floor, so it cannot express the nack contract at
    all. This fixture re-patches ``DurableQueueConsumer.consume_with_acks``
    (it depends on ``mock_queue_iterator``, so it applies second and wins)
    with a generator that models the broker's actual requeue behaviour: a
    nacked body goes back on an internal queue and is handed to the stream
    again with a fresh tag.

    OPT-IN, so the existing durable integration tests keep the simpler
    fixture. The generator ends after ``_REDELIVERY_DELIVERY_BUDGET``
    deliveries -- there is no infinite stream to cancel, and the redelivery
    wait is a real ``await`` on the nack rather than a sleep.
    """
    hooks = MockDurableRedelivery()

    async def patched_consume_with_acks(_queue_id):
        ack_tracker = AckTracker()
        hooks.ack_tracker = ack_tracker
        requeued: asyncio.Queue = asyncio.Queue()

        def _build_message(body, delivery_tag):
            message = AsyncMock(spec=aio_pika.abc.AbstractIncomingMessage)
            message.body = body
            message.delivery_tag = delivery_tag

            async def _requeue_on_nack(requeue=True):
                if requeue:
                    requeued.put_nowait(body)

            message.nack = AsyncMock(side_effect=_requeue_on_nack)
            hooks.messages.append(message)
            return message

        async def _deliver(body, delivery_tag):
            ack_tracker.track(_build_message(body, delivery_tag))
            hooks.deliveries.append((delivery_tag, body))
            return EventMessage(json.loads(body), delivery_tag=delivery_tag)

        async def _generator():
            delivery_tag = 0
            for body in _EVENT_BODIES:
                delivery_tag += 1
                yield await _deliver(body, delivery_tag)

            while len(hooks.deliveries) < _REDELIVERY_DELIVERY_BUDGET:
                body = await asyncio.wait_for(
                    requeued.get(), timeout=_REDELIVERY_TIMEOUT_S
                )
                delivery_tag += 1
                yield await _deliver(body, delivery_tag)

        return _generator(), ack_tracker

    mocker.patch.object(
        DurableQueueConsumer,
        "consume_with_acks",
        side_effect=patched_consume_with_acks,
    )

    return hooks


class MockDeadLetterMessage:
    """
    Mock dead-lettered aio-pika message for the DLQ-browse integration path.

    Mirrors the ``MockMessageWithXDeath`` shape from the rabbitmq-layer unit
    fixtures (a JSON body plus an ``x-death`` / ``x-delivery-count`` header set
    and async ``ack`` / ``nack`` spies), but lets each field be overridden so a
    DLQ integration test can build a deterministic browse set with distinct
    origin queues, reasons, and event bodies. The ``ack`` and ``nack``
    ``AsyncMock``s are the per-message observable hooks the test asserts on:
    REPLAY / DISCARD ack the DLQ copy, LEAVE / replay-failure nack it
    ``requeue=True``.

    The real ``AckTracker`` reads ``delivery_tag`` (set when the browse
    generator tracks the message) and the real ``decode_dead_lettered_message``
    reads ``headers`` and ``body``, so the production REPLAY / DISCARD / LEAVE
    code runs unchanged against these mocks.
    """

    def __init__(
        self,
        body,
        origin_queue="event-queue:queue-1",
        reason="rejected",
        delivery_count=3,
        dead_lettered_at=1700000000,
        routing_keys=("events.e1",),
        delivery_tag=0,
    ):
        self.body = body
        self.delivery_tag = delivery_tag
        self.ack = AsyncMock()
        self.nack = AsyncMock()
        self.headers = {
            "x-death": [
                {
                    "queue": origin_queue,
                    "reason": reason,
                    "count": delivery_count,
                    "time": dead_lettered_at,
                    "exchange": "events",
                    "routing-keys": list(routing_keys),
                }
            ],
            "x-delivery-count": delivery_count,
        }


def _default_dead_letter_messages():
    """
    The deterministic browse set the fixture yields when a test does not supply
    its own. Two distinct origin queues / reasons so a test can REPLAY one,
    DISCARD/LEAVE the other, and assert each landed on the right origin queue.
    """
    body_e1 = (
        b'{"event_type": "e1", "resource_id": "r1", '
        b'"principal_identity": "p1", "message": {"test": "message"}}'
    )
    body_e2 = (
        b'{"event_type": "e2", "resource_id": "r2", '
        b'"principal_identity": "p2", "message": {"test": "message"}}'
    )
    return [
        MockDeadLetterMessage(
            body_e1,
            origin_queue="event-queue:queue-1",
            reason="delivery_limit",
        ),
        MockDeadLetterMessage(
            body_e2,
            origin_queue="event-queue:queue-2",
            reason="expired",
        ),
    ]


@dataclass
class MockDeadLetterBrowse:
    """
    Observable hooks the DLQ integration test consumes from ``mock_dlq_browse``.

    :param messages: The raw ``MockDeadLetterMessage`` objects the browse
        generator yields, in order. Each carries the ``ack`` / ``nack``
        ``AsyncMock`` spies the test asserts on (REPLAY/DISCARD ack; LEAVE
        nacks ``requeue=True``).
    :param ack_tracker: The real ``AckTracker`` the servicer drives. After
        browsing, ``ack_tracker.get(tag)`` returns the same raw message keyed
        by its session ``delivery_tag`` (1-based, assignment order).
    :param channel: The mock browse channel handed to the servicer. Its
        ``default_exchange.publish`` is the republish boundary spy.
    :param publish: The ``default_exchange.publish`` ``AsyncMock`` — the
        republish-boundary hook. Assert REPLAY republished to the origin queue
        (``publish.call_args`` carries ``routing_key=<origin_queue>``,
        ``mandatory=True``) before the DLQ copy was acked. Set
        ``publish.side_effect`` to ``aio_pika.exceptions.DeliveryError`` to
        simulate an origin-queue-deleted REPLAY.
    :param message_by_tag: Convenience map of session ``delivery_tag`` -> raw
        message, populated once the generator has yielded.
    """

    messages: list = field(default_factory=list)
    ack_tracker: AckTracker = field(default_factory=AckTracker)
    channel: MagicMock = None
    publish: AsyncMock = None
    message_by_tag: dict = field(default_factory=dict)


@pytest_asyncio.fixture
def mock_dlq_browse(mocker):
    """
    Patch ``DeadLetterConsumer.browse`` for the DLQ-management RPC, analogous to
    the autouse ``mock_queue_iterator`` for the durable with-acks path — but
    OPT-IN (not autouse) so it never perturbs the existing durable / non-durable
    integration tests.

    Patches by object (``mocker.patch.object``) so a rename of ``browse`` breaks
    loudly. The patched ``browse`` returns the production
    ``(generator, AckTracker, channel)`` contract:

    - ``generator`` yields the production ``DecodedDeadLetter`` records (decoded
      by the real ``decode_dead_lettered_message`` from ``MockDeadLetterMessage``
      raw messages), tracking each raw message in the REAL ``AckTracker`` so the
      servicer's REAL ``replay`` / ``discard`` / ``leave`` / ``_origin_queue_for_tag``
      code paths run unchanged and ack / nack the tracked DLQ copy by tag;
    - ``ack_tracker`` is the real tracker (in-flight registry) the servicer
      drives by delivery tag;
    - ``channel`` is a mock whose ``default_exchange.publish`` is an
      ``AsyncMock`` standing in for the publisher-confirm republish boundary:
      a normal return models a publish-confirm (REPLAY then acks the DLQ copy);
      a ``DeliveryError`` side effect models an origin-queue-deleted REPLAY
      (the DLQ copy is left in place via nack ``requeue=True``).

    Returns a :class:`MockDeadLetterBrowse` of observable hooks. A test may
    override the browse set before invoking the RPC by replacing
    ``hooks.messages`` (the generator reads it lazily on first iteration).
    """
    hooks = MockDeadLetterBrowse()
    hooks.messages = _default_dead_letter_messages()

    channel = MagicMock(spec=aio_pika.abc.AbstractChannel)
    publish = AsyncMock(return_value=None)
    channel.default_exchange = MagicMock()
    channel.default_exchange.publish = publish
    hooks.channel = channel
    hooks.publish = publish

    async def patched_browse():
        ack_tracker = hooks.ack_tracker

        async def _generator():
            tag = 0
            for raw_message in hooks.messages:
                tag += 1
                raw_message.delivery_tag = tag
                # Mirror DeadLetterConsumer._browse_with_client_acks: track the
                # raw message (caching the decoded x-death-derived origin queue)
                # and stamp the session delivery tag onto the decoded record so
                # the servicer can REPLAY/DISCARD/LEAVE by tag.
                record = decode_dead_lettered_message(raw_message)
                record.delivery_tag = ack_tracker.track(
                    raw_message, record.origin_queue
                )
                hooks.message_by_tag[record.delivery_tag] = raw_message
                yield record

        return _generator(), ack_tracker, channel

    mocker.patch.object(
        DeadLetterConsumer,
        "browse",
        side_effect=patched_browse,
    )

    return hooks


# ---------------------------------------------------------------------------
# The expired-``jwtUnverified``-token regression.
#
# Shared by the two halves of it -- ``grpc/test_expired_jwt_unverified_token.py``
# and ``rest/test_expired_jwt_unverified_token.py`` -- which sit in sibling
# directories, so this is the nearest conftest they have in common.
#
# Everything below is OPT-IN. The two modules request these fixtures by name
# (and make the source selection autouse for themselves by overriding it), so
# no other integration test is perturbed. The ``mock_principal_id`` override
# those modules carry is deliberately NOT hoisted here: it retires the
# suite-wide principal stub, which those modules need and every other
# integration test relies on.
# ---------------------------------------------------------------------------

#: The real ``get_instance``, captured at import time. The autouse
#: ``mock_queue_iterator`` fixture above replaces it with a stand-in consumer
#: for every integration test, which would skip the very resolution the
#: non-durable half of the regression is about.
_REAL_GET_INSTANCE = NonDurableQueueConsumer.get_instance

#: The id the stubbed broker reports for a freshly created durable queue.
CREATED_QUEUE_ID = "expired-token-regression-queue"


@pytest.fixture
def jwt_unverified_source(mocker):
    """
    Select the opt-in ``jwtUnverified`` ``principal_source``.

    The suite's module-level baseline is ``jwt``, whose decoder refuses an
    expired token up front -- so the case under test only exists under
    ``jwtUnverified``, where expiry is deliberately not judged.
    """
    return mocker.patch.object(
        auth_settings, "principal_source", PrincipalSource.JWT_UNVERIFIED
    )


@pytest.fixture
def lapsed_cache_lifetime(mocker):
    """
    Make the decoder hand back a cache lifetime that has ALREADY lapsed.

    Two purposes, and neither of them is pinning the raise as behaviour
    this service wants.

    **Vacuity control.** Reproducing the 6.11.0 expiry shape is what proves
    the modules using this fixture are not passing vacuously. The
    300-second lifetime is the whole fix -- before it, this decoder
    returned the token's own ``exp``, which for an expired token is exactly
    the lapsed value stubbed here. So a request that fails under this
    fixture and succeeds without it is a request whose success is genuinely
    attributable to the derived lifetime, and the positive arms alongside
    it are running real resolution rather than a stub.

    **Tripwire for notification_common Issue b.3z5 item 1.** Today a cached
    read raises ``AuthenticationError("Token expired")`` once the stored
    lifetime is behind it, which at this source reports the cache entry
    going stale rather than the token lapsing. That is a known library
    category error, not a contract: b.3z5 item 1 will have
    ``_cached_principal_id()`` fall through and re-resolve on a stale entry
    instead of raising. When that lands, the arms driven by this fixture
    start failing -- which is the signal to revisit them, not a regression
    to chase.
    """
    lapsed = time.time() - 1
    return mocker.patch.object(
        jwt_unverified_decoder,
        "get_user_id",
        return_value=(EXPIRED_TOKEN_PRINCIPAL, lapsed),
    )


@pytest.fixture
def decodes(mocker):
    """
    Count the decodes a request performs.

    Spied on the ``jwtUnverified`` decoder rather than on ``principal_id``,
    so the resolutions that are answered from the per-request cache are
    exactly the ones that do NOT show up here -- which is what makes the
    count a statement about caching rather than about call sites.
    """
    return mocker.spy(jwt_unverified_decoder, "get_user_id")


@pytest.fixture
def durable_broker(mocker, mock_chan):
    """
    Stub the broker work ``create_queue`` does AFTER it has resolved the
    principal, leaving the resolution itself real.
    """
    queue = mocker.MagicMock()
    queue.name = f"event-queue:{CREATED_QUEUE_ID}"
    mocker.patch.object(
        DurableQueueConsumer,
        "_create_queue",
        return_value=(CREATED_QUEUE_ID, queue),
    )

    channel_context = mocker.AsyncMock()
    channel_context.__aenter__.return_value = mock_chan
    mocker.patch.object(
        DurableQueueConsumer.connector, "get_channel", return_value=channel_context
    )
    return queue


@pytest.fixture
def non_durable_broker(mocker, mock_queue_iterator, mock_chan):
    """
    Put the REAL ``get_instance`` back and stub only the broker work it
    does around the resolution.

    Depends on ``mock_queue_iterator`` so this patch lands after that
    autouse fixture's stand-in consumer, which would otherwise answer the
    attach without ever resolving the principal.
    """
    queue = mocker.MagicMock()
    queue.name = NON_DURABLE_QUEUE_NAME

    mocker.patch.object(NonDurableQueueConsumer, "get_instance", _REAL_GET_INSTANCE)
    mocker.patch.object(RabbitMQBase, "get_queue", return_value=queue)
    mocker.patch.object(
        NonDurableQueueConsumer.connector, "get_channel_direct", return_value=mock_chan
    )
    mocker.patch.object(NonDurableQueueConsumer.connector, "return_channel")
    mocker.patch.object(storage_permissions, "initialize")
    return queue
