# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
import uuid
from unittest.mock import call, AsyncMock

import aio_pika
import pytest
import pytest_asyncio
from notification_common.auth import principal_id as principal_id_module
from notification_common.errors.errors import AuthenticationError, UnprocessableError

from app.config import settings as _settings_module
from app.lib.models.event_message import EventMessage

from app.lib.permissions import permissions
from app.lib.rabbitmq import DurableQueueConsumer, rabbitmq_base
from app.lib.rabbitmq import durable_queue_consumer as dqc_module
from app.lib.rabbitmq.ack_tracker import AckTracker
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase
from tests.fixtures.queue_id_bounds import (
    MALFORMED_QUEUE_IDS,
    MALFORMED_QUEUE_ID_IDS,
    MAX_QUEUE_ID_LENGTH,
)

#: The real principal resolver, captured at import time. The suite's autouse
#: ``mock_principal_id`` fixture replaces the module attribute for every
#: test, and the authentication-disabled case below needs the genuine
#: "authentication off -> None" behaviour rather than a stand-in for it.
_REAL_PRINCIPAL_ID = principal_id_module.principal_id


@pytest.fixture
def mock_temp_chan(mocker):
    """Create a separate mock channel for temporary channel existence checks."""
    temp_chan = mocker.MagicMock()
    temp_chan.declare_queue = mocker.AsyncMock()
    return temp_chan


@pytest.fixture
def mock_channel_context(mocker, mock_chan, mock_temp_chan):
    """
    Fixture to mock both get_channel and get_temporary_channel context managers.

    The get_queue method first checks queue existence with a temporary channel,
    then uses the pool channel. Both need to be mocked for tests that call
    methods using get_queue.
    """
    # Mock the pool channel context manager. autospec pins the mock to the
    # connector's real get_channel signature, so a production call with a
    # misspelled pool kwarg (durable=True) raises TypeError instead of
    # silently satisfying the durable_pool=True assertions below.
    mock_pool_context = mocker.AsyncMock()
    mock_pool_context.__aenter__.return_value = mock_chan
    mock_get_channel = mocker.patch.object(
        DurableQueueConsumer.connector, "get_channel", autospec=True
    )
    mock_get_channel.return_value = mock_pool_context

    # Mock the temporary channel context manager
    mock_temp_context = mocker.AsyncMock()
    mock_temp_context.__aenter__.return_value = mock_temp_chan
    mocker.patch.object(
        DurableQueueConsumer.connector,
        "get_temporary_channel",
        return_value=mock_temp_context,
    )
    return mock_chan


@pytest.fixture
def consumer():
    return DurableQueueConsumer


