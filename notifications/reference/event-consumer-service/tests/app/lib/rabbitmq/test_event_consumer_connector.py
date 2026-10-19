# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import aio_pika
import pytest
import pytest_asyncio
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings
from app.lib.rabbitmq.event_consumer_connector import (
    EventConsumerConnector,
    logger as connector_logger,
)

# The stable, greppable prefix of the drift ERROR. It is a documented log
# signature operators alert on, so it is pinned exactly here even though the
# rest of the message is only checked loosely.
DRIFT_LOG_PREFIX = "Dead-letter queue argument mismatch"

# The broker's AMQP 406 PRECONDITION_FAILED text when a queue is redeclared
# with arguments that differ from its creation-time ones.
PRECONDITION_FAILED_MESSAGE = (
    "inequivalent arg 'x-max-length' for queue 'events.dlq' in vhost '/': "
    "received '500000' but current is '1000000'"
)

# A passive declare against a queue that does not exist closes the channel
# with AMQP 404, which aiormq raises as ChannelNotFoundEntity. That is a real
# broker problem, not argument drift.
NOT_FOUND_CODE = 404
NOT_FOUND_MESSAGE = "NOT_FOUND - no queue 'events.dlq' in vhost '/'"

# The prefix of the WARNING emitted when the best-effort close of the
# 406-killed channel itself fails.
CLOSE_FAILURE_LOG_PREFIX = "Failed to close the channel the broker killed"

# What the failing close raises in the best-effort tests below: a
# connection-level problem is the realistic way an in-flight close breaks.
CLOSE_FAILURE_MESSAGE = "connection lost"


def connector_error_messages(caplog):
    """The formatted ERROR records emitted by the connector module."""
    return [
        record.getMessage()
        for record in caplog.records
        if record.levelno == logging.ERROR and record.name == connector_logger.name
    ]


def connector_warning_messages(caplog):
    """The formatted WARNING records emitted by the connector module."""
    return [
        record.getMessage()
        for record in caplog.records
        if record.levelno == logging.WARNING and record.name == connector_logger.name
    ]


@pytest.fixture
def error_logs(caplog):
    """Capture the connector module's ERROR records."""
    with caplog.at_level(logging.ERROR, logger=connector_logger.name):
        yield caplog


@pytest.fixture
def warning_logs(caplog):
    """
    Capture the connector module's WARNING records.

    The threshold also admits ERROR, so a test that needs both the drift ERROR
    and the failed-close WARNING can request this one fixture.
    """
    with caplog.at_level(logging.WARNING, logger=connector_logger.name):
        yield caplog


@pytest.fixture
def mock_exchange(mocker):
    """The DLX returned by ``channel.declare_exchange``."""
    return mocker.MagicMock(name="dlx")


@pytest.fixture
def mock_queue(mocker):
    """The DLQ returned by ``channel.declare_queue`` (with an async bind)."""
    queue = mocker.MagicMock(name="dlq")
    queue.bind = mocker.AsyncMock()
    return queue


@pytest.fixture
def mock_passive_exchange(mocker):
    """The DLX declared on the SECOND channel, during a passive re-provision."""
    return mocker.MagicMock(name="passive_dlx")


@pytest.fixture
def mock_passive_queue(mocker):
    """The DLQ obtained passively on the second channel (with an async bind)."""
    queue = mocker.MagicMock(name="passive_dlq")
    queue.bind = mocker.AsyncMock()
    return queue


@pytest.fixture
def mock_durable_channel(mocker, mock_exchange, mock_queue):
    """
    The durable pool's channel, exposing async declare_exchange/queue/close.

    ``close`` MUST be an AsyncMock. The drift path awaits it, and the connector
    suppresses any exception from that await -- so a bare MagicMock ``close``
    would make ``await channel.close()`` raise ``TypeError``, get swallowed,
    and leave every assertion about the close silently unexercised.
    """
    channel = mocker.MagicMock(name="durable_channel")
    channel.declare_exchange = mocker.AsyncMock(return_value=mock_exchange)
    channel.declare_queue = mocker.AsyncMock(return_value=mock_queue)
    channel.close = mocker.AsyncMock()
    return channel


