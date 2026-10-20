# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import copy
import gc
import json

import aio_pika
import pytest
import pytest_asyncio
from notification_common.errors.errors import QueueNotFoundError

from app.config import settings as _settings_module
from app.lib.rabbitmq import rabbitmq_base
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase
from app.lib.models.event_message import EventMessage
from tests.app.lib.rabbitmq.test_rabbitmq_base import (
    MockMessage,
    MockMessageWithXDeath,
)
from tests.fixtures.consumer_liveness import (
    IDLE_POLL_FIRES_IMMEDIATELY_SECONDS,
    LIVENESS_QUEUE_NAME,
    PUMP_CONSUMER_TAG,
    FakeConnection,
    FakeLivenessChannel,
    FakeLivenessQueue,
)
from tests.fixtures.mock_filter_group import filter_group

# Re-export so DLQ test modules can import the canonical shape straight from
# the rabbitmq-layer conftest. ``MockMessageWithXDeath`` remains defined once,
# in ``test_rabbitmq_base.py``; this conftest is the single import point for
# the dead-lettered-message fixtures/factory built on top of it.
__all__ = ["MockMessageWithXDeath"]

# Default body for a dead-lettered mock message. Carries the four fields the
# decode helper reads off the JSON body: ``event_type``, ``resource_id``,
# ``principal_identity``, plus an arbitrary payload. Deep-copied per instance
# by the factory so no test can mutate another test's body.
_DEFAULT_DEAD_LETTER_BODY = {
    "event_type": "asset.updated",
    "resource_id": "resource-123",
    "principal_identity": "user-abc",
    "payload": {"key": "value"},
}

# The default single ``x-death`` entry. The factory deep-copies this so each
# message owns an independent headers dict (no shared mutable state).
_DEFAULT_X_DEATH_ENTRY = {
    "queue": "origin-queue",
    "reason": "rejected",
    "count": 1,
    "time": 1700000000,
    "exchange": "events",
    "routing-keys": ["events.foo"],
}


def _make_dead_lettered_message(
    *,
    reason="rejected",
    queue="origin-queue",
    count=1,
    delivery_tag=0,
    delivery_count=1,
    routing_keys=None,
    time=1700000000,
    exchange="events",
    body=None,
    extra_x_death_entries=None,
):
    """
    Build a ``MockMessageWithXDeath`` with overridable header/body fields.

    The decode helper reads ``x-death[0]`` for ``reason``/``queue``/``count``/
    ``time``/``routing-keys``, the top-level ``x-delivery-count`` for the
    delivery count, and the JSON body for ``event_type``/``resource_id``/
    ``principal_identity``/payload. This factory lets a test vary any of those
    without redefining the header dict inline.

    :param reason: AMQP dead-letter reason for the first ``x-death`` entry
        (e.g. ``"delivery_limit"``, ``"expired"``, ``"maxlen"``, ``"rejected"``).
    :param queue: Origin queue name for the first ``x-death`` entry.
    :param count: ``count`` field on the first ``x-death`` entry.
    :param delivery_tag: Broker delivery tag on the message.
    :param delivery_count: Top-level ``x-delivery-count`` header.
    :param routing_keys: ``routing-keys`` for the first ``x-death`` entry;
        defaults to the canonical ``["events.foo"]``.
    :param time: ``time`` field on the first ``x-death`` entry.
    :param exchange: ``exchange`` field on the first ``x-death`` entry.
    :param body: Override the JSON-body dict; defaults to a deep copy of
        ``_DEFAULT_DEAD_LETTER_BODY``.
    :param extra_x_death_entries: Additional ``x-death`` entries appended after
        the (most-recent-first) primary entry, for multi-entry variants.
    :returns: A ``MockMessageWithXDeath`` with independent ``headers``/``body``.
    """
    body_dict = copy.deepcopy(_DEFAULT_DEAD_LETTER_BODY) if body is None else body
    msg = MockMessageWithXDeath(
        json.dumps(body_dict).encode("utf-8"), delivery_tag=delivery_tag
    )

    primary_entry = copy.deepcopy(_DEFAULT_X_DEATH_ENTRY)
    primary_entry.update(
        reason=reason,
        queue=queue,
        count=count,
        time=time,
        exchange=exchange,
        **{"routing-keys": ["events.foo"] if routing_keys is None else routing_keys},
    )

    x_death = [primary_entry]
    if extra_x_death_entries:
        x_death.extend(copy.deepcopy(entry) for entry in extra_x_death_entries)

    msg.headers = {"x-death": x_death, "x-delivery-count": delivery_count}
    return msg


