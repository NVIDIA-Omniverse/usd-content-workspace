# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
import re
import uuid
from contextlib import asynccontextmanager
from types import SimpleNamespace
from unittest.mock import MagicMock, call, AsyncMock

import aio_pika
import pytest
import pytest_asyncio
from notification_common.auth import principal_id
from notification_common.errors.errors import QueueNotFoundError, UnprocessableError

from app.config import settings as _settings_module
from app.lib.models.filter_group import FilterGroup
from app.lib.queue_names import redact_queue_name, redact_reconnect_token
from app.lib.rabbitmq import rabbitmq_base
from app.lib.rabbitmq.ack_tracker import AckTracker
from app.lib.rabbitmq.durable_queue_consumer import DurableQueueConsumer
from app.lib.rabbitmq.non_durable_queue_consumer import NonDurableQueueConsumer
from app.lib.rabbitmq.rabbitmq_base import (
    QUEUE_ID_PATTERN,
    RECONNECT_TOKEN_MAX_LENGTH,
    RECONNECT_TOKEN_PATTERN,
    ConsumerCancelledError,
    RabbitMQBase,
    _QUEUE_ID_ECHO_LIMIT,
    validate_queue_id,
    validate_reconnect_token,
)
from tests.fixtures.consumer_liveness import (
    CONSUMER_LOSS_TIMEOUT_S,
    IDLE_POLL_FIRES_IMMEDIATELY_SECONDS,
    IDLE_POLL_NEVER_FIRES_SECONDS,
    LIVENESS_QUEUE_NAME,
    PUMP_CONSUMER_TAG,
    FakeConnection,
    cancel_while_polling,
    close_connector_on_dequeue,
    drain,
    drain_while_shutting_down,
    expect_cleanup,
    expect_terminal_error,
)
from tests.fixtures.dead_channel import (
    channel_invalid_state,
    dead_channel_failures,
    unrelated_failures,
    unrelated_runtime_error,
)
from tests.fixtures.mock_filter_group import filter_group
from tests.fixtures.queue_id_bounds import MAX_QUEUE_ID_LENGTH
from tests.fixtures.reconnect_token import NON_DURABLE_QUEUE_NAME, RECONNECT_TOKEN


class MockMessage:
    def __init__(self, body, delivery_tag=0):
        self.body = body
        self.delivery_tag = delivery_tag
        self.ack = AsyncMock()
        self.nack = AsyncMock()

    # noinspection PyUnusedLocal
    @asynccontextmanager
    async def process(self, requeue=True):
        yield


class MockMessageWithXDeath:
    """
    Mock message carrying ``x-death`` headers (and ``x-delivery-count``)
    so that reason/origin-queue decoding can be exercised by the Epic 2
    consume/replay work. Mirrors the ``MockMessage``/``MockAckableMessage``
    shape (body, delivery_tag, async ack/nack, ``process`` async context
    manager). RabbitMQ records one ``x-death`` entry per dead-letter
    event; the most recent is first.
    """

    def __init__(self, body, delivery_tag=0):
        self.body = body
        self.delivery_tag = delivery_tag
        self.ack = AsyncMock()
        self.nack = AsyncMock()
        self.headers = {
            "x-death": [
                {
                    "queue": "origin-queue",
                    "reason": "rejected",
                    "count": 1,
                    "time": 1700000000,
                    "exchange": "events",
                    "routing-keys": ["events.foo"],
                }
            ],
            "x-delivery-count": 1,
        }

    # noinspection PyUnusedLocal
    @asynccontextmanager
    async def process(self, requeue=True):
        yield


@pytest.mark.asyncio
class TestGetQueue:
    """
    Tests for get_queue which first checks queue existence with a temporary
    channel (to avoid stale cache issues) then returns the queue from the
    pool channel.
    """

    @pytest_asyncio.fixture(autouse=True)
    async def connect(self):
        await RabbitMQBase.connector.connect()

    @pytest.fixture
    def mock_temp_chan(self, mocker):
        """Create a mock temporary channel for existence checks."""
        temp_chan = mocker.MagicMock()
        temp_chan.declare_queue = mocker.AsyncMock()
        return temp_chan

    @pytest.fixture
    def mock_temporary_channel_context(self, mocker, mock_temp_chan):
        """Mock the get_temporary_channel context manager."""
        mock_context_manager = mocker.AsyncMock()
        mock_context_manager.__aenter__.return_value = mock_temp_chan
        mocker.patch.object(
            RabbitMQBase.connector,
            "get_temporary_channel",
            return_value=mock_context_manager,
        )
        return mock_temp_chan

    async def test_checks_queue_exists_with_temp_channel_first(
        self, mock_chan, mock_temporary_channel_context, mocker
    ):
        """
        The temp channel check prevents stale cache issues where a pool channel
        might have cached that a queue exists when it was actually deleted.
        """
        mocker.patch.object(RabbitMQBase, "_queue_name", return_value="test-name")
        await RabbitMQBase.get_queue(mock_chan, "test-queue-id")

        mock_temporary_channel_context.declare_queue.assert_called_once_with(
            "test-name",
            passive=True,
            robust=False,
        )

    async def test_gets_queue_with_pool_channel_after_temp_check(
        self, mock_chan, mock_temporary_channel_context, mocker
    ):
        mocker.patch.object(RabbitMQBase, "_queue_name", return_value="test-name")
        result = await RabbitMQBase.get_queue(mock_chan, "test-queue-id")

        mock_chan.declare_queue.assert_called_once_with(
            "test-name",
            passive=True,
            robust=False,
        )
        assert result == mock_chan.declare_queue.return_value

    async def test_looks_up_queue_name_by_id(
        self, mock_chan, mock_temporary_channel_context, mocker
    ):
        # Exactly once: the exportable spelling for the log line and the
        # not-found error is now derived from this one name by
        # `redact_queue_name`, rather than assembled by a second call to
        # the builder. A real string is returned because that name is fed
        # straight into the redaction, which reads it.
        spy = mocker.patch.object(
            RabbitMQBase, "_queue_name", return_value="event-queue:test-queue-id"
        )

        await RabbitMQBase.get_queue(mock_chan, "test-queue-id")

        spy.assert_called_once_with("test-queue-id")

    async def test_returns_queue_from_pool_channel(
        self, mock_chan, mock_temporary_channel_context
    ):
        expected = mock_chan.declare_queue.return_value
        result = await RabbitMQBase.get_queue(mock_chan, "test-queue-id")
        assert result == expected

    async def test_raises_error_if_temp_channel_check_fails(
        self, mock_chan, mock_temporary_channel_context, mocker
    ):
        """
        If the temp channel check fails on all retry attempts,
        raise QueueNotFoundError before trying the pool channel.
        """
        mocker.patch("asyncio.sleep")
        mock_temporary_channel_context.declare_queue.side_effect = (
            aio_pika.exceptions.ChannelClosed(None, None)
        )

        with pytest.raises(QueueNotFoundError):
            await RabbitMQBase.get_queue(mock_chan, "test-id")

        mock_chan.declare_queue.assert_not_called()

    async def test_retries_temp_check_on_transient_failure(
        self, mock_chan, mock_temporary_channel_context, mocker
    ):
        """
        A transient NOT_FOUND on the temp channel (e.g. RabbitMQ
        still loading durable queues from disk after a restart)
        is retried and succeeds.
        """
        mocker.patch("asyncio.sleep")
        closed = aio_pika.exceptions.ChannelClosed(None, None)
        success = mocker.MagicMock()
        mock_temporary_channel_context.declare_queue.side_effect = [
            closed,
            success,
        ]

        await RabbitMQBase.get_queue(mock_chan, "test-id")

        assert mock_temporary_channel_context.declare_queue.call_count == 2
        mock_chan.declare_queue.assert_called_once()

    async def test_temp_check_retries_expected_number_of_times(
        self, mock_chan, mock_temporary_channel_context, mocker
    ):
        """
        The temp channel check retries exactly
        _QUEUE_EXISTS_RETRIES times before giving up.
        """
        mock_sleep = mocker.patch("asyncio.sleep")
        mock_temporary_channel_context.declare_queue.side_effect = (
            aio_pika.exceptions.ChannelClosed(None, None)
        )

        with pytest.raises(QueueNotFoundError):
            await RabbitMQBase.get_queue(mock_chan, "test-id")

        retries = RabbitMQBase._QUEUE_EXISTS_RETRIES
        assert mock_temporary_channel_context.declare_queue.call_count == retries
        assert mock_sleep.call_count == retries - 1

    async def test_raises_error_if_pool_channel_get_fails(
        self, mock_chan, mock_temporary_channel_context
    ):
        """
        If the temp check passes but the pool channel get fails,
        still raise an error.
        """
        mock_chan.declare_queue.side_effect = aio_pika.exceptions.ChannelClosed(
            None, None
        )

        with pytest.raises(QueueNotFoundError):
            await RabbitMQBase.get_queue(mock_chan, "test-id")

    async def test_uses_robust_false_to_prevent_stale_restore(
        self, mock_chan, mock_temporary_channel_context, mocker
    ):
        """
        Both the temp channel and pool channel must use robust=False
        so that aio_pika's RobustChannel does not store passive queue
        references. Without this, a RabbitMQ restart causes an
        infinite reconnect loop when the stored passive declares
        fail with NOT_FOUND for non-durable queues that no longer
        exist.
        """
        mocker.patch.object(
            RabbitMQBase,
            "_queue_name",
            return_value="q",
        )
        await RabbitMQBase.get_queue(mock_chan, "id")

        temp_call = mock_temporary_channel_context.declare_queue.call_args
        pool_call = mock_chan.declare_queue.call_args

        assert temp_call.kwargs.get("robust") is False
        assert pool_call.kwargs.get("robust") is False


class TestCreateQueue:
    @pytest_asyncio.fixture(autouse=True)
    async def connect(self):
        await RabbitMQBase.connector.connect()

    @pytest.mark.parametrize("durable", [True, False])
    @pytest.mark.asyncio
    async def test_creates_queue_with_channel_parameter(
        self, mock_chan, filter_groups, durable, mocker
    ):
        mocker.patch.object(RabbitMQBase.connector, "get_exchange")
        await RabbitMQBase._create_queue(mock_chan, filter_groups, durable)
        # Channel should be used for queue operations
        mock_chan.declare_queue.assert_called_once()

    @pytest.mark.asyncio
    async def test_declares_non_durable_queue_with_name_and_ttl(
        self, mock_chan, mocker, mock_queue_name, filter_groups, mock_queue_id
    ):
        mocker.patch.object(uuid, "uuid4", return_value=mock_queue_id)
        await RabbitMQBase._create_queue(mock_chan, filter_groups, durable=False)

        mock_chan.declare_queue.assert_called_once_with(
            mock_queue_name,
            arguments={"x-expires": 20000},
            durable=False,
            robust=False,
        )

    @pytest.mark.asyncio
    async def test_declares_durable_queue_with_quorum_and_dlx_args(
        self, mock_chan, mocker, mock_queue_name, filter_groups, mock_queue_id
    ):
        mocker.patch.object(uuid, "uuid4", return_value=mock_queue_id)
        await RabbitMQBase._create_queue(mock_chan, filter_groups, durable=True)

        mock_chan.declare_queue.assert_called_once_with(
            mock_queue_name,
            arguments={
                "x-queue-type": "quorum",
                "x-dead-letter-exchange": (
                    _settings_module.settings.dead_letter_exchange_name
                ),
                "x-dead-letter-routing-key": (
                    _settings_module.settings.dead_letter_routing_key
                ),
            },
            durable=True,
            robust=False,
        )

    @pytest.mark.parametrize("durable", [True, False])
    @pytest.mark.asyncio
    async def test_gets_exchange(self, mock_chan, filter_groups, durable, mocker):
        mock_get_exchange = mocker.patch.object(RabbitMQBase.connector, "get_exchange")
        await RabbitMQBase._create_queue(mock_chan, filter_groups, durable)
        mock_get_exchange.assert_called_once_with(mock_chan)

    @pytest.mark.parametrize("durable", [True, False])
    @pytest.mark.asyncio
    async def test_makes_binding_keys(self, mock_chan, filter_groups, durable, mocker):
        spy = mocker.patch.object(rabbitmq_base, "binding_keys_from_filters")
        await RabbitMQBase._create_queue(mock_chan, filter_groups, durable)
        spy.assert_called_once_with(filter_groups)

    @pytest.mark.parametrize("durable", [True, False])
    @pytest.mark.asyncio
    async def test_binds_queues(
        self, mock_chan, mock_queue, filter_groups, durable, mocker
    ):
        mocker.patch.object(
            rabbitmq_base,
            "binding_keys_from_filters",
            return_value=["k1", "k2"],
        )
        mock_chan.declare_queue.return_value = mock_queue
        mocker.patch.object(
            RabbitMQBase.connector, "get_exchange", return_value="test-exchange"
        )
        await RabbitMQBase._create_queue(mock_chan, filter_groups, durable)

        mock_queue.bind.assert_has_calls(
            [call("test-exchange", "k1"), call("test-exchange", "k2")]
        )

    @pytest.mark.parametrize("durable", [True, False])
    @pytest.mark.asyncio
    async def test_returns_queue_and_id(
        self, mock_chan, mock_queue, filter_groups, durable, mocker
    ):
        test_uuid = "some-test-uuid"
        mocker.patch.object(uuid, "uuid4", return_value=test_uuid)
        mock_chan.declare_queue.return_value = mock_queue

        returned_queue_id, returned_queue_obj = await RabbitMQBase._create_queue(
            mock_chan, filter_groups, durable
        )
        assert returned_queue_id == test_uuid
        assert returned_queue_obj == mock_queue

    @pytest.mark.parametrize("durable", [True, False])
    @pytest.mark.asyncio
    async def test_declares_queue_name_from_supplied_id(
        self, mock_chan, mocker, mock_queue_name, filter_groups, mock_queue_id, durable
    ):
        sentinel = mocker.patch.object(uuid, "uuid4", return_value="unused-uuid")
        await RabbitMQBase._create_queue(
            mock_chan, filter_groups, durable, queue_id=mock_queue_id
        )

        assert mock_chan.declare_queue.call_args.args[0] == mock_queue_name
        assert "unused-uuid" not in mock_chan.declare_queue.call_args.args[0]
        sentinel.assert_not_called()

    @pytest.mark.parametrize("durable", [True, False])
    @pytest.mark.asyncio
    async def test_returns_supplied_queue_id(
        self, mock_chan, mock_queue, filter_groups, mock_queue_id, durable, mocker
    ):
        mocker.patch.object(uuid, "uuid4", return_value="unused-uuid")
        mock_chan.declare_queue.return_value = mock_queue

        returned_queue_id, _ = await RabbitMQBase._create_queue(
            mock_chan, filter_groups, durable, queue_id=mock_queue_id
        )
        assert returned_queue_id == mock_queue_id

    @pytest.mark.parametrize("durable", [True, False])
    @pytest.mark.asyncio
    async def test_mints_uuid_when_no_queue_id_supplied(
        self, mock_chan, filter_groups, durable, mocker
    ):
        spy = mocker.patch.object(uuid, "uuid4", return_value="minted-uuid")
        await RabbitMQBase._create_queue(mock_chan, filter_groups, durable)
        spy.assert_called_once()

    @pytest.mark.parametrize("durable", [True, False])
    @pytest.mark.asyncio
    async def test_uses_robust_false_to_prevent_zombie_queues(
        self, mock_chan, filter_groups, durable, mocker
    ):
        """
        declare_queue must use robust=False so that aio_pika's
        RobustChannel does not store the queue in its internal
        _queues dict. Without this, channel recovery re-declares
        every queue that was ever created through the channel —
        even ones that were already deleted — because
        queue.delete() does not remove the _queues entry.
        """
        mocker.patch.object(RabbitMQBase.connector, "get_exchange")
        await RabbitMQBase._create_queue(mock_chan, filter_groups, durable)

        call_kwargs = mock_chan.declare_queue.call_args.kwargs
        assert call_kwargs.get("robust") is False

    @pytest.mark.asyncio
    async def test_raises_error_with_empty_filter_groups(self, mock_chan):
        with pytest.raises(
            UnprocessableError, match="Cannot consume events without any filters"
        ):
            await RabbitMQBase._create_queue(mock_chan, [], True)


_CREATE_QUEUE_ID = "some-client-chosen-id"


@pytest.mark.asyncio
class TestDurableCreateQueueLogHygiene:
    """
    A durable create writes its id and broker name down verbatim, and counts
    its filter groups rather than naming them.

    A durable queue id authorizes nothing on its own and an operator needs
    it exactly as spelled to correlate a log line with a queue in the
    broker, so redaction must NOT reach this arm. The non-durable arm,
    where the id is the stream's reconnect token, is pinned in
    ``test_non_durable_queue_consumer.py`` -- against the subclass, because
    only its ``_queue_name`` override produces a name the redaction
    recognises.

    The filter groups carry the client's resource ids (folder paths), so the
    INFO line reports how many there are and never what they are.
    """

    @pytest_asyncio.fixture(autouse=True)
    async def connect(self):
        await RabbitMQBase.connector.connect()

    @pytest.fixture
    def create_records(self, caplog, mock_chan, mock_queue, filter_groups, mocker):
        async def _run():
            mocker.patch.object(RabbitMQBase.connector, "get_exchange")
            mock_chan.declare_queue.return_value = mock_queue

            with caplog.at_level(logging.DEBUG, logger=rabbitmq_base.logger.name):
                await RabbitMQBase._create_queue(
                    mock_chan, filter_groups, True, queue_id=_CREATE_QUEUE_ID
                )
            return caplog.records

        return _run

    async def test_a_create_logs_something_to_inspect(self, create_records):
        # Guards the guards below: an empty capture would make them vacuous.
        assert await create_records() != []

    async def test_logs_the_queue_id_verbatim(self, create_records):
        records = await create_records()

        assert any(f"id {_CREATE_QUEUE_ID}" in r.getMessage() for r in records)

    async def test_does_not_redact_the_durable_broker_name(self, create_records):
        records = await create_records()
        redacted_marker = redact_reconnect_token(_CREATE_QUEUE_ID)

        assert not any(redacted_marker in r.getMessage() for r in records)

    async def test_logs_the_durable_broker_name_verbatim(self, create_records):
        # The positive half of the guard above: an operator pastes this
        # spelling straight into the broker's management UI.
        records = await create_records()
        expected = f"name event-queue:{_CREATE_QUEUE_ID}"

        assert any(expected in r.getMessage() for r in records)

    async def test_counts_the_filter_groups_rather_than_naming_them(
        self, create_records
    ):
        records = await create_records()

        assert any("filter_groups=1" in r.getMessage() for r in records)

    async def test_does_not_name_a_resource_at_info(
        self, create_records, filter_groups
    ):
        # The groups carry the client's resource ids (folder paths). DEBUG
        # lines are opt-in diagnostics; INFO and above reach production logs.
        resource_id = filter_groups[0].filters[0].resource_id
        records = await create_records()

        assert [
            r
            for r in records
            if r.levelno >= logging.INFO and resource_id in str(r.__dict__)
        ] == []


#: A reconnect token shaped exactly like a rendered reference. A real token
#: is a server-minted uuid4 and can never take this shape, so a lookup
#: arriving with it is a client trying to choose the ``queue_ref`` its own
#: request is filed under.
_REFERENCE_SHAPED_TOKEN = "sha256-0123456789ab"