@pytest.fixture
def mock_second_durable_channel(mocker, mock_passive_exchange, mock_passive_queue):
    """
    A second, distinct durable-pool channel.

    The broker closes the channel that took the 406, so the passive
    re-provision has to run on a fresh one. Its exchange and queue mocks are
    distinct from the first channel's so no assertion about the passive pass
    can pass by accidentally observing the failed active pass.

    Its ``close`` is an AsyncMock for the same reason as the first channel's,
    and so the assertion that this healthy channel is NOT closed is real rather
    than an accident of a swallowed ``TypeError``.
    """
    channel = mocker.MagicMock(name="second_durable_channel")
    channel.declare_exchange = mocker.AsyncMock(return_value=mock_passive_exchange)
    channel.declare_queue = mocker.AsyncMock(return_value=mock_passive_queue)
    channel.close = mocker.AsyncMock()
    return channel


@pytest.fixture
def dlq_argument_drift(
    mock_durable_pool, mock_durable_channel, mock_second_durable_channel
):
    """
    Put the connector in front of a DLQ that already exists with different
    arguments.

    The active declare fails with the broker's AMQP 406, and the pool hands
    out a second channel for the passive re-provision that follows.
    """
    mock_durable_channel.declare_queue.side_effect = (
        aio_pika.exceptions.ChannelPreconditionFailed(PRECONDITION_FAILED_MESSAGE)
    )
    mock_durable_pool.get_channel.side_effect = [
        mock_durable_channel,
        mock_second_durable_channel,
    ]


@pytest.fixture
def mock_non_durable_channel(mocker, mock_exchange, mock_queue):
    """The non-durable pool's channel, kept distinct from the durable one."""
    channel = mocker.MagicMock(name="non_durable_channel")
    channel.declare_exchange = mocker.AsyncMock(return_value=mock_exchange)
    channel.declare_queue = mocker.AsyncMock(return_value=mock_queue)
    return channel


@pytest.fixture
def mock_durable_pool(mocker, mock_durable_channel):
    """The durable channel pool, which the dead-letter topology must use."""
    pool = mocker.MagicMock(name="durable_channel_pool")
    pool.get_channel = mocker.AsyncMock(return_value=mock_durable_channel)
    pool.return_channel = mocker.AsyncMock()
    return pool


@pytest.fixture
def mock_non_durable_pool(mocker, mock_non_durable_channel):
    """
    The non-durable channel pool.

    Wired alongside the durable pool purely so a mis-routed acquisition or
    release lands somewhere observable: every pool test below pairs its
    positive assertion on the durable pool with a negative one here.
    """
    pool = mocker.MagicMock(name="non_durable_channel_pool")
    pool.get_channel = mocker.AsyncMock(return_value=mock_non_durable_channel)
    pool.return_channel = mocker.AsyncMock()
    return pool


@pytest.fixture
def connector(mock_durable_pool, mock_non_durable_pool):
    """An EventConsumerConnector wired to both mock pools."""
    instance = EventConsumerConnector()
    instance._durable_channel_pool = mock_durable_pool
    instance._channel_pool = mock_non_durable_pool
    return instance


@pytest.mark.asyncio
class TestDeclareExchange:
    """
    declare_exchange must declare the shared events exchange (via super())
    and then provision the dead-letter topology.
    """

    @pytest_asyncio.fixture(autouse=True)
    async def patch_super(self, mocker):
        self.super_declare = mocker.patch.object(
            RabbitMQConnector, "declare_exchange", new=mocker.AsyncMock()
        )

    async def test_calls_super_declare_exchange(self, connector):
        await connector.declare_exchange()
        self.super_declare.assert_awaited_once()

    async def test_declares_dead_letter_topology(self, connector, mock_durable_channel):
        await connector.declare_exchange()
        mock_durable_channel.declare_exchange.assert_awaited_once()

    async def test_survives_dlq_argument_drift(self, connector, dlq_argument_drift):
        # The b.rca guard at the PUBLIC entrypoint: the pre-fix code let the
        # broker's AMQP 406 out of here into connect(), crash-looping the pod.
        await connector.declare_exchange()