@pytest.fixture
def dead_lettered_message():
    """
    A default dead-lettered mock message with a single ``x-death`` entry.

    Its ``headers`` decode to a reason (``rejected`` -> ``DELIVERY_LIMIT``),
    origin queue, delivery count, timestamp, and original routing keys; its
    body decodes to ``event_type``/``resource_id``/``principal_identity`` and
    payload. Each request returns a fresh instance with independent state.
    """
    return _make_dead_lettered_message()


@pytest.fixture
def make_dead_lettered_message():
    """
    Factory fixture returning ``_make_dead_lettered_message``.

    Lets a test vary ``reason``, origin ``queue``, ``count``, ``delivery_tag``,
    ``delivery_count``, ``routing_keys``, ``time``, ``exchange``, ``body``, and
    ``extra_x_death_entries`` without redefining the header dict inline. Every
    call produces a message with independent ``headers``/``body``.
    """
    return _make_dead_lettered_message


@pytest.fixture
def multi_entry_dead_lettered_message():
    """
    A dead-lettered mock message with multiple ``x-death`` entries.

    RabbitMQ records the most-recent dead-letter event first, so the decode
    helper must select ``x-death[0]``. The primary (first) entry here reports
    ``maxlen`` from ``newest-queue``; an older ``expired`` entry from
    ``older-queue`` follows. A correct decode picks the ``maxlen`` entry.
    """
    return _make_dead_lettered_message(
        reason="maxlen",
        queue="newest-queue",
        count=2,
        delivery_count=2,
        routing_keys=["events.bar"],
        time=1700000500,
        extra_x_death_entries=[
            {
                "queue": "older-queue",
                "reason": "expired",
                "count": 1,
                "time": 1700000000,
                "exchange": "events",
                "routing-keys": ["events.foo"],
            }
        ],
    )


@pytest.fixture(autouse=True)
def no_op_dead_letter_topology(mocker):
    """
    Stub the connector's connect-time dead-letter topology provisioning.

    ``EventConsumerConnector.declare_exchange`` calls
    ``_declare_dead_letter_topology``, which declares the DLQ on the shared
    ``mock_chan`` during ``connect()``. That pollutes the ``mock_chan``
    assertions in the ``connect``-autouse test classes
    (``TestGetQueue``/``TestCreateQueue``/``TestUpdateBindings``), turning
    ``assert_called_once*`` on ``declare_queue`` into two calls and making
    those tests order-dependent. The topology itself is unit-tested in
    ``test_event_consumer_connector.py``, so it is a no-op here. This
    conftest-level autouse fixture runs before the per-class ``connect``
    autouse fixtures so the stub is in place before ``connect()`` fires.
    """
    mocker.patch.object(
        RabbitMQBase.connector,
        "_declare_dead_letter_topology",
        new_callable=mocker.AsyncMock,
    )


@pytest.fixture(autouse=True)
def _finalize_leaked_consumers():
    """
    Force finalization of GC-pending consumers at each test boundary.

    ``NonDurableQueueConsumer.__del__`` is a safety net that, when the
    instance is collected while an event loop is running, schedules
    ``cleanup_channel()`` — which calls the module-global
    ``rabbitmq_connector.return_channel``. Tests patch that global
    (see ``mock_return_channel``), so a consumer leaked by an *earlier* test
    (e.g. ``test_del_warns_when_the_loop_lookup_raises``, which leaves
    ``channel`` set) and collected part-way through a *later* test's event
    loop lands a stray ``return_channel`` call on the later test's mock. That
    breaks ``assert_called_once`` nondeterministically under ``--random-order``.

    Collecting garbage here — before and after each test, while no loop is
    running — makes any such ``__del__`` fire on the no-task warning branch
    (``loop.is_running()`` is False), so it never touches ``return_channel``.
    This contains each test's leaked consumers to its own boundary instead of
    bleeding across tests. Same order-dependence rationale as
    ``no_op_dead_letter_topology`` above.
    """
    gc.collect()
    yield
    gc.collect()