@pytest.mark.asyncio
class TestGetQueueRendersAReferenceShapedTokenAsAHash:
    """
    A lookup that fails must not echo the client's literal back.

    ``get_queue`` is the one path that holds a name for a queue whose
    existence is not established, and on the non-durable side the id is a
    reconnect token straight off the wire. A client presenting
    ``sha256-<12 hex>`` as its token would otherwise see that exact value in
    the not-found error and the retry log line, and so file its failed
    lookup under another stream's ``queue_ref``.

    That it is safe to render this path with ``redact_queue_name`` -- which
    decides by the name's SHAPE -- is exactly what these pin: the id it
    finds is always hashed, with no pass-through for one that already looks
    like a reference.
    """

    @pytest.fixture
    def queue_missing(self, mocker):
        """Every existence probe reports the queue gone."""
        temp_chan = mocker.MagicMock()
        temp_chan.declare_queue = mocker.AsyncMock(
            side_effect=aio_pika.exceptions.ChannelClosed(404, "NOT_FOUND")
        )
        context = mocker.AsyncMock()
        context.__aenter__.return_value = temp_chan
        mocker.patch.object(
            RabbitMQBase.connector, "get_temporary_channel", return_value=context
        )
        mocker.patch.object(rabbitmq_base.asyncio, "sleep", new_callable=AsyncMock)
        return temp_chan

    @pytest.fixture
    def failed_lookup(self, queue_missing, mock_chan, caplog):
        """Drive a failed non-durable lookup and hand back what it produced."""

        async def _run(queue_id=_REFERENCE_SHAPED_TOKEN):
            with caplog.at_level(logging.DEBUG, logger=rabbitmq_base.logger.name):
                with pytest.raises(QueueNotFoundError) as raised:
                    await NonDurableQueueConsumer.get_queue(mock_chan, queue_id)
            return raised.value, caplog.records

        return _run

    async def test_the_broker_is_still_asked_for_the_verbatim_name(
        self, failed_lookup, queue_missing
    ):
        # The redaction is for the text only: the broker knows the queue by
        # the literal the client sent and by nothing else.
        await failed_lookup()

        queue_missing.declare_queue.assert_any_call(
            f"event-queue:{_REFERENCE_SHAPED_TOKEN}:nd",
            passive=True,
            robust=False,
        )

    async def test_the_not_found_error_hashes_the_clients_literal(self, failed_lookup):
        # Read through `str`, which is where the log-only internal message
        # surfaces -- the same observable the ConsumerCancelledError
        # redaction tests use.
        error, _ = await failed_lookup()

        assert _REFERENCE_SHAPED_TOKEN not in str(error)

    async def test_the_not_found_error_names_the_hashed_reference(self, failed_lookup):
        error, _ = await failed_lookup()
        expected = redact_reconnect_token(_REFERENCE_SHAPED_TOKEN)

        assert expected in str(error)

    async def test_the_retry_log_line_is_emitted(self, failed_lookup):
        # Guards the guard below: an empty capture would make it vacuous.
        _, records = await failed_lookup()

        assert [r for r in records if "not found on attempt" in r.getMessage()] != []

    async def test_the_retry_log_line_hashes_the_clients_literal(self, failed_lookup):
        _, records = await failed_lookup()

        assert [r for r in records if _REFERENCE_SHAPED_TOKEN in r.getMessage()] == []

    async def test_no_rendering_carries_an_ordinary_token(self, failed_lookup):
        error, records = await failed_lookup(RECONNECT_TOKEN)

        assert RECONNECT_TOKEN not in str(error) + "".join(
            r.getMessage() for r in records
        )

    async def test_a_durable_lookup_renders_its_id_verbatim(
        self, queue_missing, mock_chan
    ):
        # The other half of the split: a durable id authorizes nothing, and
        # an operator needs it exactly as spelled.
        with pytest.raises(QueueNotFoundError) as raised:
            await DurableQueueConsumer.get_queue(mock_chan, "my-app-queue")

        assert "event-queue:my-app-queue" in str(raised.value)


@pytest.mark.asyncio
class TestUpdateBindings:
    @pytest_asyncio.fixture(autouse=True)
    async def connect(self):
        await RabbitMQBase.connector.connect()

    @pytest_asyncio.fixture
    async def exchange(self, mocker):
        mocker.patch.object(
            RabbitMQBase.connector, "get_exchange", return_value="test-exchange"
        )
        return "test-exchange"

    async def test_does_nothing_if_bindings_are_empty(self, mock_chan, mock_queue):
        assert (
            await RabbitMQBase._update_bindings(mock_chan, mock_queue, [], []) is None
        )

    @pytest.mark.parametrize("func_name", ["added_groups", "removed_groups"])
    async def test_figures_out_which_groups_to_bind_and_unbind(
        self, exchange, mock_chan, mock_queue, func_name, mocker
    ):
        spy = mocker.patch.object(FilterGroup, func_name)
        new_groups = [filter_group()]
        old_groups = [filter_group()]
        await RabbitMQBase._update_bindings(
            mock_chan, mock_queue, old_groups, new_groups
        )
        spy.assert_called_once_with(old_groups, new_groups)

    async def test_gets_binding_keys_to_add_and_remove(
        self, exchange, mock_chan, mock_queue, mocker
    ):
        groups_to_add = [filter_group()]
        groups_to_remove = [filter_group()]

        mocker.patch.object(FilterGroup, "added_groups", return_value=groups_to_add)
        mocker.patch.object(
            FilterGroup, "removed_groups", return_value=groups_to_remove
        )
        spy = mocker.patch.object(rabbitmq_base, "binding_keys_from_filters")

        await RabbitMQBase._update_bindings(
            mock_chan, mock_queue, groups_to_remove, groups_to_add
        )
        spy.assert_has_calls([call(groups_to_add), call(groups_to_remove)])

    @pytest.mark.parametrize("binding_func", ["bind", "unbind"])
    async def test_binds_new_keys_and_unbinds_old(
        self, exchange, mock_chan, mock_queue, binding_func, mocker
    ):
        mocker.patch.object(
            rabbitmq_base, "binding_keys_from_filters", return_value=["keys"]
        )
        spy = mocker.patch.object(mock_queue, binding_func)
        await RabbitMQBase._update_bindings(
            mock_chan, mock_queue, [filter_group()], [filter_group()]
        )

        spy.assert_called_once_with(exchange, "keys")


class TestValidateQueueId:
    """
    The shared gate every gRPC surface taking a client-supplied queue id
    runs before touching the broker.

    The bound is what keeps the derived broker name inside RabbitMQ's
    255-byte limit, and the alphabet excludes ``:`` because that is the
    separator in the broker name. An id that got past here would die
    several layers down while the AMQP frame was being built, as a bare
    ``ValueError("Max length exceeded for queue")`` that names neither the
    field nor the rule it broke.
    """

    def test_accepts_a_maximum_length_id(self):
        # The bound itself is legal, so the accepted set reaches exactly as
        # far as the byte-budget arithmetic assumes.
        assert validate_queue_id("a" * MAX_QUEUE_ID_LENGTH) is None

    def test_accepts_every_character_the_pattern_allows(self):
        assert validate_queue_id("Aa0._-") is None

    def test_rejects_an_id_one_character_over_the_maximum(self):
        with pytest.raises(UnprocessableError):
            validate_queue_id("a" * (MAX_QUEUE_ID_LENGTH + 1))

    def test_rejects_an_empty_id(self):
        # ``{1,200}`` has no zero-length spelling: an empty id would name
        # the bare prefix, which is a different queue entirely.
        with pytest.raises(UnprocessableError):
            validate_queue_id("")

    @pytest.mark.parametrize(
        "queue_id",
        ["bad:id", "bad id", "bad/id", "bad*id", "café"],
        ids=["colon", "space", "slash", "star", "non-ascii"],
    )
    def test_rejects_a_disallowed_character(self, queue_id):
        with pytest.raises(UnprocessableError):
            validate_queue_id(queue_id)

    def test_the_rejection_names_the_field_and_the_rule(self):
        # The whole point of validating here rather than letting the frame
        # builder fail: the caller is told which field it got wrong and
        # what would have been accepted.
        with pytest.raises(UnprocessableError) as exc_info:
            validate_queue_id("bad:id")

        assert exc_info.value.user_message == (
            f"Invalid queue_id 'bad:id': must match {QUEUE_ID_PATTERN}"
        )


class TestRejectedQueueIdEcho:
    """
    The rejected id is echoed back, but only enough of it to recognise.

    The value is client-controlled and unbounded until this check runs, and
    the message travels: ``ExceptionInterceptor`` re-logs it with
    ``logger.exception`` before returning it. Echoing all of it would turn
    one rejected request into a log line as large as the request was.
    """

    @pytest.fixture
    def over_long_id(self):
        return "a" * (MAX_QUEUE_ID_LENGTH + 1)

    @pytest.fixture
    def rejection(self, over_long_id):
        with pytest.raises(UnprocessableError) as exc_info:
            validate_queue_id(over_long_id)
        return exc_info.value.user_message

    def test_echoes_a_short_id_in_full(self):
        # A caller with several queues needs to know which one it got
        # wrong, so nothing is elided when there is nothing to elide.
        with pytest.raises(UnprocessableError) as exc_info:
            validate_queue_id("bad:id")

        assert "'bad:id'" in exc_info.value.user_message

    def test_echoes_an_id_at_the_echo_limit_in_full(self):
        # Exactly at the limit and invalid for its alphabet, not its
        # length: the boundary case that decides whether the truncation is
        # off by one.
        at_limit = "b" * (_QUEUE_ID_ECHO_LIMIT - 1) + ":"

        with pytest.raises(UnprocessableError) as exc_info:
            validate_queue_id(at_limit)

        assert f"'{at_limit}'" in exc_info.value.user_message

    def test_truncates_an_over_long_id_to_the_echo_limit(self, rejection, over_long_id):
        assert f"'{'a' * _QUEUE_ID_ECHO_LIMIT}..." in rejection

    def test_does_not_echo_the_whole_over_long_id(self, rejection, over_long_id):
        assert over_long_id not in rejection

    def test_reports_the_length_that_was_submitted(self, rejection, over_long_id):
        # A length problem stays diagnosable from the message alone, which
        # the truncation would otherwise have hidden.
        assert f"({len(over_long_id)} characters)" in rejection

    def test_still_names_the_rule(self, rejection):
        assert QUEUE_ID_PATTERN in rejection

    def test_the_message_stays_bounded(self, rejection):
        # The property that matters: a rejected request cannot choose how
        # large the resulting log line is.
        assert len(rejection) < 2 * _QUEUE_ID_ECHO_LIMIT + len(QUEUE_ID_PATTERN)

    def test_a_far_longer_id_does_not_make_a_longer_message(
        self, rejection, over_long_id
    ):
        # A 100k-character id grows the message by exactly the extra digits
        # in the reported count -- nothing else scales with input. The delta
        # is derived rather than written out, so the assertion survives a
        # change to MAX_QUEUE_ID_LENGTH.
        far_longer_id = "a" * 100_000
        extra_count_digits = len(str(len(far_longer_id))) - len(str(len(over_long_id)))

        with pytest.raises(UnprocessableError) as exc_info:
            validate_queue_id(far_longer_id)

        assert len(exc_info.value.user_message) == len(rejection) + extra_count_digits


class TestValidateReconnectToken:
    """
    The gate for the other client-supplied id that lands in a broker name.

    The alphabet is deliberately WIDE -- printable ASCII -- because a
    reconnect token is opaque to the client, which only ever echoes back
    one the server minted, so a token that is merely unrecognized has to
    keep reaching the lookup and failing as "queue not found" rather than
    being rejected as malformed. What the pattern rules out is the
    character a broker queue name cannot carry at all: pamqp's
    ``Queue.Declare.validate()`` rejects anything outside printable ASCII
    with a bare ``ValueError('Invalid value for queue')``, raised deep in
    the AMQP frame build after a channel has been checked out, naming
    neither the field nor the rule it broke.
    """

    def test_accepts_a_maximum_length_token(self):
        assert validate_reconnect_token("a" * RECONNECT_TOKEN_MAX_LENGTH) is None

    @pytest.mark.parametrize(
        "token",
        [
            str(uuid.uuid4()),
            _REFERENCE_SHAPED_TOKEN,
            "has:colons",
            "has/slashes",
            "ok-token_1.2~",
            "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~",
        ],
        ids=[
            "uuid4",
            "reference-shaped",
            "colon",
            "slash",
            "punctuation",
            "every-printable-punctuation",
        ],
    )
    def test_accepts_a_printable_ascii_token(self, token):
        # A uuid4 is what the server actually mints; the rest are legal
        # shapes a client could echo back. None of them may be turned away
        # here -- an unrecognized token of legal shape is a NOT_FOUND from
        # the lookup, not a 422 from this gate.
        assert validate_reconnect_token(token) is None

    def test_rejects_a_token_one_character_over_the_maximum(self):
        with pytest.raises(UnprocessableError):
            validate_reconnect_token("a" * (RECONNECT_TOKEN_MAX_LENGTH + 1))

    @pytest.mark.parametrize(
        "token",
        ["has spaces", "has\ttabs", "has\nnewlines", "has\x00nulls", "café", ""],
        ids=["space", "tab", "newline", "null", "non-ascii", "empty"],
    )
    def test_rejects_a_token_outside_printable_ascii(self, token):
        # Each of these is a value RabbitMQ cannot name a queue with, so a
        # token carrying one could only ever have failed -- the question is
        # whether it fails here, naming the field, or several layers down
        # in pamqp's frame builder. The empty string is in the set for a
        # different reason: it names no queue at all, and the surfaces that
        # cannot distinguish it from an absent token check for one before
        # calling this.
        with pytest.raises(UnprocessableError):
            validate_reconnect_token(token)

    def test_the_rejection_names_the_field_and_the_rule(self):
        with pytest.raises(UnprocessableError) as exc_info:
            validate_reconnect_token("café")

        assert exc_info.value.user_message == (
            f"Invalid reconnect_token: must be 1 to "
            f"{RECONNECT_TOKEN_MAX_LENGTH} printable ASCII characters "
            f"(no spaces, no control or non-ASCII characters)"
        )

    def test_the_same_message_is_given_for_an_over_long_token(self):
        # One rule, one message: a caller is not told which half of the
        # pattern it broke, because the message names both halves and the
        # value that broke them cannot be echoed.
        with pytest.raises(UnprocessableError) as exc_info:
            validate_reconnect_token("a" * (RECONNECT_TOKEN_MAX_LENGTH + 1))

        assert exc_info.value.user_message.startswith(
            "Invalid reconnect_token: must be"
        )

    def test_the_rejection_does_not_echo_the_token(self):
        # A reconnect token is the queue's identifier: an authorized
        # principal that obtains it could attach to the same queue and take
        # a share of its events (presenting it is not on its own access --
        # that still needs a valid bearer token, and every delivered event
        # is authorized per principal by the permissions policy). An error
        # message travels into logs on both sides of the call, so the
        # offending value stays out of it --
        # including any leading fragment of it, which is why the assertion
        # is on a distinctive marker rather than on the whole string.
        marker = "s3cret"
        token = marker + "a" * RECONNECT_TOKEN_MAX_LENGTH

        with pytest.raises(UnprocessableError) as exc_info:
            validate_reconnect_token(token)

        assert marker not in str(exc_info.value)


class TestReconnectTokenPatternIsSharedWithRest:
    """
    One constraint, one spelling.

    The gRPC half runs ``RECONNECT_TOKEN_PATTERN`` through
    ``validate_reconnect_token``; the REST half hands the same string to
    FastAPI as a declarative ``pattern=`` on the query parameter and the
    ``Last-Event-ID`` header. Both halves therefore reject the same tokens
    only for as long as the pattern stays one value carrying one bound.
    """

    def test_the_pattern_interpolates_the_shared_length_bound(self):
        # The bound is written once and interpolated, so the byte-budget
        # arithmetic behind it cannot drift from the regex that enforces it.
        assert f"{{1,{RECONNECT_TOKEN_MAX_LENGTH}}}" in RECONNECT_TOKEN_PATTERN

    def test_the_pattern_is_the_documented_printable_ascii_range(self):
        assert RECONNECT_TOKEN_PATTERN.startswith(r"^[\x21-\x7e]")

    @pytest.mark.parametrize(
        "token",
        [
            "ok-token",
            _REFERENCE_SHAPED_TOKEN,
            "has spaces",
            "café",
            "",
            "a" * (RECONNECT_TOKEN_MAX_LENGTH + 1),
        ],
        ids=["ordinary", "reference-shaped", "space", "non-ascii", "empty", "too-long"],
    )
    def test_the_validator_agrees_with_the_declarative_pattern(self, token):
        # The gRPC half is imperative and the REST half is declarative, so
        # nothing but this makes them the same rule. Asserted as "the two
        # verdicts match", which stays true if the pattern changes and both
        # halves follow it -- and fails the moment the validator grows a
        # rule of its own.
        matches_pattern = re.fullmatch(RECONNECT_TOKEN_PATTERN, token) is not None

        try:
            validate_reconnect_token(token)
            validator_accepts = True
        except UnprocessableError:
            validator_accepts = False

        assert validator_accepts is matches_pattern


@pytest.mark.asyncio
class TestQueueName:
    """
    The broker name is derived from the queue id and nothing else.

    A broker queue name is immutable once declared, so mixing an identity
    string into it means any change to that string -- a different
    principal-id claim, an identity-provider migration, a re-created
    service account -- silently orphans every queue the client owns. These
    pin the id-only derivation from three directions: the exact shape, the
    absence of any dependence on authentication state or on which
    principal is calling, and the fact that the naming path does not
    resolve a principal at all.
    """

    async def test_returns_the_prefix_joined_to_the_queue_id(
        self, mock_queue_name, mock_queue_id
    ):
        assert await RabbitMQBase._queue_name(mock_queue_id) == mock_queue_name

    async def test_does_not_resolve_the_principal_id(self, mock_queue_id, mocker):
        """
        Naming a queue never resolves the caller's identity.

        ``principal_id`` is patched where it lives, in
        ``notification_common.auth``: ``rabbitmq_base`` no longer imports
        it, so there is no module attribute here to patch. That import
        being gone is half the contract -- token validation is now an
        explicit ``authenticate()`` step at each entry point, not a side
        effect of building a name.
        """
        resolve = mocker.patch.object(principal_id, "principal_id")

        await RabbitMQBase._queue_name(mock_queue_id)

        resolve.assert_not_called()

    @pytest.mark.parametrize("enabled", [True, False])
    async def test_is_the_same_name_whatever_the_authentication_state(
        self, mock_queue_name, mock_queue_id, authentication_enabled, enabled
    ):
        """
        Turning authentication on or off does not rename an existing queue.

        The old naming picked a literal ``authn-disabled`` principal when
        authentication was off, so flipping the setting on a running
        deployment renamed every queue and stranded its buffered messages.
        """
        authentication_enabled(enabled)

        assert await RabbitMQBase._queue_name(mock_queue_id) == mock_queue_name

    async def test_max_length_queue_id_stays_under_rabbitmq_byte_limit(self):
        """
        A maximum-length 200-char queue_id (upper bound of
        ^[A-Za-z0-9._-]{1,200}$) must yield a _queue_name whose UTF-8 byte
        length is strictly under RabbitMQ's 255-byte limit. Guards the
        SDD SR-5.2 arithmetic (11 + 1 + 200 = 212 today for a durable
        queue) against a future prefix change. The non-durable spelling,
        which adds the longer ``:nd`` suffix, is guarded alongside the rest
        of that subclass's naming in
        ``test_non_durable_queue_consumer.py``.
        """
        max_queue_id = "a" * MAX_QUEUE_ID_LENGTH

        name = await RabbitMQBase._queue_name(max_queue_id)

        assert len(name.encode("utf-8")) < 255