@pytest.mark.asyncio
class TestDeclareDeadLetterTopology:
    """
    _declare_dead_letter_topology declares the DLX, the self-bounded DLQ,
    and the DLX->DLQ binding over a DURABLE-pool channel, acquired by
    reaching into the pool directly rather than via get_channel().
    """

    async def test_no_durable_pool_is_noop(self, mock_non_durable_pool):
        instance = EventConsumerConnector()
        instance._durable_channel_pool = None
        # The non-durable pool is present so this proves which attribute the
        # guard reads: it must skip on the durable one being absent, not fall
        # back to the non-durable one.
        instance._channel_pool = mock_non_durable_pool
        # Should not raise when there is no durable pool to acquire from.
        await instance._declare_dead_letter_topology()

    async def test_no_durable_pool_does_not_use_non_durable_pool(
        self, mock_non_durable_pool
    ):
        instance = EventConsumerConnector()
        instance._durable_channel_pool = None
        instance._channel_pool = mock_non_durable_pool
        await instance._declare_dead_letter_topology()
        mock_non_durable_pool.get_channel.assert_not_awaited()

    async def test_acquires_a_durable_pool_channel(self, connector, mock_durable_pool):
        await connector._declare_dead_letter_topology()
        mock_durable_pool.get_channel.assert_awaited_once_with()

    async def test_does_not_acquire_a_non_durable_pool_channel(
        self, connector, mock_non_durable_pool
    ):
        await connector._declare_dead_letter_topology()
        mock_non_durable_pool.get_channel.assert_not_awaited()

    async def test_returns_the_channel_to_the_durable_pool(
        self, connector, mock_durable_pool, mock_durable_channel
    ):
        await connector._declare_dead_letter_topology()
        mock_durable_pool.return_channel.assert_awaited_once_with(mock_durable_channel)

    async def test_does_not_return_a_channel_to_the_non_durable_pool(
        self, connector, mock_non_durable_pool
    ):
        await connector._declare_dead_letter_topology()
        mock_non_durable_pool.return_channel.assert_not_awaited()

    async def test_returns_the_channel_to_the_durable_pool_on_error(
        self, connector, mock_durable_pool, mock_durable_channel
    ):
        mock_durable_channel.declare_exchange.side_effect = RuntimeError("boom")
        with pytest.raises(RuntimeError):
            await connector._declare_dead_letter_topology()
        mock_durable_pool.return_channel.assert_awaited_once_with(mock_durable_channel)

    async def test_does_not_return_a_channel_to_the_non_durable_pool_on_error(
        self, connector, mock_non_durable_pool, mock_durable_channel
    ):
        mock_durable_channel.declare_exchange.side_effect = RuntimeError("boom")
        with pytest.raises(RuntimeError):
            await connector._declare_dead_letter_topology()
        mock_non_durable_pool.return_channel.assert_not_awaited()

    async def test_does_not_trigger_connect(self, connector, mocker):
        # The declare runs from declare_exchange(), which connect() awaits
        # while holding its non-reentrant lock. Re-entering connect() here
        # (as get_channel()/get_channel_direct() would) deadlocks startup
        # permanently, so the direct pool reach-in must never call it.
        mock_connect = mocker.patch.object(connector, "connect", new=mocker.AsyncMock())
        await connector._declare_dead_letter_topology()
        mock_connect.assert_not_awaited()

    # --- DLX assertions -----------------------------------------------------

    async def test_declares_dlx_with_configured_name(
        self, connector, mock_durable_channel
    ):
        await connector._declare_dead_letter_topology()
        args, _ = mock_durable_channel.declare_exchange.call_args
        assert args[0] == settings.settings.dead_letter_exchange_name

    async def test_declares_dlx_as_direct(self, connector, mock_durable_channel):
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_durable_channel.declare_exchange.call_args
        assert kwargs["type"] == "direct"

    async def test_declares_dlx_durable(self, connector, mock_durable_channel):
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_durable_channel.declare_exchange.call_args
        assert kwargs["durable"] is True

    # --- DLQ assertions -----------------------------------------------------

    async def test_declares_dlq_with_configured_name(
        self, connector, mock_durable_channel
    ):
        await connector._declare_dead_letter_topology()
        args, _ = mock_durable_channel.declare_queue.call_args
        assert args[0] == settings.settings.dead_letter_queue_name

    async def test_declares_dlq_durable(self, connector, mock_durable_channel):
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_durable_channel.declare_queue.call_args
        assert kwargs["durable"] is True

    async def test_declares_dlq_as_quorum(self, connector, mock_durable_channel):
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_durable_channel.declare_queue.call_args
        assert kwargs["arguments"]["x-queue-type"] == "quorum"

    async def test_dlq_message_ttl_from_settings(self, connector, mock_durable_channel):
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_durable_channel.declare_queue.call_args
        assert (
            kwargs["arguments"]["x-message-ttl"]
            == settings.settings.dead_letter_queue_message_ttl_ms
        )

    async def test_dlq_max_length_from_settings(self, connector, mock_durable_channel):
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_durable_channel.declare_queue.call_args
        assert (
            kwargs["arguments"]["x-max-length"]
            == settings.settings.dead_letter_queue_max_length
        )

    async def test_dlq_overflow_drop_head(self, connector, mock_durable_channel):
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_durable_channel.declare_queue.call_args
        assert kwargs["arguments"]["x-overflow"] == "drop-head"

    async def test_dlq_carries_no_other_arguments(
        self, connector, mock_durable_channel
    ):
        # Notably no x-dead-letter-exchange/-routing-key: the DLQ must not
        # dead-letter onward, or the topology cycles.
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_durable_channel.declare_queue.call_args
        assert set(kwargs["arguments"]) == {
            "x-queue-type",
            "x-message-ttl",
            "x-max-length",
            "x-overflow",
        }

    # --- Binding assertions -------------------------------------------------

    async def test_binds_dlq_to_dlx(self, connector, mock_queue, mock_exchange):
        await connector._declare_dead_letter_topology()
        mock_queue.bind.assert_awaited_once_with(
            mock_exchange, settings.settings.dead_letter_routing_key
        )

    # --- Active-mode assertions (no drift) ----------------------------------

    async def test_does_not_declare_the_dlq_passively(
        self, connector, mock_durable_channel
    ):
        # Passive is the drift fallback only: a clean startup must declare the
        # arguments, otherwise a fresh broker never gets a bounded DLQ.
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_durable_channel.declare_queue.call_args
        assert not kwargs.get("passive")

    async def test_declares_the_dlq_exactly_once(self, connector, mock_durable_channel):
        await connector._declare_dead_letter_topology()
        mock_durable_channel.declare_queue.assert_awaited_once()

    async def test_does_not_close_the_channel(self, connector, mock_durable_channel):
        # The explicit close belongs to the 406 path alone. A channel that was
        # never killed goes back to the pool OPEN, for reuse; closing it here
        # would evict a healthy channel on every provisioning pass.
        await connector._declare_dead_letter_topology()
        mock_durable_channel.close.assert_not_awaited()

    async def test_logs_no_error(self, connector, error_logs):
        await connector._declare_dead_letter_topology()
        assert connector_error_messages(error_logs) == []


