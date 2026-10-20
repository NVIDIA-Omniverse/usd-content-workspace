# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REAL-broker integration test for the client nack chain (b.qsa).

One live episode drives the whole documented chain, end to end, and every
assertion below reads a field off it:

  a delivery tag arrives in ``nack_delivery_tags`` on a real
  ``ConsumeDurableEventsWithAcksRequest``
    -> the servicer's REAL ``_listen_for_durable_acks`` settles it through the
       REAL ``AckTracker``
    -> the REAL ``aio_pika`` message is nacked ``requeue=True`` on a REAL
       quorum queue
    -> the broker redelivers it ON THE SAME live consume stream, with a fresh
       delivery tag
    -> repeated nacks push the message's cumulative delivery count past the
       queue's delivery limit
    -> the broker dead-letters it into the service's DLQ with reason
       ``delivery_limit``.

Only the gRPC transport is out of frame: the request objects are the real
protobuf messages and the settle path is the production code that reads them.
Everything downstream of that -- tracker, aio-pika, quorum queue, delivery
counting, dead-lettering -- is real. This is the arm that proves the outcomes
the black-box conformance suite structurally cannot reach (it has no way to
publish an event into a durable queue; see that suite's module docstring).

Scope/isolation notes:

- The autouse fakes from the parent conftests are bypassed the same way the
  sibling live modules do it: the real ``_consume_with_client_acks`` is
  captured at import time, before ``mock_queue_iterator`` replaces the
  attribute, and all broker traffic runs on the ``class_live_broker_connection``
  fixture opened with the real ``connect_robust``.
- The origin queue is ``uuid``-named, the published message carries a ``uuid``
  correlation id, and the dead-letter topology this episode points at is a
  ``uuid``-named DLX/DLQ pair of its own -- the same type, arguments and
  routing key the service provisions, only the names are private. That last
  part is what makes the module safe under ``pytest -n auto``: xdist hands
  each of this module's tests to a different worker and each worker builds its
  own copy of the episode below, so with a shared DLQ every worker's poller
  was competing for every other worker's dead letter. See the scope notes in
  ``test_dead_letter_capture.py`` for the measured failure that caused.
  Teardown deletes the queue and the topology.
- ``OMNI_EVENTS_REQUIRE_LIVE_BROKER`` governs the no-broker case, via
  ``_connect_or_skip`` in this package's ``conftest.py``: set means a missing
  broker is a hard failure, unset means a clean skip.
"""

import asyncio
import uuid
from types import SimpleNamespace

import aio_pika
import pytest
import pytest_asyncio

from app.config import settings
from app.grpc.event_consumer_servicer import EventConsumerServicer
from app.lib.rabbitmq.ack_tracker import AckTracker
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ConsumeDurableEventsWithAcksRequest,
)
from tests.integration_tests.rabbitmq.test_dead_letter_capture import (
    _drain_dlq_for,
    _origin_queue_args,
    _x_death_for_origin,
    _declare_dead_letter_topology,
    _delete_dead_letter_topology,
    _private_topology,
)

# Captured at import time, before the autouse ``mock_queue_iterator`` fixture
# in ``tests/integration_tests/conftest.py`` replaces the attribute with a
# canned generator. Calling this reference runs the REAL consume loop.
_REAL_CONSUME_WITH_CLIENT_ACKS = RabbitMQBase._consume_with_client_acks

_ORIGIN_QUEUE_PREFIX = "test-nack-origin"

# The queue's delivery limit for this episode. Quorum queues dead-letter a
# message once its delivery count EXCEEDS the limit, so 2 means the third
# delivery's nack is the one that dead-letters it -- small enough to keep the
# episode quick, above 1 so a redelivery is genuinely observed first.
DELIVERY_LIMIT = 2

# Hard ceiling on the nack loop, so a fix that never dead-letters ends the
# episode as a bounded record instead of spinning against the broker.
MAX_DELIVERY_ATTEMPTS = DELIVERY_LIMIT + 3

# How long to wait for each (re)delivery to come out of the real consume
# generator. Generous against broker/CI latency, but bounded so a message that
# is never redelivered ends the loop rather than hanging the suite.
DELIVERY_TIMEOUT_S = 15.0

# Held at the settings floor (``consumer_liveness_poll_seconds`` is ``ge=1.0``)
# so the idle liveness poll runs often during the episode: the nack chain must
# survive a consumer that is being actively health-checked.
TEST_POLL_SECONDS = 1.0

PROBE_BODY = b'{"event_type": "nack-redelivery-probe", "message": {"test": "m"}}'

# The two ways "no further delivery arrived" reaches the caller: the wait
# budget lapsed, or the generator ended. Kept as a module-level tuple so the
# `except` below stays a NAME -- Black 25 strips the parens from an inline
# `except (A, B):` under py314 targeting, and the stripped form is a
# SyntaxError under 3.13-grammar tooling (see b.8qg).
_NO_MORE_DELIVERIES_EXC = (asyncio.TimeoutError, StopAsyncIteration)


async def _publish_probe(connection, queue_name, correlation_id):
    """Publish the single probe message to the origin queue, tagged for the DLQ."""
    channel = await connection.channel()
    try:
        await channel.default_exchange.publish(
            aio_pika.Message(
                body=PROBE_BODY,
                correlation_id=correlation_id,
                delivery_mode=aio_pika.DeliveryMode.PERSISTENT,
            ),
            routing_key=queue_name,
        )
    finally:
        await channel.close()


async def _nack_through_the_grpc_field(ack_tracker, delivery_tag):
    """
    Settle one tag exactly as a gRPC client would: a real
    ``ConsumeDurableEventsWithAcksRequest`` carrying it in
    ``nack_delivery_tags``, fed to the production ``_listen_for_durable_acks``.

    Going through the servicer rather than calling ``ack_tracker.nack``
    directly is the point -- it is what puts the proto field, the ack-then-nack
    loop ordering, and the unknown-tag handling in the path under test.
    """

    async def _requests():
        yield ConsumeDurableEventsWithAcksRequest(nack_delivery_tags=[delivery_tag])

    async for _ in EventConsumerServicer._listen_for_durable_acks(
        _requests(), ack_tracker
    ):
        pass


async def _next_delivery(generator):
    """The next event off the live stream, or ``None`` if none arrives in time."""
    try:
        return await asyncio.wait_for(anext(generator), timeout=DELIVERY_TIMEOUT_S)
    except _NO_MORE_DELIVERIES_EXC:
        return None


async def _best_effort_delete_queue(connection, queue_name):
    """Delete a test-created queue on its own channel; failures are ignored."""
    channel = await connection.channel()
    try:
        await channel.queue_delete(queue_name)
    except Exception:  # noqa: BLE001 - cleanup is best-effort
        pass
    finally:
        try:
            await channel.close()
        except Exception:  # noqa: BLE001 - cleanup is best-effort
            pass


@pytest_asyncio.fixture(scope="module", loop_scope="module")
async def nack_redelivery_episode(module_live_broker_connection, module_mocker):
    """
    Drive one message through the full nack chain against the live broker and
    record what happened.

    Steps:

    1. Declare a uuid-named DLX/DLQ pair (the service's own exchange type,
       DLQ arguments and routing key under private names) and a uuid-named
       quorum origin queue pointed at it, carrying the service's own DLX
       arguments plus a low ``x-delivery-limit``.
    2. Publish one correlation-id-tagged probe message.
    3. Start the REAL ``_consume_with_client_acks`` on the queue with a real
       ``AckTracker``, and loop: take the delivery off the stream, nack it
       through the real gRPC request field, take the redelivery, and so on
       until the broker stops redelivering (it dead-lettered the message) or
       the attempt ceiling is reached.
    4. Poll this episode's own DLQ for our correlation id.

    Records rather than asserts, so the expensive live drive runs once and
    each test below reads one field of it.

    MODULE-scoped, not class-scoped, because the assertions that read it are
    split across two classes -- the redelivery contract and the
    dead-lettering one. At class scope the drive ran twice for one episode's
    worth of findings. Measured against a live broker under ``-n0``: 30.5s
    (two ~15.1s setups) before, 15.6s (one) after. Each drive costs a full
    ``DELIVERY_TIMEOUT_S`` on its last wait, because the point at which the
    broker stops redelivering IS a delivery that never arrives.

    CAVEAT, per the b.9gj lesson recorded in ``test_consumer_liveness.py``'s
    module docstring: ``pytest.ini`` sets ``addopts = -n auto`` and xdist's
    default ``--dist load`` hands out tests individually, so under the repo
    default each worker builds its own copy of this fixture and the module
    still drives one rig per test. The widening is therefore worth exactly
    the ``-n0`` and ``--dist loadscope`` cases -- which is where a developer
    debugging this module actually runs it -- and costs nothing under the
    default. Changing that scheduler default is a repo-wide decision, not
    this module's to make.
    """
    connection = module_live_broker_connection

    module_mocker.patch.object(
        settings.settings, "consumer_liveness_poll_seconds", TEST_POLL_SECONDS
    )
    module_mocker.patch.object(RabbitMQBase.connector, "_connection", connection)
    module_mocker.patch.object(RabbitMQBase.connector, "closing", False)

    topology = _private_topology()
    await _declare_dead_letter_topology(connection, topology)

    queue_name = f"{_ORIGIN_QUEUE_PREFIX}-{uuid.uuid4()}"
    correlation_id = str(uuid.uuid4())

    consume_channel = None
    generator = None
    try:
        consume_channel = await connection.channel()
        queue = await consume_channel.declare_queue(
            queue_name,
            durable=True,
            robust=False,
            arguments=_origin_queue_args(
                topology, **{"x-delivery-limit": DELIVERY_LIMIT}
            ),
        )

        await _publish_probe(connection, queue_name, correlation_id)

        async def _authz(_message):
            return True

        ack_tracker = AckTracker()
        generator = _REAL_CONSUME_WITH_CLIENT_ACKS(queue, _authz, ack_tracker)

        deliveries = []
        delivery_counts = []
        for _ in range(MAX_DELIVERY_ATTEMPTS):
            event_message = await _next_delivery(generator)
            if event_message is None:
                break
            deliveries.append(event_message)
            # Read the broker's own cumulative delivery count off the raw
            # message while it is still tracked -- the settle below pops it.
            # The header is absent on a first delivery, which is the broker
            # spelling "0"; normalising here keeps the recorded series
            # comparable across deliveries.
            raw_message = ack_tracker.get(event_message.delivery_tag)
            headers = raw_message.headers or {}
            delivery_counts.append(headers.get("x-delivery-count") or 0)
            await _nack_through_the_grpc_field(ack_tracker, event_message.delivery_tag)

        dead_letter = await _drain_dlq_for(connection, topology, correlation_id)

        yield SimpleNamespace(
            queue_name=queue_name,
            dlq_name=topology.dlq_name,
            correlation_id=correlation_id,
            deliveries=deliveries,
            delivery_counts=delivery_counts,
            dead_letter=dead_letter,
            x_death=(
                None
                if dead_letter is None
                else _x_death_for_origin(dead_letter, queue_name)
            ),
        )
    finally:
        if generator is not None:
            await generator.aclose()
        await _best_effort_delete_queue(connection, queue_name)
        await _delete_dead_letter_topology(connection, topology)
        if consume_channel is not None:
            try:
                await consume_channel.close()
            except Exception:  # noqa: BLE001 - cleanup is best-effort
                pass


class TestClientNackRedelivery:
    """A nack sent in nack_delivery_tags requeues the message for redelivery."""

    @pytest.mark.asyncio
    async def test_the_message_was_delivered_at_least_once(
        self, nack_redelivery_episode
    ):
        # The non-vacuity control: everything else this module measures is
        # meaningless if the live rig never delivered the probe at all.
        assert nack_redelivery_episode.deliveries, (
            "the real consume generator never delivered the probe message; "
            "the live rig is broken, so nothing this module measures would "
            "mean anything"
        )

    @pytest.mark.asyncio
    async def test_a_nacked_message_comes_back_on_the_same_stream(
        self, nack_redelivery_episode
    ):
        # Redelivery is immediate and lands on the stream that nacked it --
        # the documented behaviour clients must be ready for (a retry delay is
        # theirs to implement, by backing off before nacking).
        assert len(nack_redelivery_episode.deliveries) > 1, (
            "a message nacked with requeue=True was never redelivered on the "
            "same stream; the broker only handed it over "
            f"{len(nack_redelivery_episode.deliveries)} time(s)"
        )

    @pytest.mark.asyncio
    async def test_the_redelivery_carries_the_same_event(self, nack_redelivery_episode):
        event_types = {
            delivery.event_type for delivery in nack_redelivery_episode.deliveries
        }

        assert event_types == {"nack-redelivery-probe"}

    @pytest.mark.asyncio
    async def test_each_redelivery_carries_a_fresh_delivery_tag(
        self, nack_redelivery_episode
    ):
        # Tags are per-channel and monotonic, so a redelivery is a NEW tag.
        # A client that re-sent the old tag would get the unknown-tag skip,
        # which is why the tag on each response is the one it must settle.
        tags = [
            delivery.delivery_tag for delivery in nack_redelivery_episode.deliveries
        ]

        assert len(set(tags)) == len(
            tags
        ), f"delivery tags repeated across deliveries: {tags}"


class TestRepeatedNacksDeadLetter:
    """
    Nacks feed the same cumulative delivery count every other requeue feeds,
    so a client that keeps nacking eventually dead-letters the message rather
    than looping on it forever.
    """

    @pytest.mark.asyncio
    async def test_the_broker_stopped_redelivering(self, nack_redelivery_episode):
        # The loop is bounded by MAX_DELIVERY_ATTEMPTS; hitting that ceiling
        # would mean the message was still being redelivered, i.e. the
        # delivery limit never bit.
        assert len(nack_redelivery_episode.deliveries) < MAX_DELIVERY_ATTEMPTS, (
            "the message was still being redelivered after "
            f"{MAX_DELIVERY_ATTEMPTS} nacks; the delivery limit of "
            f"{DELIVERY_LIMIT} never took effect"
        )

    @pytest.mark.asyncio
    async def test_the_message_lands_in_the_dead_letter_queue(
        self, nack_redelivery_episode
    ):
        assert nack_redelivery_episode.dead_letter is not None, (
            "a message nacked past the queue's delivery limit did not appear "
            f"in the DLQ {nack_redelivery_episode.dlq_name!r}"
        )

    @pytest.mark.asyncio
    async def test_the_dead_letter_records_the_origin_queue(
        self, nack_redelivery_episode
    ):
        assert nack_redelivery_episode.x_death is not None, (
            "the dead-lettered message has no x-death entry for origin queue "
            f"{nack_redelivery_episode.queue_name!r}"
        )

    @pytest.mark.asyncio
    async def test_the_dead_letter_reason_is_delivery_limit(
        self, nack_redelivery_episode
    ):
        # The reason is what the DLQ-browse surface reports to an operator
        # triaging the message, and "delivery_limit" is what distinguishes a
        # client that kept failing from a TTL expiry or a max-length eviction.
        assert nack_redelivery_episode.x_death["reason"] == "delivery_limit"

    @pytest.mark.asyncio
    async def test_every_nack_advanced_the_delivery_count(
        self, nack_redelivery_episode
    ):
        # The measured basis of the "lifetime budget, not a per-nack one"
        # claim: the broker's x-delivery-count on each successive delivery of
        # the SAME message is strictly increasing, so nacks accumulate toward
        # the limit rather than resetting it. Read on the origin side because
        # that is where the broker reports it -- the dead-lettered copy
        # carries x-death (one entry per queue/reason) and no delivery count.
        counts = nack_redelivery_episode.delivery_counts

        assert counts == sorted(set(counts)), (
            "the broker's x-delivery-count did not strictly increase across "
            f"redeliveries of the same message: {counts}"
        )

    @pytest.mark.asyncio
    async def test_the_last_delivery_reached_the_queues_limit(
        self, nack_redelivery_episode
    ):
        # The dead-lettering was caused by THIS queue's delivery limit, not by
        # some unrelated broker condition: the final delivery the stream saw
        # already carried a count at the limit, so the nack that followed
        # pushed it over.
        assert nack_redelivery_episode.delivery_counts[-1] == DELIVERY_LIMIT