@pytest.mark.asyncio
class TestCreatQueue:
    @pytest_asyncio.fixture(autouse=True)
    async def connect(self, consumer):
        await consumer.connector.connect()

    async def test_raises_error_if_not_allowed_to_create_durable_queues(
        self, consumer, mocker
    ):
        mocker.patch.object(
            permissions,
            "raise_if_cant_create_durable_queue",
            side_effect=PermissionError("pe"),
        )

        with pytest.raises(PermissionError, match="pe"):
            await consumer.create_queue([])

    async def test_acquires_from_the_durable_pool(
        self, consumer, filter_groups, mock_channel_context, mocker
    ):
        mocker.patch.object(consumer.connector, "get_exchange")

        await consumer.create_queue(filter_groups)

        # noinspection PyUnresolvedReferences
        DurableQueueConsumer.connector.get_channel.assert_called_once_with(
            durable_pool=True
        )

    async def test_existence_probe_takes_no_pool_checkout(
        self,
        mock_chan,
        mock_temp_chan,
        filter_groups,
        consumer,
        mock_channel_context,
        mock_queue_id,
        mocker,
    ):
        # The passive existence probe runs on a temporary channel that
        # bypasses both pools by design. A refactor that moved it onto a
        # pool would double the durable pool's checkouts per create.
        mock_get_channel_direct = mocker.patch.object(
            consumer.connector, "get_channel_direct", autospec=True
        )

        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)

        mock_get_channel_direct.assert_not_called()

    async def test_declares_queue_with_id_in_name(
        self,
        mock_chan,
        consumer,
        mock_queue_name,
        filter_groups,
        mock_channel_context,
        mocker,
    ):
        mocker.patch.object(uuid, "uuid4", return_value="test-queue-id")
        # Mock get_exchange so the connector's dead-letter topology
        # declare does not land on mock_chan and break the single-call
        # assertion on the durable queue declaration below.
        mocker.patch.object(consumer.connector, "get_exchange")
        await consumer.create_queue(filter_groups)
        mock_chan.declare_queue.assert_called_once_with(
            mock_queue_name,
            durable=True,
            arguments={
                "x-queue-type": "quorum",
                "x-dead-letter-exchange": (
                    _settings_module.settings.dead_letter_exchange_name
                ),
                "x-dead-letter-routing-key": (
                    _settings_module.settings.dead_letter_routing_key
                ),
            },
            robust=False,
        )

    async def test_gets_exchange(
        self, mock_chan, filter_groups, consumer, mock_channel_context, mocker
    ):
        mock_get_exchange = mocker.patch.object(consumer.connector, "get_exchange")

        await consumer.create_queue(filter_groups)
        mock_get_exchange.assert_called_once_with(mock_chan)

    async def test_gets_binding_keys(
        self,
        mock_chan,
        mock_queue,
        consumer,
        filter_groups,
        mock_channel_context,
        mocker,
    ):
        spy = mocker.patch.object(rabbitmq_base, "binding_keys_from_filters")
        groups = filter_groups

        await consumer.create_queue(groups)
        spy.assert_called_once_with(groups)

    async def test_binds_queues(
        self,
        mock_chan,
        mock_queue,
        consumer,
        filter_groups,
        mock_channel_context,
        mocker,
    ):
        mocker.patch.object(
            rabbitmq_base, "binding_keys_from_filters", return_value=["e1", "e2"]
        )
        mock_chan.declare_queue.return_value = mock_queue
        mocker.patch.object(
            consumer.connector, "get_exchange", return_value="test-exchange"
        )
        await consumer.create_queue(filter_groups)

        mock_queue.bind.assert_has_calls(
            [call("test-exchange", "e1"), call("test-exchange", "e2")]
        )

    async def test_returns_queue_id(
        self, mock_chan, mocker, filter_groups, consumer, mock_channel_context
    ):
        mocker.patch.object(uuid, "uuid4", return_value="test-id")
        queue_id, _ = await consumer.create_queue(filter_groups)
        assert queue_id == "test-id"

    async def test_no_id_path_reports_created_true(
        self, mock_chan, mocker, filter_groups, consumer, mock_channel_context
    ):
        # Backwards-compat: with no supplied queue_id the server mints a
        # uuid4 and always declares a fresh queue, so created is True.
        mocker.patch.object(uuid, "uuid4", return_value="test-id")
        _, created = await consumer.create_queue(filter_groups)
        assert created is True

    async def test_no_id_path_does_not_run_existence_probe(
        self, mock_chan, mocker, filter_groups, consumer, mock_channel_context
    ):
        # The passive existence probe is only for the ensure-exists
        # (supplied-id) path; the no-id path never opens a temporary
        # channel.
        mocker.patch.object(uuid, "uuid4", return_value="test-id")
        await consumer.create_queue(filter_groups)
        # noinspection PyUnresolvedReferences
        DurableQueueConsumer.connector.get_temporary_channel.assert_not_called()

    async def test_supplied_new_id_returns_that_id(
        self,
        mock_chan,
        mock_temp_chan,
        mocker,
        filter_groups,
        consumer,
        mock_channel_context,
        mock_queue_id,
    ):
        # Probe misses (ChannelClosed) -> queue is created and the
        # supplied id is returned unchanged.
        mock_temp_chan.declare_queue.side_effect = aio_pika.exceptions.ChannelClosed(
            0, "not found"
        )
        mocker.patch.object(consumer.connector, "get_exchange")
        queue_id, _ = await consumer.create_queue(filter_groups, queue_id=mock_queue_id)
        assert queue_id == mock_queue_id

    async def test_supplied_new_id_reports_created_true(
        self,
        mock_chan,
        mock_temp_chan,
        mocker,
        filter_groups,
        consumer,
        mock_channel_context,
        mock_queue_id,
    ):
        mock_temp_chan.declare_queue.side_effect = aio_pika.exceptions.ChannelClosed(
            0, "not found"
        )
        mocker.patch.object(consumer.connector, "get_exchange")
        _, created = await consumer.create_queue(filter_groups, queue_id=mock_queue_id)
        assert created is True

    async def test_supplied_new_id_declares_queue_with_id_in_name(
        self,
        mock_chan,
        mock_temp_chan,
        mocker,
        filter_groups,
        consumer,
        mock_channel_context,
        mock_queue_id,
        mock_queue_name,
    ):
        mock_temp_chan.declare_queue.side_effect = aio_pika.exceptions.ChannelClosed(
            0, "not found"
        )
        mocker.patch.object(consumer.connector, "get_exchange")
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)
        mock_chan.declare_queue.assert_called_once_with(
            mock_queue_name,
            durable=True,
            arguments={
                "x-queue-type": "quorum",
                "x-dead-letter-exchange": (
                    _settings_module.settings.dead_letter_exchange_name
                ),
                "x-dead-letter-routing-key": (
                    _settings_module.settings.dead_letter_routing_key
                ),
            },
            robust=False,
        )

    async def test_supplied_new_id_binds_queue(
        self,
        mock_chan,
        mock_temp_chan,
        mock_queue,
        mocker,
        filter_groups,
        consumer,
        mock_channel_context,
        mock_queue_id,
    ):
        mock_temp_chan.declare_queue.side_effect = aio_pika.exceptions.ChannelClosed(
            0, "not found"
        )
        mocker.patch.object(
            rabbitmq_base, "binding_keys_from_filters", return_value=["e1", "e2"]
        )
        mock_chan.declare_queue.return_value = mock_queue
        mocker.patch.object(
            consumer.connector, "get_exchange", return_value="test-exchange"
        )
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)
        mock_queue.bind.assert_has_calls(
            [call("test-exchange", "e1"), call("test-exchange", "e2")]
        )

    async def test_supplied_new_id_does_not_use_assert_queue_exists(
        self,
        mock_chan,
        mock_temp_chan,
        mocker,
        filter_groups,
        consumer,
        mock_channel_context,
        mock_queue_id,
    ):
        # The probe is a single passive declare on a temporary channel,
        # NOT the retrying _assert_queue_exists helper. Using the helper
        # would incur a multi-second retry/sleep on every create-new
        # call (SR-4.2a).
        mock_temp_chan.declare_queue.side_effect = aio_pika.exceptions.ChannelClosed(
            0, "not found"
        )
        mocker.patch.object(consumer.connector, "get_exchange")
        spy = mocker.patch.object(RabbitMQBase, "_assert_queue_exists")
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)
        spy.assert_not_called()

    async def test_existing_id_returns_that_id(
        self,
        mock_chan,
        mock_temp_chan,
        filter_groups,
        consumer,
        mock_channel_context,
        mock_queue_id,
    ):
        # Probe succeeds (passive declare returns) -> the queue already
        # exists and the supplied id is returned unchanged.
        queue_id, _ = await consumer.create_queue(filter_groups, queue_id=mock_queue_id)
        assert queue_id == mock_queue_id

    async def test_existing_id_reports_created_false(
        self,
        mock_chan,
        mock_temp_chan,
        filter_groups,
        consumer,
        mock_channel_context,
        mock_queue_id,
    ):
        _, created = await consumer.create_queue(filter_groups, queue_id=mock_queue_id)
        assert created is False

    async def test_existing_id_does_not_declare_on_pool_channel(
        self,
        mock_chan,
        mock_temp_chan,
        filter_groups,
        consumer,
        mock_channel_context,
        mock_queue_id,
    ):
        # The already-exists path must not re-declare the queue on the
        # pool channel (the probe succeeded).
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)
        mock_chan.declare_queue.assert_not_called()

    async def test_existing_id_does_not_bind(
        self,
        mock_chan,
        mock_temp_chan,
        mock_queue,
        filter_groups,
        consumer,
        mock_channel_context,
        mock_queue_id,
    ):
        # Bindings are left untouched on the already-exists path.
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)
        mock_queue.bind.assert_not_called()

    async def test_existing_id_ignores_empty_filter_groups(
        self,
        mock_chan,
        mock_temp_chan,
        consumer,
        mock_channel_context,
        mock_queue_id,
    ):
        # On the already-exists path filter_groups are ignored: the
        # empty-filters guard (which would otherwise raise) is never
        # reached because the queue is returned before _create_queue.
        queue_id, created = await consumer.create_queue([], queue_id=mock_queue_id)
        assert (queue_id, created) == (mock_queue_id, False)

    async def test_supplied_new_id_still_requires_filter_groups(
        self,
        mock_chan,
        mock_temp_chan,
        mocker,
        consumer,
        mock_channel_context,
        mock_queue_id,
    ):
        # On the create path (probe misses) empty filter_groups still
        # trips the empty-filters guard.
        mock_temp_chan.declare_queue.side_effect = aio_pika.exceptions.ChannelClosed(
            0, "not found"
        )
        mocker.patch.object(consumer.connector, "get_exchange")
        with pytest.raises(UnprocessableError, match="without any filters"):
            await consumer.create_queue([], queue_id=mock_queue_id)