@pytest.mark.asyncio
class TestDeadLetterTopologyArgumentDrift:
    """
    Changing OMNI_EVENTS_DEAD_LETTER_QUEUE_MAX_LENGTH or
    ..._MESSAGE_TTL_MS against a broker whose DLQ already exists makes the
    active declare fail with AMQP 406 (queue arguments are immutable). That
    used to escape through connect() and crash-loop every pod (b.rca), so it
    must now be absorbed: log the drift, then finish provisioning passively
    on a fresh channel -- the 406 closed the first one before the bind ran.
    """

    @pytest.fixture(autouse=True)
    def _drift(self, dlq_argument_drift):
        """Every test in this class runs against a drifted DLQ."""

    @pytest_asyncio.fixture
    async def drift_error_message(self, dlq_argument_drift, connector, error_logs):
        """
        The drift ERROR, after the topology has been declared.

        ``dlq_argument_drift`` is requested explicitly rather than left to the
        autouse ``_drift`` so the drift is armed before this fixture runs the
        declare. Its being the ONLY error is pinned by
        ``test_logs_exactly_one_error``.
        """
        await connector._declare_dead_letter_topology()
        return connector_error_messages(error_logs)[0]

    # --- The regression guard ------------------------------------------------

    async def test_precondition_failure_does_not_propagate(self, connector):
        # THE b.rca regression guard: the pre-fix code let this
        # ChannelPreconditionFailed out of _declare_dead_letter_topology,
        # through declare_exchange() and connect(), crash-looping the pod.
        await connector._declare_dead_letter_topology()

    # --- The operator-facing ERROR ------------------------------------------

    async def test_logs_exactly_one_error(self, connector, error_logs):
        await connector._declare_dead_letter_topology()
        assert len(connector_error_messages(error_logs)) == 1

    async def test_error_uses_the_documented_prefix(self, drift_error_message):
        assert drift_error_message.startswith(DRIFT_LOG_PREFIX)

    async def test_error_names_the_dead_letter_queue(self, drift_error_message):
        assert settings.settings.dead_letter_queue_name in drift_error_message

    async def test_error_reports_the_desired_message_ttl(self, drift_error_message):
        ttl = settings.settings.dead_letter_queue_message_ttl_ms
        assert f"x-message-ttl={ttl}" in drift_error_message

    async def test_error_reports_the_desired_max_length(self, drift_error_message):
        max_length = settings.settings.dead_letter_queue_max_length
        assert f"x-max-length={max_length}" in drift_error_message

    async def test_error_reports_the_desired_overflow(self, drift_error_message):
        assert "x-overflow=drop-head" in drift_error_message

    async def test_error_reports_the_broker_error(self, drift_error_message):
        assert PRECONDITION_FAILED_MESSAGE in drift_error_message

    async def test_error_offers_the_policy_remedy_for_lowering(
        self, drift_error_message
    ):
        # Checked loosely: the remedy wording may be reworded, but it has to
        # keep pointing at a broker policy and its min-wins resolution.
        message = drift_error_message.lower()
        assert "policy" in message
        assert "min-wins" in message

    async def test_error_offers_the_delete_remedy_for_raising(
        self, drift_error_message
    ):
        # Likewise loose: a bound can only be raised by recreating the queue.
        message = drift_error_message.lower()
        assert "drain" in message
        assert "delete" in message

    # --- The deterministic close of the 406-killed channel -------------------

    async def test_closes_the_channel_the_broker_killed(
        self, connector, mock_durable_channel
    ):
        # aio-pika's RobustChannel AUTO-REOPENS a channel the broker closed, so
        # without an explicit close the pool sees a restored, open channel and
        # may keep it -- or, losing the race against the restore, evict and
        # forget a channel that then reopens untracked: one orphaned AMQP
        # channel per drift-reconnect. Closing it makes eviction deterministic.
        await connector._declare_dead_letter_topology()
        mock_durable_channel.close.assert_awaited_once()

    async def test_closes_the_channel_before_returning_it_to_the_pool(
        self, connector, mocker, mock_durable_channel, mock_durable_pool
    ):
        # Order is the whole mechanism: the pool keys eviction on
        # ``is_closed`` at return time, so a close issued after the return
        # would evict nothing.
        recorder = mocker.MagicMock(name="call_order")
        recorder.attach_mock(mock_durable_channel.close, "close")
        recorder.attach_mock(mock_durable_pool.return_channel, "return_channel")
        await connector._declare_dead_letter_topology()
        assert [call[0] for call in recorder.mock_calls] == [
            "close",
            "return_channel",
            "return_channel",
        ]

    async def test_does_not_close_the_second_channel(
        self, connector, mock_second_durable_channel
    ):
        # The re-provisioning channel is healthy. Its lifecycle stays with the
        # pool it is handed back to; the provisioning must not close it.
        await connector._declare_dead_letter_topology()
        mock_second_durable_channel.close.assert_not_awaited()

    # --- The passive re-provision -------------------------------------------

    async def test_acquires_a_second_durable_channel(
        self, connector, mock_durable_pool
    ):
        # The 406 closed the first channel, so the bind cannot be retried on
        # it -- the re-provision needs a fresh channel from the same pool.
        await connector._declare_dead_letter_topology()
        assert mock_durable_pool.get_channel.await_count == 2

    async def test_does_not_acquire_a_non_durable_pool_channel(
        self, connector, mock_non_durable_pool
    ):
        await connector._declare_dead_letter_topology()
        mock_non_durable_pool.get_channel.assert_not_awaited()

    async def test_redeclares_the_dlx_on_the_second_channel(
        self, connector, mock_second_durable_channel
    ):
        await connector._declare_dead_letter_topology()
        args, _ = mock_second_durable_channel.declare_exchange.call_args
        assert args[0] == settings.settings.dead_letter_exchange_name

    async def test_redeclares_the_dlx_as_a_durable_direct_exchange(
        self, connector, mock_second_durable_channel
    ):
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_second_durable_channel.declare_exchange.call_args
        assert (kwargs["type"], kwargs["durable"]) == ("direct", True)

    async def test_obtains_the_dlq_by_configured_name(
        self, connector, mock_second_durable_channel
    ):
        await connector._declare_dead_letter_topology()
        args, _ = mock_second_durable_channel.declare_queue.call_args
        assert args[0] == settings.settings.dead_letter_queue_name

    async def test_obtains_the_dlq_passively(
        self, connector, mock_second_durable_channel
    ):
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_second_durable_channel.declare_queue.call_args
        assert kwargs["passive"] is True

    async def test_obtains_the_dlq_non_robustly(
        self, connector, mock_second_durable_channel
    ):
        # robust=False keeps aio-pika's RobustChannel from storing the passive
        # reference and re-declaring it on every reconnect.
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_second_durable_channel.declare_queue.call_args
        assert kwargs["robust"] is False

    async def test_passive_declare_sends_no_arguments(
        self, connector, mock_second_durable_channel
    ):
        # Re-sending the desired arguments would just take the same 406.
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_second_durable_channel.declare_queue.call_args
        assert "arguments" not in kwargs

    async def test_binds_the_existing_dlq_to_the_dlx(
        self, connector, mock_passive_queue, mock_passive_exchange
    ):
        # The bind is the whole point of the second pass: without it the DLX
        # has no queue and dead-lettered messages are dropped.
        await connector._declare_dead_letter_topology()
        mock_passive_queue.bind.assert_awaited_once_with(
            mock_passive_exchange, settings.settings.dead_letter_routing_key
        )

    async def test_returns_both_channels_to_the_durable_pool(
        self,
        connector,
        mock_durable_pool,
        mock_durable_channel,
        mock_second_durable_channel,
    ):
        await connector._declare_dead_letter_topology()
        returned = [
            call.args[0] for call in mock_durable_pool.return_channel.await_args_list
        ]
        assert returned == [mock_durable_channel, mock_second_durable_channel]

    async def test_does_not_return_a_channel_to_the_non_durable_pool(
        self, connector, mock_non_durable_pool
    ):
        await connector._declare_dead_letter_topology()
        mock_non_durable_pool.return_channel.assert_not_awaited()