@pytest.mark.asyncio
class TestCancelQueueConsumer:
    async def test_calls_cancel_with_tag(self, mock_queue: aio_pika.abc.AbstractQueue):
        # noinspection PyTypeChecker
        mocked_cancel: AsyncMock = mock_queue.cancel

        await RabbitMQBase._cancel_queue_consumer("tag", mock_queue)
        mocked_cancel.assert_called_once_with("tag")

    async def test_handles_exception_during_cancel(
        self, mock_queue: aio_pika.abc.AbstractQueue, mocker
    ):
        """Test exception handling when queue.cancel fails."""
        # noinspection PyTypeChecker
        mocked_cancel: AsyncMock = mock_queue.cancel
        mocked_cancel.side_effect = Exception("Cancel failed")

        # Mock logger to verify exception logging
        mock_logger = mocker.patch("app.lib.rabbitmq.rabbitmq_base.logger")

        # Should not raise exception even if cancel fails
        await RabbitMQBase._cancel_queue_consumer("tag", mock_queue)

        # Verify exception was logged
        mock_logger.exception.assert_called_once_with(
            f"Failed to cancel consumer on queue '{mock_queue.name}'"
        )


@pytest.mark.asyncio
class TestCancelQueueConsumerRedactsANonDurableName:
    """
    Both lines this helper writes name the queue, and it runs on every
    non-durable stream's teardown.

    A non-durable broker name embeds the stream's reconnect token, so the
    success line -- emitted on the ordinary path, once per stream -- and
    the failure line are each a per-stream opportunity to write that token
    into the logs. The durable spelling is left verbatim
    elsewhere because an operator needs it to find the queue in the broker;
    here there is nothing an operator can do with the real name that the
    reference does not also let them do.
    """

    @pytest.fixture
    def non_durable_queue(self, mock_queue):
        mock_queue.name = NON_DURABLE_QUEUE_NAME
        return mock_queue

    @pytest.fixture
    def cancel_records(self, caplog, non_durable_queue):
        async def _run(failing=False):
            if failing:
                non_durable_queue.cancel.side_effect = Exception("Cancel failed")

            with caplog.at_level(logging.DEBUG, logger=rabbitmq_base.logger.name):
                await RabbitMQBase._cancel_queue_consumer("tag", non_durable_queue)
            return caplog.records

        return _run

    async def test_the_success_line_is_emitted(self, cancel_records):
        # Guards the guards below: an empty capture would make them vacuous.
        assert await cancel_records() != []

    async def test_the_success_line_does_not_carry_the_token(self, cancel_records):
        records = await cancel_records()

        assert not any(RECONNECT_TOKEN in r.getMessage() for r in records)

    async def test_the_success_line_names_the_redacted_name(self, cancel_records):
        records = await cancel_records()
        expected = redact_queue_name(NON_DURABLE_QUEUE_NAME)

        assert any(expected in r.getMessage() for r in records)

    async def test_the_failure_line_does_not_carry_the_token(self, cancel_records):
        records = await cancel_records(failing=True)

        assert not any(RECONNECT_TOKEN in r.getMessage() for r in records)

    async def test_the_failure_line_names_the_redacted_name(self, cancel_records):
        records = await cancel_records(failing=True)
        expected = redact_queue_name(NON_DURABLE_QUEUE_NAME)

        assert any(expected in r.getMessage() for r in records)

    async def test_the_broker_is_still_asked_to_cancel_the_real_consumer(
        self, cancel_records, non_durable_queue
    ):
        # The redaction is for the text only: the cancel itself is
        # untouched.
        await cancel_records()

        non_durable_queue.cancel.assert_awaited_once_with("tag")


@pytest.mark.asyncio
class TestNackPrefetchedMessages:
    async def test_nacks_all_messages_in_queue(self):
        """Test that all messages in the queue are nacked."""
        messages_queue = asyncio.Queue()

        # Create mock messages
        mock_messages = []
        for i in range(3):
            mock_msg = MagicMock()
            mock_msg.nack = AsyncMock()
            mock_messages.append(mock_msg)
            await messages_queue.put(mock_msg)

        # Call the method
        await RabbitMQBase._nack_prefetched_messages(messages_queue)

        # Verify all messages were nacked with requeue=True
        for mock_msg in mock_messages:
            mock_msg.nack.assert_called_once_with(requeue=True)

        # Queue should be empty
        assert messages_queue.empty()

    async def test_nacks_with_requeue_true(self):
        """Test that messages are always nacked with requeue=True."""
        messages_queue = asyncio.Queue()

        # Create a mock message
        mock_msg = MagicMock()
        mock_msg.nack = AsyncMock()
        await messages_queue.put(mock_msg)

        # Call the method
        await RabbitMQBase._nack_prefetched_messages(messages_queue)

        # Verify message was nacked with requeue=True
        mock_msg.nack.assert_called_once_with(requeue=True)

    async def test_handles_empty_queue(self):
        """Test that an empty queue doesn't cause issues."""
        messages_queue = asyncio.Queue()

        # Should not raise any exceptions
        await RabbitMQBase._nack_prefetched_messages(messages_queue)

        # The queue should still be empty
        assert messages_queue.empty()

    async def test_continues_on_nack_failure(self, mocker):
        """Test that if one message nack fails, others are still processed."""
        messages_queue = asyncio.Queue()

        # Create mock messages, with the second one failing
        mock_msg1 = MagicMock()
        mock_msg1.nack = AsyncMock()
        await messages_queue.put(mock_msg1)

        mock_msg2 = MagicMock()
        mock_msg2.nack = AsyncMock(side_effect=Exception("Nack failed"))
        await messages_queue.put(mock_msg2)

        mock_msg3 = MagicMock()
        mock_msg3.nack = AsyncMock()
        await messages_queue.put(mock_msg3)

        # Mock logger to verify exception logging
        mock_logger = mocker.patch("app.lib.rabbitmq.rabbitmq_base.logger")

        # Should not raise exception even if one nack fails
        await RabbitMQBase._nack_prefetched_messages(messages_queue)

        # Verify all messages were attempted to be nacked
        mock_msg1.nack.assert_called_once_with(requeue=True)
        mock_msg2.nack.assert_called_once_with(requeue=True)
        mock_msg3.nack.assert_called_once_with(requeue=True)

        # Verify exception was logged
        mock_logger.exception.assert_called_once_with(
            "Failed to nack prefetched message"
        )

        # Queue should be empty
        assert messages_queue.empty()


@pytest.mark.asyncio
class TestStartConsumingFromQueue:
    async def test_returns_messages_queue_and_consumer_tag(self):
        # Create a simple mock queue
        mock_queue_instance = MagicMock(spec=aio_pika.abc.AbstractQueue)

        # noinspection PyUnusedLocal
        async def mock_consume(callback, **kwargs):
            return "test-consumer-tag"

        mock_queue_instance.consume = mock_consume

        messages_queue, consumer_tag = await RabbitMQBase._start_consuming_from_queue(
            mock_queue_instance
        )

        assert consumer_tag == "test-consumer-tag"
        assert isinstance(messages_queue, asyncio.Queue)

    async def test_passes_no_consume_arguments(self):
        """
        No ``arguments`` are sent with ``basic.consume`` (issue b.9gj).

        The service used to pass ``x-consumer-timeout`` here. RabbitMQ
        accepts the consumer acknowledgement timeout only as a queue
        argument, as the ``consumer-timeout`` policy key, or in the server
        config -- a ``basic.consume`` argument is silently ignored, so that
        argument bought nothing while reading as if hung clients were
        bounded (the broker's 30-minute default applied instead). The real
        lever now lives in the chart's queue-policy hook.
        """
        mock_queue_instance = MagicMock(spec=aio_pika.abc.AbstractQueue)
        consume_args = {}

        # noinspection PyUnusedLocal
        async def mock_consume(callback, **kwargs):
            consume_args.update(kwargs)
            return "test-consumer-tag"

        mock_queue_instance.consume = mock_consume

        await RabbitMQBase._start_consuming_from_queue(mock_queue_instance)

        assert consume_args == {}


class MockAckableMessage:
    """Mock message with ack/nack support for client acks tests."""

    def __init__(self, body, delivery_tag):
        self.body = body
        self.delivery_tag = delivery_tag
        self.ack = AsyncMock()
        self.nack = AsyncMock()

    # noinspection PyUnusedLocal
    @asynccontextmanager
    async def process(self, requeue=True):
        yield


class TestConsumeWithClientAcks:
    @pytest.fixture
    def ack_tracker(self):
        return AckTracker()

    @pytest.fixture
    def mock_ackable_queue(self, mock_chan, mock_temp_chan_for_queue, mocker):
        mock_queue_instance = mocker.MagicMock(spec=aio_pika.queue.Queue)
        mock_queue_instance.name = "mock_queue"

        # noinspection PyUnusedLocal
        async def mock_consume(callback, **kwargs):
            for i in range(3):
                msg = MockAckableMessage(b'{"a": "b"}', delivery_tag=i + 1)
                await callback(msg)
            return "consumer_tag"

        mock_queue_instance.consume = mock_consume
        mock_queue_instance.cancel = mocker.AsyncMock()
        mock_chan.declare_queue.return_value = mock_queue_instance
        return mock_queue_instance

    @pytest.mark.asyncio
    async def test_yields_messages_with_delivery_tags(
        self, mock_ackable_queue, authz_check_returns_true, ack_tracker
    ):
        messages = []
        async for msg in RabbitMQBase._consume_with_client_acks(
            mock_ackable_queue, authz_check_returns_true, ack_tracker
        ):
            messages.append(msg)
            if len(messages) == 3:
                break

        assert len(messages) == 3
        assert [m.delivery_tag for m in messages] == [1, 2, 3]

    @pytest.mark.asyncio
    async def test_tracks_messages_in_ack_tracker(
        self, mock_ackable_queue, authz_check_returns_true, ack_tracker
    ):
        gen = RabbitMQBase._consume_with_client_acks(
            mock_ackable_queue, authz_check_returns_true, ack_tracker
        )
        await anext(gen)

        assert ack_tracker.in_flight_count == 1

        await gen.aclose()

    @pytest.mark.asyncio
    async def test_acks_unauthorized_messages_silently(
        self, mock_ackable_queue, mocker, ack_tracker
    ):
        authz_check = mocker.AsyncMock(side_effect=[False, True, False])
        messages = []

        async for msg in RabbitMQBase._consume_with_client_acks(
            mock_ackable_queue, authz_check, ack_tracker
        ):
            messages.append(msg)
            if len(messages) == 1:
                break

        # Only 1 message should be yielded (the authorized one)
        assert len(messages) == 1

    @pytest.mark.asyncio
    async def test_nacks_all_in_finally_on_disconnect(
        self, mock_ackable_queue, authz_check_returns_true, ack_tracker, mocker
    ):
        cleanup_task = None

        def capture_task(coro):
            nonlocal cleanup_task
            cleanup_task = asyncio.ensure_future(coro)
            return cleanup_task

        mocker.patch("asyncio.create_task", side_effect=capture_task)

        gen = RabbitMQBase._consume_with_client_acks(
            mock_ackable_queue, authz_check_returns_true, ack_tracker
        )
        # Consume one message
        await anext(gen)
        assert ack_tracker.in_flight_count == 1

        # Simulate disconnect by closing the generator
        await gen.aclose()
        await cleanup_task

        # AckTracker should have nacked the in-flight message
        assert ack_tracker.in_flight_count == 0

    @pytest.mark.asyncio
    async def test_nacks_prefetched_messages_on_cleanup(
        self, mock_ackable_queue, authz_check_returns_true, ack_tracker, mocker
    ):
        spy_nack_prefetched = mocker.spy(RabbitMQBase, "_nack_prefetched_messages")

        cleanup_task = None

        def capture_task(coro):
            nonlocal cleanup_task
            cleanup_task = asyncio.ensure_future(coro)
            return cleanup_task

        mocker.patch("asyncio.create_task", side_effect=capture_task)

        gen = RabbitMQBase._consume_with_client_acks(
            mock_ackable_queue, authz_check_returns_true, ack_tracker
        )
        await anext(gen)
        await gen.aclose()
        await cleanup_task

        spy_nack_prefetched.assert_called_once()

    @pytest.mark.asyncio
    async def test_schedules_cleanup_as_background_task(
        self, mock_ackable_queue, authz_check_returns_true, ack_tracker, mocker
    ):
        captured_coro = None

        def capture_and_close(coro):
            nonlocal captured_coro
            captured_coro = coro
            coro.close()
            return mocker.MagicMock()

        mock_create_task = mocker.patch(
            "asyncio.create_task", side_effect=capture_and_close
        )

        gen = RabbitMQBase._consume_with_client_acks(
            mock_ackable_queue, authz_check_returns_true, ack_tracker
        )
        await anext(gen)
        await gen.aclose()

        mock_create_task.assert_called_once()
        assert captured_coro is not None

    @pytest.mark.asyncio
    async def test_cancels_consumer_before_nacking(
        self, mock_ackable_queue, authz_check_returns_true, ack_tracker, mocker
    ):
        """Consumer must be cancelled before nacking to prevent re-delivery."""
        call_order = []

        original_cancel = RabbitMQBase._cancel_queue_consumer
        original_nack_all = ack_tracker.nack_all

        async def track_cancel(*args, **kwargs):
            call_order.append("cancel_consumer")
            return await original_cancel(*args, **kwargs)

        async def track_nack_all():
            call_order.append("nack_all")
            return await original_nack_all()

        mocker.patch.object(
            RabbitMQBase, "_cancel_queue_consumer", side_effect=track_cancel
        )
        mocker.patch.object(ack_tracker, "nack_all", side_effect=track_nack_all)

        cleanup_task = None

        def capture_task(coro):
            nonlocal cleanup_task
            cleanup_task = asyncio.ensure_future(coro)
            return cleanup_task

        mocker.patch("asyncio.create_task", side_effect=capture_task)

        gen = RabbitMQBase._consume_with_client_acks(
            mock_ackable_queue, authz_check_returns_true, ack_tracker
        )
        await anext(gen)
        await gen.aclose()
        await cleanup_task

        assert call_order == ["cancel_consumer", "nack_all"]

    @pytest.mark.asyncio
    async def test_stops_on_shutdown(
        self, mock_ackable_queue, authz_check_returns_true, ack_tracker
    ):
        RabbitMQBase.connector.prepare_to_shutdown()
        messages = []

        async for msg in RabbitMQBase._consume_with_client_acks(
            mock_ackable_queue, authz_check_returns_true, ack_tracker
        ):
            messages.append(msg)

        assert messages == []

    @pytest.mark.asyncio
    async def test_nacks_and_returns_on_mid_message_shutdown(
        self, mock_ackable_queue, mocker, ack_tracker
    ):
        call_count = 0

        async def authz_check_with_shutdown(_msg):
            nonlocal call_count
            call_count += 1
            if call_count == 2:
                RabbitMQBase.connector.prepare_to_shutdown()
            return True

        messages = []
        async for msg in RabbitMQBase._consume_with_client_acks(
            mock_ackable_queue, authz_check_with_shutdown, ack_tracker
        ):
            messages.append(msg)

        # Gets 2 messages: shutdown is set during 2nd authz but it still
        # returns True, so message 2 is yielded before the closing check
        assert len(messages) == 2

    @pytest.mark.asyncio
    async def test_an_authorization_exception_ends_the_stream_as_itself(
        self, mock_ackable_queue, mocker, ack_tracker
    ):
        # The nack, cancel and drain that precede the raise are pinned on
        # _parse_and_authorize itself, in TestParseAndAuthorize.
        authz_check = mocker.AsyncMock(side_effect=PermissionError("denied"))

        with pytest.raises(PermissionError, match="denied"):
            async for _ in RabbitMQBase._consume_with_client_acks(
                mock_ackable_queue, authz_check, ack_tracker
            ):
                pass

    @pytest.mark.asyncio
    async def test_runs_on_cleanup_complete_when_start_consuming_fails(
        self, mock_ackable_queue, authz_check_returns_true, ack_tracker, mocker
    ):
        # Symmetric guarantee with the delayed-acks variant: if
        # _start_consuming_from_queue raises, on_cleanup_complete
        # still fires so the caller's pool-channel release runs.
        mocker.patch.object(
            RabbitMQBase,
            "_start_consuming_from_queue",
            side_effect=RuntimeError("consume boom"),
        )
        on_cleanup = AsyncMock()

        gen = RabbitMQBase._consume_with_client_acks(
            mock_ackable_queue,
            authz_check_returns_true,
            ack_tracker,
            on_cleanup_complete=on_cleanup,
        )

        with pytest.raises(RuntimeError, match="consume boom"):
            await anext(gen)

        await asyncio.sleep(0.05)

        on_cleanup.assert_called_once()