@pytest.mark.asyncio
class TestCreateQueueValidatesTheSuppliedId:
    """
    The ``QUEUE_ID_PATTERN`` bound is enforced by core create, not only by
    the surfaces in front of it.

    An id reaching here has already passed the gRPC or REST boundary check,
    so a rejection at this layer means one of those was bypassed. Keeping
    the check here too is what makes "no over-long id ever becomes a broker
    name" a property of the code rather than a claim about an enumerated
    list of call sites -- a new surface cannot quietly fall off it.
    """

    @pytest_asyncio.fixture(autouse=True)
    async def connect(self, consumer):
        await consumer.connector.connect()

    @pytest.fixture
    def raise_if_cant_create(self, mocker):
        return mocker.patch.object(permissions, "raise_if_cant_create_durable_queue")

    @pytest.mark.parametrize(
        "invalid_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_rejects_a_malformed_supplied_id(
        self, invalid_id, filter_groups, consumer, mock_channel_context
    ):
        with pytest.raises(UnprocessableError, match="Invalid queue_id"):
            await consumer.create_queue(filter_groups, queue_id=invalid_id)

    @pytest.mark.parametrize(
        "invalid_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_rejects_before_the_permissions_round_trip(
        self,
        invalid_id,
        raise_if_cant_create,
        filter_groups,
        consumer,
        mock_channel_context,
    ):
        with pytest.raises(UnprocessableError):
            await consumer.create_queue(filter_groups, queue_id=invalid_id)

        raise_if_cant_create.assert_not_called()

    @pytest.mark.parametrize(
        "invalid_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_rejects_before_any_broker_work(
        self,
        invalid_id,
        filter_groups,
        consumer,
        mock_channel_context,
        mock_chan,
        mock_temp_chan,
    ):
        # Neither the existence probe's temporary channel nor the pool
        # channel's declare may be reached: an over-long id that got that
        # far would die building the AMQP frame instead, as a bare
        # ValueError naming neither the field nor the rule it broke.
        with pytest.raises(UnprocessableError):
            await consumer.create_queue(filter_groups, queue_id=invalid_id)

        mock_temp_chan.declare_queue.assert_not_called()
        mock_chan.declare_queue.assert_not_called()

    async def test_accepts_an_id_at_the_length_bound(
        self,
        mocker,
        filter_groups,
        consumer,
        mock_channel_context,
        mock_chan,
        mock_temp_chan,
    ):
        mock_temp_chan.declare_queue.side_effect = aio_pika.exceptions.ChannelClosed(
            0, "not found"
        )
        mocker.patch.object(consumer.connector, "get_exchange")

        queue_id, created = await consumer.create_queue(
            filter_groups, queue_id="a" * MAX_QUEUE_ID_LENGTH
        )

        assert (queue_id, created) == ("a" * MAX_QUEUE_ID_LENGTH, True)

    async def test_does_not_validate_the_server_minted_id(
        self, mocker, filter_groups, consumer, mock_channel_context
    ):
        # The no-id path mints a uuid4 itself, so there is nothing
        # client-supplied to reject; the check must not fire on it.
        mocker.patch.object(consumer.connector, "get_exchange")

        _, created = await consumer.create_queue(filter_groups)

        assert created is True


@pytest.mark.asyncio
class TestResolvePrincipal:
    """
    Naming the caller, whose principal the create log line reports.

    ``create_queue`` calls this before the permissions round trip and before
    any broker work, so the one thing that can fail here -- producing that
    name -- fails as a pre-flight 401 with no side effects rather than after
    the queue has been declared and bound.
    """

    @pytest.fixture
    def authentication_on(self, authentication_enabled):
        authentication_enabled(True)

    @pytest.fixture
    def authentication_off(self, authentication_enabled):
        authentication_enabled(False)

    async def test_returns_the_principal_id_when_authentication_is_on(
        self, consumer, mock_principal, authentication_on
    ):
        assert await consumer._resolve_principal() == mock_principal

    async def test_reports_authn_disabled_when_authentication_is_off(
        self, consumer, authentication_off
    ):
        assert await consumer._resolve_principal() == "authn-disabled"

    async def test_does_not_resolve_a_principal_when_authentication_is_off(
        self, mocker, consumer, authentication_off
    ):
        resolve = mocker.patch.object(principal_id_module, "principal_id")

        await consumer._resolve_principal()

        resolve.assert_not_called()

    async def test_reports_authn_disabled_when_the_resolver_returns_none(
        self, mocker, consumer, authentication_on
    ):
        # A resolver that answers None would otherwise render as an empty
        # field an operator reads as a bug.
        mocker.patch.object(principal_id_module, "principal_id", return_value=None)

        assert await consumer._resolve_principal() == "authn-disabled"

    async def test_propagates_an_authentication_error(
        self, mocker, consumer, authentication_on
    ):
        mocker.patch.object(
            principal_id_module,
            "principal_id",
            side_effect=AuthenticationError("token expired"),
        )

        with pytest.raises(AuthenticationError):
            await consumer._resolve_principal()


@pytest.mark.asyncio
class TestCreateQueueResolvesThePrincipalFirst:
    """
    The principal is resolved before anything with a side effect.

    That resolution can still raise -- a cache entry whose stored lifetime
    has lapsed reports as an authentication failure -- and a 401 raised
    after the declare-and-bind would fail a create that had already
    happened, leaving a queue nobody was told about.
    """

    @pytest_asyncio.fixture(autouse=True)
    async def connect(self, consumer):
        await consumer.connector.connect()

    @pytest.fixture
    def lapsed_token(self, mocker, authentication_enabled):
        authentication_enabled(True)
        mocker.patch.object(
            principal_id_module,
            "principal_id",
            side_effect=AuthenticationError("token expired"),
        )

    async def test_a_lapsed_token_fails_the_create(
        self, consumer, filter_groups, lapsed_token, mock_channel_context
    ):
        with pytest.raises(AuthenticationError):
            await consumer.create_queue(filter_groups)

    async def test_a_lapsed_token_skips_the_permissions_round_trip(
        self, mocker, consumer, filter_groups, lapsed_token, mock_channel_context
    ):
        raise_if_cant_create = mocker.patch.object(
            permissions, "raise_if_cant_create_durable_queue"
        )

        with pytest.raises(AuthenticationError):
            await consumer.create_queue(filter_groups)

        raise_if_cant_create.assert_not_called()

    async def test_a_lapsed_token_does_no_broker_work(
        self,
        consumer,
        filter_groups,
        lapsed_token,
        mock_channel_context,
        mock_chan,
        mock_temp_chan,
    ):
        with pytest.raises(AuthenticationError):
            await consumer.create_queue(filter_groups)

        mock_temp_chan.declare_queue.assert_not_called()
        mock_chan.declare_queue.assert_not_called()

    async def test_the_resolved_principal_is_what_the_audit_line_is_given(
        self,
        mocker,
        consumer,
        filter_groups,
        mock_principal,
        mock_channel_context,
        mock_queue,
        mock_chan,
        authentication_enabled,
    ):
        # The line must report the principal resolved up front, not one
        # re-read after the work it describes.
        authentication_enabled(True)
        mocker.patch.object(consumer.connector, "get_exchange")
        mock_chan.declare_queue.return_value = mock_queue
        log_queue_created = mocker.patch.object(consumer, "_log_queue_created")

        await consumer.create_queue(filter_groups)

        assert log_queue_created.call_args.args[2] == mock_principal