@pytest.mark.asyncio
class TestKilledChannelCloseIsBestEffort:
    """
    The explicit close of the 406-killed channel must not be able to break
    drift handling.

    Closing an already-closed channel does not raise, but a close that reaches
    the broker (the channel having been auto-restored first) can still fail on
    a connection-level problem. Letting that escape would turn graceful drift
    handling back into the b.rca startup crash-loop -- an orphaned channel is a
    slow, bounded leak, while a raise here is an unbootable pod. So the failure
    is suppressed and reported at WARNING, and everything downstream of it
    still runs.
    """

    @pytest.fixture(autouse=True)
    def _drift_with_a_failing_close(self, dlq_argument_drift, mock_durable_channel):
        """Drifted DLQ, and the close of the killed channel fails too."""
        mock_durable_channel.close.side_effect = RuntimeError(CLOSE_FAILURE_MESSAGE)

    async def test_close_failure_does_not_propagate(self, connector):
        # The regression this suppression exists to prevent.
        await connector._declare_dead_letter_topology()

    async def test_still_logs_the_drift_error(self, connector, warning_logs):
        # The operator-facing ERROR is the actionable one; a failed close must
        # not cost them the message that names the remedy.
        await connector._declare_dead_letter_topology()
        errors = connector_error_messages(warning_logs)
        assert [message.startswith(DRIFT_LOG_PREFIX) for message in errors] == [True]

    async def test_logs_a_warning_naming_the_failed_close(
        self, connector, warning_logs
    ):
        await connector._declare_dead_letter_topology()
        warnings = connector_warning_messages(warning_logs)
        assert any(message.startswith(CLOSE_FAILURE_LOG_PREFIX) for message in warnings)

    async def test_warning_reports_the_underlying_close_error(
        self, connector, warning_logs
    ):
        # Without the cause, an operator cannot tell a benign already-closed
        # race from a connection that is genuinely gone.
        await connector._declare_dead_letter_topology()
        warnings = connector_warning_messages(warning_logs)
        assert any(CLOSE_FAILURE_MESSAGE in message for message in warnings)

    async def test_still_reprovisions_passively_on_a_second_channel(
        self, connector, mock_second_durable_channel
    ):
        await connector._declare_dead_letter_topology()
        _, kwargs = mock_second_durable_channel.declare_queue.call_args
        assert kwargs["passive"] is True

    async def test_still_binds_the_existing_dlq_to_the_dlx(
        self, connector, mock_passive_queue, mock_passive_exchange
    ):
        # The bind is what keeps dead-lettered messages from being dropped, so
        # it has to survive a failed close.
        await connector._declare_dead_letter_topology()
        mock_passive_queue.bind.assert_awaited_once_with(
            mock_passive_exchange, settings.settings.dead_letter_routing_key
        )

    async def test_still_returns_both_channels_to_the_durable_pool(
        self,
        connector,
        mock_durable_pool,
        mock_durable_channel,
        mock_second_durable_channel,
    ):
        await connector._declare_dead_letter_topology()
        returned = [
            call.args[0] for call in mock_durable_pool.return_channel.await_args_list
        ]
        assert returned == [mock_durable_channel, mock_second_durable_channel]