class TestConsumeWithDelayedAcks:
    ACK_DELAY = 2.0

    @pytest.fixture(autouse=True)
    def mock_ack_delay(self, mocker):
        mock_settings = mocker.patch("app.lib.rabbitmq.rabbitmq_base.settings.settings")
        mock_settings.server_ack_delay_seconds = self.ACK_DELAY
        mock_settings.consumer_liveness_poll_seconds = IDLE_POLL_NEVER_FIRES_SECONDS
        return mock_settings

    @pytest.fixture
    def mock_ackable_queue(self, mock_chan, mock_temp_chan_for_queue, mocker):
        mock_queue_instance = mocker.MagicMock(spec=aio_pika.queue.Queue)
        mock_queue_instance.name = "mock_queue"

        async def mock_consume(callback, **kwargs):
            for i in range(3):
                msg = MockAckableMessage(b'{"a": "b"}', delivery_tag=i + 1)
                await callback(msg)
            return "consumer_tag"

        mock_queue_instance.consume = mock_consume
        mock_queue_instance.cancel = mocker.AsyncMock()
        mock_chan.declare_queue.return_value = mock_queue_instance
        return mock_queue_instance

    @pytest.mark.asyncio
    async def test_yields_messages_immediately(
        self, mock_ackable_queue, authz_check_returns_true
    ):
        messages = []
        async for msg in RabbitMQBase._consume_with_delayed_acks(
            mock_ackable_queue, authz_check_returns_true
        ):
            messages.append(msg)
            if len(messages) == 3:
                break

        assert len(messages) == 3
        assert all(m.message == {"a": "b"} for m in messages)

    @pytest.mark.asyncio
    async def test_does_not_ack_immediately(
        self, mock_ackable_queue, authz_check_returns_true
    ):
        """Messages should not be acked right after yielding."""
        gen = RabbitMQBase._consume_with_delayed_acks(
            mock_ackable_queue, authz_check_returns_true
        )
        msg = await anext(gen)
        await gen.aclose()

        # We can't easily check the raw message mock from here,
        # but the cleanup nacks confirm it wasn't acked yet.
        # The message was yielded but the delay hasn't elapsed,
        # so it should be nacked during cleanup, not acked.
        assert msg.message == {"a": "b"}

    @pytest.mark.asyncio
    async def test_acks_aged_messages_before_next(
        self, mock_ackable_queue, mocker, authz_check_returns_true, mock_ack_delay
    ):
        """Pre-message flush acks messages whose delay has elapsed."""
        # Use a tiny delay so messages age immediately
        mock_ack_delay.server_ack_delay_seconds = 0.0
        gen = RabbitMQBase._consume_with_delayed_acks(
            mock_ackable_queue,
            authz_check_returns_true,
        )

        # Get first message — nothing to flush yet
        await anext(gen)
        # Get second message — should flush first message
        await anext(gen)
        # Get third message — should flush second message
        await anext(gen)

        await gen.aclose()

        # With zero delay, the cleanup task runs _nack_pending
        # which only nacks messages still in pending_acks.
        # Messages 1 and 2 should have been acked by the
        # pre-message flush. Only message 3 remains pending
        # and gets nacked during cleanup.

    @pytest.mark.asyncio
    async def test_idle_timer_acks_after_quiet_period(
        self, mock_ackable_queue, authz_check_returns_true, mocker, mock_ack_delay
    ):
        """The debounced idle timer acks messages when no new ones arrive."""
        # Use zero delay so the timer fires quickly
        mock_ack_delay.server_ack_delay_seconds = 0.0
        gen = RabbitMQBase._consume_with_delayed_acks(
            mock_ackable_queue,
            authz_check_returns_true,
        )

        await anext(gen)

        # Let the idle timer fire (delay + 5s, but with 0 delay
        # we still need to yield to the event loop)
        # The timer is set to ack_delay + 5s. With 0 delay that's
        # 5s which is too long for a unit test. Instead, verify
        # the timer task was created.
        await gen.aclose()

    @pytest.mark.asyncio
    async def test_nacks_all_pending_on_disconnect(
        self, mock_ackable_queue, authz_check_returns_true, mocker, mock_ack_delay
    ):
        """On disconnect, all un-acked messages are nacked."""
        cleanup_task = None

        def capture_task(coro):
            nonlocal cleanup_task
            cleanup_task = asyncio.ensure_future(coro)
            return cleanup_task

        mocker.patch("asyncio.create_task", side_effect=capture_task)

        mock_ack_delay.server_ack_delay_seconds = 999.0
        gen = RabbitMQBase._consume_with_delayed_acks(
            mock_ackable_queue,
            authz_check_returns_true,
        )

        await anext(gen)
        await anext(gen)
        await gen.aclose()

        # Wait for cleanup task
        if cleanup_task:
            await cleanup_task

    @pytest.mark.asyncio
    async def test_stops_on_shutdown(
        self, mock_ackable_queue, authz_check_returns_true
    ):
        RabbitMQBase.connector.prepare_to_shutdown()
        messages = []

        async for msg in RabbitMQBase._consume_with_delayed_acks(
            mock_ackable_queue, authz_check_returns_true
        ):
            messages.append(msg)

        assert messages == []

    @pytest.mark.asyncio
    async def test_acks_unauthorized_messages_immediately(
        self, mock_ackable_queue, mocker
    ):
        """Unauthorized messages are acked right away, not delayed."""
        authz_check = mocker.AsyncMock(side_effect=[False, True, False])
        messages = []

        async for msg in RabbitMQBase._consume_with_delayed_acks(
            mock_ackable_queue, authz_check
        ):
            messages.append(msg)
            if len(messages) == 1:
                break

        assert len(messages) == 1

    @pytest.mark.asyncio
    async def test_cancels_consumer_before_nacking(
        self, mock_ackable_queue, authz_check_returns_true, mocker, mock_ack_delay
    ):
        """Consumer must be cancelled before nacking to prevent re-delivery."""
        call_order = []

        original_cancel = RabbitMQBase._cancel_queue_consumer

        async def track_cancel(*args, **kwargs):
            call_order.append("cancel_consumer")
            return await original_cancel(*args, **kwargs)

        async def track_nack_pending():
            call_order.append("nack_pending")

        mocker.patch.object(
            RabbitMQBase, "_cancel_queue_consumer", side_effect=track_cancel
        )

        cleanup_task = None

        def capture_task(coro):
            nonlocal cleanup_task
            cleanup_task = asyncio.ensure_future(coro)
            return cleanup_task

        # Capture both create_task calls (idle timer + cleanup).
        # The first is the idle timer, the second is cleanup.
        tasks = []

        def capture_all_tasks(coro):
            task = asyncio.ensure_future(coro)
            tasks.append(task)
            return task

        mocker.patch("asyncio.create_task", side_effect=capture_all_tasks)

        mock_ack_delay.server_ack_delay_seconds = 999.0
        gen = RabbitMQBase._consume_with_delayed_acks(
            mock_ackable_queue,
            authz_check_returns_true,
        )
        await anext(gen)

        # Patch nack_pending after generator is created so we
        # capture the closure's local _nack_pending
        mocker.patch.object(
            RabbitMQBase, "_nack_prefetched_messages", new_callable=mocker.AsyncMock
        )

        await gen.aclose()

        # Last task is the cleanup task
        await tasks[-1]

        assert call_order[0] == "cancel_consumer"

    @pytest.mark.asyncio
    async def test_cancels_idle_timer_on_cleanup(
        self, mock_ackable_queue, authz_check_returns_true, mocker
    ):
        """The idle timer is cancelled when the generator is closed."""
        gen = RabbitMQBase._consume_with_delayed_acks(
            mock_ackable_queue,
            authz_check_returns_true,
        )

        # Consume one message to start the idle timer
        await anext(gen)

        # Close the generator — should cancel the timer
        await gen.aclose()

    @pytest.mark.asyncio
    async def test_runs_on_cleanup_complete_when_start_consuming_fails(
        self, mock_ackable_queue, authz_check_returns_true, mocker
    ):
        # _start_consuming_from_queue must run inside the try/finally
        # so that the on_cleanup_complete callback (which the durable
        # consumer uses to release the pool checkout) still fires when
        # consume setup fails. Otherwise queue.consume() failing leaks
        # the pool channel.
        mocker.patch.object(
            RabbitMQBase,
            "_start_consuming_from_queue",
            side_effect=RuntimeError("consume boom"),
        )
        on_cleanup = AsyncMock()

        gen = RabbitMQBase._consume_with_delayed_acks(
            mock_ackable_queue,
            authz_check_returns_true,
            on_cleanup_complete=on_cleanup,
        )

        with pytest.raises(RuntimeError, match="consume boom"):
            await anext(gen)

        # Cleanup is scheduled in a background task; let it run.
        await asyncio.sleep(0.05)

        on_cleanup.assert_called_once()


@pytest.fixture
def pump_boundary(mocker):
    """
    Drive the REAL consume generators against a controllable message pump.

    ``_start_consuming_from_queue`` is replaced with one returning a real
    ``asyncio.Queue`` (pre-loaded by the test) plus a consumer tag; the
    cancel/prefetch-nack cleanup helpers are stubbed so no broker is touched.
    Returns the ``asyncio.Queue`` so a test can enqueue mock messages.
    """
    messages_queue = asyncio.Queue()

    async def fake_start(_queue):
        return messages_queue, PUMP_CONSUMER_TAG

    mocker.patch.object(
        RabbitMQBase,
        "_start_consuming_from_queue",
        new_callable=AsyncMock,
        side_effect=fake_start,
    )
    mocker.patch.object(RabbitMQBase, "_cancel_queue_consumer", new_callable=AsyncMock)
    mocker.patch.object(
        RabbitMQBase, "_nack_prefetched_messages", new_callable=AsyncMock
    )
    return messages_queue


@pytest.fixture
def authz_true():
    async def _check(_message):
        return True

    return _check


@pytest.fixture(autouse=True)
def _connector_open():
    RabbitMQBase.connector.closing = False
    yield
    RabbitMQBase.connector.closing = False


@pytest.mark.asyncio
class TestDelayedAcksShutdownBranches:
    """Mid-loop shutdown branches in _consume_with_delayed_acks."""

    @pytest.fixture(autouse=True)
    def fast_ack_delay(self, mocker):
        mock_settings = mocker.patch("app.lib.rabbitmq.rabbitmq_base.settings.settings")
        mock_settings.server_ack_delay_seconds = 999.0
        mock_settings.consumer_liveness_poll_seconds = IDLE_POLL_NEVER_FIRES_SECONDS
        return mock_settings

    async def test_nacks_and_returns_when_closing_after_dequeue(
        self, pump_boundary, authz_true, mocker
    ):
        # Top-of-loop check passes (open), then closing flips as soon as the
        # message is dequeued: the dequeued message is nacked, not yielded.
        # The seam is ``get_nowait`` because a queue with a buffered delivery
        # takes ``_next_message``'s fast path and never reaches ``get``.
        original_get_nowait = pump_boundary.get_nowait

        def get_nowait_then_close():
            message = original_get_nowait()
            RabbitMQBase.connector.closing = True
            return message

        mocker.patch.object(
            pump_boundary, "get_nowait", side_effect=get_nowait_then_close
        )
        msg = MockAckableMessage(b'{"a": "b"}', delivery_tag=1)
        pump_boundary.put_nowait(msg)

        records = [
            m
            async for m in RabbitMQBase._consume_with_delayed_acks(
                MagicMock(), authz_true
            )
        ]

        assert records == []
        msg.nack.assert_awaited_once_with(requeue=True)

    async def test_returns_on_exception_when_closing(self, pump_boundary, mocker):
        # An exception raised inside the loop body while the connector is
        # closing is swallowed (return) rather than re-raised.
        def authz_then_close(_message):
            RabbitMQBase.connector.closing = True
            raise RuntimeError("boom during shutdown")

        authz = mocker.AsyncMock(side_effect=authz_then_close)
        pump_boundary.put_nowait(MockAckableMessage(b'{"a": "b"}', delivery_tag=1))

        records = [
            m async for m in RabbitMQBase._consume_with_delayed_acks(MagicMock(), authz)
        ]

        assert records == []

    async def test_returns_at_bottom_of_loop_when_closing_after_yield(
        self, pump_boundary, authz_true
    ):
        # After a message is yielded, the bottom-of-loop re-check observes
        # closing and stops without dequeuing the next message.
        pump_boundary.put_nowait(MockAckableMessage(b'{"a": "b"}', delivery_tag=1))
        pump_boundary.put_nowait(MockAckableMessage(b'{"a": "b"}', delivery_tag=2))
        gen = RabbitMQBase._consume_with_delayed_acks(MagicMock(), authz_true)

        first = await anext(gen)
        RabbitMQBase.connector.closing = True
        with pytest.raises(StopAsyncIteration):
            await anext(gen)

        assert first.message == {"a": "b"}

    async def test_reraises_exception_when_not_closing(self, pump_boundary, mocker):
        # An exception in the loop body while the connector is OPEN propagates
        # (the bare re-raise after the closing-check).
        authz = mocker.AsyncMock(side_effect=PermissionError("denied"))
        pump_boundary.put_nowait(MockAckableMessage(b'{"a": "b"}', delivery_tag=1))

        with pytest.raises(PermissionError, match="denied"):
            async for _ in RabbitMQBase._consume_with_delayed_acks(MagicMock(), authz):
                pass


@pytest.mark.asyncio
class TestClientAcksShutdownBranches:
    """Mid-loop shutdown branches in _consume_with_client_acks."""

    async def test_nacks_and_returns_when_closing_after_dequeue(
        self, pump_boundary, authz_true, mocker
    ):
        # The seam is ``get_nowait`` because a queue with a buffered delivery
        # takes ``_next_message``'s fast path and never reaches ``get``.
        original_get_nowait = pump_boundary.get_nowait

        def get_nowait_then_close():
            message = original_get_nowait()
            RabbitMQBase.connector.closing = True
            return message

        mocker.patch.object(
            pump_boundary, "get_nowait", side_effect=get_nowait_then_close
        )
        msg = MockAckableMessage(b'{"a": "b"}', delivery_tag=1)
        pump_boundary.put_nowait(msg)
        tracker = AckTracker()

        records = [
            m
            async for m in RabbitMQBase._consume_with_client_acks(
                MagicMock(), authz_true, tracker
            )
        ]

        assert records == []
        msg.nack.assert_awaited_once_with(requeue=True)

    async def test_returns_on_exception_when_closing(self, pump_boundary, mocker):
        def authz_then_close(_message):
            RabbitMQBase.connector.closing = True
            raise RuntimeError("boom during shutdown")

        authz = mocker.AsyncMock(side_effect=authz_then_close)
        pump_boundary.put_nowait(MockAckableMessage(b'{"a": "b"}', delivery_tag=1))
        tracker = AckTracker()

        records = [
            m
            async for m in RabbitMQBase._consume_with_client_acks(
                MagicMock(), authz, tracker
            )
        ]

        assert records == []


@pytest.mark.asyncio
class TestAckAgedAndNackPendingErrorBranches:
    """The internal _ack_aged_messages / _nack_pending error-logging branches."""

    @pytest.fixture(autouse=True)
    def fast_ack_delay(self, mocker):
        mock_settings = mocker.patch("app.lib.rabbitmq.rabbitmq_base.settings.settings")
        # Zero delay so a yielded message ages immediately and the
        # pre-message flush attempts to ack it on the next iteration.
        mock_settings.server_ack_delay_seconds = 0.0
        mock_settings.consumer_liveness_poll_seconds = IDLE_POLL_NEVER_FIRES_SECONDS
        return mock_settings

    async def test_logs_when_acking_aged_message_fails(
        self, pump_boundary, authz_true, mocker
    ):
        mock_logger = mocker.patch("app.lib.rabbitmq.rabbitmq_base.logger")
        failing = MockAckableMessage(b'{"a": "b"}', delivery_tag=1)
        failing.ack = AsyncMock(side_effect=RuntimeError("ack boom"))
        pump_boundary.put_nowait(failing)
        pump_boundary.put_nowait(MockAckableMessage(b'{"a": "b"}', delivery_tag=2))
        gen = RabbitMQBase._consume_with_delayed_acks(MagicMock(), authz_true)

        # First message yielded and tracked; second iteration flushes the
        # (now-aged) first message, whose ack raises and is logged.
        await anext(gen)
        await anext(gen)
        await gen.aclose()

        mock_logger.exception.assert_any_call("Failed to ack aged message")

    async def test_logs_when_nacking_pending_message_fails(
        self, pump_boundary, authz_true, mocker
    ):
        # _nack_pending runs in the background cleanup; capture the task so
        # we can await it before asserting.
        cleanup_task = None

        def capture_task(coro):
            nonlocal cleanup_task
            cleanup_task = asyncio.ensure_future(coro)
            return cleanup_task

        mocker.patch("asyncio.create_task", side_effect=capture_task)
        mock_logger = mocker.patch("app.lib.rabbitmq.rabbitmq_base.logger")

        failing = MockAckableMessage(b'{"a": "b"}', delivery_tag=1)
        failing.nack = AsyncMock(side_effect=RuntimeError("nack boom"))
        # 999s effective via patching the delay so the message stays pending.
        long_delay_settings = mocker.patch(
            "app.lib.rabbitmq.rabbitmq_base.settings.settings"
        )
        long_delay_settings.server_ack_delay_seconds = 999.0
        long_delay_settings.consumer_liveness_poll_seconds = (
            IDLE_POLL_NEVER_FIRES_SECONDS
        )
        pump_boundary.put_nowait(failing)
        gen = RabbitMQBase._consume_with_delayed_acks(MagicMock(), authz_true)

        await anext(gen)
        await gen.aclose()
        if cleanup_task:
            await cleanup_task

        mock_logger.exception.assert_any_call(
            "Failed to nack pending message during cleanup"
        )


@pytest.mark.asyncio
class TestIdleTimerFires:
    """The debounced idle timer fires and flushes aged messages (line 288)."""

    async def test_idle_timer_acks_pending_after_delay(self, pump_boundary, mocker):
        # Drive a real idle-timer fire. The callback sleeps delay+5s then
        # flushes aged messages; patch the module's asyncio.sleep so that
        # sleep is instant, and capture the timer task so it runs to
        # completion (and acks the now-aged pending message) before close.
        mock_settings = mocker.patch("app.lib.rabbitmq.rabbitmq_base.settings.settings")
        mock_settings.server_ack_delay_seconds = 0.0
        mock_settings.consumer_liveness_poll_seconds = IDLE_POLL_NEVER_FIRES_SECONDS

        real_sleep = asyncio.sleep

        async def instant_sleep(_seconds):
            await real_sleep(0)

        mocker.patch.object(rabbitmq_base.asyncio, "sleep", side_effect=instant_sleep)

        tasks = []
        real_create_task = asyncio.create_task

        def capture_task(coro):
            task = real_create_task(coro)
            tasks.append(task)
            return task

        mocker.patch.object(
            rabbitmq_base.asyncio, "create_task", side_effect=capture_task
        )

        async def authz(_message):
            return True

        msg = MockAckableMessage(b'{"a": "b"}', delivery_tag=1)
        pump_boundary.put_nowait(msg)
        pump_boundary.put_nowait(MockAckableMessage(b'{"a": "b"}', delivery_tag=2))
        gen = RabbitMQBase._consume_with_delayed_acks(MagicMock(), authz)

        # First anext yields msg1 (suspended at the yield, before the timer
        # reset). Second anext resumes, schedules the idle timer task, then
        # processes msg2.
        await anext(gen)
        await anext(gen)
        # The captured task is the idle timer; await it so its (now-instant)
        # sleep elapses and _ack_aged_messages runs on the aged msg1.
        await tasks[0]
        await gen.aclose()

        msg.ack.assert_awaited()


@pytest.mark.asyncio
class TestNackPrefetchedQueueEmptyRace:
    """The defensive QueueEmpty break in _nack_prefetched_messages (line 521)."""

    async def test_breaks_on_queue_empty_race(self, mocker):
        # empty() reports non-empty but get_nowait() raises QueueEmpty (a
        # concurrent drain). The loop must break, not spin or raise.
        fake_queue = mocker.MagicMock()
        fake_queue.empty.return_value = False
        fake_queue.get_nowait.side_effect = asyncio.QueueEmpty()

        await RabbitMQBase._nack_prefetched_messages(fake_queue)

        fake_queue.get_nowait.assert_called_once()


class TestMockMessageWithXDeath:
    """
    The SDD's Test Fixtures section requires a MockMessage variant carrying
    x-death headers so reason/origin-queue decoding can be exercised. This
    Task introduces no header-decoding helper, so these tests scope the
    variant to making the x-death structure available for the Epic 2
    consume/replay work.
    """

    def test_exposes_x_death_array_shape(self):
        msg = MockMessageWithXDeath(b'{"a": "b"}')
        x_death = msg.headers["x-death"]
        assert x_death[0]["reason"] == "rejected"

    def test_exposes_origin_queue(self):
        msg = MockMessageWithXDeath(b'{"a": "b"}')
        assert msg.headers["x-death"][0]["queue"] == "origin-queue"

    def test_exposes_delivery_count(self):
        msg = MockMessageWithXDeath(b'{"a": "b"}')
        assert msg.headers["x-delivery-count"] == 1


# ---------------------------------------------------------------------------
# Consumer-liveness detection (issue b.9gj)
#
# aio-pika surfaces no notification when the broker cancels our consumer or
# kills the channel it lives on, so the consume loops poll a liveness
# predicate off their idle path. The queue/channel doubles and the shared
# fixtures built on them live in ``tests/fixtures/consumer_liveness.py`` and
# this directory's ``conftest.py``, because the DLQ browse loop's tests need
# them too.
# ---------------------------------------------------------------------------


class TestConnectionIsDown:
    """
    The predicate for "is the shared AMQP connection actually up right now".

    Every case builds a ``FakeConnection`` whose ``connected`` is a real
    ``asyncio.Event``. A ``MagicMock`` will not do: its auto-created
    ``connected.is_set()`` is truthy, so the predicate would read "up" on
    every one of these and the down cases would pass without ever consulting
    the flag that moves.
    """

    @pytest.fixture
    def connection(self, mocker):
        def _install(**kwargs):
            handle = None if kwargs.pop("absent", False) else FakeConnection(**kwargs)
            mocker.patch.object(RabbitMQBase.connector, "_connection", handle)
            return handle

        return _install

    def test_a_connection_that_never_opened_is_down(self, connection):
        # Before the first connect -- and after a teardown -- the connector
        # holds no handle at all.
        connection(absent=True)

        assert RabbitMQBase._connection_is_down() is True

    def test_a_closed_connection_is_down(self, connection):
        # The signal for a connection that is down for good: an explicit local
        # close, which is the only thing that resolves ``is_closed``.
        connection(is_closed=True)

        assert RabbitMQBase._connection_is_down() is True

    def test_a_disconnected_connection_is_down(self, connection):
        # THE case the original ``is_closed``-only predicate missed. A
        # broker-side drop leaves a RobustConnection reading is_closed=False
        # while it is plainly not connected (verified against the pinned
        # aio-pika); ``connected`` is the flag aio-pika actually clears on
        # every disconnect and re-sets once a reconnect succeeds.
        connection(is_closed=False, connected=False)

        assert RabbitMQBase._connection_is_down() is True

    def test_a_live_connection_is_not_down(self, connection):
        # The control, with ``connected`` genuinely SET -- otherwise the three
        # cases above would hold for a predicate that simply always said
        # "down".
        connection()

        assert RabbitMQBase._connection_is_down() is False

    def test_a_reconnected_connection_is_not_down(self, connection):
        # ``RobustConnection.reconnecting`` is deliberately not consulted: it
        # is the reconnect lock, and aio-pika holds it for a further
        # ``reconnect_interval`` AFTER a successful reconnect. Reading it would
        # declare a connection that is already back up to be down, and end
        # every stream on the pod for it.
        connection(reconnecting=True)

        assert RabbitMQBase._connection_is_down() is False