@pytest.fixture
def filter_groups():
    return [filter_group()]


@pytest.fixture
def authz_check_returns_true():
    async def authz_check(_message):
        return True

    return authz_check


@pytest_asyncio.fixture
async def mock_consume(mocker, authz_check_returns_true):
    async def mock_consume_gen(*_args, **_kwargs):
        for _ in range(3):
            yield EventMessage({"a": "b"})

    mocker.patch.object(
        RabbitMQBase,
        "_consume_with_delayed_acks",
        side_effect=mock_consume_gen,
    )


@pytest.fixture
def mock_temp_chan_for_queue(mocker):
    """
    Mock the temporary channel used by get_queue to check queue existence.

    This fixture mocks the get_temporary_channel context manager so that
    the temp channel existence check passes, allowing the pool channel to
    then be used to get the actual queue.
    """
    temp_chan = mocker.MagicMock()
    temp_chan.declare_queue = mocker.AsyncMock()

    mock_context_manager = mocker.AsyncMock()
    mock_context_manager.__aenter__.return_value = temp_chan
    mocker.patch.object(
        RabbitMQBase.connector,
        "get_temporary_channel",
        return_value=mock_context_manager,
    )
    return temp_chan


@pytest.fixture
def mock_queue(mock_chan, mock_temp_chan_for_queue, mocker):
    """
    Mock queue fixture that also sets up the temporary channel mock.

    The get_queue method first checks queue existence with a temporary
    channel, then uses the pool channel to get the queue. This fixture
    sets up both mocks.
    """
    mock_queue_instance = mocker.MagicMock(spec=aio_pika.queue.Queue)
    mock_queue_instance.name = "mock_queue_name_from_fixture"

    # noinspection PyUnusedLocal
    async def mock_consume(callback, **kwargs):
        for i in range(3):
            mock_message = MockMessage(b'{"a": "b"}', delivery_tag=i + 1)
            await callback(mock_message)
        return "consumer_tag"

    mock_queue_instance.consume = mock_consume

    async def mock_cancel(_consumer_tag: str):
        pass

    mock_queue_instance.cancel = mocker.AsyncMock(side_effect=mock_cancel)

    mock_chan.declare_queue.return_value = mock_queue_instance

    return mock_queue_instance


# ---------------------------------------------------------------------------
# Consumer-liveness fixtures (issue b.9gj)
#
# Shared by the two durable consume loops in ``test_rabbitmq_base.py`` and the
# DLQ browse loop in ``test_dead_letter_consumer.py``. The doubles they build
# on live in ``tests/fixtures/consumer_liveness.py``.
# ---------------------------------------------------------------------------


@pytest.fixture
def make_consumer_queue():
    """
    Build a queue whose channel reports a chosen consumer-liveness state.

    Defaults to the healthy case (channel open, our tag registered) so each
    test overrides only the one condition it is about.
    """

    def _make(
        *,
        is_closed=False,
        consumer_tags=(PUMP_CONSUMER_TAG,),
        underlay_error=None,
    ):
        channel = FakeLivenessChannel(
            is_closed=is_closed,
            consumer_tags=consumer_tags,
            underlay_error=underlay_error,
        )
        return FakeLivenessQueue(channel)

    return _make


@pytest.fixture
def live_queue(make_consumer_queue):
    """A queue whose consumer is healthy: channel open, tag registered."""
    return make_consumer_queue()


@pytest.fixture
def cancellable_queue(make_consumer_queue):
    """
    The queue a pumped consume/browse loop runs against.

    Its consumer starts healthy and is killed mid-stream with
    ``queue.channel.cancel_consumer(PUMP_CONSUMER_TAG)``, modelling the
    broker's ``Basic.Cancel``.
    """
    return make_consumer_queue()


@pytest.fixture
def broker_connection_up(mocker):
    """
    Give the connector the live shared AMQP connection it has in production.

    ``_raise_if_consumer_lost`` skips its classifying round trip entirely when
    ``_connection_is_down``, and a connector that no test ever connected leaves
    that attribute at None. Without this fixture every classification test
    would silently take the connection-down short circuit instead of the path
    it means to exercise.
    """
    connection = FakeConnection()
    mocker.patch.object(RabbitMQBase.connector, "_connection", connection)
    return connection