@pytest.mark.asyncio
class TestGetQueue:
    """
    The one door every operation on an EXISTING durable queue goes through.

    Consume, consume-with-acks, update, and delete all resolve their queue
    here, so validating the id here makes the bound a property of the code
    rather than a claim about an enumerated list of call sites.
    """

    @pytest_asyncio.fixture(autouse=True)
    async def connect(self, consumer):
        await consumer.connector.connect()

    @pytest.mark.parametrize(
        "invalid_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_rejects_a_malformed_id(
        self, invalid_id, consumer, mock_chan, mock_temp_chan_for_queue
    ):
        with pytest.raises(UnprocessableError, match="Invalid queue_id"):
            await consumer.get_queue(mock_chan, invalid_id)

    @pytest.mark.parametrize(
        "invalid_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_does_not_reach_the_base_lookup_for_a_malformed_id(
        self, invalid_id, mocker, consumer, mock_chan, mock_temp_chan_for_queue
    ):
        base_get_queue = mocker.patch.object(
            RabbitMQBase, "get_queue", new_callable=mocker.AsyncMock
        )

        with pytest.raises(UnprocessableError):
            await consumer.get_queue(mock_chan, invalid_id)

        base_get_queue.assert_not_called()

    async def test_delegates_a_valid_id_to_the_base_lookup(
        self, consumer, mock_chan, mock_queue, mock_queue_id, mock_queue_name
    ):
        result = await consumer.get_queue(mock_chan, mock_queue_id)

        assert result is mock_queue

    async def test_delegates_using_the_durable_broker_name(
        self, consumer, mock_chan, mock_queue, mock_queue_id, mock_queue_name
    ):
        await consumer.get_queue(mock_chan, mock_queue_id)

        mock_chan.declare_queue.assert_called_once_with(
            mock_queue_name, passive=True, robust=False
        )

    async def test_accepts_an_id_at_the_length_bound(
        self, consumer, mock_chan, mock_queue, mock_temp_chan_for_queue
    ):
        result = await consumer.get_queue(mock_chan, "a" * MAX_QUEUE_ID_LENGTH)

        assert result is mock_queue


#: The id a server-minted create lands on with uuid4 pinned, and the broker
#: name derived from it.
SERVER_MINTED_QUEUE_ID = "server-minted-id"
SERVER_MINTED_QUEUE_NAME = f"event-queue:{SERVER_MINTED_QUEUE_ID}"

#: The prefixes of the two audit lines under test. Matching on them keeps
#: the assertions clear of the other INFO lines a create emits, and keeps
#: the two branches' lines distinguishable from each other: exactly one of
#: them is emitted per create, and which one is the fact being pinned.
_CREATED_LOG_PREFIX = "Durable queue created:"
_ATTACHED_LOG_PREFIX = "Durable queue attached (existing):"


@pytest.mark.asyncio
class TestLogQueueCreated:
    """
    Which principal created a durable queue is recorded, once, for the
    queue this call actually declared.

    Logging is the contract here, not a side effect: a broker queue name no
    longer carries anything about its creator, so this line is the only
    place the pair is written down. An operator looking at a queue in the
    broker finds who asked for it by searching the service logs for it. It
    must NOT appear on the ensure-exists path, where the queue belongs to
    whoever created it first -- a line there would name the wrong principal
    as the creator, which is worse than no line at all.
    """

    @pytest_asyncio.fixture(autouse=True)
    async def connect(self, consumer):
        await consumer.connector.connect()

    @pytest.fixture
    def queue_created_logs(self, caplog):
        """
        Capture the module's own INFO records for one test.

        The level is raised on the module logger reached through the module
        attribute, so the record clears its originating logger regardless of
        where the root logger sits.
        """
        with caplog.at_level(logging.INFO, logger=dqc_module.logger.name):
            yield caplog

    @staticmethod
    def created_records(caplog):
        """The audit lines, isolated from every other record captured."""
        return [
            record
            for record in caplog.records
            if record.name == dqc_module.logger.name
            and record.getMessage().startswith(_CREATED_LOG_PREFIX)
        ]

    @pytest.fixture
    def server_minted_create(
        self, mocker, consumer, mock_chan, mock_queue, mock_channel_context
    ):
        """Arrange a create with no supplied id, which always declares."""
        mocker.patch.object(uuid, "uuid4", return_value=SERVER_MINTED_QUEUE_ID)
        mock_queue.name = SERVER_MINTED_QUEUE_NAME
        mock_chan.declare_queue.return_value = mock_queue
        mocker.patch.object(consumer.connector, "get_exchange")

    @pytest.fixture
    def client_supplied_create(
        self,
        mocker,
        consumer,
        mock_chan,
        mock_queue,
        mock_temp_chan,
        mock_channel_context,
        mock_queue_id,
        mock_queue_name,
    ):
        """
        Arrange a create with a supplied id the broker does not yet hold.

        The passive probe misses (``ChannelClosed``), so this call declares
        the queue itself and is therefore a genuine creation, not an
        attachment to someone else's queue.
        """
        mock_temp_chan.declare_queue.side_effect = aio_pika.exceptions.ChannelClosed(
            0, "not found"
        )
        mock_queue.name = mock_queue_name
        mock_chan.declare_queue.return_value = mock_queue
        mocker.patch.object(consumer.connector, "get_exchange")

    async def test_logs_once_on_the_server_minted_path(
        self, consumer, filter_groups, server_minted_create, queue_created_logs
    ):
        await consumer.create_queue(filter_groups)

        assert len(self.created_records(queue_created_logs)) == 1

    async def test_names_the_queue_id(
        self, consumer, filter_groups, server_minted_create, queue_created_logs
    ):
        # The client-facing id, which is what a client reports in a support
        # request and what the operator searches on.
        await consumer.create_queue(filter_groups)

        message = self.created_records(queue_created_logs)[0].getMessage()
        assert f"id={SERVER_MINTED_QUEUE_ID}" in message

    async def test_names_the_broker_queue_name(
        self, consumer, filter_groups, server_minted_create, queue_created_logs
    ):
        # The other half of the correlation: the spelling the operator sees
        # in the broker's own queue listing.
        await consumer.create_queue(filter_groups)

        message = self.created_records(queue_created_logs)[0].getMessage()
        assert f"name={SERVER_MINTED_QUEUE_NAME}" in message

    async def test_names_the_principal(
        self,
        consumer,
        filter_groups,
        mock_principal,
        server_minted_create,
        queue_created_logs,
    ):
        await consumer.create_queue(filter_groups)

        message = self.created_records(queue_created_logs)[0].getMessage()
        assert f"principal={mock_principal}" in message

    async def test_logs_once_on_the_client_supplied_path(
        self,
        consumer,
        filter_groups,
        mock_queue_id,
        client_supplied_create,
        queue_created_logs,
    ):
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)

        assert len(self.created_records(queue_created_logs)) == 1

    async def test_names_the_client_supplied_queue_id(
        self,
        consumer,
        filter_groups,
        mock_queue_id,
        mock_queue_name,
        client_supplied_create,
        queue_created_logs,
    ):
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)

        message = self.created_records(queue_created_logs)[0].getMessage()
        assert f"id={mock_queue_id} name={mock_queue_name}" in message

    async def test_does_not_log_on_the_ensure_exists_path(
        self,
        consumer,
        filter_groups,
        mock_chan,
        mock_temp_chan,
        mock_channel_context,
        mock_queue_id,
        queue_created_logs,
    ):
        # The probe succeeds, so the queue already existed and this call
        # created nothing. Naming this caller as its creator would send an
        # operator after the wrong principal.
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)

        assert self.created_records(queue_created_logs) == []

    async def test_reports_authn_disabled_when_authentication_is_disabled(
        self,
        mocker,
        consumer,
        filter_groups,
        server_minted_create,
        queue_created_logs,
        authentication_enabled,
    ):
        # There is no principal to name on a tokenless deployment, which is
        # reported as such rather than as an empty field an operator would
        # read as a bug. The real resolver is put back so the None comes
        # from the authentication gate itself.
        mocker.patch.object(principal_id_module, "principal_id", _REAL_PRINCIPAL_ID)
        authentication_enabled(False)

        await consumer.create_queue(filter_groups)

        message = self.created_records(queue_created_logs)[0].getMessage()
        assert "principal=authn-disabled" in message