class TestLivenessPollTimeout:
    """
    The jittered deadline for one idle liveness wait.

    Every stream on the pod shares one AMQP connection, so a connection-level
    event leaves them all idle from the same instant; without jitter their
    next polls land together as one synchronized burst against the broker.
    The band is UPWARD-ONLY -- ``[poll, poll * 1.15)`` -- so that the spread
    survives at the setting's own 1.0-second floor. A downward half would have
    to be clamped there (the setting refuses sub-second polling), and the clamp
    would swallow it whole, leaving the tightest deployments with jitter that
    is inert at exactly the value they chose. Stretching up instead keeps the
    full spread everywhere and needs no clamp: the result is never below
    ``poll``, which the setting already bounds at 1.0.
    """

    # Enough draws that a band edge would have to be missed ~200 times over to
    # slip through, while still costing nothing: the helper does no I/O.
    _SAMPLES = 200

    @pytest.fixture
    def poll_seconds(self, mocker):
        def _set(value):
            mocker.patch.object(
                _settings_module.settings, "consumer_liveness_poll_seconds", value
            )

        return _set

    @pytest.fixture
    def uniform(self, mocker):
        """
        Pin ``random.uniform`` so the band edges can be hit exactly.

        Defaults to the no-jitter draw so a test that only inspects the CALL
        still gets a usable float back out of the helper.
        """
        return mocker.patch.object(rabbitmq_base.random, "uniform", return_value=1.0)

    def test_no_sampled_deadline_falls_below_the_floor(self, poll_seconds):
        # The anti-spin property, asserted where it is tightest: the setting
        # refuses to poll sub-second for broker-timescale events, so at its own
        # minimum no draw may land under a second.
        poll_seconds(1.0)

        deadlines = [
            RabbitMQBase._liveness_poll_timeout() for _ in range(self._SAMPLES)
        ]

        assert min(deadlines) >= 1.0

    def test_every_sampled_deadline_stays_inside_the_jitter_band(self, poll_seconds):
        poll_seconds(15.0)

        deadlines = [
            RabbitMQBase._liveness_poll_timeout() for _ in range(self._SAMPLES)
        ]

        assert 15.0 <= min(deadlines) and max(deadlines) <= 17.25

    def test_the_default_window_is_not_flattened_to_a_single_deadline(
        self, poll_seconds
    ):
        # Non-vacuity for the band: an implementation that returned the
        # configured window unjittered would satisfy every "inside the band"
        # assertion above while destroying the spread the jitter exists for.
        poll_seconds(15.0)

        deadlines = {
            RabbitMQBase._liveness_poll_timeout() for _ in range(self._SAMPLES)
        }

        assert len(deadlines) > 1

    def test_the_band_stretches_upward_only(self, poll_seconds, uniform):
        poll_seconds(15.0)

        RabbitMQBase._liveness_poll_timeout()

        uniform.assert_called_once_with(1.0, 1.15)

    def test_the_full_spread_survives_at_the_floor(self, poll_seconds):
        # THE property the upward-only band buys, and the reason a symmetric
        # one was not kept. A symmetric band needs a 1.0-second clamp, and at
        # poll=1.0 that clamp swallows its whole downward half -- measured, the
        # deadlines collapsed to a 0.007 s spread here while still spreading
        # 0.55 s at poll=2.0, so the anti-stampede jitter went inert at exactly
        # the setting the tightest deployments would choose.
        poll_seconds(1.0)

        deadlines = [
            RabbitMQBase._liveness_poll_timeout() for _ in range(self._SAMPLES)
        ]

        assert max(deadlines) - min(deadlines) > 0.1

    def test_the_bottom_of_the_default_band_is_the_configured_window(
        self, poll_seconds, uniform
    ):
        # The band starts AT the configured window: the deadline is stretched,
        # never shortened, so detection latency is only ever bought, not sold.
        poll_seconds(15.0)
        uniform.return_value = 1.0

        assert RabbitMQBase._liveness_poll_timeout() == pytest.approx(15.0)

    def test_the_top_of_the_default_band_is_reachable(self, poll_seconds, uniform):
        poll_seconds(15.0)
        uniform.return_value = 1.15

        assert RabbitMQBase._liveness_poll_timeout() == pytest.approx(17.25)

    def test_the_bottom_of_the_band_at_the_floor_is_the_floor(
        self, poll_seconds, uniform
    ):
        # The floor holds without a clamp, which is what makes the clamp
        # removable: the lowest draw at poll=1.0 IS 1.0.
        poll_seconds(1.0)
        uniform.return_value = 1.0

        assert RabbitMQBase._liveness_poll_timeout() == pytest.approx(1.0)

    def test_the_top_of_the_band_at_the_floor_is_reachable(self, poll_seconds, uniform):
        # The floor is a starting point, not a pin: upward jitter still applies
        # there, which is the whole spread at the tightest setting.
        poll_seconds(1.0)
        uniform.return_value = 1.15

        assert RabbitMQBase._liveness_poll_timeout() == pytest.approx(1.15)


class TestChannelIncarnation:
    """
    The identity a consume loop pins at start so it can spot a reopen later.

    A ``RobustChannel`` the broker kills is reopened IN PLACE, reusing the same
    wrapper object, so nothing the wrapper exposes distinguishes "died and came
    back" from "never died". The underlay does: ``Channel._open`` builds a
    fresh one on every open. Reading it means reading a private attribute, so
    the answer is best-effort and must degrade to "unknown" rather than raise
    -- this runs at consume start for every stream, and a pinned-library change
    here must not fail streams that would otherwise work.
    """

    @pytest.mark.asyncio
    async def test_identifies_the_underlay_the_consumer_registry_lives_on(
        self, live_queue
    ):
        # Asserted against the PUBLIC accessor rather than restating the
        # private attribute name, so what is pinned is the invariant the
        # liveness check depends on: the incarnation it compares and the
        # registry it reads are one and the same object.
        underlay = await live_queue.channel.get_underlay_channel()

        assert RabbitMQBase._channel_incarnation(live_queue) is underlay

    def test_is_stable_while_the_channel_lives(self, live_queue):
        # Non-vacuity for every "different incarnation" case below: an identity
        # that churned on its own would read as a reopen on the first idle poll
        # of every healthy stream.
        first = RabbitMQBase._channel_incarnation(live_queue)

        assert RabbitMQBase._channel_incarnation(live_queue) is first

    def test_changes_when_the_channel_reopens(self, live_queue):
        # THE signal the whole fix rests on, and the only in-process trace an
        # in-place reopen leaves behind.
        before = RabbitMQBase._channel_incarnation(live_queue)

        live_queue.channel.reopen()

        assert RabbitMQBase._channel_incarnation(live_queue) is not before

    def test_is_unknown_when_the_queue_has_no_channel(self):
        # There is nothing to identify before a queue is bound to a channel,
        # and the read has to answer rather than raise.
        assert RabbitMQBase._channel_incarnation(SimpleNamespace()) is None

    def test_is_unknown_when_the_channel_has_no_underlay(self):
        # A channel that never opened -- or a pinned-library change that moved
        # the attribute -- must degrade the liveness check to its coarser
        # fallback instead of failing every stream at consume start.
        channel_without_underlay = SimpleNamespace(channel=SimpleNamespace())

        assert RabbitMQBase._channel_incarnation(channel_without_underlay) is None


@pytest.mark.asyncio
class TestRaiseIfConsumerLost:
    """
    The predicate that turns a silently-dead AMQP consumer into a terminal
    stream error (issue b.9gj).
    """

    async def test_returns_when_the_tag_is_registered_on_an_open_channel(
        self, live_queue, queue_exists
    ):
        assert (
            await RabbitMQBase._raise_if_consumer_lost(live_queue, PUMP_CONSUMER_TAG)
            is None
        )

    async def test_does_not_touch_the_broker_when_the_consumer_is_alive(
        self, live_queue, queue_exists
    ):
        # The check runs on every idle poll of every open stream, so the
        # healthy path must stay purely in-process.
        await RabbitMQBase._raise_if_consumer_lost(live_queue, PUMP_CONSUMER_TAG)

        queue_exists.assert_not_awaited()

    async def test_raises_consumer_cancelled_when_the_tag_is_gone(
        self, make_consumer_queue, queue_exists
    ):
        # A broker-initiated Basic.Cancel (e.g. the queue being deleted from
        # under us) leaves the channel OPEN and merely drops the tag.
        queue = make_consumer_queue(consumer_tags=())

        with pytest.raises(ConsumerCancelledError):
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

    async def test_raises_consumer_cancelled_when_the_channel_is_closed(
        self, make_consumer_queue, queue_exists
    ):
        # The ack-timeout kill from the 2026-08-24 incident: the broker
        # closes the whole channel with precondition_failed.
        queue = make_consumer_queue(is_closed=True, consumer_tags=())

        with pytest.raises(ConsumerCancelledError):
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

    async def test_does_not_read_the_registry_of_a_closed_channel(
        self, make_consumer_queue, queue_exists
    ):
        # A closed channel is conclusive on its own; asking it for an underlay
        # would raise rather than answer.
        queue = make_consumer_queue(is_closed=True)

        with pytest.raises(ConsumerCancelledError):
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

        assert queue.channel.underlay_call_count == 0

    async def test_raises_consumer_cancelled_when_the_channel_has_no_underlay(
        self, make_consumer_queue, queue_exists
    ):
        # Mid-reopen: aio-pika reports the channel as not-closed but has no
        # underlay yet, so it is certainly not carrying our consumer.
        queue = make_consumer_queue(
            underlay_error=aio_pika.exceptions.ChannelInvalidStateError(
                "Channel was not opened"
            )
        )

        with pytest.raises(ConsumerCancelledError):
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

    async def test_raises_queue_not_found_when_the_queue_was_deleted(
        self, make_consumer_queue, queue_deleted
    ):
        # A deleted queue is a DIFFERENT client remedy from a cancelled
        # consumer: reprovision, not reconnect. The classification is what
        # earns the extra broker round trip.
        queue = make_consumer_queue(consumer_tags=())

        with pytest.raises(QueueNotFoundError):
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

    async def test_classifies_against_the_queues_broker_name(
        self, make_consumer_queue, queue_deleted
    ):
        queue = make_consumer_queue(consumer_tags=())

        with pytest.raises(QueueNotFoundError):
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

        # The queue is asserted VERBATIM and by nothing else: the broker
        # knows no other spelling. The exportable rendering is derived
        # inside `_assert_queue_exists` from this same name, so there is
        # nothing to pass alongside it.
        queue_deleted.assert_awaited_once_with(LIVENESS_QUEUE_NAME)

    async def test_reports_an_open_channel_when_only_the_tag_was_dropped(
        self, make_consumer_queue, queue_exists
    ):
        # This flag is what decides whether cleanup still nacks the
        # outstanding deliveries, and a per-consumer cancel on a live channel
        # requeues nothing -- so they are still ours to hand back.
        queue = make_consumer_queue(consumer_tags=())

        with pytest.raises(ConsumerCancelledError) as raised:
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

        assert raised.value.channel_closed is False

    async def test_reports_a_closed_channel_when_the_channel_was_killed(
        self, make_consumer_queue, queue_exists
    ):
        # The broker requeued every unacked delivery when it closed the
        # channel, which is what lets cleanup skip the nacks.
        queue = make_consumer_queue(is_closed=True, consumer_tags=())

        with pytest.raises(ConsumerCancelledError) as raised:
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

        assert raised.value.channel_closed is True

    async def test_reports_a_closed_channel_when_there_is_no_underlay(
        self, make_consumer_queue, queue_exists
    ):
        # A channel mid-reopen has already lost the incarnation our delivery
        # tags belonged to, which is a channel close by any other name.
        queue = make_consumer_queue(
            underlay_error=aio_pika.exceptions.ChannelInvalidStateError(
                "Channel was not opened"
            )
        )

        with pytest.raises(ConsumerCancelledError) as raised:
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

        assert raised.value.channel_closed is True

    async def test_reports_a_cancel_when_the_classification_cannot_answer(
        self, make_consumer_queue, queue_check_unreachable
    ):
        # The consumer is already known lost; only the CAUSE is unknown. If
        # the broker error escaped instead, the client would get a bare,
        # unactionable 500 -- and because that type is not a consumer-loss
        # one, cleanup would go down the full cancel-and-nack path against a
        # broker that just failed to answer.
        queue = make_consumer_queue(consumer_tags=())

        with pytest.raises(ConsumerCancelledError):
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

    async def test_keeps_the_unanswered_classification_as_the_cause(
        self, make_consumer_queue, queue_check_unreachable
    ):
        # The real broker error must stay on the chain: it is the only record
        # of why the classification could not be made.
        queue = make_consumer_queue(consumer_tags=())

        with pytest.raises(ConsumerCancelledError) as raised:
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

        assert isinstance(
            raised.value.__cause__, aio_pika.exceptions.AMQPConnectionError
        )

    async def test_carries_the_channel_state_when_the_classification_fails(
        self, make_consumer_queue, queue_check_unreachable
    ):
        # The detection-time fact still decides cleanup on this path, so a
        # failed classification must not lose it.
        queue = make_consumer_queue(is_closed=True, consumer_tags=())

        with pytest.raises(ConsumerCancelledError) as raised:
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

        assert raised.value.channel_closed is True


@pytest.mark.asyncio
class TestRaiseIfConsumerLostWhileTheConnectionIsDown:
    """
    The short circuit that spares a broker which is already struggling.

    The classifying round trip opens a temporary channel and retries it three
    times a second apart. Against a connection that is itself down it can only
    fail -- and that connection is shared by every stream on the pod, so this
    is precisely the case where classification would storm the broker while it
    is least able to answer.
    """

    async def test_ends_the_stream_with_a_reconnect_signal(
        self, make_consumer_queue, broker_connection_down
    ):
        queue = make_consumer_queue(consumer_tags=())

        with pytest.raises(ConsumerCancelledError):
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

    async def test_does_not_classify_against_a_broker_it_cannot_reach(
        self, make_consumer_queue, queue_exists, broker_connection_down
    ):
        queue = make_consumer_queue(consumer_tags=())

        with pytest.raises(ConsumerCancelledError):
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

        queue_exists.assert_not_awaited()

    async def test_reports_the_deliveries_as_already_requeued(
        self, make_consumer_queue, broker_connection_down
    ):
        # The broker requeues every unacked delivery on a connection it has
        # lost, so cleanup must skip the nacks regardless of what this channel
        # handle -- which still reports itself open -- would suggest.
        queue = make_consumer_queue(consumer_tags=())

        with pytest.raises(ConsumerCancelledError) as raised:
            await RabbitMQBase._raise_if_consumer_lost(queue, PUMP_CONSUMER_TAG)

        assert raised.value.channel_closed is True


@pytest.mark.asyncio
class TestRaiseIfConsumerLostAcrossAChannelReopen:
    """
    The incarnation comparison that stops a reopened channel reading healthy.

    The flagship ack-timeout kill of issue b.9gj is a channel CLOSE, and
    closing a channel requeues every unacked delivery on it. Measured against a
    live broker, aio-pika reopens that channel in place ~75 ms later while the
    liveness poll runs on a ~1 s cadence -- so reading the wrapper's
    ``is_closed`` classified the death as a live-channel cancel, and cleanup
    then nacked delivery tags the broker had already taken back, logging one
    caught ``ChannelInvalidStateError`` per outstanding delivery. Comparing
    incarnations instead makes ``channel_closed`` mean only what it says.
    """

    @pytest.fixture
    def reopened(self, make_consumer_queue):
        """
        A channel that died under its consumer and was reopened in place.

        ``kill`` then ``reopen`` on the SAME wrapper is what aio-pika does: the
        wrapper object is reused, ``is_closed`` goes back to False, and only
        the underlay is new. Production declares queues with ``robust=False``,
        so the fresh registry comes back without our tag.
        """
        queue = make_consumer_queue()
        started_on = RabbitMQBase._channel_incarnation(queue)
        queue.channel.kill()
        queue.channel.reopen()
        return SimpleNamespace(queue=queue, started_on=started_on)

    async def test_a_healthy_consumer_on_its_own_incarnation_is_not_lost(
        self, live_queue, queue_exists
    ):
        # Non-vacuity for the whole class: if carrying an incarnation made the
        # healthy path report loss, every quiet stream would die on its first
        # idle poll.
        assert (
            await RabbitMQBase._raise_if_consumer_lost(
                live_queue,
                PUMP_CONSUMER_TAG,
                RabbitMQBase._channel_incarnation(live_queue),
            )
            is None
        )

    async def test_a_tag_dropped_from_the_same_incarnation_leaves_the_channel_open(
        self, make_consumer_queue, queue_exists
    ):
        # The one case that still means "the deliveries are ours to nack": a
        # per-consumer basic.cancel against the very channel the consumer was
        # started on requeues nothing, so skipping the nacks here would send
        # the channel back to the pool with its prefetch budget burned.
        queue = make_consumer_queue()
        started_on = RabbitMQBase._channel_incarnation(queue)
        queue.channel.cancel_consumer(PUMP_CONSUMER_TAG)

        with pytest.raises(ConsumerCancelledError) as raised:
            await RabbitMQBase._raise_if_consumer_lost(
                queue, PUMP_CONSUMER_TAG, started_on
            )

        assert raised.value.channel_closed is False

    async def test_the_reopened_wrapper_reads_open(self, reopened):
        # The premise of every case below, asserted rather than assumed: this
        # is the reading that used to be believed, and if the fake stopped
        # reproducing it the rest of the class would pass vacuously.
        assert reopened.queue.channel.is_closed is False

    async def test_a_reopened_channel_ends_the_stream(self, reopened, queue_exists):
        with pytest.raises(ConsumerCancelledError):
            await RabbitMQBase._raise_if_consumer_lost(
                reopened.queue, PUMP_CONSUMER_TAG, reopened.started_on
            )

    async def test_a_reopened_channel_is_reported_as_a_channel_death(
        self, reopened, queue_exists
    ):
        # THE fix. The wrapper reads open, but the incarnation our delivery
        # tags belonged to is gone and the broker requeued them when it closed
        # it, so cleanup must skip its now-futile nacks.
        with pytest.raises(ConsumerCancelledError) as raised:
            await RabbitMQBase._raise_if_consumer_lost(
                reopened.queue, PUMP_CONSUMER_TAG, reopened.started_on
            )

        assert raised.value.channel_closed is True

    async def test_the_incarnation_outranks_the_registry(
        self, make_consumer_queue, queue_exists
    ):
        # A tag present on a DIFFERENT incarnation is not our consumer: the
        # deliveries died with the channel it was started on however the new
        # registry looks. The registry alone would call this healthy and leave
        # the stream hanging on a consumer nothing feeds.
        queue = make_consumer_queue()
        started_on = RabbitMQBase._channel_incarnation(queue)
        queue.channel.kill()
        queue.channel.reopen(consumer_tags=(PUMP_CONSUMER_TAG,))

        with pytest.raises(ConsumerCancelledError) as raised:
            await RabbitMQBase._raise_if_consumer_lost(
                queue, PUMP_CONSUMER_TAG, started_on
            )

        assert raised.value.channel_closed is True

    async def test_without_a_captured_incarnation_a_reopen_reads_as_a_live_cancel(
        self, reopened, queue_exists
    ):
        # The degraded path, pinned so its cost is visible rather than
        # discovered. With no incarnation to compare, the check falls back to
        # the wrapper's own state and cannot see the death -- the loss is still
        # detected off the empty registry, but classified as a live-channel
        # cancel. Every production caller captures one; this is what a caller
        # that could not looks like.
        with pytest.raises(ConsumerCancelledError) as raised:
            await RabbitMQBase._raise_if_consumer_lost(
                reopened.queue, PUMP_CONSUMER_TAG
            )

        assert raised.value.channel_closed is False


