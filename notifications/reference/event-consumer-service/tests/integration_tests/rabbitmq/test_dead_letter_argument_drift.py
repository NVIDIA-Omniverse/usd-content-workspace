# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REAL-broker integration tests for DLQ argument-drift recovery (issue b.rca).

The unit suite in ``tests/app/lib/rabbitmq/test_event_consumer_connector.py``
proves the drift handling against a mocked aio-pika: the 406 is synthesized,
the "second channel" is a second mock checkout, and the bind is an assertion
on a mock call. None of that exercises the parts that actually broke in
production. This module runs the same code path against a LIVE RabbitMQ 4.x
broker so every link in the chain is real:

1. The DLQ is pre-created with the SAME quorum type but a DIFFERENT numeric
   bound (``x-max-length`` one below the configured value), which is the exact
   shape of the reported incident: an operator retunes
   ``OMNI_EVENTS_DEAD_LETTER_QUEUE_MAX_LENGTH`` against a broker whose queue
   already exists.
2. ``EventConsumerConnector.declare_exchange()`` -- the real public entrypoint
   ``connect()`` calls -- issues its active declare and the BROKER returns a
   genuine AMQP 406 ``PRECONDITION_FAILED`` ("inequivalent arg").
3. The 406 REALLY closes the channel, and the real ``ChannelPool`` really
   evicts it on return and really hands out a second one.
4. The passive re-provision on that second channel really asserts the
   DLX->DLQ binding against the live, drifted queue.
5. A message published to the DLX with the configured routing key really
   arrives in the pre-existing queue.

Step 5 is the load-bearing assertion. Steps 1-4 could all be "green" while the
binding was silently never issued -- and a DLQ that exists but is unbound
loses every dead-lettered message. Only an end-to-end delivery proves the
recovery actually finished the topology.

The second class (``TestFreshDeadLetterQueueProvisioning``) runs the same
probe against a queue name that does NOT pre-exist. It is the harness's own
control: if the drift test fails while this one passes, the drift path is
broken; if BOTH fail, the harness (broker, names, publish/consume plumbing)
is at fault rather than the code under test.

Isolation
---------

Nothing here touches the shared broker's real ``events``/``events.dlx``/
``events.dlq`` topology. Every broker object this module creates carries a
``uuid`` suffix, applied by patching the settings singleton (the connector
reads ``settings.settings`` late, on every call, so the patch reaches it), and
teardown deletes each one best-effort while the connection is still open. The
routing key is deliberately NOT overridden: the assertion that a message
published with the CONFIGURED ``dead_letter_routing_key`` arrives is what
proves the connector bound with the configured key rather than some other one.

Scope notes
-----------

- The autouse fakes from the parent conftests are bypassed exactly as
  ``test_dead_letter_capture.py`` bypasses them; see this directory's
  ``conftest.py``. In particular the connector is never allowed to call
  ``connect()`` (whose ``aio_pika.connect_robust`` is patched to a mock).
  Instead its connection and both channel pools are populated from the real
  ``live_broker_connection``, so ``declare_exchange()`` runs unmodified
  production code against the real broker. This mirrors the real
  ``ChannelPool``-over-a-live-connection rig in
  ``test_channel_fatal_blast_radius.py``.
- The durable pool is built with the SHIPPED ``DurableChannelPoolConfig()``
  (``min_size=0``, ``client_limit=1``), because the eviction-and-second-channel
  behavior under test is a property of that exact config.
- Runs in CI's ``integration:`` job against a ``rabbitmq:4.1.3`` service with
  ``OMNI_EVENTS_REQUIRE_LIVE_BROKER=1`` so a missing broker HARD-fails; skips
  cleanly in a dev sandbox with no broker.