@pytest.mark.asyncio
class TestLogQueueAttached:
    """
    A create that attached to somebody else's queue says so.

    Queue ids are a deployment-wide namespace and ensure-exists is silent
    about it: the caller gets ``created: false`` and a working queue whether
    it is re-creating its own queue after a restart or has collided with
    another client's id. Only the service sees both sides, so this line is
    where an operator finds out that a queue has more than one principal
    creating against it. Logging is the contract here, not a side effect --
    it is the continuation, for every caller after the first, of the pairing
    ``_log_queue_created`` records for the first.

    It must NOT be confusable with the create line: a queue this call did
    not declare, reported as created, sends an operator after the wrong
    principal.
    """

    @pytest_asyncio.fixture(autouse=True)
    async def connect(self, consumer):
        await consumer.connector.connect()

    @pytest.fixture
    def attach_logs(self, caplog):
        """
        Capture the module's own INFO records for one test.

        The level is raised on the module logger reached through the module
        attribute, so the record clears its originating logger regardless of
        where the root logger sits.
        """
        with caplog.at_level(logging.INFO, logger=dqc_module.logger.name):
            yield caplog

    @staticmethod
    def records_with_prefix(caplog, prefix):
        return [
            record
            for record in caplog.records
            if record.name == dqc_module.logger.name
            and record.getMessage().startswith(prefix)
        ]

    @pytest.fixture
    def attaching_create(
        self, mocker, consumer, mock_chan, mock_temp_chan, mock_channel_context
    ):
        """
        Arrange a create whose passive probe HITS.

        The probe succeeding is the whole ensure-exists branch: the queue
        already existed, so this call declares and binds nothing and returns
        ``created=False``.
        """
        mocker.patch.object(consumer.connector, "get_exchange")

    @pytest.fixture
    def declaring_create(
        self,
        mocker,
        consumer,
        mock_chan,
        mock_queue,
        mock_temp_chan,
        mock_channel_context,
        mock_queue_name,
    ):
        """Arrange a create whose passive probe MISSES, so it declares."""
        mock_temp_chan.declare_queue.side_effect = aio_pika.exceptions.ChannelClosed(
            0, "not found"
        )
        mock_queue.name = mock_queue_name
        mock_chan.declare_queue.return_value = mock_queue
        mocker.patch.object(consumer.connector, "get_exchange")

    async def test_the_attach_path_reports_it_created_nothing(
        self, consumer, filter_groups, mock_queue_id, attaching_create
    ):
        # Guards every assertion below: they are all about the branch this
        # return value identifies.
        _, created = await consumer.create_queue(filter_groups, queue_id=mock_queue_id)

        assert created is False

    async def test_logs_once_on_the_attach_path(
        self, consumer, filter_groups, mock_queue_id, attaching_create, attach_logs
    ):
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)

        assert len(self.records_with_prefix(attach_logs, _ATTACHED_LOG_PREFIX)) == 1

    async def test_names_the_queue_id(
        self, consumer, filter_groups, mock_queue_id, attaching_create, attach_logs
    ):
        # The client-facing id, which is the only thing the two principals
        # collided on and the only thing an operator can search for.
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)

        message = self.records_with_prefix(attach_logs, _ATTACHED_LOG_PREFIX)[
            0
        ].getMessage()
        assert f"id={mock_queue_id}" in message

    async def test_names_the_principal(
        self,
        consumer,
        filter_groups,
        mock_queue_id,
        mock_principal,
        attaching_create,
        attach_logs,
    ):
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)

        message = self.records_with_prefix(attach_logs, _ATTACHED_LOG_PREFIX)[
            0
        ].getMessage()
        assert f"principal={mock_principal}" in message

    async def test_reports_authn_disabled_when_authentication_is_disabled(
        self,
        mocker,
        consumer,
        filter_groups,
        mock_queue_id,
        attaching_create,
        attach_logs,
        authentication_enabled,
    ):
        # Same rendering as the create line: there is no principal to name
        # on a tokenless deployment, reported as such rather than as an
        # empty field an operator would read as a bug. The real resolver is
        # put back so the None comes from the authentication gate itself.
        mocker.patch.object(principal_id_module, "principal_id", _REAL_PRINCIPAL_ID)
        authentication_enabled(False)

        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)

        message = self.records_with_prefix(attach_logs, _ATTACHED_LOG_PREFIX)[
            0
        ].getMessage()
        assert "principal=authn-disabled" in message

    async def test_the_attach_path_does_not_emit_the_created_line(
        self, consumer, filter_groups, mock_queue_id, attaching_create, attach_logs
    ):
        # The two lines are alternatives, not a pair: an operator reading
        # "created" for a queue this caller merely attached to would go
        # looking for the wrong principal.
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)

        assert self.records_with_prefix(attach_logs, _CREATED_LOG_PREFIX) == []

    async def test_the_declaring_path_does_not_emit_the_attached_line(
        self, consumer, filter_groups, mock_queue_id, declaring_create, attach_logs
    ):
        # The other half. This caller IS the creator, and saying it
        # attached to a queue somebody else made is the same error the
        # other way round.
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)

        assert self.records_with_prefix(attach_logs, _ATTACHED_LOG_PREFIX) == []

    async def test_the_server_minted_path_does_not_emit_the_attached_line(
        self, mocker, consumer, filter_groups, mock_chan, mock_queue, attach_logs
    ):
        # A server-minted uuid4 has nothing to collide with, and that path
        # never reaches the probe at all.
        mocker.patch.object(uuid, "uuid4", return_value=SERVER_MINTED_QUEUE_ID)
        mock_queue.name = SERVER_MINTED_QUEUE_NAME
        mock_chan.declare_queue.return_value = mock_queue
        mocker.patch.object(consumer.connector, "get_exchange")

        await consumer.create_queue(filter_groups)

        assert self.records_with_prefix(attach_logs, _ATTACHED_LOG_PREFIX) == []

    async def test_the_line_is_emitted_at_info(
        self, consumer, filter_groups, mock_queue_id, attaching_create, attach_logs
    ):
        # An operator finds this by reading ordinary service logs, so it
        # cannot sit below the level a production deployment ships at.
        await consumer.create_queue(filter_groups, queue_id=mock_queue_id)

        record = self.records_with_prefix(attach_logs, _ATTACHED_LOG_PREFIX)[0]
        assert record.levelno == logging.INFO


