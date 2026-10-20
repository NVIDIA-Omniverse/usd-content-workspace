# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REAL-broker integration tests for durable-queue dead-letter capture.

These verify the SR-1 implementation end to end against a live RabbitMQ 4.x
broker: a durable origin queue is declared quorum with the service's DLX
arguments (matching ``RabbitMQBase._create_queue``), and a message that is

- redelivered past its ``x-delivery-limit`` (reason ``delivery_limit``),
- expired by its ``x-message-ttl`` (reason ``expired``), or
- dropped (head) by ``x-max-length`` overflow (reason ``maxlen``)

is removed from its origin queue and lands in the dead-letter queue its origin
queue points at, tagged with the corresponding ``x-death`` reason and its
origin-queue name.

Quorum ``delivery-limit`` dead-lettering requires RabbitMQ 4.x; the topology
under assertion (DLX/DLQ arguments, routing key, origin-queue args) is read
from ``app/config/settings.py`` and the provisioning code, not hard-coded.

Scope/isolation notes -- why each reason test gets its OWN DLX/DLQ pair:

- An AMQP queue is a destructive-read work queue, so it cannot be observed
  concurrently by independent parties: whoever takes a message owns it until
  it is settled. Every test in this package that watched the single global
  ``events.dlq`` was therefore competing for the SAME messages, and under
  ``pytest -n auto`` that competition deadlocked -- each poller checked out
  some other test's dead letter, held it while hunting for its own, and left
  the queue with nothing ready for anyone. Measured on a live broker: ten
  concurrent pollers, ten dead letters, ``messages_ready=0``,
  ``messages_unacknowledged=9`` spread over eight channels for the pollers'
  entire timeout, then everyone failing with "did not appear in the DLQ".
- So each reason test declares its own ``uuid``-named DLX/DLQ pair and points
  its origin queue's ``x-dead-letter-exchange`` at it -- exactly the shape a
  production origin queue has, just not the shared *name*. Nothing else can
  reach that queue, so there is no contention at any worker count.
- ``_drain_dlq_for`` never holds and never requeues: it acks every message it
  takes. Holding is what deadlocked the shared queue; requeueing is what
  burned the DLQ's quorum ``delivery-limit`` budget before that (a foreign
  message nack-requeued once per poll interval was discarded by the broker on
  the 21st hand-back). Acking is safe here precisely because the queue is
  private.
- ``TestSharedGlobalDeadLetterQueue`` keeps the one thing per-test topology
  gives up -- that the service's OWN configured ``events.dlx``/``events.dlq``
  names really are a working dead-letter path -- and is serialised across
  workers by a broker-held mutex, so it is safe however many workers run and
  however the scheduler distributes them.
- The autouse fakes from the parent conftests are bypassed; see this
  module's ``conftest.py``. All broker traffic here goes through ``aio_pika``
  on the ``live_broker_connection`` fixture.