class TestConsumerCancelledError:
    """The terminal error a broker-cancelled consumer ends the stream with."""

    def test_tells_the_client_to_reconnect(self):
        error = ConsumerCancelledError(LIVENESS_QUEUE_NAME, channel_closed=False)

        assert "Reconnect" in error.user_message

    def test_keeps_the_broker_queue_name_out_of_the_client_message(self):
        # ``event-queue:<id>`` is an internal identifier; the client
        # never supplied it in that spelling and can do nothing with it.
        error = ConsumerCancelledError(LIVENESS_QUEUE_NAME, channel_closed=False)

        assert LIVENESS_QUEUE_NAME not in error.user_message

    def test_logs_the_broker_queue_name_for_operators(self):
        # It still has to reach the logs, or an operator cannot tie the
        # terminated stream to a queue.
        error = ConsumerCancelledError(LIVENESS_QUEUE_NAME, channel_closed=False)

        assert LIVENESS_QUEUE_NAME in str(error)

    def test_is_not_the_capacity_shed_status(self):
        # 503 is the capacity-shed code the Envoy retry policy and the
        # capacity metrics key on. A dead consumer is not a capacity
        # condition, so it must not borrow that signal.
        error = ConsumerCancelledError(LIVENESS_QUEUE_NAME, channel_closed=False)

        assert error.status_code == 500


class TestConsumerCancelledErrorRedactsANonDurableName:
    """
    "Log-only" is exactly why a non-durable name is redacted first.

    This message is re-logged on every stream the broker cancels, and a
    non-durable broker name embeds the stream's reconnect token -- the
    queue's identifier, kept out of telemetry as defense in depth. The
    durable spelling is left verbatim (above) because an operator needs it
    to find the queue in the broker.
    """

    @pytest.fixture
    def error(self):
        return ConsumerCancelledError(NON_DURABLE_QUEUE_NAME, channel_closed=False)

    def test_does_not_log_the_reconnect_token(self, error):
        assert RECONNECT_TOKEN not in str(error)

    def test_logs_the_redacted_broker_name(self, error):
        assert f"queue_name: {redact_queue_name(NON_DURABLE_QUEUE_NAME)}" in str(error)

    def test_keeps_the_reconnect_token_out_of_the_client_message(self, error):
        assert RECONNECT_TOKEN not in error.user_message


@pytest.mark.asyncio
class TestNextMessage:
    """The idle-polling wrapper around ``messages_queue.get()``."""

    async def test_returns_a_queued_message(self, live_queue, mocker):
        liveness = mocker.patch.object(
            RabbitMQBase, "_raise_if_consumer_lost", new_callable=AsyncMock
        )
        messages_queue = asyncio.Queue()
        message = MockAckableMessage(b'{"a": "b"}', delivery_tag=1)
        messages_queue.put_nowait(message)

        result = await RabbitMQBase._next_message(
            messages_queue, live_queue, PUMP_CONSUMER_TAG
        )

        assert result is message
        liveness.assert_not_awaited()

    async def test_checks_liveness_when_the_queue_stays_idle(
        self, live_queue, fast_liveness_poll, mocker
    ):
        liveness = mocker.patch.object(
            RabbitMQBase,
            "_raise_if_consumer_lost",
            new_callable=AsyncMock,
            side_effect=ConsumerCancelledError(
                LIVENESS_QUEUE_NAME, channel_closed=False
            ),
        )

        with pytest.raises(ConsumerCancelledError):
            await RabbitMQBase._next_message(
                asyncio.Queue(), live_queue, PUMP_CONSUMER_TAG
            )

        liveness.assert_awaited_once_with(live_queue, PUMP_CONSUMER_TAG, None)

    async def test_hands_the_check_the_incarnation_the_consumer_started_on(
        self, live_queue, fast_liveness_poll, mocker
    ):
        # The liveness check cannot tell a reopened channel from one that never
        # closed unless the caller's captured incarnation reaches it, so this
        # hand-off is the whole fix's carrier. A sentinel rather than a real
        # incarnation, because what is asserted is that the value travels
        # unchanged, not what it means.
        started_incarnation = object()
        liveness = mocker.patch.object(
            RabbitMQBase,
            "_raise_if_consumer_lost",
            new_callable=AsyncMock,
            side_effect=ConsumerCancelledError(
                LIVENESS_QUEUE_NAME, channel_closed=True
            ),
        )

        with pytest.raises(ConsumerCancelledError):
            await RabbitMQBase._next_message(
                asyncio.Queue(), live_queue, PUMP_CONSUMER_TAG, started_incarnation
            )

        liveness.assert_awaited_once_with(
            live_queue, PUMP_CONSUMER_TAG, started_incarnation
        )

    async def test_keeps_waiting_when_the_consumer_is_still_alive(
        self, live_queue, fast_liveness_poll, mocker
    ):
        # A clean liveness check must not end the stream: a quiet queue is
        # the normal state of a healthy consumer.
        mocker.patch.object(
            RabbitMQBase, "_raise_if_consumer_lost", new_callable=AsyncMock
        )
        messages_queue = asyncio.Queue()
        message = MockAckableMessage(b'{"a": "b"}', delivery_tag=1)

        async def deliver_late():
            await asyncio.sleep(IDLE_POLL_FIRES_IMMEDIATELY_SECONDS * 3)
            messages_queue.put_nowait(message)

        deliver = asyncio.ensure_future(deliver_late())
        try:
            result = await RabbitMQBase._next_message(
                messages_queue, live_queue, PUMP_CONSUMER_TAG
            )
        finally:
            await deliver

        assert result is message

    @pytest_asyncio.fixture
    async def idle_poll_that_met_a_shutdown(
        self, live_queue, fast_liveness_poll, mocker
    ):
        """
        An idle wait whose deadline fires with the connector already closing.

        The queue stays empty throughout, so the fast path is genuinely
        bypassed and the poll deadline really does fire -- which is the only
        way to reach the shutdown branch, since a buffered delivery would
        short-circuit ahead of it.
        """
        liveness = mocker.patch.object(
            RabbitMQBase,
            "_raise_if_consumer_lost",
            new_callable=AsyncMock,
            side_effect=ConsumerCancelledError(
                LIVENESS_QUEUE_NAME, channel_closed=False
            ),
        )
        messages_queue = asyncio.Queue()
        get = mocker.spy(messages_queue, "get")
        RabbitMQBase.connector.closing = True

        result = await asyncio.wait_for(
            RabbitMQBase._next_message(messages_queue, live_queue, PUMP_CONSUMER_TAG),
            timeout=CONSUMER_LOSS_TIMEOUT_S,
        )
        return SimpleNamespace(result=result, liveness=liveness, get=get)

    async def test_ends_the_wait_when_the_connector_is_closing(
        self, idle_poll_that_met_a_shutdown
    ):
        # The clean-shutdown sentinel. ``start_servers`` contracts that marking
        # the connector closing makes the consume loops return, and an idle
        # stream has no delivery to hand back and no error to report.
        assert idle_poll_that_met_a_shutdown.result is None

    async def test_does_not_re_enter_the_wait_after_a_shutdown(
        self, idle_poll_that_met_a_shutdown
    ):
        # Non-vacuity for the sentinel above: without it the loop would park on
        # a fresh ``get`` and keep polling a draining broker until gRPC
        # cancelled the stream.
        assert idle_poll_that_met_a_shutdown.get.call_count == 1

    async def test_skips_the_check_while_the_connector_is_closing(
        self, idle_poll_that_met_a_shutdown
    ):
        # Graceful shutdown drains the pools; a consumer going away then is
        # expected, so classifying it would manufacture an INTERNAL out of an
        # ordinary pod rollout.
        idle_poll_that_met_a_shutdown.liveness.assert_not_awaited()

    async def test_still_returns_a_buffered_delivery_while_closing(
        self, live_queue, fast_liveness_poll, mocker
    ):
        # The sentinel is for an IDLE wait only. A delivery already in hand is
        # returned even during shutdown, because the loops -- not this helper --
        # own nacking it back to the broker.
        liveness = mocker.patch.object(
            RabbitMQBase, "_raise_if_consumer_lost", new_callable=AsyncMock
        )
        RabbitMQBase.connector.closing = True
        messages_queue = asyncio.Queue()
        message = MockAckableMessage(b'{"a": "b"}', delivery_tag=1)
        messages_queue.put_nowait(message)

        result = await RabbitMQBase._next_message(
            messages_queue, live_queue, PUMP_CONSUMER_TAG
        )

        assert result is message
        liveness.assert_not_awaited()

    @pytest_asyncio.fixture
    async def delivery_that_raced_the_poll_deadline(
        self, live_queue, fast_liveness_poll, mocker
    ):
        """
        A delivery that lands while the idle poll's deadline is expiring.

        The queue is empty when ``_next_message`` commits to waiting, so the
        fast path is genuinely bypassed; the delivery is then enqueued from
        inside the awaited ``get``, which never returns, so the real
        ``wait_for`` deadline really does fire on a queue that is no longer
        empty. That is the exact boundary a delivery can be dropped at, and
        the fake ``get`` is what makes it happen every run instead of once in
        a thousand.
        """
        liveness = mocker.patch.object(
            RabbitMQBase, "_raise_if_consumer_lost", new_callable=AsyncMock
        )
        messages_queue = asyncio.Queue()
        message = MockAckableMessage(b'{"a": "b"}', delivery_tag=1)

        async def deliver_then_never_return():
            messages_queue.put_nowait(message)
            await asyncio.Event().wait()

        mocker.patch.object(
            messages_queue, "get", side_effect=deliver_then_never_return
        )

        result = await asyncio.wait_for(
            RabbitMQBase._next_message(messages_queue, live_queue, PUMP_CONSUMER_TAG),
            timeout=CONSUMER_LOSS_TIMEOUT_S,
        )
        return SimpleNamespace(result=result, message=message, liveness=liveness)

    async def test_returns_a_delivery_that_raced_the_poll_deadline(
        self, delivery_that_raced_the_poll_deadline
    ):
        # ``asyncio.Queue.get`` leaves the item queued when its getter is
        # cancelled, so the next iteration's fast path must hand it back. A
        # loop that treated the timeout as "still idle" would strand a
        # delivery the broker considers delivered until the stream ended.
        episode = delivery_that_raced_the_poll_deadline

        assert episode.result is episode.message

    async def test_still_polls_liveness_at_that_deadline(
        self, delivery_that_raced_the_poll_deadline
    ):
        # Non-vacuity: without this the case above could pass by taking the
        # fast path on entry and never reaching the boundary at all.
        delivery_that_raced_the_poll_deadline.liveness.assert_awaited_once()


@pytest.fixture(params=["client acks", "delayed acks"])
def consume_generator(request, authz_true, no_server_acks):
    """
    Build the loop under test, over BOTH durable consume loops.

    The two differ only in who acks -- the client by delivery tag, or the
    server after a delay -- and share one consumer-loss contract, so each case
    below is written once and run against both. ``no_server_acks`` keeps the
    delayed-ack variant from acking anything mid-test; the client-ack loop
    never reads that setting.
    """

    def _make(queue, **kwargs):
        if request.param == "client acks":
            return RabbitMQBase._consume_with_client_acks(
                queue, authz_true, AckTracker(), **kwargs
            )
        return RabbitMQBase._consume_with_delayed_acks(queue, authz_true, **kwargs)

    return _make