@pytest.fixture(
    params=["no connection", "closed connection", "disconnected connection"]
)
def broker_connection_down(request, mocker, queue_exists):
    """
    Take the connector's shared AMQP connection down, every way it goes down.

    ``disconnected connection`` is the broker blip, and the case the original
    ``is_closed``-only predicate missed: aio-pika resolves ``is_closed`` only
    on an explicit local ``close()``, so a broker-side drop leaves a
    RobustConnection reading ``is_closed=False`` while it is plainly not
    connected. Its ``connected`` event is the flag that actually moves.

    Depends on ``queue_exists`` so this patch lands AFTER that fixture's
    live-connection one regardless of the order a test lists them: the point
    of these cases is that the classification is skipped even though it would
    have succeeded.
    """
    connection = None
    if request.param == "closed connection":
        connection = FakeConnection(is_closed=True)
    elif request.param == "disconnected connection":
        connection = FakeConnection(connected=False)
    mocker.patch.object(RabbitMQBase.connector, "_connection", connection)


@pytest.fixture
def queue_exists(mocker, broker_connection_up):
    """The classifying broker round trip finds the queue still present."""
    return mocker.patch.object(
        RabbitMQBase, "_assert_queue_exists", new_callable=mocker.AsyncMock
    )


@pytest.fixture
def queue_deleted(mocker, broker_connection_up):
    """The classifying broker round trip finds the queue gone."""
    return mocker.patch.object(
        RabbitMQBase,
        "_assert_queue_exists",
        new_callable=mocker.AsyncMock,
        side_effect=QueueNotFoundError(queue_name=LIVENESS_QUEUE_NAME),
    )


@pytest.fixture
def queue_check_unreachable(mocker, broker_connection_up):
    """
    The classifying round trip fails without answering the question.

    ``_assert_queue_exists`` converts only a ``ChannelClosed`` into
    ``QueueNotFoundError``; a connection-level failure, an invalid channel
    state or a timeout escapes as itself, which is the case this models.
    """
    return mocker.patch.object(
        RabbitMQBase,
        "_assert_queue_exists",
        new_callable=mocker.AsyncMock,
        side_effect=aio_pika.exceptions.AMQPConnectionError("broker unreachable"),
    )


@pytest.fixture
def fast_liveness_poll(mocker):
    """
    Reach the consume loops' idle-poll branch immediately and predictably.

    The setting alone would leave the deadline a random draw:
    ``_liveness_poll_timeout`` stretches it over an upward jitter band, so
    every idle-poll test would wait a different, unpinnable amount. The
    deadline helper is therefore patched to the flat value, which is also the
    honest seam -- these tests are about what the LOOPS do once a poll fires,
    while the jitter arithmetic is pinned directly in
    ``TestLivenessPollTimeout``. The setting is still patched so nothing reads
    a production-sized window out from under them.
    """
    mocker.patch.object(
        _settings_module.settings,
        "consumer_liveness_poll_seconds",
        IDLE_POLL_FIRES_IMMEDIATELY_SECONDS,
    )
    mocker.patch.object(
        RabbitMQBase,
        "_liveness_poll_timeout",
        return_value=IDLE_POLL_FIRES_IMMEDIATELY_SECONDS,
    )


@pytest.fixture
def no_server_acks(mocker):
    """Keep the delayed-ack path from acking anything mid-test."""
    mocker.patch.object(_settings_module.settings, "server_ack_delay_seconds", 999.0)


@pytest.fixture
def background_tasks(mocker):
    """
    Capture the consume loops' background tasks in creation order.

    Every loop schedules its cleanup with ``asyncio.create_task``; the
    delayed-ack loop schedules its idle timer the same way but cancels it in
    the ``finally`` before scheduling cleanup, so the LAST captured task is
    always the cleanup one.
    """
    tasks = []
    real_create_task = asyncio.create_task

    def capture(coro):
        task = real_create_task(coro)
        tasks.append(task)
        return task

    mocker.patch.object(rabbitmq_base.asyncio, "create_task", side_effect=capture)
    return tasks