@pytest.mark.asyncio
class TestDeleteQueue:
    @pytest_asyncio.fixture(autouse=True)
    async def connect(self, consumer):
        await consumer.connector.connect()

    async def test_raises_error_if_not_allowed_to_delete_durable_queues(
        self, consumer, mocker
    ):
        mocker.patch.object(
            permissions,
            "raise_if_cant_delete_durable_queue",
            side_effect=PermissionError("pe"),
        )

        with pytest.raises(PermissionError, match="pe"):
            await consumer.delete_queue("")

    async def test_acquires_from_the_durable_pool(
        self, mock_chan, consumer, mock_channel_context
    ):
        await consumer.delete_queue("test-queue-id")

        # noinspection PyUnresolvedReferences
        DurableQueueConsumer.connector.get_channel.assert_called_once_with(
            durable_pool=True
        )

    async def test_gets_queue_with_name(
        self, mock_chan, consumer, mock_queue_name, mock_channel_context
    ):
        await consumer.delete_queue("test-queue-id")
        mock_chan.declare_queue.assert_called_once_with(
            mock_queue_name,
            passive=True,
            robust=False,
        )

    async def test_deletes_queue(
        self, mock_chan, mock_queue, consumer, mock_channel_context
    ):
        mock_chan.declare_queue.return_value = mock_queue

        await consumer.delete_queue("test-queue-id")
        # Regression guard: durable queues are quorum queues, which reject
        # delete with if_unused=True (aio-pika's default) -> RabbitMQ raises
        # 540 NOT_IMPLEMENTED and closes the connection. We must pass
        # if_unused=False explicitly. Dropping it (reverting to the default)
        # must fail this test.
        mock_queue.delete.assert_awaited_once_with(if_unused=False, if_empty=False)


@pytest.mark.asyncio
class TestUpdateQueue:
    async def test_raises_error_if_not_allowed_to_create_durable_queues(
        self, consumer, mocker
    ):
        mocker.patch.object(
            permissions,
            "raise_if_cant_create_durable_queue",
            side_effect=PermissionError("pe"),
        )

        with pytest.raises(PermissionError, match="pe"):
            await consumer.update_queue("", [], [])

    async def test_acquires_from_the_durable_pool(
        self, mock_chan, consumer, mock_channel_context, mocker
    ):
        mocker.patch.object(consumer, "get_queue")

        await consumer.update_queue("test-id", [], [])

        # noinspection PyUnresolvedReferences
        DurableQueueConsumer.connector.get_channel.assert_called_once_with(
            durable_pool=True
        )

    async def test_gets_queue_by_id(
        self, mock_chan, consumer, mock_channel_context, mocker
    ):
        spy = mocker.patch.object(consumer, "get_queue")
        await consumer.update_queue("test-id", [], [])
        spy.assert_called_once_with(mock_chan, "test-id")

    async def test_updates_bindings_on_the_queue(
        self, mock_chan, consumer, mock_channel_context, mocker
    ):
        queue = mocker.AsyncMock()
        mocker.patch.object(consumer, "get_queue", return_value=queue)
        spy = mocker.patch.object(consumer, "_update_bindings")

        # noinspection PyTypeChecker
        await consumer.update_queue("", ["a"], ["b"])

        spy.assert_called_once_with(mock_chan, queue, ["a"], ["b"])


@pytest.fixture
def mock_channel_direct(mocker, mock_chan, mock_temp_chan):
    """Mock get_channel_direct, return_channel, and get_temporary_channel
    for consume_with_acks tests."""
    # autospec (rather than a bare AsyncMock, which accepts any kwarg) pins
    # the mock to the connector's real get_channel_direct signature, so a
    # production call with a misspelled pool kwarg (durable=True) raises
    # TypeError instead of silently satisfying the durable_pool=True
    # assertions below.
    mock_get_channel_direct = mocker.patch.object(
        DurableQueueConsumer.connector,
        "get_channel_direct",
        autospec=True,
    )
    mock_get_channel_direct.return_value = mock_chan
    mocker.patch.object(
        DurableQueueConsumer.connector,
        "return_channel",
    )
    # Mock get_temporary_channel so get_queue -> _assert_queue_exists
    # doesn't hit the real connector (which can trigger pool channel
    # creation and a spurious set_qos call on mock_chan).
    mock_temp_context = mocker.AsyncMock()
    mock_temp_context.__aenter__.return_value = mock_temp_chan
    mocker.patch.object(
        DurableQueueConsumer.connector,
        "get_temporary_channel",
        return_value=mock_temp_context,
    )
    return mock_chan