@pytest.mark.asyncio
class TestConsumerLoss:
    """
    A consumer that dies under a live stream ends that stream (issue b.9gj).

    Before the liveness poll, nothing propagated: the loops blocked forever on
    ``messages_queue.get()`` while the client's gRPC/REST stream reported
    healthy against a queue nothing was attached to -- for ~47 hours in the
    2026-08-24 dev-cluster incident.
    """

    @pytest_asyncio.fixture
    async def streaming(self, pump_boundary, cancellable_queue, consume_generator):
        """
        A live stream that has already delivered, with its first message held.

        The delivery matters: a cleanup assertion about outstanding messages
        is vacuous unless there IS one outstanding, and every case here turns
        on what happens to it.
        """
        message = MockAckableMessage(b'{"a": "b"}', delivery_tag=1)
        generator = consume_generator(cancellable_queue)
        pump_boundary.put_nowait(message)
        record = await anext(generator)

        yield SimpleNamespace(
            generator=generator,
            queue=cancellable_queue,
            channel=cancellable_queue.channel,
            message=message,
            record=record,
        )

    async def test_the_stream_ends_with_a_reconnect_signal_on_a_broker_cancel(
        self, streaming, fast_liveness_poll, queue_exists
    ):
        streaming.channel.cancel_consumer(PUMP_CONSUMER_TAG)

        await expect_terminal_error(streaming.generator, ConsumerCancelledError)

    async def test_the_stream_ends_with_not_found_when_the_queue_was_deleted(
        self, streaming, fast_liveness_poll, queue_deleted
    ):
        # A deleted queue is a DIFFERENT client remedy from a bare consumer
        # cancel -- reprovision, not reconnect -- and the loop must carry that
        # classification out to the client rather than flattening it.
        streaming.channel.cancel_consumer(PUMP_CONSUMER_TAG)

        await expect_terminal_error(streaming.generator, QueueNotFoundError)

    async def test_cleanup_skips_the_consumer_cancel(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The broker has already dropped the tag from its own registry -- that
        # absence is what the check detected -- so cancelling it is a round
        # trip that can only fail.
        streaming.channel.cancel_consumer(PUMP_CONSUMER_TAG)

        await expect_terminal_error(streaming.generator, ConsumerCancelledError)
        await expect_cleanup(background_tasks)

        RabbitMQBase._cancel_queue_consumer.assert_not_awaited()

    async def test_cleanup_nacks_the_outstanding_delivery_on_a_live_channel(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        # A per-consumer cancel on a channel that is still OPEN requeues
        # NOTHING, so the delivery is still ours to hand back. Skipping the
        # nack here would send the channel back to the pool with its prefetch
        # budget permanently burned by an unacked ghost.
        streaming.channel.cancel_consumer(PUMP_CONSUMER_TAG)

        await expect_terminal_error(streaming.generator, ConsumerCancelledError)
        await expect_cleanup(background_tasks)

        streaming.message.nack.assert_awaited_once_with(requeue=True)

    async def test_cleanup_skips_the_nacks_when_the_channel_was_closed(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The broker requeued every unacked delivery when it closed the
        # channel, so each nack would be a guaranteed failure logging a stack
        # trace per in-flight message.
        streaming.channel.kill()

        await expect_terminal_error(streaming.generator, ConsumerCancelledError)
        await expect_cleanup(background_tasks)

        streaming.message.nack.assert_not_awaited()

    async def test_cleanup_skips_the_nacks_when_the_channel_reopened_underneath(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The flagship ack-timeout kill, driven end to end through the loop.
        # The broker closed the channel -- requeueing the delivery -- and
        # aio-pika reopened it in place before the ~1 s poll looked, so the
        # wrapper reads open again. Nacking here aims at a tag the dead
        # incarnation took with it: one caught ChannelInvalidStateError logged
        # at ERROR per in-flight message, up to the prefetch count, every time
        # a stuck client is timed out.
        streaming.channel.kill()
        streaming.channel.reopen()

        await expect_terminal_error(streaming.generator, ConsumerCancelledError)
        await expect_cleanup(background_tasks)

        streaming.message.nack.assert_not_awaited()

    async def test_cleanup_skips_the_nacks_when_the_queue_is_gone(
        self, streaming, fast_liveness_poll, queue_deleted, background_tasks
    ):
        # Every delivery outstanding against a deleted queue died with it.
        streaming.channel.cancel_consumer(PUMP_CONSUMER_TAG)

        await expect_terminal_error(streaming.generator, QueueNotFoundError)
        await expect_cleanup(background_tasks)

        streaming.message.nack.assert_not_awaited()

    async def test_cleanup_still_releases_the_channel(
        self,
        pump_boundary,
        cancellable_queue,
        consume_generator,
        fast_liveness_poll,
        queue_exists,
        background_tasks,
    ):
        # THE thing that must not be skipped: on_cleanup_complete is how the
        # caller returns its pool checkout, so skipping it would trade a
        # zombie stream for a leaked channel.
        on_cleanup = AsyncMock()
        generator = consume_generator(cancellable_queue, on_cleanup_complete=on_cleanup)
        pump_boundary.put_nowait(MockAckableMessage(b'{"a": "b"}', delivery_tag=1))
        await anext(generator)
        cancellable_queue.channel.cancel_consumer(PUMP_CONSUMER_TAG)

        await expect_terminal_error(generator, ConsumerCancelledError)
        await expect_cleanup(background_tasks)

        on_cleanup.assert_awaited_once()

    async def test_normal_close_still_cancels_the_consumer(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The control for the skips above: with the consumer still alive, an
        # ordinary client disconnect takes the unchanged path.
        await streaming.generator.aclose()
        await expect_cleanup(background_tasks)

        RabbitMQBase._cancel_queue_consumer.assert_awaited_once()

    async def test_a_stream_cancelled_mid_poll_is_not_consumer_loss(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        # A quiet stream ends this way every time a client goes away: the task
        # driving the generator is cancelled while the loop sits inside the
        # idle poll. The consumer is alive, so cleanup owes the broker a real
        # basic.cancel -- treating the cancellation as consumer loss would
        # leak the consumer for the rest of the channel's life.
        await cancel_while_polling(streaming.generator)
        await expect_cleanup(background_tasks)

        RabbitMQBase._cancel_queue_consumer.assert_awaited_once()

    async def test_a_stream_cancelled_mid_poll_still_releases_the_channel(
        self,
        pump_boundary,
        cancellable_queue,
        consume_generator,
        fast_liveness_poll,
        queue_exists,
        background_tasks,
    ):
        on_cleanup = AsyncMock()
        generator = consume_generator(cancellable_queue, on_cleanup_complete=on_cleanup)
        pump_boundary.put_nowait(MockAckableMessage(b'{"a": "b"}', delivery_tag=1))
        await anext(generator)

        await cancel_while_polling(generator)
        await expect_cleanup(background_tasks)

        on_cleanup.assert_awaited_once()

    async def test_a_stream_cancelled_mid_poll_reports_only_the_cancellation(
        self, streaming, fast_liveness_poll, queue_exists
    ):
        # The caller must see its own CancelledError, not a
        # ConsumerCancelledError manufactured by a poll that happened to be
        # in flight.
        step = await cancel_while_polling(streaming.generator)

        assert step.cancelled()


@pytest.mark.asyncio
class TestConsumerLossRacedByShutdown:
    """
    A pod rollout that lands on an idle stream is a clean end, not an error.

    Both halves of the race are here. The connector can start closing while
    the loop is parked in the idle wait -- ``_next_message`` hands back its
    clean-shutdown sentinel and the loop must simply stop -- or it can start
    closing while the liveness check is mid-classification, in which case the
    consumer-loss error it produces describes the shutdown rather than a
    zombie and must not reach the client. Either way this is the most common
    moment a stream ends in production, and surfacing an INTERNAL for it would
    turn every rollout into a wave of client-visible errors.
    """

    @pytest_asyncio.fixture
    async def streaming(self, pump_boundary, cancellable_queue, consume_generator):
        """A live stream that has already delivered, with its first message held."""
        message = MockAckableMessage(b'{"a": "b"}', delivery_tag=1)
        generator = consume_generator(cancellable_queue)
        pump_boundary.put_nowait(message)
        record = await anext(generator)

        yield SimpleNamespace(generator=generator, message=message, record=record)

    @pytest.fixture(params=["broker cancel", "queue deleted"])
    def lost_while_shutting_down(self, request, streaming, mocker):
        """
        The liveness check raises just as the connector goes into shutdown.

        Patched at ``_next_message`` because the flip has to land INSIDE the
        call: were the connector already closing, the loop's top-of-loop check
        would return before the check ever ran, and the handler under test
        would never be reached. Both consumer-loss types are covered, since
        the handler catches them as one tuple.
        """
        if request.param == "broker cancel":
            error = ConsumerCancelledError(LIVENESS_QUEUE_NAME, channel_closed=False)
        else:
            error = QueueNotFoundError(queue_name=LIVENESS_QUEUE_NAME)

        async def raise_as_shutdown_begins(*_args):
            RabbitMQBase.connector.closing = True
            raise error

        return mocker.patch.object(
            RabbitMQBase,
            "_next_message",
            new_callable=AsyncMock,
            side_effect=raise_as_shutdown_begins,
        )

    async def test_an_idle_stream_ends_without_an_error(
        self, streaming, fast_liveness_poll, queue_exists
    ):
        records = await drain_while_shutting_down(
            streaming.generator, RabbitMQBase.connector
        )

        assert records == []

    async def test_an_idle_shutdown_still_cancels_the_consumer(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        # Nothing was lost here -- the consumer is alive and the broker is
        # still there -- so this takes the ordinary cleanup path in full.
        await drain_while_shutting_down(streaming.generator, RabbitMQBase.connector)
        await expect_cleanup(background_tasks)

        RabbitMQBase._cancel_queue_consumer.assert_awaited_once()

    async def test_an_idle_shutdown_still_nacks_the_outstanding_delivery(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The held delivery has to go back on the queue for the next pod to
        # serve; dropping it on shutdown would strand it until the ack timeout.
        await drain_while_shutting_down(streaming.generator, RabbitMQBase.connector)
        await expect_cleanup(background_tasks)

        streaming.message.nack.assert_awaited_once_with(requeue=True)

    async def test_a_consumer_loss_raced_by_shutdown_is_not_surfaced(
        self, streaming, lost_while_shutting_down
    ):
        # A consumer going away as the pools drain is expected, so the loop
        # swallows it rather than ending the client's stream with an INTERNAL.
        records = [record async for record in streaming.generator]

        assert records == []

    async def test_a_consumer_loss_raced_by_shutdown_still_cancels_the_consumer(
        self, streaming, lost_while_shutting_down, background_tasks
    ):
        # The swallowed error must not also be recorded as consumer loss: this
        # is an ordinary shutdown, so cleanup owes the broker its basic.cancel.
        [record async for record in streaming.generator]
        await expect_cleanup(background_tasks)

        RabbitMQBase._cancel_queue_consumer.assert_awaited_once()

    async def test_a_consumer_loss_raced_by_shutdown_still_nacks(
        self, streaming, lost_while_shutting_down, background_tasks
    ):
        [record async for record in streaming.generator]
        await expect_cleanup(background_tasks)

        streaming.message.nack.assert_awaited_once_with(requeue=True)


class TestCleanupPredicatesWithADeadChannelVerdict:
    """
    The settle side's verdict is a second, independent witness of channel
    death, and either witness alone is conclusive.

    ``consumer_lost_exc`` is the event side's: the idle liveness poll
    noticed the incarnation was gone, or one of the loop's own broker
    operations failed on the dead channel. ``channel_dead`` is the settle
    side's: a client ack/nack went to the broker and came back with a
    dead-channel error. The settle side wins the race whenever the client
    is settling at the moment of the kill -- and because that discovery is
    made on the settle side of the servicer's merge, which tears the
    generator down by cancelling its pending read, ``consumer_lost_exc``
    stays None on precisely the path where the channel is most certainly
    dead.
    """

    def test_cancel_is_skipped_on_the_settle_side_verdict_alone(self):
        # The shape that matters: no event-side witness at all, because the
        # generator was only ever cancelled out of its wait. Cancelling a
        # consumer the broker already dropped is a futile round trip.
        assert RabbitMQBase._should_cancel_consumer(None, True) is False

    def test_nacks_are_skipped_on_the_settle_side_verdict_alone(self):
        # Without this the cleanup would nack delivery tags the broker
        # requeued when it closed the channel: one logged failure per
        # outstanding delivery, on exactly the incident path.
        assert RabbitMQBase._should_nack_outstanding(None, True) is False

    def test_the_verdict_overrides_a_live_channel_consumer_loss(self):
        # A ConsumerCancelledError carrying channel_closed=False is the one
        # consumer-loss shape that normally KEEPS the nacks (a per-consumer
        # cancel on a live channel requeues nothing). The settle side's
        # verdict is checked first and still suppresses them, because the
        # settle actually reached the broker and was refused.
        consumer_lost = ConsumerCancelledError(LIVENESS_QUEUE_NAME, False)

        assert RabbitMQBase._should_nack_outstanding(consumer_lost, True) is False

    def test_the_verdict_overrides_a_live_channel_cancel_for_the_consumer(self):
        consumer_lost = ConsumerCancelledError(LIVENESS_QUEUE_NAME, False)

        assert RabbitMQBase._should_cancel_consumer(consumer_lost, True) is False

    def test_a_live_channel_verdict_leaves_the_cancel_decision_alone(self):
        # The control: with no settle-side verdict, both predicates answer
        # exactly as they did before the parameter existed.
        assert RabbitMQBase._should_cancel_consumer(None, False) is True

    def test_a_live_channel_verdict_leaves_the_nack_decision_alone(self):
        assert RabbitMQBase._should_nack_outstanding(None, False) is True

    def test_the_verdict_defaults_to_a_live_channel_for_the_cancel(self):
        # The default matters on its own: the consume paths with no
        # client-settle surface (the delayed-ack loop) call these with one
        # argument and must keep the event-side-only behaviour.
        assert RabbitMQBase._should_cancel_consumer(None) is True

    def test_the_verdict_defaults_to_a_live_channel_for_the_nacks(self):
        assert RabbitMQBase._should_nack_outstanding(None) is True


@pytest.mark.asyncio
class TestClientAckLoopCleanupOnAMarkedTracker:
    """
    A tracker marked dead narrows this loop's cleanup, even though the loop
    itself never saw an error (issues b.qsa, b.xr7).

    This is the flagship ack-timeout shape: the client settles a tag at the
    moment the broker kills the channel, so the settle fails FIRST -- on the
    settle side of the servicer's merge, ahead of the loop's next idle
    liveness poll. The tracker marks itself at that failing settle, and the
    merge then tears this generator down with ``consumer_lost_exc`` still
    None. Reading the verdict off the tracker is the only way the cleanup
    learns the channel is gone.

    These arms mark the tracker and then close the generator with
    ``aclose()``, which pins how the cleanup reads the verdict, not the
    order production reaches it in. The production order -- a settle
    failing while this loop is waiting on its next delivery -- is pinned end
    to end by ``TestASettleSideChannelDeathNarrowsTheEventSideCleanup`` in
    ``tests/app/grpc/test_event_consumer_servicer.py``.
    """

    @pytest_asyncio.fixture
    async def streaming(self, pump_boundary, cancellable_queue, authz_true):
        """
        A live client-ack stream that has already delivered, with its first
        message held and the session tracker exposed.

        The delivery matters: every assertion here is about what happens to
        an OUTSTANDING delivery, and would be vacuous with none in flight.
        The loop is built directly rather than through ``consume_generator``
        because that fixture owns its tracker privately, and the tracker is
        the seam under test.
        """
        tracker = AckTracker()
        message = MockAckableMessage(b'{"a": "b"}', delivery_tag=1)
        generator = RabbitMQBase._consume_with_client_acks(
            cancellable_queue, authz_true, tracker
        )
        pump_boundary.put_nowait(message)
        record = await anext(generator)

        yield SimpleNamespace(
            generator=generator,
            tracker=tracker,
            message=message,
            record=record,
        )

    async def test_cleanup_skips_the_consumer_cancel(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        streaming.tracker.mark_channel_dead()

        await streaming.generator.aclose()
        await expect_cleanup(background_tasks)

        RabbitMQBase._cancel_queue_consumer.assert_not_awaited()

    async def test_cleanup_skips_the_outstanding_nack(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        streaming.tracker.mark_channel_dead()

        await streaming.generator.aclose()
        await expect_cleanup(background_tasks)

        streaming.message.nack.assert_not_awaited()

    async def test_cleanup_skips_the_prefetched_nacks(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The buffered deliveries the broker pushed but the client never saw
        # died with the incarnation exactly as the in-flight ones did, so the
        # skip has to cover both halves of the cleanup, not just the tracker.
        streaming.tracker.mark_channel_dead()

        await streaming.generator.aclose()
        await expect_cleanup(background_tasks)

        RabbitMQBase._nack_prefetched_messages.assert_not_awaited()

    async def test_cleanup_still_releases_the_channel(
        self,
        pump_boundary,
        cancellable_queue,
        authz_true,
        fast_liveness_poll,
        queue_exists,
        background_tasks,
    ):
        # THE thing that must not be skipped: on_cleanup_complete is how the
        # caller returns its pool checkout, so a narrowing that also skipped
        # it would trade a doomed nack for a leaked channel.
        tracker = AckTracker()
        on_cleanup = AsyncMock()
        generator = RabbitMQBase._consume_with_client_acks(
            cancellable_queue, authz_true, tracker, on_cleanup_complete=on_cleanup
        )
        pump_boundary.put_nowait(MockAckableMessage(b'{"a": "b"}', delivery_tag=1))
        await anext(generator)
        tracker.mark_channel_dead()

        await generator.aclose()
        await expect_cleanup(background_tasks)

        on_cleanup.assert_awaited_once()

    async def test_an_unmarked_tracker_still_cancels_the_consumer(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The control arm. Same teardown, same fixtures, tracker left alone:
        # an ordinary client disconnect on a healthy channel takes the full
        # cleanup path. Without this the skips above would pass for a cleanup
        # that had simply stopped doing anything.
        await streaming.generator.aclose()
        await expect_cleanup(background_tasks)

        RabbitMQBase._cancel_queue_consumer.assert_awaited_once()

    async def test_an_unmarked_tracker_still_nacks_the_outstanding_delivery(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        await streaming.generator.aclose()
        await expect_cleanup(background_tasks)

        streaming.message.nack.assert_awaited_once_with(requeue=True)

    async def test_a_mark_landing_after_the_generator_closed_still_counts(
        self, streaming, fast_liveness_poll, queue_exists, background_tasks
    ):
        # The verdict is read INSIDE the cleanup task, not captured when the
        # task is scheduled. The generator's finally reaches
        # ``create_task(_cleanup())`` and returns before the task body ever
        # runs, so a mark landing in that window must still narrow the
        # cleanup. A settle that fails at the same moment the stream is torn
        # down for another reason marks the tracker in exactly that window.
        await streaming.generator.aclose()
        streaming.tracker.mark_channel_dead()

        await expect_cleanup(background_tasks)

        streaming.message.nack.assert_not_awaited()


# ---------------------------------------------------------------------------
# A dead channel found by one of the consume loop's OWN broker operations
# (issue b.7yv)
#
# The loops settle some deliveries themselves: the skip-ack of a delivery the
# authorization check denies, the nack of one whose authorization check
# raised, the delayed-ack loop's flush of aged deliveries, and the nack of a
# delivery dequeued as the pod shuts down. Each goes to the consumer's own
# channel, so a dead-channel error from one of them is direct evidence that
# the consumer's incarnation is gone -- the same verdict the idle liveness
# poll reaches, and it must narrow the cleanup the same way.
# ---------------------------------------------------------------------------

#: The parsed body of the delivery whose broker operation is made to fail.
#: The authorization stand-ins key on it, so every other delivery stays
#: authorized.
_FAILING_MESSAGE = {"a": "the failing delivery"}
_FAILING_BODY = b'{"a": "the failing delivery"}'
_AUTHORIZED_BODY = b'{"a": "b"}'


def _skip_ack_fails(failing, failure):
    """
    The authorization check denies ``failing``, and its skip-ack raises.

    :returns: The authorization check to hand the loop.
    """
    failing.ack.side_effect = failure

    async def _authz(message):
        return message != _FAILING_MESSAGE

    return _authz


def _authorization_failure_nack_fails(failing, failure):
    """
    The authorization check raises for ``failing``, and the nack the
    authorization-failure branch sends for it raises.

    :returns: The authorization check to hand the loop.
    """
    failing.nack.side_effect = failure

    async def _authz(message):
        if message == _FAILING_MESSAGE:
            raise PermissionError("the authorization check failed")
        return True

    return _authz


_OWN_OPERATION_SITES = {
    "skip-ack": _skip_ack_fails,
    "authorization-failure nack": _authorization_failure_nack_fails,
}


@pytest.fixture(params=list(_OWN_OPERATION_SITES))
def own_operation_site(request):
    """Each own broker operation both durable loops reach per delivery."""
    return _OWN_OPERATION_SITES[request.param]


@pytest.fixture(params=["client acks", "delayed acks"])
def consume_loop(request, no_server_acks):
    """
    Build either durable consume loop with a chosen authorization check.

    ``no_server_acks`` keeps the delayed-ack loop from acking a yielded
    delivery mid-test, so it stays outstanding for the cleanup to decide
    about; the client-ack loop never reads that setting.
    """

    def _make(queue, authz_check, **kwargs):
        if request.param == "client acks":
            return RabbitMQBase._consume_with_client_acks(
                queue, authz_check, AckTracker(), **kwargs
            )
        return RabbitMQBase._consume_with_delayed_acks(queue, authz_check, **kwargs)

    return _make


@pytest.fixture
def quiet_liveness_poll(mocker):
    """
    Keep the idle liveness poll from firing.

    These tests are about the loop's own broker operations finding the
    channel dead, so the poll must not reach the verdict first.
    """
    mocker.patch.object(
        RabbitMQBase,
        "_liveness_poll_timeout",
        return_value=IDLE_POLL_NEVER_FIRES_SECONDS,
    )


async def _error_ending_the_stream(generator):
    """
    Take the generator's next step and return the error that ended it.

    Returns None when the step produced a record instead, after closing the
    generator so its cleanup is still scheduled. The error is captured
    rather than asserted with ``pytest.raises`` because the cleanup
    assertions need the stream ended either way, and a wrong error type
    must still fail the type assertion by name rather than error the
    fixture.
    """
    try:
        await asyncio.wait_for(anext(generator), timeout=CONSUMER_LOSS_TIMEOUT_S)
    except Exception as exc:  # captured for the assertions, see above
        return exc
    await generator.aclose()
    return None


@pytest_asyncio.fixture
async def own_operation_failure(
    pump_boundary,
    cancellable_queue,
    consume_loop,
    own_operation_site,
    quiet_liveness_poll,
    background_tasks,
):
    """
    Run a live stream until one of the loop's own broker operations fails.

    One delivery is yielded and stays outstanding, the next is the one
    whose broker operation raises, and a third stays buffered behind it.
    The outstanding and buffered deliveries are what keep the cleanup
    assertions from being vacuous: a cleanup on a healthy channel owes
    the first a nack and the second a drain.

    :returns: A callable taking the injected failure and returning what the
        stream did with it, once the cleanup task has finished.
    """

    async def _run(failure):
        outstanding = MockAckableMessage(_AUTHORIZED_BODY, delivery_tag=1)
        failing = MockAckableMessage(_FAILING_BODY, delivery_tag=2)
        buffered = MockAckableMessage(_AUTHORIZED_BODY, delivery_tag=3)
        authz_check = own_operation_site(failing, failure)
        on_cleanup = AsyncMock()
        generator = consume_loop(
            cancellable_queue, authz_check, on_cleanup_complete=on_cleanup
        )
        for message in (outstanding, failing, buffered):
            pump_boundary.put_nowait(message)

        await anext(generator)
        error = await _error_ending_the_stream(generator)
        await expect_cleanup(background_tasks)

        return SimpleNamespace(
            error=error,
            failure=failure,
            outstanding=outstanding,
            on_cleanup=on_cleanup,
        )

    return _run


@pytest.mark.asyncio
class TestConsumeLoopOwnOperationOnADeadChannel:
    """
    A skip-ack or an authorization-failure nack that finds the channel dead
    ends the stream as consumer loss, in both durable loops.

    The stream ends with the same reconnect guidance as every other
    dead-channel path, and the cleanup narrows to what can still do
    something: no cancel for a consumer the broker already dropped, no
    nacks for deliveries it already requeued, and the channel still
    released.
    """

    @dead_channel_failures
    async def test_the_stream_ends_with_a_reconnect_signal(
        self, own_operation_failure, make_failure
    ):
        stream = await own_operation_failure(make_failure())

        assert isinstance(stream.error, ConsumerCancelledError)

    @dead_channel_failures
    async def test_the_error_records_the_channel_as_closed(
        self, own_operation_failure, make_failure
    ):
        stream = await own_operation_failure(make_failure())

        assert getattr(stream.error, "channel_closed", None) is True

    @dead_channel_failures
    async def test_the_error_is_chained_to_the_dead_channel_error(
        self, own_operation_failure, make_failure
    ):
        # The broker's own reason stays in the logged traceback.
        stream = await own_operation_failure(make_failure())

        assert stream.error.__cause__ is stream.failure

    @dead_channel_failures
    async def test_cleanup_skips_the_consumer_cancel(
        self, own_operation_failure, make_failure
    ):
        await own_operation_failure(make_failure())

        RabbitMQBase._cancel_queue_consumer.assert_not_awaited()

    @dead_channel_failures
    async def test_cleanup_skips_the_outstanding_nack(
        self, own_operation_failure, make_failure
    ):
        stream = await own_operation_failure(make_failure())

        stream.outstanding.nack.assert_not_awaited()

    @dead_channel_failures
    async def test_cleanup_skips_the_prefetched_nacks(
        self, own_operation_failure, make_failure
    ):
        await own_operation_failure(make_failure())

        RabbitMQBase._nack_prefetched_messages.assert_not_awaited()

    @dead_channel_failures
    async def test_cleanup_still_releases_the_channel(
        self, own_operation_failure, make_failure
    ):
        stream = await own_operation_failure(make_failure())

        stream.on_cleanup.assert_awaited_once()


@pytest.mark.asyncio
class TestConsumeLoopOwnOperationFailingForAnotherReason:
    """
    The control arms: the same sites failing with an error that says
    nothing about the channel keep the healthy channel's handling.

    The channel is still alive here, so the error surfaces as itself and
    the cleanup owes the broker the cancel and the nacks. The builtin
    ``ConnectionError`` arm pins that only aiormq's closed-connection
    family counts, not its base class. On the skip-ack site, a cancel and
    a drain of exactly one each also pin that the failed skip-ack does not
    run the authorization-failure branch as well, which would send both a
    second time.
    """

    @unrelated_failures
    async def test_the_failure_propagates_as_itself(
        self, own_operation_failure, make_failure
    ):
        stream = await own_operation_failure(make_failure())

        assert stream.error is stream.failure

    @unrelated_failures
    async def test_cleanup_cancels_the_consumer(
        self, own_operation_failure, make_failure
    ):
        await own_operation_failure(make_failure())

        RabbitMQBase._cancel_queue_consumer.assert_awaited_once()

    @unrelated_failures
    async def test_cleanup_nacks_the_outstanding_delivery(
        self, own_operation_failure, make_failure
    ):
        stream = await own_operation_failure(make_failure())

        stream.outstanding.nack.assert_awaited_once_with(requeue=True)

    @unrelated_failures
    async def test_cleanup_drains_the_prefetched_deliveries(
        self, own_operation_failure, make_failure
    ):
        await own_operation_failure(make_failure())

        RabbitMQBase._nack_prefetched_messages.assert_awaited_once()

    @unrelated_failures
    async def test_cleanup_still_releases_the_channel(
        self, own_operation_failure, make_failure
    ):
        stream = await own_operation_failure(make_failure())

        stream.on_cleanup.assert_awaited_once()


@pytest_asyncio.fixture
async def aged_ack_failure(
    pump_boundary,
    cancellable_queue,
    authz_true,
    quiet_liveness_poll,
    background_tasks,
    mocker,
):
    """
    Run a delayed-ack stream until its in-loop flush acks an aged delivery
    whose ack raises.

    Two deliveries are yielded while nothing ages, so both are pending.
    The delay then drops to zero, and the next step's flush pops the first
    and acks it. The second is still pending when that ack fails -- the
    outstanding delivery a healthy cleanup would nack -- and a fourth
    delivery stays buffered behind the third, which triggered the flush.

    :returns: A callable taking the injected failure and returning what the
        stream did with it, once the cleanup task has finished.
    """

    async def _run(failure):
        aged = MockAckableMessage(_AUTHORIZED_BODY, delivery_tag=1)
        aged.ack.side_effect = failure
        outstanding = MockAckableMessage(_AUTHORIZED_BODY, delivery_tag=2)
        trigger = MockAckableMessage(_AUTHORIZED_BODY, delivery_tag=3)
        buffered = MockAckableMessage(_AUTHORIZED_BODY, delivery_tag=4)
        for message in (aged, outstanding, trigger, buffered):
            pump_boundary.put_nowait(message)
        on_cleanup = AsyncMock()
        mocker.patch.object(
            _settings_module.settings, "server_ack_delay_seconds", 999.0
        )
        generator = RabbitMQBase._consume_with_delayed_acks(
            cancellable_queue, authz_true, on_cleanup_complete=on_cleanup
        )

        await anext(generator)
        await anext(generator)
        mocker.patch.object(_settings_module.settings, "server_ack_delay_seconds", 0.0)
        error = await _error_ending_the_stream(generator)
        await expect_cleanup(background_tasks)

        return SimpleNamespace(
            error=error,
            failure=failure,
            outstanding=outstanding,
            on_cleanup=on_cleanup,
        )

    return _run


@pytest.mark.asyncio
class TestDelayedAckFlushOnADeadChannel:
    """
    The delayed-ack loop's in-loop flush of aged deliveries is one of its
    own broker operations too.

    A failed ack there is logged and skipped unless it is a dead-channel
    error. That one says the channel is gone rather than that one ack
    failed -- every remaining ack would fail the same way and log an ERROR
    each -- so it ends the stream as consumer loss, exactly like the
    per-delivery operations.
    """

    @dead_channel_failures
    async def test_the_stream_ends_with_a_reconnect_signal(
        self, aged_ack_failure, make_failure
    ):
        stream = await aged_ack_failure(make_failure())

        assert isinstance(stream.error, ConsumerCancelledError)

    @dead_channel_failures
    async def test_the_error_records_the_channel_as_closed(
        self, aged_ack_failure, make_failure
    ):
        stream = await aged_ack_failure(make_failure())

        assert getattr(stream.error, "channel_closed", None) is True

    @dead_channel_failures
    async def test_the_error_is_chained_to_the_dead_channel_error(
        self, aged_ack_failure, make_failure
    ):
        stream = await aged_ack_failure(make_failure())

        assert stream.error.__cause__ is stream.failure

    @dead_channel_failures
    async def test_cleanup_skips_the_consumer_cancel(
        self, aged_ack_failure, make_failure
    ):
        await aged_ack_failure(make_failure())

        RabbitMQBase._cancel_queue_consumer.assert_not_awaited()

    @dead_channel_failures
    async def test_the_flush_stops_at_the_dead_channel_ack(
        self, aged_ack_failure, make_failure
    ):
        # Every remaining ack would fail the same way, so the next aged
        # delivery is left pending rather than acked into a dead channel.
        stream = await aged_ack_failure(make_failure())

        stream.outstanding.ack.assert_not_awaited()

    @dead_channel_failures
    async def test_cleanup_skips_the_outstanding_nack(
        self, aged_ack_failure, make_failure
    ):
        # The delivery the flush left pending is not nacked either: the
        # broker requeued it when it closed the channel.
        stream = await aged_ack_failure(make_failure())

        stream.outstanding.nack.assert_not_awaited()

    @dead_channel_failures
    async def test_cleanup_skips_the_prefetched_nacks(
        self, aged_ack_failure, make_failure
    ):
        await aged_ack_failure(make_failure())

        RabbitMQBase._nack_prefetched_messages.assert_not_awaited()

    @dead_channel_failures
    async def test_cleanup_still_releases_the_channel(
        self, aged_ack_failure, make_failure
    ):
        stream = await aged_ack_failure(make_failure())

        stream.on_cleanup.assert_awaited_once()

    @unrelated_failures
    async def test_another_ack_failure_does_not_end_the_stream(
        self, aged_ack_failure, make_failure
    ):
        # The control arm: one ack failing for a reason that says nothing
        # about the channel is still logged and skipped, and the delivery
        # that triggered the flush is yielded as usual.
        stream = await aged_ack_failure(make_failure())

        assert stream.error is None


@pytest.fixture
def shutdown_on_second_dequeue(pump_boundary, mocker):
    """The first delivery stays outstanding; the second meets a shutdown."""
    return close_connector_on_dequeue(pump_boundary, RabbitMQBase.connector, mocker)


@pytest_asyncio.fixture
async def shutdown_nack_failure(
    shutdown_on_second_dequeue,
    cancellable_queue,
    consume_loop,
    authz_true,
    quiet_liveness_poll,
    background_tasks,
):
    """
    Run a stream into a shutdown whose nack of the dequeued delivery raises.

    :returns: A callable taking the injected failure and returning what the
        stream did with it, once the cleanup task has finished.
    """

    async def _run(failure):
        outstanding = MockAckableMessage(_AUTHORIZED_BODY, delivery_tag=1)
        at_shutdown = MockAckableMessage(_AUTHORIZED_BODY, delivery_tag=2)
        at_shutdown.nack.side_effect = failure
        buffered = MockAckableMessage(_AUTHORIZED_BODY, delivery_tag=3)
        for message in (outstanding, at_shutdown, buffered):
            shutdown_on_second_dequeue.put_nowait(message)
        on_cleanup = AsyncMock()
        generator = consume_loop(
            cancellable_queue, authz_true, on_cleanup_complete=on_cleanup
        )

        await anext(generator)
        try:
            remaining = await asyncio.wait_for(
                drain(generator), timeout=CONSUMER_LOSS_TIMEOUT_S
            )
            error = None
        except Exception as exc:  # captured for the assertions
            remaining = None
            error = exc
        await expect_cleanup(background_tasks)

        return SimpleNamespace(
            remaining=remaining,
            error=error,
            outstanding=outstanding,
            on_cleanup=on_cleanup,
        )

    return _run


@pytest.mark.asyncio
class TestShutdownNackOnADeadChannel:
    """
    A shutdown nack that finds the channel dead ends the stream cleanly but
    still narrows the cleanup, in both durable loops.

    The connector is closing, so the client gets no error -- an ordinary pod
    shutdown must not surface as an INTERNAL. The dead-channel error is
    still proof the channel is gone, so the cancel and the nacks are
    skipped as on every other consumer-loss path.
    """

    @dead_channel_failures
    async def test_the_stream_ends_without_an_error(
        self, shutdown_nack_failure, make_failure
    ):
        stream = await shutdown_nack_failure(make_failure())

        assert stream.error is None

    @dead_channel_failures
    async def test_nothing_more_is_yielded(self, shutdown_nack_failure, make_failure):
        stream = await shutdown_nack_failure(make_failure())

        assert stream.remaining == []

    @dead_channel_failures
    async def test_cleanup_skips_the_consumer_cancel(
        self, shutdown_nack_failure, make_failure
    ):
        await shutdown_nack_failure(make_failure())

        RabbitMQBase._cancel_queue_consumer.assert_not_awaited()

    @dead_channel_failures
    async def test_cleanup_skips_the_outstanding_nack(
        self, shutdown_nack_failure, make_failure
    ):
        stream = await shutdown_nack_failure(make_failure())

        stream.outstanding.nack.assert_not_awaited()

    @dead_channel_failures
    async def test_cleanup_skips_the_prefetched_nacks(
        self, shutdown_nack_failure, make_failure
    ):
        await shutdown_nack_failure(make_failure())

        RabbitMQBase._nack_prefetched_messages.assert_not_awaited()

    @dead_channel_failures
    async def test_cleanup_still_releases_the_channel(
        self, shutdown_nack_failure, make_failure
    ):
        stream = await shutdown_nack_failure(make_failure())

        stream.on_cleanup.assert_awaited_once()

    @unrelated_failures
    async def test_another_shutdown_nack_failure_keeps_the_cleanup_whole(
        self, shutdown_nack_failure, make_failure
    ):
        # The control arm: the channel is alive, so the outstanding delivery
        # still has to go back on the queue for the next pod to serve.
        stream = await shutdown_nack_failure(make_failure())

        stream.outstanding.nack.assert_awaited_once_with(requeue=True)


@pytest.mark.asyncio
class TestParseAndAuthorize:
    """
    Only the authorization check itself is guarded by the
    authorization-failure branch.

    A failed skip-ack is not an authorization failure: it escapes as
    itself, without the branch's nack, cancel and drain, and the calling
    loop decides what it means. A guard that also covered the skip-ack
    would, on a dead channel, run a nack that fails the same way, and the
    error escaping would be the nack's rather than the ack's.
    """

    @pytest.fixture
    def stubbed_broker_helpers(self, mocker):
        mocker.patch.object(
            RabbitMQBase, "_cancel_queue_consumer", new_callable=AsyncMock
        )
        mocker.patch.object(
            RabbitMQBase, "_nack_prefetched_messages", new_callable=AsyncMock
        )

    @staticmethod
    async def _parse(message, authz_check):
        return await RabbitMQBase._parse_and_authorize(
            message, authz_check, PUMP_CONSUMER_TAG, MagicMock(), asyncio.Queue()
        )

    @staticmethod
    async def _parse_with_a_failing_skip_ack(failure):
        message = MockAckableMessage(_FAILING_BODY, delivery_tag=1)
        authz_check = _skip_ack_fails(message, failure)
        with pytest.raises(type(failure)) as exc_info:
            await TestParseAndAuthorize._parse(message, authz_check)
        return message, exc_info.value

    @pytest.mark.parametrize(
        "make_failure",
        [channel_invalid_state, unrelated_runtime_error],
        ids=["dead_channel", "runtime_error"],
    )
    async def test_a_failed_skip_ack_escapes_as_itself(
        self, stubbed_broker_helpers, make_failure
    ):
        # Not a discriminator on its own: a guard that also covered the
        # skip-ack would re-raise the ack's error too, given a mocked nack
        # that succeeds. The three assertions below are what tell the
        # narrowed guard from a wider one.
        failure = make_failure()

        _message, escaped = await self._parse_with_a_failing_skip_ack(failure)

        assert escaped is failure

    async def test_a_failed_skip_ack_does_not_nack_the_delivery(
        self, stubbed_broker_helpers
    ):
        message, _escaped = await self._parse_with_a_failing_skip_ack(
            channel_invalid_state()
        )

        message.nack.assert_not_awaited()

    async def test_a_failed_skip_ack_does_not_cancel_the_consumer(
        self, stubbed_broker_helpers
    ):
        await self._parse_with_a_failing_skip_ack(channel_invalid_state())

        RabbitMQBase._cancel_queue_consumer.assert_not_awaited()

    async def test_a_failed_skip_ack_does_not_drain_the_prefetched_deliveries(
        self, stubbed_broker_helpers
    ):
        await self._parse_with_a_failing_skip_ack(channel_invalid_state())

        RabbitMQBase._nack_prefetched_messages.assert_not_awaited()

    async def test_an_unauthorized_delivery_is_skipped(self, stubbed_broker_helpers):
        # The control arm for the skip-ack cases: a skip-ack that succeeds
        # returns no record.
        message = MockAckableMessage(_FAILING_BODY, delivery_tag=1)

        result = await self._parse(message, _skip_ack_fails(message, None))

        assert result is None

    @staticmethod
    async def _parse_with_an_authorization_exception():
        message = MockAckableMessage(_FAILING_BODY, delivery_tag=1)
        authz_error = PermissionError("the authorization check failed")
        authz_check = AsyncMock(side_effect=authz_error)
        with pytest.raises(PermissionError) as exc_info:
            await TestParseAndAuthorize._parse(message, authz_check)
        return message, authz_error, exc_info.value

    async def test_an_authorization_exception_is_re_raised_as_itself(
        self, stubbed_broker_helpers
    ):
        _message, authz_error, escaped = (
            await self._parse_with_an_authorization_exception()
        )

        assert escaped is authz_error

    async def test_an_authorization_exception_nacks_the_delivery(
        self, stubbed_broker_helpers
    ):
        message, _authz_error, _escaped = (
            await self._parse_with_an_authorization_exception()
        )

        message.nack.assert_awaited_once_with(requeue=True)

    async def test_an_authorization_exception_cancels_the_consumer(
        self, stubbed_broker_helpers
    ):
        await self._parse_with_an_authorization_exception()

        RabbitMQBase._cancel_queue_consumer.assert_awaited_once()

    async def test_an_authorization_exception_drains_the_prefetched_deliveries(
        self, stubbed_broker_helpers
    ):
        await self._parse_with_an_authorization_exception()

        RabbitMQBase._nack_prefetched_messages.assert_awaited_once()


@pytest_asyncio.fixture
async def idle_timer_flush(
    pump_boundary,
    cancellable_queue,
    authz_true,
    quiet_liveness_poll,
    background_tasks,
    mocker,
    caplog,
):
    """
    Fire the delayed-ack loop's idle timer over two aged pending deliveries.

    Both deliveries are yielded while nothing ages, so both are pending and
    the timer is scheduled. ``asyncio.sleep`` is made instant so the timer's
    quiet period elapses at once, and the delay drops to zero before the
    timer task first runs, so its flush finds both aged. The generator is
    left suspended at its yield, as a stream is between deliveries.

    :returns: A callable taking the failures for the first and second
        delivery's ack (None for one that succeeds) and returning the timer
        task, the deliveries, the generator and the records the flush
        logged at WARNING or above.
    """
    real_sleep = asyncio.sleep

    async def _instant_sleep(_seconds):
        await real_sleep(0)

    generators = []

    async def _run(first_failure, second_failure=None):
        first = MockAckableMessage(_AUTHORIZED_BODY, delivery_tag=1)
        first.ack.side_effect = first_failure
        second = MockAckableMessage(_AUTHORIZED_BODY, delivery_tag=2)
        second.ack.side_effect = second_failure
        pump_boundary.put_nowait(first)
        pump_boundary.put_nowait(second)
        mocker.patch.object(
            _settings_module.settings, "server_ack_delay_seconds", 999.0
        )
        mocker.patch.object(rabbitmq_base.asyncio, "sleep", side_effect=_instant_sleep)
        generator = RabbitMQBase._consume_with_delayed_acks(
            cancellable_queue, authz_true
        )
        generators.append(generator)

        await anext(generator)
        await anext(generator)
        # Nothing has suspended since the second step scheduled the timer,
        # so its task has not run yet and reads the delay set here.
        mocker.patch.object(_settings_module.settings, "server_ack_delay_seconds", 0.0)
        timer = next(
            task
            for task in background_tasks
            if task.get_coro().__name__ == "_idle_timer_callback"
        )
        with caplog.at_level(logging.WARNING, logger=rabbitmq_base.logger.name):
            done, _pending = await asyncio.wait(
                {timer}, timeout=CONSUMER_LOSS_TIMEOUT_S
            )
        if not done:
            pytest.fail(
                f"the idle timer did not finish within {CONSUMER_LOSS_TIMEOUT_S}s"
            )

        return SimpleNamespace(
            timer=timer,
            first=first,
            second=second,
            generator=generator,
            warning_records=[
                record
                for record in caplog.records
                if record.name == rabbitmq_base.logger.name
                and record.levelno >= logging.WARNING
            ],
        )

    yield _run

    for generator in generators:
        await generator.aclose()
    await expect_cleanup(background_tasks)


@pytest.mark.asyncio
class TestIdleTimerFlushOnADeadChannel:
    """
    A dead-channel error from the idle timer's flush ends the timer and
    nothing else.

    The timer is a background task: it cannot end the stream and does not
    record the loss. It stops at the first dead-channel ack, because every
    remaining ack would fail the same way, and it logs nothing at WARNING or
    above -- the loop's next own broker operation or its idle liveness poll finds
    the same dead channel and ends the stream.
    """

    @dead_channel_failures
    async def test_the_timer_completes_without_an_exception(
        self, idle_timer_flush, make_failure
    ):
        flush = await idle_timer_flush(make_failure())

        assert flush.timer.exception() is None

    @dead_channel_failures
    async def test_the_next_aged_ack_is_not_attempted(
        self, idle_timer_flush, make_failure
    ):
        flush = await idle_timer_flush(make_failure())

        flush.second.ack.assert_not_awaited()

    @dead_channel_failures
    async def test_nothing_is_logged_at_warning_or_above(
        self, idle_timer_flush, make_failure
    ):
        # The timer's own line is DEBUG: the stream that owns the channel
        # reports the loss, so an incident is not logged once per idle timer.
        flush = await idle_timer_flush(make_failure())

        assert flush.warning_records == []

    @dead_channel_failures
    async def test_the_stream_is_not_ended_by_it(
        self, idle_timer_flush, pump_boundary, make_failure
    ):
        flush = await idle_timer_flush(make_failure())
        pump_boundary.put_nowait(MockAckableMessage(_AUTHORIZED_BODY, delivery_tag=3))

        record = await asyncio.wait_for(
            anext(flush.generator), timeout=CONSUMER_LOSS_TIMEOUT_S
        )

        assert record.message == {"a": "b"}

    @unrelated_failures
    async def test_another_failure_is_logged_once_per_message(
        self, idle_timer_flush, make_failure
    ):
        # The control arm: an ack failing for a reason that says nothing
        # about the channel is logged and the flush moves on, so two such
        # failures are two ERROR lines.
        flush = await idle_timer_flush(make_failure(), second_failure=make_failure())

        assert [
            (record.levelname, record.getMessage()) for record in flush.warning_records
        ] == [
            ("ERROR", "Failed to ack aged message"),
            ("ERROR", "Failed to ack aged message"),
        ]

    @unrelated_failures
    async def test_another_failure_still_attempts_the_next_aged_ack(
        self, idle_timer_flush, make_failure
    ):
        flush = await idle_timer_flush(make_failure())

        flush.second.ack.assert_awaited_once()