@pytest.mark.asyncio
class TestDeadLetterTopologyFatalFailures:
    """
    Only a precondition failure on the DLQ declare is drift. Everything else
    is a real broker problem and must still propagate out of
    _declare_dead_letter_topology rather than being swallowed by the new
    tolerance.
    """

    async def test_dlx_precondition_failure_propagates(
        self, connector, mock_durable_channel
    ):
        # An exchange redeclared with a different type is a genuine topology
        # conflict; a passive DLQ pass would not fix it.
        mock_durable_channel.declare_exchange.side_effect = (
            aio_pika.exceptions.ChannelPreconditionFailed(PRECONDITION_FAILED_MESSAGE)
        )
        with pytest.raises(aio_pika.exceptions.ChannelPreconditionFailed):
            await connector._declare_dead_letter_topology()

    async def test_dlx_precondition_failure_is_not_reported_as_drift(
        self, connector, mock_durable_channel, error_logs
    ):
        mock_durable_channel.declare_exchange.side_effect = (
            aio_pika.exceptions.ChannelPreconditionFailed(PRECONDITION_FAILED_MESSAGE)
        )
        with pytest.raises(aio_pika.exceptions.ChannelPreconditionFailed):
            await connector._declare_dead_letter_topology()
        assert connector_error_messages(error_logs) == []

    async def test_dlx_precondition_failure_does_not_reprovision(
        self, connector, mock_durable_channel, mock_durable_pool
    ):
        mock_durable_channel.declare_exchange.side_effect = (
            aio_pika.exceptions.ChannelPreconditionFailed(PRECONDITION_FAILED_MESSAGE)
        )
        with pytest.raises(aio_pika.exceptions.ChannelPreconditionFailed):
            await connector._declare_dead_letter_topology()
        mock_durable_pool.get_channel.assert_awaited_once_with()

    async def test_non_precondition_dlq_failure_propagates(
        self, connector, mock_durable_channel
    ):
        mock_durable_channel.declare_queue.side_effect = RuntimeError("boom")
        with pytest.raises(RuntimeError, match="boom"):
            await connector._declare_dead_letter_topology()

    async def test_non_precondition_dlq_failure_does_not_reprovision(
        self, connector, mock_durable_channel, mock_durable_pool
    ):
        mock_durable_channel.declare_queue.side_effect = RuntimeError("boom")
        with pytest.raises(RuntimeError, match="boom"):
            await connector._declare_dead_letter_topology()
        mock_durable_pool.get_channel.assert_awaited_once_with()

    async def test_non_precondition_dlq_failure_does_not_close_the_channel(
        self, connector, mock_durable_channel
    ):
        # The explicit close is scoped to the 406, which is the only failure
        # that leaves a broker-killed channel behind. Any other error leaves
        # channel lifecycle entirely to the pool.
        mock_durable_channel.declare_queue.side_effect = RuntimeError("boom")
        with pytest.raises(RuntimeError, match="boom"):
            await connector._declare_dead_letter_topology()
        mock_durable_channel.close.assert_not_awaited()

    async def test_missing_queue_on_the_passive_probe_propagates(
        self, connector, dlq_argument_drift, mock_second_durable_channel
    ):
        # The queue being genuinely absent is not drift: nothing sensible can
        # be done with it here, so startup should fail loudly.
        mock_second_durable_channel.declare_queue.side_effect = (
            aio_pika.exceptions.ChannelNotFoundEntity(NOT_FOUND_CODE, NOT_FOUND_MESSAGE)
        )
        with pytest.raises(aio_pika.exceptions.ChannelNotFoundEntity):
            await connector._declare_dead_letter_topology()

    async def test_precondition_failure_on_the_passive_probe_propagates(
        self, connector, dlq_argument_drift, mock_second_durable_channel
    ):
        # The mismatch handler runs once; it must not recurse into a second
        # fallback and mask a broker that keeps refusing the declare.
        mock_second_durable_channel.declare_queue.side_effect = (
            aio_pika.exceptions.ChannelPreconditionFailed(PRECONDITION_FAILED_MESSAGE)
        )
        with pytest.raises(aio_pika.exceptions.ChannelPreconditionFailed):
            await connector._declare_dead_letter_topology()

    async def test_passive_probe_failure_still_returns_both_channels(
        self,
        connector,
        dlq_argument_drift,
        mock_durable_pool,
        mock_durable_channel,
        mock_second_durable_channel,
    ):
        mock_second_durable_channel.declare_queue.side_effect = (
            aio_pika.exceptions.ChannelNotFoundEntity(NOT_FOUND_CODE, NOT_FOUND_MESSAGE)
        )
        with pytest.raises(aio_pika.exceptions.ChannelNotFoundEntity):
            await connector._declare_dead_letter_topology()
        returned = [
            call.args[0] for call in mock_durable_pool.return_channel.await_args_list
        ]
        assert returned == [mock_durable_channel, mock_second_durable_channel]