@pytest.fixture
def mock_consume_with_acks(mocker):
    """Mock _consume_with_client_acks to yield test messages."""

    async def mock_gen(_queue, _authz_check, _ack_tracker, on_cleanup_complete=None):
        try:
            for i in range(3):
                yield EventMessage({"a": "b"}, delivery_tag=i + 1)
        finally:
            if on_cleanup_complete:
                await on_cleanup_complete()

    mocker.patch.object(
        RabbitMQBase,
        "_consume_with_client_acks",
        side_effect=mock_gen,
    )


@pytest.mark.asyncio
class TestConsumeWithAcks:

    @pytest_asyncio.fixture(autouse=True)
    async def connect_rabbitmq(self, consumer):
        await consumer.connector.connect()

    async def test_raises_error_if_not_allowed(self, consumer, mocker):
        mocker.patch.object(
            permissions,
            "raise_if_cant_consume_durable",
            side_effect=PermissionError("pe"),
        )

        with pytest.raises(PermissionError, match="pe"):
            await consumer.consume_with_acks("")

    async def test_returns_generator_and_ack_tracker(
        self,
        consumer,
        mock_consume_with_acks,
        mock_channel_direct,
        mock_temp_chan,
    ):
        generator, ack_tracker = await consumer.consume_with_acks("test-id")

        assert ack_tracker is not None
        assert isinstance(ack_tracker, AckTracker)

        # Clean up the generator
        await generator.aclose()

    async def test_acquires_from_the_durable_pool(
        self,
        consumer,
        mock_consume_with_acks,
        mock_channel_direct,
        mock_temp_chan,
    ):
        DurableQueueConsumer.connector.get_channel_direct.reset_mock()
        generator, _ = await consumer.consume_with_acks("test-id")
        await generator.aclose()

        DurableQueueConsumer.connector.get_channel_direct.assert_called_once_with(
            durable_pool=True
        )

    async def test_sets_prefetch_on_channel(
        self,
        consumer,
        mock_consume_with_acks,
        mock_channel_direct,
        mock_temp_chan,
    ):
        mock_channel_direct.set_qos.reset_mock()
        generator, _ = await consumer.consume_with_acks("test-id")
        await generator.aclose()

        mock_channel_direct.set_qos.assert_called_once_with(prefetch_count=20)

    async def test_uses_configured_prefetch_count(
        self,
        consumer,
        mock_consume_with_acks,
        mock_channel_direct,
        mock_temp_chan,
        mocker,
    ):
        mocker.patch.object(
            _settings_module.settings,
            "durable_consumer_prefetch_count",
            50,
        )

        mock_channel_direct.set_qos.reset_mock()

        generator, _ = await consumer.consume_with_acks("test-id")
        await generator.aclose()

        mock_channel_direct.set_qos.assert_any_call(prefetch_count=50)

    async def test_yields_messages_with_delivery_tags(
        self,
        consumer,
        mock_consume_with_acks,
        mock_channel_direct,
        mock_temp_chan,
    ):
        generator, _ = await consumer.consume_with_acks("test-id")

        messages = []
        async for msg in generator:
            messages.append(msg)

        assert len(messages) == 3
        assert [m.delivery_tag for m in messages] == [1, 2, 3]

    async def test_returns_channel_on_cleanup(
        self,
        consumer,
        mock_consume_with_acks,
        mock_channel_direct,
        mock_temp_chan,
    ):
        generator, _ = await consumer.consume_with_acks("test-id")
        await anext(generator)
        await generator.aclose()

        DurableQueueConsumer.connector.return_channel.assert_called_once_with(
            mock_channel_direct
        )

    async def test_passes_authz_check(
        self,
        consumer,
        mock_consume_with_acks,
        mock_channel_direct,
        mock_temp_chan,
    ):
        generator, _ = await consumer.consume_with_acks("test-id")
        # Must iterate to trigger the inner generator code
        await anext(generator)
        await generator.aclose()

        mock = RabbitMQBase._consume_with_client_acks
        mock.assert_called_once()
        call_args = mock.call_args
        assert call_args[0][1] == permissions.can_consume_durable_queue_event

    async def test_releases_channel_when_set_qos_raises(
        self,
        consumer,
        mock_channel_direct,
        mock_temp_chan,
        mocker,
    ):
        # Repro: between get_channel_direct() and the consumer's own
        # try/finally, set_qos can raise (broker error, connection
        # drop). The pool checkout must be released even though the
        # consumer was never constructed.
        mocker.patch.object(
            mock_channel_direct,
            "set_qos",
            new=AsyncMock(side_effect=RuntimeError("qos boom")),
        )

        with pytest.raises(RuntimeError, match="qos boom"):
            await consumer.consume_with_acks("test-id")

        DurableQueueConsumer.connector.return_channel.assert_called_once_with(
            mock_channel_direct
        )

    async def test_releases_channel_when_get_queue_raises(
        self,
        consumer,
        mock_channel_direct,
        mock_temp_chan,
        mocker,
    ):
        # Same gap, exercised on the get_queue step (which itself can
        # raise QueueNotFoundError on a passive declare).
        mocker.patch.object(
            RabbitMQBase,
            "get_queue",
            side_effect=RuntimeError("queue boom"),
        )

        with pytest.raises(RuntimeError, match="queue boom"):
            await consumer.consume_with_acks("test-id")

        DurableQueueConsumer.connector.return_channel.assert_called_once_with(
            mock_channel_direct
        )

    async def test_releases_channel_on_cancellation_in_setup(
        self,
        consumer,
        mock_channel_direct,
        mock_temp_chan,
        mocker,
    ):
        # CancelledError variant: a gRPC stream cancelled mid-setup
        # must still release the pool checkout. CancelledError is
        # BaseException (since 3.8), not Exception, so a bare
        # `except Exception:` would not catch it.
        mocker.patch.object(
            RabbitMQBase,
            "get_queue",
            side_effect=asyncio.CancelledError(),
        )

        with pytest.raises(asyncio.CancelledError):
            await consumer.consume_with_acks("test-id")

        DurableQueueConsumer.connector.return_channel.assert_called_once_with(
            mock_channel_direct
        )


@pytest.fixture
def mock_consume_delayed(mocker):
    """Mock _consume_with_delayed_acks to yield test messages and run cleanup."""

    async def mock_gen(_queue, _authz_check, on_cleanup_complete=None):
        try:
            for i in range(3):
                yield EventMessage({"a": "b"}, delivery_tag=i + 1)
        finally:
            if on_cleanup_complete:
                await on_cleanup_complete()

    mocker.patch.object(
        RabbitMQBase,
        "_consume_with_delayed_acks",
        side_effect=mock_gen,
    )