"""

import asyncio
import uuid
from types import SimpleNamespace

import aio_pika
import pytest
import pytest_asyncio

from app.config import settings
from app.lib.rabbitmq.event_consumer_connector import EventConsumerConnector

_ORIGIN_QUEUE_PREFIX = "test-dlq-origin"

# How long to poll a DLQ for our own dead-letter before giving up.
_DLQ_POLL_TIMEOUT_S = 30.0
_DLQ_POLL_INTERVAL_S = 0.25


def _topology(dlx_name, dlq_name):
    """A DLX/DLQ/routing-key triple that the helpers below operate on."""
    return SimpleNamespace(
        dlx_name=dlx_name,
        dlq_name=dlq_name,
        # Always the service's own routing key: only the NAMES are made
        # private below, never the wiring under test.
        routing_key=settings.settings.dead_letter_routing_key,
    )


# The service's real, shared dead-letter topology, read from settings rather
# than hard-coded. Only ``TestSharedGlobalDeadLetterQueue`` uses it.
_SHARED_TOPOLOGY = _topology(
    settings.settings.dead_letter_exchange_name,
    settings.settings.dead_letter_queue_name,
)


def _private_topology():
    """
    A ``uuid``-named DLX/DLQ pair, private to one test.

    Same type, same arguments, same routing key as ``_SHARED_TOPOLOGY``; only
    the names differ, which is what makes it unreachable by any other worker.
    """
    suffix = uuid.uuid4()
    return _topology(f"test-dlq-dlx-{suffix}", f"test-dlq-dlq-{suffix}")


def _origin_queue_args(topology, **extra) -> dict:
    """
    The quorum + DLX argument set that ``RabbitMQBase._create_queue`` applies
    to durable origin queues -- pointed at ``topology``'s exchange -- plus any
    test-specific bound (``extra``).
    """
    args = {
        "x-queue-type": "quorum",
        "x-dead-letter-exchange": topology.dlx_name,
        "x-dead-letter-routing-key": topology.routing_key,
    }
    args.update(extra)
    return args


async def _declare_dead_letter_topology(connection, topology):
    """
    Idempotently declare ``topology``'s DLX, DLQ, and DLX->DLQ binding over the
    live connection, mirroring ``EventConsumerConnector._declare_dead_letter_topology``
    exactly: the exchange type, durability and routing key are the service's,
    and the DLQ's arguments come from the connector's own
    ``_dead_letter_queue_arguments()``, so this cannot drift from the
    provisioning code.

    The connector's own ``connect()`` provisioning path can't be used directly
    here because the autouse ``before_each`` fixture patches
    ``aio_pika.connect_robust`` to a mock, so the connector would never reach
    the live broker. Declaring against ``connection`` keeps the test on the
    real broker while asserting against the exact provisioned topology.
    """
    channel = await connection.channel()
    try:
        exchange = await channel.declare_exchange(
            topology.dlx_name, type="direct", durable=True
        )
        queue = await channel.declare_queue(
            topology.dlq_name,
            durable=True,
            arguments=EventConsumerConnector._dead_letter_queue_arguments(),
        )
        await queue.bind(exchange, topology.routing_key)
    finally:
        await channel.close()


async def _delete_dead_letter_topology(connection, topology):
    """Best-effort teardown of a private DLX/DLQ pair."""
    await _delete_queue(connection, topology.dlq_name)
    await _delete_exchange(connection, topology.dlx_name)


@pytest_asyncio.fixture
async def private_dead_letter_topology(live_broker_connection):
    """A DLX/DLQ pair declared for one test and deleted afterwards."""
    topology = _private_topology()
    await _declare_dead_letter_topology(live_broker_connection, topology)
    try:
        yield topology
    finally:
        await _delete_dead_letter_topology(live_broker_connection, topology)


async def _publish_to_origin(connection, origin_queue_name, correlation_id):
    """
    Publish a single persistent message to the origin queue via the default
    exchange (routing key == queue name), tagged with ``correlation_id`` so
    it can be identified once it is dead-lettered.
    """
    channel = await connection.channel()
    try:
        await channel.default_exchange.publish(
            aio_pika.Message(
                body=b'{"event_type": "dead-letter-test"}',
                correlation_id=correlation_id,
                delivery_mode=aio_pika.DeliveryMode.PERSISTENT,
            ),
            routing_key=origin_queue_name,
        )
    finally:
        await channel.close()


async def _drain_dlq_for(connection, topology, correlation_id):
    """
    Poll ``topology``'s DLQ for the message bearing ``correlation_id``, and
    return it (acked) or ``None`` on timeout.

    EVERY message this takes is acked, ours or not. That is the load-bearing
    property, not a shortcut, and it is what makes the poll safe at any worker
    count. The two alternatives both broke under ``-n auto`` against the
    shared global DLQ:

    - Nack-requeueing each foreign message spends one of the DLQ's quorum
      ``delivery-limit`` hand-backs per poll interval. Measured against this
      deployment's RabbitMQ 4.x, the default budget is 20: a poller running
      beside another worker's episode burned it in about five seconds and the
      broker DISCARDED the other worker's dead letter.
    - Holding foreign messages unacked until the channel closes fixed the
      budget burn and introduced a deadlock instead. With N concurrent
      pollers and N dead letters, each poller checks out a message that is
      not its own and keeps it; the queue goes empty-ready; nobody ever sees
      their own message; all N time out together. Measured on a live broker
      at N=10: ``messages_ready=0``, ``messages_unacknowledged=9`` across
      eight channels, held for the full poll window.

    Acking unconditionally has neither failure mode -- nothing is held and
    nothing is requeued -- and it is correct because callers pass a topology
    no other test can reach. The one caller that passes ``_SHARED_TOPOLOGY``
    holds the broker mutex (see ``shared_dead_letter_queue_lock``), so it too
    is the only party reading that queue; acking a stray left behind by an
    earlier run is the right disposition there as well.
    """
    channel = await connection.channel()
    try:
        dlq = await channel.get_queue(topology.dlq_name, ensure=False)
        loop = asyncio.get_event_loop()
        deadline = loop.time() + _DLQ_POLL_TIMEOUT_S
        while loop.time() < deadline:
            message = await dlq.get(fail=False, timeout=5)
            if message is None:
                await asyncio.sleep(_DLQ_POLL_INTERVAL_S)
                continue
            await message.ack()
            if message.correlation_id == correlation_id:
                return message
        return None
    finally:
        await channel.close()


def _x_death_for_origin(message, origin_queue_name):
    """
    Return the ``x-death`` entry recorded for ``origin_queue_name`` from a
    dead-lettered message's headers, or ``None`` if absent. RabbitMQ records
    one ``x-death`` entry per (queue, reason) pair.
    """
    headers = message.headers or {}
    x_death = headers.get("x-death") or []
    for entry in x_death:
        if entry.get("queue") == origin_queue_name:
            return entry
    return None


async def _delete_queue(connection, queue_name):
    """Best-effort cleanup of a test-created queue, on its own channel."""
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


async def _delete_exchange(connection, exchange_name):
    """
    Best-effort cleanup of a test-created exchange.

    A fresh channel per deletion: a failed delete closes the channel and would
    otherwise take any later cleanup step down with it.
    """
    channel = await connection.channel()
    try:
        await channel.exchange_delete(exchange_name)
    except Exception:  # noqa: BLE001 - cleanup is best-effort
        pass
    finally:
        try:
            await channel.close()
        except Exception:  # noqa: BLE001 - cleanup is best-effort
            pass


class TestDeliveryLimitDeadLetter:
    """A message redelivered past x-delivery-limit lands in the DLQ."""

    @pytest.mark.asyncio
    async def test_redelivered_past_limit_is_dead_lettered(
        self, live_broker_connection, private_dead_letter_topology
    ):
        connection = live_broker_connection
        topology = private_dead_letter_topology

        origin_name = f"{_ORIGIN_QUEUE_PREFIX}-dl-{uuid.uuid4()}"
        correlation_id = str(uuid.uuid4())
        delivery_limit = 2

        declare_channel = await connection.channel()
        try:
            await declare_channel.declare_queue(
                origin_name,
                durable=True,
                arguments=_origin_queue_args(
                    topology, **{"x-delivery-limit": delivery_limit}
                ),
            )
        finally:
            await declare_channel.close()

        try:
            await _publish_to_origin(connection, origin_name, correlation_id)

            # Consume-and-nack-with-requeue until the broker's delivery count
            # exceeds the limit and the message is dead-lettered. Quorum
            # queues dead-letter once x-delivery-count > x-delivery-limit.
            consume_channel = await connection.channel()
            try:
                origin = await consume_channel.get_queue(origin_name, ensure=False)
                for _ in range(delivery_limit + 3):
                    message = await origin.get(fail=False, timeout=5)
                    if message is None:
                        break
                    await message.nack(requeue=True)
                    await asyncio.sleep(0.1)
            finally:
                await consume_channel.close()

            dead_letter = await _drain_dlq_for(connection, topology, correlation_id)
            assert dead_letter is not None, (
                "message redelivered past delivery-limit did not appear in "
                f"the DLQ {topology.dlq_name!r}"
            )

            entry = _x_death_for_origin(dead_letter, origin_name)
            assert entry is not None, (
                "dead-lettered message has no x-death entry for origin queue "
                f"{origin_name!r}"
            )
            assert entry["reason"] == "delivery_limit"
        finally:
            await _delete_queue(connection, origin_name)


class TestExpiredDeadLetter:
    """A message that exceeds x-message-ttl lands in the DLQ as expired."""

    @pytest.mark.asyncio
    async def test_ttl_expired_message_is_dead_lettered(
        self, live_broker_connection, private_dead_letter_topology
    ):
        connection = live_broker_connection
        topology = private_dead_letter_topology

        origin_name = f"{_ORIGIN_QUEUE_PREFIX}-ttl-{uuid.uuid4()}"
        correlation_id = str(uuid.uuid4())

        declare_channel = await connection.channel()
        try:
            await declare_channel.declare_queue(
                origin_name,
                durable=True,
                # Low per-queue TTL: the message ages out and is dead-lettered
                # without any consumer touching it.
                arguments=_origin_queue_args(topology, **{"x-message-ttl": 500}),
            )
        finally:
            await declare_channel.close()

        try:
            await _publish_to_origin(connection, origin_name, correlation_id)

            dead_letter = await _drain_dlq_for(connection, topology, correlation_id)
            assert dead_letter is not None, (
                "TTL-expired message did not appear in the DLQ "
                f"{topology.dlq_name!r}"
            )

            entry = _x_death_for_origin(dead_letter, origin_name)
            assert entry is not None, (
                "dead-lettered message has no x-death entry for origin queue "
                f"{origin_name!r}"
            )
            assert entry["reason"] == "expired"
        finally:
            await _delete_queue(connection, origin_name)


class TestMaxLengthDeadLetter:
    """A head message dropped by max-length overflow lands in the DLQ."""

    @pytest.mark.asyncio
    async def test_maxlen_dropped_head_is_dead_lettered(
        self, live_broker_connection, private_dead_letter_topology
    ):
        connection = live_broker_connection
        topology = private_dead_letter_topology

        origin_name = f"{_ORIGIN_QUEUE_PREFIX}-maxlen-{uuid.uuid4()}"
        # The head (first) message is the one that gets dropped on overflow;
        # tag it so we can find it in the DLQ.
        head_correlation_id = str(uuid.uuid4())

        declare_channel = await connection.channel()
        try:
            await declare_channel.declare_queue(
                origin_name,
                durable=True,
                arguments=_origin_queue_args(
                    topology, **{"x-max-length": 1, "x-overflow": "drop-head"}
                ),
            )
        finally:
            await declare_channel.close()

        try:
            # First message occupies the single slot; the second pushes the
            # queue over max-length and drop-head evicts (dead-letters) the
            # head — the first message.
            await _publish_to_origin(connection, origin_name, head_correlation_id)
            await _publish_to_origin(connection, origin_name, str(uuid.uuid4()))

            dead_letter = await _drain_dlq_for(
                connection, topology, head_correlation_id
            )
            assert dead_letter is not None, (
                "max-length-dropped head message did not appear in the DLQ "
                f"{topology.dlq_name!r}"
            )

            entry = _x_death_for_origin(dead_letter, origin_name)
            assert entry is not None, (
                "dead-lettered message has no x-death entry for origin queue "
                f"{origin_name!r}"
            )
            assert entry["reason"] == "maxlen"
        finally:
            await _delete_queue(connection, origin_name)


class TestSharedGlobalDeadLetterQueue:
    """
    The service's OWN configured names really are a working dead-letter path.

    This is the one fidelity the per-test DLX/DLQ pairs above give up: they
    prove the mechanism (quorum dead-lettering, the connector's DLQ arguments,
    the service's routing key, x-death) but against private names, so a
    settings-level or provisioning-level break in ``events.dlx``/``events.dlq``
    itself would slip past them. This class closes that gap by driving one
    dead letter through the real, shared topology -- the fan-in a production
    origin queue does.

    It is the ONLY test in the suite that reads the shared DLQ, and it takes a
    broker-held mutex first, so it is the only reader even if that ever stops
    being true. The reason under test is ``expired`` because TTL needs no
    consumer, making it the shortest possible occupancy of a serialised
    critical section; the reason itself is not what this class is for.
    """

    # A scheduling hint only: it takes effect under ``--dist loadgroup``, and
    # ``pytest.ini`` sets the default ``--dist load``. The mutex, not this
    # mark, is what actually guarantees exclusivity.
    @pytest.mark.xdist_group("shared-global-dlq")
    @pytest.mark.asyncio
    async def test_dead_letter_reaches_the_service_named_global_dlq(
        self, live_broker_connection, shared_dead_letter_queue_lock
    ):
        connection = live_broker_connection
        await _declare_dead_letter_topology(connection, _SHARED_TOPOLOGY)

        origin_name = f"{_ORIGIN_QUEUE_PREFIX}-shared-{uuid.uuid4()}"
        correlation_id = str(uuid.uuid4())

        declare_channel = await connection.channel()
        try:
            await declare_channel.declare_queue(
                origin_name,
                durable=True,
                arguments=_origin_queue_args(
                    _SHARED_TOPOLOGY, **{"x-message-ttl": 500}
                ),
            )
        finally:
            await declare_channel.close()

        try:
            await _publish_to_origin(connection, origin_name, correlation_id)

            dead_letter = await _drain_dlq_for(
                connection, _SHARED_TOPOLOGY, correlation_id
            )
            assert dead_letter is not None, (
                "a dead letter from a service-shaped origin queue never "
                f"reached the service's configured DLQ "
                f"{_SHARED_TOPOLOGY.dlq_name!r} via its configured DLX "
                f"{_SHARED_TOPOLOGY.dlx_name!r}"
            )

            entry = _x_death_for_origin(dead_letter, origin_name)
            assert entry is not None, (
                "dead-lettered message has no x-death entry for origin queue "
                f"{origin_name!r}"
            )
            assert entry["reason"] == "expired"
        finally:
            await _delete_queue(connection, origin_name)