"""

import asyncio
import logging
import uuid
from types import SimpleNamespace

import aio_pika
import pytest
import pytest_asyncio
from notification_common.rabbitmq.channel_pool import (
    ChannelPool,
    ChannelPoolConfig,
    DurableChannelPoolConfig,
)
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings
from app.lib.rabbitmq.event_consumer_connector import (
    EventConsumerConnector,
    logger as connector_logger,
)

# The stable, greppable prefix of the drift ERROR. It is a documented log
# signature operators alert on (in the source repository's internal
# production-diagnostics runbook, which is not published), and the unit suite
# pins the same constant.
DRIFT_LOG_PREFIX = "Dead-letter queue argument mismatch"

# Throwaway bounds for the test DLQ, small so the declared quorum queue is
# cheap. The shipped defaults (14-day TTL, 1,000,000 messages) are irrelevant
# here: what matters is only that the pre-created queue and the connector's
# declare DISAGREE on a numeric bound.
TEST_DLQ_MESSAGE_TTL_MS = 600000
TEST_DLQ_MAX_LENGTH = 10

# How long to wait for the probe message to land in the DLQ.
DELIVERY_TIMEOUT_S = 30.0
DELIVERY_POLL_INTERVAL_S = 0.25

# Per-``basic.get`` wait while polling for the probe message.
GET_TIMEOUT_S = 5


def _connector_error_messages(caplog) -> list[str]:
    """The formatted ERROR records emitted by the connector module."""
    return [
        record.getMessage()
        for record in caplog.records
        if record.levelno == logging.ERROR and record.name == connector_logger.name
    ]


def _drifted_dlq_arguments() -> dict:
    """
    The connector's own DLQ arguments with ONE numeric bound perturbed.

    Sourced from ``EventConsumerConnector._dead_letter_queue_arguments()``
    rather than restated, so this cannot drift from the provisioning code --
    the same drift-proofing ``test_dead_letter_capture.py`` uses. Only
    ``x-max-length`` is changed (by one), which keeps the queue type quorum
    and every other argument identical: the resulting broker rejection is
    therefore the classic ``inequivalent arg 'x-max-length'`` 406 and nothing
    else.
    """
    arguments = dict(EventConsumerConnector._dead_letter_queue_arguments())
    arguments["x-max-length"] = arguments["x-max-length"] - 1
    return arguments


async def _declare_queue(connection, queue_name, arguments) -> None:
    """Declare a durable queue with the given arguments on its own channel."""
    channel = await connection.channel()
    try:
        await channel.declare_queue(queue_name, durable=True, arguments=arguments)
    finally:
        await channel.close()


async def _publish_to_dead_letter_exchange(
    connection, exchange_name, routing_key, correlation_id
) -> None:
    """
    Publish one persistent, MANDATORY message to the DLX.

    The exchange is obtained with ``ensure=False`` so no passive declare can
    kill this channel and confuse the outcome. ``mandatory=True`` (aio-pika's
    default, restated here because the test depends on it) means an unrouted
    message -- i.e. the DLX->DLQ binding was never issued -- surfaces as a
    ``DeliveryError`` rather than as a silent drop that would look like a
    slow delivery.
    """
    channel = await connection.channel()
    try:
        exchange = await channel.get_exchange(exchange_name, ensure=False)
        await exchange.publish(
            aio_pika.Message(
                body=b'{"event_type": "dlq-argument-drift-probe"}',
                correlation_id=correlation_id,
                delivery_mode=aio_pika.DeliveryMode.PERSISTENT,
            ),
            routing_key=routing_key,
            mandatory=True,
        )
    finally:
        await channel.close()


async def _await_message(connection, queue_name, correlation_id):
    """
    Poll ``queue_name`` for the message bearing ``correlation_id``.

    Acks and returns it, or returns ``None`` once ``DELIVERY_TIMEOUT_S``
    elapses.

    EVERY message taken is acked, ours or not, mirroring
    ``test_dead_letter_capture._drain_dlq_for``. The queue is uuid-suffixed
    and therefore private to this test, so there is nothing to preserve for
    anyone else -- and the two alternatives are both actively harmful on a
    quorum queue: nack-requeueing spends a hand-back from the queue's
    ``delivery-limit`` budget on every poll interval until the broker discards
    the message, and holding unacked hides it from its owner for the life of
    the channel.
    """
    channel = await connection.channel()
    try:
        queue = await channel.get_queue(queue_name, ensure=False)
        loop = asyncio.get_running_loop()
        deadline = loop.time() + DELIVERY_TIMEOUT_S
        while loop.time() < deadline:
            message = await queue.get(fail=False, timeout=GET_TIMEOUT_S)
            if message is None:
                await asyncio.sleep(DELIVERY_POLL_INTERVAL_S)
                continue
            await message.ack()
            if message.correlation_id == correlation_id:
                return message
        return None
    finally:
        await channel.close()


async def _delete_queue(connection, queue_name) -> None:
    """
    Best-effort delete of a test-created queue, on its own channel.

    Each deletion gets a fresh channel because a failed delete (a 404 on a
    queue that was never created, say) closes the channel and would take every
    later cleanup step down with it.
    """
    channel = await connection.channel()
    try:
        await channel.queue_delete(queue_name)
    except Exception:  # noqa: BLE001 - cleanup is best-effort
        logging.getLogger(__name__).debug(
            "Best-effort cleanup could not delete queue %r", queue_name, exc_info=True
        )
    finally:
        try:
            await channel.close()
        except Exception:  # noqa: BLE001 - cleanup is best-effort
            pass


async def _delete_exchange(connection, exchange_name) -> None:
    """Best-effort delete of a test-created exchange, on its own channel."""
    channel = await connection.channel()
    try:
        await channel.exchange_delete(exchange_name)
    except Exception:  # noqa: BLE001 - cleanup is best-effort
        logging.getLogger(__name__).debug(
            "Best-effort cleanup could not delete exchange %r",
            exchange_name,
            exc_info=True,
        )
    finally:
        try:
            await channel.close()
        except Exception:  # noqa: BLE001 - cleanup is best-effort
            pass


class ProvisioningObservations(SimpleNamespace):
    """The measured record of one live dead-letter provisioning episode."""


async def _provision_and_probe(connection, connector, names, caplog):
    """
    Run the connector's real provisioning entrypoint, then probe the binding.

    Records rather than asserts (the record-then-assert pattern of
    ``test_channel_fatal_blast_radius.py``), so the expensive live sequence
    runs once per test class and each assertion reads a recorded field:

    - ``provisioning_error``: whatever escaped ``declare_exchange()``, or
      ``None``. Drift must NOT escape -- that escape is the crash-loop b.rca
      reports.
    - ``error_messages``: the connector module's ERROR records.
    - ``publish_error``: a ``DeliveryError`` here means the DLX->DLQ binding
      was never issued (mandatory publish, nothing to route to).
    - ``delivered``: the probe message as consumed from the DLQ, or ``None``
      on timeout.
    """
    correlation_id = str(uuid.uuid4())

    provisioning_error = None
    with caplog.at_level(logging.ERROR, logger=connector_logger.name):
        try:
            await connector.declare_exchange()
        except Exception as exc:  # noqa: BLE001 - the escape IS the observation
            provisioning_error = exc

    # Snapshot the ERROR records HERE rather than letting the tests read
    # ``caplog`` themselves. This helper runs from a fixture, i.e. in pytest's
    # SETUP phase, and pytest swaps in a fresh capture handler for each phase --
    # so ``caplog.records`` read from a test body (the CALL phase) would not
    # contain anything logged during setup.
    error_messages = _connector_error_messages(caplog)

    publish_error = None
    delivered = None
    if provisioning_error is None:
        try:
            await _publish_to_dead_letter_exchange(
                connection, names.dlx_name, names.routing_key, correlation_id
            )
        # An unroutable mandatory publish (DeliveryError) IS the observation
        # here, so it is recorded rather than allowed to error the test.
        except Exception as exc:  # noqa: BLE001
            publish_error = exc
        else:
            delivered = await _await_message(connection, names.dlq_name, correlation_id)

    return ProvisioningObservations(
        provisioning_error=provisioning_error,
        error_messages=error_messages,
        publish_error=publish_error,
        delivered=delivered,
        correlation_id=correlation_id,
    )


@pytest.fixture
def throwaway_topology(mocker):
    """
    Point the connector at uuid-suffixed, disposable broker object names.

    Patching the settings singleton is how the rest of the integration suite
    redirects service behavior (``mocker.patch.object(settings.settings, ...)``),
    and it reaches the connector because the connector re-reads
    ``settings.settings`` on every call rather than snapshotting at import.

    The numeric bounds are shrunk too, so the quorum queue this test creates is
    cheap; the drift under test is a DISAGREEMENT between two declares, not a
    property of the shipped values.

    ``dead_letter_routing_key`` is deliberately left at its configured value --
    the end-to-end probe publishes with it, so a connector that bound the queue
    under some other key would fail the delivery assertion.
    """
    suffix = uuid.uuid4()
    names = SimpleNamespace(
        events_exchange_name=f"test-dlq-drift-events-{suffix}",
        dlx_name=f"test-dlq-drift-dlx-{suffix}",
        dlq_name=f"test-dlq-drift-dlq-{suffix}",
        routing_key=settings.settings.dead_letter_routing_key,
    )
    mocker.patch.object(settings.settings, "dead_letter_exchange_name", names.dlx_name)
    mocker.patch.object(settings.settings, "dead_letter_queue_name", names.dlq_name)
    mocker.patch.object(
        settings.settings, "dead_letter_queue_message_ttl_ms", TEST_DLQ_MESSAGE_TTL_MS
    )
    mocker.patch.object(
        settings.settings, "dead_letter_queue_max_length", TEST_DLQ_MAX_LENGTH
    )
    return names


@pytest_asyncio.fixture
async def live_connector(live_broker_connection, throwaway_topology, mocker):
    """
    An ``EventConsumerConnector`` wired to the LIVE connection and real pools.

    ``connect()`` is deliberately never called: the autouse ``before_each``
    fixture patches ``aio_pika.connect_robust`` to a mock, so a connector that
    connected itself would never reach the broker. Populating ``_connection``
    and both pools from ``live_broker_connection`` instead leaves
    ``declare_exchange()`` -> ``_declare_dead_letter_topology()`` running
    completely unmodified against the real broker, which is the whole point.

    The durable pool -- the one the dead-letter topology checks out of, and the
    one whose eviction of the 406-killed channel is under test -- uses the
    SHIPPED ``DurableChannelPoolConfig()``. The non-durable pool is shrunk to a
    single channel because its only job here is the one ``events``-exchange
    declare; its shipped ``min_size`` of 75 would open 75 channels for nothing.

    ``EXCHANGE_NAME`` is overridden per instance so the shared broker's real
    ``events`` exchange is not touched either.

    Teardown closes the POOLS only. The connection belongs to
    ``live_broker_connection``, which closes it, and calling
    ``connector.close()`` here would close it early -- out from under the
    cleanup that still has queues and exchanges to delete.
    """
    connector = EventConsumerConnector()
    connector._connection = live_broker_connection
    connector._channel_pool = ChannelPool(
        live_broker_connection,
        ChannelPoolConfig(pool_size=1, min_size=1),
        name=RabbitMQConnector.NON_DURABLE_POOL_NAME,
    )
    await connector._channel_pool.initialize()
    connector._durable_channel_pool = ChannelPool(
        live_broker_connection,
        DurableChannelPoolConfig(),
        name=RabbitMQConnector.DURABLE_POOL_NAME,
    )
    await connector._durable_channel_pool.initialize()

    mocker.patch.object(
        connector, "EXCHANGE_NAME", throwaway_topology.events_exchange_name
    )

    try:
        yield connector
    finally:
        for pool in (connector._channel_pool, connector._durable_channel_pool):
            await pool.close_all()


@pytest_asyncio.fixture
async def drift_episode(
    live_broker_connection, live_connector, throwaway_topology, caplog
):
    """
    Pre-create the DLQ with a DIFFERENT bound, then provision and probe.

    This is the reported b.rca scenario: the queue already exists (same quorum
    type, same TTL, same overflow) but with an ``x-max-length`` one below what
    settings now ask for, so the connector's active declare draws a real AMQP
    406 from the broker.
    """
    connection = live_broker_connection
    names = throwaway_topology
    try:
        await _declare_queue(connection, names.dlq_name, _drifted_dlq_arguments())
        yield await _provision_and_probe(connection, live_connector, names, caplog)
    finally:
        await _delete_queue(connection, names.dlq_name)
        await _delete_exchange(connection, names.dlx_name)
        await _delete_exchange(connection, names.events_exchange_name)


@pytest_asyncio.fixture
async def fresh_episode(
    live_broker_connection, live_connector, throwaway_topology, caplog
):
    """
    Provision and probe with NO pre-existing DLQ -- the control episode.

    Identical to ``drift_episode`` except that the connector's active declare
    creates the queue rather than colliding with one. It exists so a failure
    can be attributed: drift-only failure means the recovery path is broken,
    while both failing means this harness is.
    """
    connection = live_broker_connection
    names = throwaway_topology
    try:
        yield await _provision_and_probe(connection, live_connector, names, caplog)
    finally:
        await _delete_queue(connection, names.dlq_name)
        await _delete_exchange(connection, names.dlx_name)
        await _delete_exchange(connection, names.events_exchange_name)


class TestDeadLetterQueueArgumentDrift:
    """A live 406 on the DLQ declare is survived, reported, and recovered."""

    @pytest.mark.asyncio
    async def test_provisioning_does_not_raise(self, drift_episode):
        assert drift_episode.provisioning_error is None, (
            "declare_exchange() raised against a DLQ with drifted arguments: "
            f"{drift_episode.provisioning_error!r}. This escape is exactly the "
            "startup crash-loop issue b.rca reports."
        )

    @pytest.mark.asyncio
    async def test_logs_exactly_one_error(self, drift_episode):
        assert len(drift_episode.error_messages) == 1, (
            "expected exactly one connector ERROR for the drift, got: "
            f"{drift_episode.error_messages!r}"
        )

    @pytest.mark.asyncio
    async def test_error_uses_the_documented_prefix(self, drift_episode):
        assert drift_episode.error_messages
        assert drift_episode.error_messages[0].startswith(DRIFT_LOG_PREFIX)

    @pytest.mark.asyncio
    async def test_error_reports_the_brokers_inequivalent_argument(self, drift_episode):
        """
        The broker's own 406 text is echoed into the ERROR.

        The remedy the message prescribes depends on WHICH argument differs, so
        the operator needs the broker's own wording rather than just this
        service's desired values. ``inequivalent arg`` is RabbitMQ's literal
        406 reply-text for an argument mismatch (the unit suite pins a captured
        instance of it in ``PRECONDITION_FAILED_MESSAGE``), and it appears
        NOWHERE in the connector's own prose -- unlike the argument name
        ``x-max-length``, which the message's "Desired:" clause always
        contains, and asserting on which would therefore pass even if the echo
        were dropped entirely.
        """
        assert drift_episode.error_messages
        assert "inequivalent arg" in drift_episode.error_messages[0]

    @pytest.mark.asyncio
    async def test_probe_message_is_routable_after_recovery(self, drift_episode):
        """
        The mandatory publish to the DLX was accepted.

        A ``DeliveryError`` here would mean the passive re-provision never
        issued the DLX->DLQ bind, leaving a dead-letter exchange that drops
        everything on the floor.
        """
        assert drift_episode.publish_error is None, (
            "publishing to the dead-letter exchange after drift recovery "
            f"failed: {drift_episode.publish_error!r}; the DLX->DLQ binding "
            "was most likely never issued"
        )

    @pytest.mark.asyncio
    async def test_dead_letter_delivery_survives_the_drift(self, drift_episode):
        """
        THE load-bearing assertion: end-to-end delivery still works.

        Everything above can pass while the topology is silently unfinished.
        A message published to the DLX with the configured routing key
        arriving in the pre-existing, drifted queue is the only proof that the
        real 406, the real channel close, the real pool eviction, and the real
        passive re-provision added up to a working dead-letter path.
        """
        assert drift_episode.delivered is not None, (
            "a message published to the dead-letter exchange never arrived in "
            "the pre-existing dead-letter queue; the binding did not survive "
            "the argument-drift recovery"
        )


class TestFreshDeadLetterQueueProvisioning:
    """Control: with no pre-existing queue there is no drift and no ERROR."""

    @pytest.mark.asyncio
    async def test_provisioning_does_not_raise(self, fresh_episode):
        assert fresh_episode.provisioning_error is None, (
            "declare_exchange() raised while provisioning a brand-new "
            f"dead-letter queue: {fresh_episode.provisioning_error!r}"
        )

    @pytest.mark.asyncio
    async def test_logs_no_error(self, fresh_episode):
        assert fresh_episode.error_messages == [], (
            "provisioning a brand-new dead-letter queue logged an ERROR: "
            f"{fresh_episode.error_messages!r}"
        )

    @pytest.mark.asyncio
    async def test_dead_letter_delivery_works(self, fresh_episode):
        assert fresh_episode.publish_error is None, (
            "publishing to a freshly provisioned dead-letter exchange failed: "
            f"{fresh_episode.publish_error!r}"
        )
        assert fresh_episode.delivered is not None, (
            "a message published to a freshly provisioned dead-letter exchange "
            "never arrived in its dead-letter queue"
        )