@pytest.mark.asyncio
class TestConsume:
    """The server-delayed-ack durable consume path (DurableQueueConsumer.consume)."""

    @pytest_asyncio.fixture(autouse=True)
    async def connect_rabbitmq(self, consumer):
        await consumer.connector.connect()

    async def test_raises_error_if_not_allowed(self, consumer, mocker):
        mocker.patch.object(
            permissions,
            "raise_if_cant_consume_durable",
            side_effect=PermissionError("pe"),
        )

        with pytest.raises(PermissionError, match="pe"):
            async for _ in consumer.consume(""):
                pass

    async def test_acquires_from_the_durable_pool(
        self, consumer, mock_consume_delayed, mock_channel_direct, mock_temp_chan
    ):
        DurableQueueConsumer.connector.get_channel_direct.reset_mock()

        async for _ in consumer.consume("test-id"):
            break

        DurableQueueConsumer.connector.get_channel_direct.assert_called_once_with(
            durable_pool=True
        )

    async def test_sets_configured_prefetch_on_channel(
        self,
        consumer,
        mock_consume_delayed,
        mock_channel_direct,
        mock_temp_chan,
        mocker,
    ):
        mocker.patch.object(
            _settings_module.settings, "durable_consumer_prefetch_count", 7
        )
        mock_channel_direct.set_qos.reset_mock()

        async for _ in consumer.consume("test-id"):
            break

        mock_channel_direct.set_qos.assert_any_call(prefetch_count=7)

    async def test_yields_messages(
        self, consumer, mock_consume_delayed, mock_channel_direct, mock_temp_chan
    ):
        messages = [msg async for msg in consumer.consume("test-id")]

        assert len(messages) == 3
        assert all(m.message == {"a": "b"} for m in messages)

    async def test_passes_durable_authz_check(
        self, consumer, mock_consume_delayed, mock_channel_direct, mock_temp_chan
    ):
        async for _ in consumer.consume("test-id"):
            break

        mock = RabbitMQBase._consume_with_delayed_acks
        mock.assert_called_once()
        assert mock.call_args[0][1] == permissions.can_consume_durable_queue_event

    async def test_returns_channel_on_cleanup(
        self, consumer, mock_consume_delayed, mock_channel_direct, mock_temp_chan
    ):
        # Drain fully so the outer generator's finally (consumer.aclose())
        # runs, which triggers the inner generator's cleanup callback.
        async for _ in consumer.consume("test-id"):
            pass

        DurableQueueConsumer.connector.return_channel.assert_called_once_with(
            mock_channel_direct
        )

    async def test_releases_channel_when_set_qos_raises(
        self, consumer, mock_channel_direct, mock_temp_chan, mocker
    ):
        # set_qos can raise between checkout and the consumer's own
        # try/finally; the pool checkout must still be released.
        mocker.patch.object(
            mock_channel_direct,
            "set_qos",
            new=AsyncMock(side_effect=RuntimeError("qos boom")),
        )

        with pytest.raises(RuntimeError, match="qos boom"):
            async for _ in consumer.consume("test-id"):
                pass

        DurableQueueConsumer.connector.return_channel.assert_called_once_with(
            mock_channel_direct
        )

    async def test_releases_channel_when_get_queue_raises(
        self, consumer, mock_channel_direct, mock_temp_chan, mocker
    ):
        mocker.patch.object(
            RabbitMQBase, "get_queue", side_effect=RuntimeError("queue boom")
        )

        with pytest.raises(RuntimeError, match="queue boom"):
            async for _ in consumer.consume("test-id"):
                pass

        DurableQueueConsumer.connector.return_channel.assert_called_once_with(
            mock_channel_direct
        )

    async def test_releases_channel_on_cancellation_in_setup(
        self, consumer, mock_channel_direct, mock_temp_chan, mocker
    ):
        # CancelledError is BaseException, not Exception; the bare
        # `except BaseException` guard must still release the checkout.
        mocker.patch.object(
            RabbitMQBase, "get_queue", side_effect=asyncio.CancelledError()
        )

        with pytest.raises(asyncio.CancelledError):
            async for _ in consumer.consume("test-id"):
                pass

        DurableQueueConsumer.connector.return_channel.assert_called_once_with(
            mock_channel_direct
        )


@pytest.mark.asyncio
class TestEveryOperationResolvesThroughTheValidatingOverride:
    """
    The id check really is a property of the code, not of a list.

    ``TestGetQueue`` pins that this subclass's override rejects a malformed
    id, and each entry point's own tests pin that a malformed id never
    reaches the broker. Neither says the two are the same check: an
    operation that resolved its queue through ``RabbitMQBase.get_queue``
    directly -- the base method, which deliberately does NOT apply
    ``QUEUE_ID_PATTERN``, because the non-durable reconnect path shares it
    -- would satisfy both and still be unvalidated.

    So this asserts the funnel itself: every durable operation on an
    EXISTING queue goes through THIS class's ``get_queue``. Patched on
    ``DurableQueueConsumer`` rather than on the base, so a call that
    resolved through the base would leave the mock uncalled.
    """

    @pytest_asyncio.fixture(autouse=True)
    async def connect(self, consumer):
        await consumer.connector.connect()

    @pytest.fixture
    def override(self, mocker, mock_queue):
        return mocker.patch.object(
            DurableQueueConsumer, "get_queue", return_value=mock_queue
        )

    async def test_consume_resolves_through_it(
        self, consumer, override, mock_consume_delayed, mock_channel_direct
    ):
        async for _ in consumer.consume("test-id"):
            break

        override.assert_awaited_once()

    async def test_consume_with_acks_resolves_through_it(
        self, consumer, override, mock_consume_with_acks, mock_channel_direct
    ):
        generator, _ = await consumer.consume_with_acks("test-id")
        await generator.aclose()

        override.assert_awaited_once()

    async def test_delete_queue_resolves_through_it(
        self, consumer, override, mock_channel_context
    ):
        await consumer.delete_queue("test-id")

        override.assert_awaited_once()

    async def test_update_queue_resolves_through_it(
        self, consumer, override, mock_channel_context, filter_groups, mocker
    ):
        mocker.patch.object(consumer.connector, "get_exchange")

        await consumer.update_queue("test-id", filter_groups, filter_groups)

        override.assert_awaited_once()

    @pytest.mark.parametrize(
        "invalid_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_delete_queue_is_stopped_by_it(
        self, invalid_id, consumer, mock_channel_context
    ):
        # The consequence of the funnel, with the real override in place:
        # the entry point inherits the check without restating it.
        with pytest.raises(UnprocessableError, match="Invalid queue_id"):
            await consumer.delete_queue(invalid_id)

    @pytest.mark.parametrize(
        "invalid_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_update_queue_is_stopped_by_it(
        self, invalid_id, consumer, mock_channel_context, filter_groups
    ):
        with pytest.raises(UnprocessableError, match="Invalid queue_id"):
            await consumer.update_queue(invalid_id, filter_groups, filter_groups)
