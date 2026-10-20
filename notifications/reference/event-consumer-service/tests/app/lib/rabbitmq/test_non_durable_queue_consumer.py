# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
import asyncio
import contextvars
import logging
import uuid

import pytest
import pytest_asyncio
from notification_common.auth import principal_id
from notification_common.errors.errors import (
    AuthenticationError,
    BadRequestError,
    ServerError,
)

from app.config import settings as _settings_module
from app.lib.models.event_message import EventMessage
from app.lib.permissions import permissions
from app.lib.permissions.storage_permissions import (
    storage_permissions,
    storage_permissions_cache,
)
from app.lib.permissions.storage_permissions.storage_cache import StorageCache
from app.lib.rabbitmq import NonDurableQueueConsumer
from app.lib.rabbitmq import non_durable_queue_consumer as ndq_module
from app.lib.rabbitmq.non_durable_queue_consumer import logger
from app.lib.queue_names import (
    NO_PRINCIPAL,
    queue_ref_for_name,
    redact_queue_name,
    redact_reconnect_token,
)
from app.lib.permissions.storage_permissions import cache_writer as cache_writer_module
from app.lib.permissions.storage_permissions.cache_scope import CacheScope
from app.lib.permissions.storage_permissions.cache_writer import get_writer_for_queue
from app.lib.rabbitmq import rabbitmq_base as ndq_base_module
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase, rabbitmq_connector
from tests.fixtures.cache_scope import (
    SECOND_PRINCIPAL,
    SECOND_PRINCIPAL_REF,
    TEST_PRINCIPAL,
    TEST_PRINCIPAL_REF,
)
from tests.fixtures.dropped_storage_events import (
    DROPPED,
    unauthorized_entry_with_drops,
)
from tests.fixtures.mock_filter_group import filter_group
from tests.fixtures.queue_id_bounds import MAX_QUEUE_ID_LENGTH
from tests.fixtures.reconnect_token import RECONNECT_TOKEN as SECRET_TOKEN


@pytest_asyncio.fixture(autouse=True)
async def setup(mocker):
    mocker.patch.object(storage_permissions, "initialize")


@pytest.fixture
def expected_scope(mock_queue):
    """
    The cache scope ``get_instance`` derives under the suite's default
    fixtures: the broker queue name the queue fixture reports, paired with
    the redacted reference of the principal the autouse ``mock_principal_id``
    fixture resolves.
    """
    return CacheScope(mock_queue.name, TEST_PRINCIPAL_REF)


@pytest.fixture
def mock_pool_channel(mocker):
    """
    Mock ``rabbitmq_connector.get_channel_direct`` to return a mock
    channel — one direct checkout from the non-durable pool.
    """
    mock_channel = mocker.MagicMock()
    mocker.patch.object(
        rabbitmq_connector,
        "get_channel_direct",
        return_value=mock_channel,
    )
    return mock_channel


@pytest.fixture
def mock_return_channel(mocker):
    """Mock ``rabbitmq_connector.return_channel`` (the pool release)."""
    return mocker.patch.object(
        rabbitmq_connector,
        "return_channel",
    )


@pytest.fixture
def consumer(mock_queue, mock_chan):
    return NonDurableQueueConsumer(mock_queue, "test-queue-id", mock_chan)


@pytest.mark.asyncio
class TestConsume:
    @pytest_asyncio.fixture(autouse=True)
    async def connect(self, consumer):
        await consumer.connector.connect()

    async def test_passes_authz_check_to_consume(self, consumer, mocker):
        spy = mocker.spy(RabbitMQBase, "_consume_with_delayed_acks")

        async for _ in consumer.consume():
            break

        spy.assert_called_once()
        call_args = spy.call_args
        assert call_args[0][1] == permissions.can_consume_non_durable_queue_event

    async def test_does_not_set_prefetch_on_channel(
        self, consumer, mock_chan, mock_consume
    ):
        """Non-durable consumption is unaffected by the durable prefetch
        reduction: the durable ``durable_consumer_prefetch_count`` (20) must
        never leak into the non-durable path. Any ``set_qos`` call with the
        channel pool's own default (500) is orthogonal and expected, so we
        only assert the durable value never appears.
        """
        async for _ in consumer.consume():
            pass

        durable_prefetch = _settings_module.settings.durable_consumer_prefetch_count
        durable_qos_calls = [
            c
            for c in mock_chan.set_qos.call_args_list
            if c.kwargs.get("prefetch_count") == durable_prefetch
        ]
        assert durable_qos_calls == []

    async def test_yields_messages_from_consume(self, consumer, mock_consume):
        messages = []
        async for m in consumer.consume():
            messages.append(m)

        assert len(messages) == 3
        assert all(msg.message == {"a": "b"} for msg in messages)

    async def test_del_schedules_cleanup_task(
        self,
        mock_queue,
        mock_chan,
        mocker,
        mock_return_channel,
    ):
        mocked_create_task = mocker.patch.object(asyncio, "create_task")
        mock_loop = mocker.MagicMock()
        mock_loop.is_running.return_value = True
        mocker.patch.object(asyncio, "get_event_loop", return_value=mock_loop)

        consumer = NonDurableQueueConsumer(mock_queue, "test-id", mock_chan)
        consumer.__del__()

        mocked_create_task.assert_called_once()
        cleanup_coro = mocked_create_task.call_args[0][0]
        await cleanup_coro

        mock_return_channel.assert_called_once_with(mock_chan)
        assert consumer.channel is None

    @pytest.mark.parametrize(
        "raised",
        [
            pytest.param(RuntimeError("No event loop"), id="no_event_loop"),
            pytest.param(AttributeError("no attribute"), id="attribute_error"),
        ],
    )
    async def test_del_warns_when_the_loop_lookup_raises(
        self, mock_queue, mock_chan, mocker, raised
    ):
        # `_DEL_CLEANUP_EXC` is a module-level tuple constant so the formatter
        # has no `except (A, B):` parens to strip -- the stripped form is a
        # SyntaxError under 3.13-grammar tooling (`b.8qg`). This pins the
        # second leg: narrowing it to `(RuntimeError,)` fails `attribute_error`.
        mocker.patch.object(asyncio, "get_event_loop", side_effect=raised)
        mock_warn = mocker.patch.object(logger, "warning")

        consumer = NonDurableQueueConsumer(mock_queue, "test-id", mock_chan)
        consumer.__del__()

        mock_warn.assert_called_once_with(
            "Could not return channel in __del__ for queue %s",
            redact_reconnect_token("test-id"),
        )

    async def test_del_warning_does_not_name_the_reconnect_token(
        self, mock_queue, mock_chan, mocker
    ):
        mocker.patch.object(
            asyncio,
            "get_event_loop",
            side_effect=RuntimeError("No event loop"),
        )
        mock_warn = mocker.patch.object(logger, "warning")

        consumer = NonDurableQueueConsumer(mock_queue, SECRET_TOKEN, mock_chan)
        consumer.__del__()

        assert SECRET_TOKEN not in "".join(str(a) for a in mock_warn.call_args.args)

    async def test_del_warning_falls_back_to_unknown_when_the_queue_id_is_unbound(
        self, mock_queue, mock_chan, mocker
    ):
        # The id is always set once `__del__` is reachable, so this test
        # unbinds it synthetically to pin the defensive `getattr` fallback
        # (an AttributeError raised inside `__del__` would be swallowed by
        # the interpreter, taking the warning with it).
        mocker.patch.object(
            asyncio,
            "get_event_loop",
            side_effect=RuntimeError("No event loop"),
        )
        mock_warn = mocker.patch.object(logger, "warning")

        consumer = NonDurableQueueConsumer(mock_queue, "test-id", mock_chan)
        del consumer.queue_id
        consumer.__del__()

        mock_warn.assert_called_once_with(
            "Could not return channel in __del__ for queue %s",
            "unknown",
        )

    async def test_del_warning_does_not_invent_a_reference_for_a_missing_id(
        self, mock_queue, mock_chan, mocker
    ):
        # Redacting an absent id would render the digest of the empty
        # string: one constant value shared by every such warning, reading
        # exactly like a real stream's reference and aggregating unrelated
        # failures under a queue nobody can find.
        mocker.patch.object(
            asyncio,
            "get_event_loop",
            side_effect=RuntimeError("No event loop"),
        )
        mock_warn = mocker.patch.object(logger, "warning")

        consumer = NonDurableQueueConsumer(mock_queue, "test-id", mock_chan)
        del consumer.queue_id
        consumer.__del__()

        assert redact_reconnect_token("") not in mock_warn.call_args.args

    async def test_cleanup_channel_handles_release_failure(
        self, mock_queue, mock_chan, mocker
    ):
        mocker.patch.object(
            rabbitmq_connector,
            "return_channel",
            side_effect=Exception("Connection failed"),
        )
        mock_exc = mocker.patch.object(logger, "exception")

        consumer = NonDurableQueueConsumer(mock_queue, "test-id", mock_chan)
        await consumer.cleanup_channel()

        mock_exc.assert_called_once_with(
            "Failed to return channel in cleanup "
            f"for queue {redact_reconnect_token('test-id')}"
        )

    async def test_cleanup_channel_failure_does_not_name_the_reconnect_token(
        self, mock_queue, mock_chan, mocker
    ):
        mocker.patch.object(
            rabbitmq_connector,
            "return_channel",
            side_effect=Exception("Connection failed"),
        )
        mock_exc = mocker.patch.object(logger, "exception")

        consumer = NonDurableQueueConsumer(mock_queue, SECRET_TOKEN, mock_chan)
        await consumer.cleanup_channel()

        assert SECRET_TOKEN not in mock_exc.call_args.args[0]

    async def test_cleanup_channel_is_idempotent(self, consumer, mock_return_channel):
        await consumer.cleanup_channel()
        await consumer.cleanup_channel()

        mock_return_channel.assert_called_once()


class _ChannelAssignmentWitness(NonDurableQueueConsumer):
    """
    A consumer that records what ``__init__`` had already bound at the
    moment it assigned ``self.channel``.

    The finished object cannot tell the assignment order apart -- every
    attribute is present once ``__init__`` returns -- so the ``channel``
    setter is where the order is observable.
    """

    def __init__(self, *args, **kwargs):
        self.bound_at_channel_assignment = {}
        super().__init__(*args, **kwargs)

    @property
    def channel(self):
        return self._channel

    @channel.setter
    def channel(self, value):
        if not self.bound_at_channel_assignment:
            self.bound_at_channel_assignment = {
                "queue_id": hasattr(self, "queue_id"),
                "_cache_scope": hasattr(self, "_cache_scope"),
            }
        self._channel = value


class TestInitAssignmentOrder:
    """
    ``__del__``'s ``hasattr(self, "channel")`` guard is read as "``__init__``
    got at least as far as ``queue_id``" -- by the handler's own comment and
    by four doc surfaces that repeat it. That reading holds only while
    ``queue_id`` is assigned BEFORE ``channel``, and ``_cache_scope`` after
    it (see the ``__init__`` comment the consumer carries). Reordering those
    lines is a silent way to break every one of those claims, so the order
    is pinned here.
    """

    @pytest.fixture
    def bound_at_channel_assignment(self, mock_queue, mock_chan):
        witness = _ChannelAssignmentWitness(mock_queue, "test-queue-id", mock_chan)
        return witness.bound_at_channel_assignment

    def test_queue_id_is_bound_before_the_channel(self, bound_at_channel_assignment):
        assert bound_at_channel_assignment["queue_id"] is True

    def test_the_cache_scope_is_bound_after_the_channel(
        self, bound_at_channel_assignment
    ):
        assert bound_at_channel_assignment["_cache_scope"] is False


@pytest.mark.asyncio
class TestGetInstance:
    async def test_raises_error_with_no_groups_or_queue_id(self, consumer):
        with pytest.raises(
            BadRequestError,
            match="Must provide either filter_groups " "or queue_id",
        ):
            await NonDurableQueueConsumer.get_instance()

    class TestWithQueueId:
        @pytest.fixture(autouse=True)
        def mocked_get_queue(self, consumer, mock_queue, mock_chan, mocker):
            return mocker.patch.object(
                RabbitMQBase,
                "get_queue",
                return_value=mock_queue,
            )

        @pytest.mark.parametrize(
            "new_filters, prev_filters, message",
            [
                (
                    None,
                    "p",
                    "Must provide new filter_groups " "when updating filters",
                ),
                (
                    "n",
                    None,
                    "Must provide previous_filter_groups " "when updating filters",
                ),
            ],
        )
        async def test_raises_error_with_missing_filters(
            self,
            new_filters,
            prev_filters,
            message,
            mock_pool_channel,
            mocker,
        ):
            spy = mocker.patch.object(NonDurableQueueConsumer, "update_queue")

            with pytest.raises(BadRequestError, match=message):
                # noinspection PyTypeChecker
                await NonDurableQueueConsumer.get_instance(
                    "test-queue-id",
                    new_filters,
                    prev_filters,
                )

            spy.assert_not_called()

        async def test_it_looks_up_queue(self, mocked_get_queue, mock_pool_channel):
            await NonDurableQueueConsumer.get_instance(queue_id="test-queue-id")
            mocked_get_queue.assert_called_once_with(mock_pool_channel, "test-queue-id")

        async def test_it_doesnt_update_the_queue(self, mock_pool_channel, mocker):
            spy = mocker.spy(NonDurableQueueConsumer, "update_queue")
            await NonDurableQueueConsumer.get_instance("test-queue-id")
            spy.assert_not_called()

        async def test_initializes_storage_permissions(
            self, mock_queue, mock_pool_channel, expected_scope, mocker
        ):
            mocked_init = mocker.patch.object(
                storage_permissions,
                "initialize",
                return_value=mock_queue,
            )
            await NonDurableQueueConsumer.get_instance("test-queue-id")
            mocked_init.assert_called_once_with(expected_scope, [])

        async def test_returns_instance_with_queue_and_id(
            self, mock_queue, mock_pool_channel
        ):
            instance = await NonDurableQueueConsumer.get_instance("test-queue-id-arg")
            assert isinstance(instance, NonDurableQueueConsumer)
            assert instance.queue_id == "test-queue-id-arg"
            assert instance.queue == mock_queue
            assert instance.channel == mock_pool_channel

    class TestWithFilterGroupsAndNoQueueId:
        @pytest.fixture(autouse=True)
        def mocked_create_queue(self, mock_queue, mocker):
            return mocker.patch.object(
                RabbitMQBase,
                "_create_queue",
                return_value=(
                    "test-queue-id",
                    mock_queue,
                ),
            )

        async def test_it_creates_a_queue(
            self,
            mocked_create_queue,
            filter_groups,
            mocker,
        ):
            mock_channel = mocker.MagicMock()
            mocker.patch.object(
                rabbitmq_connector,
                "get_channel_direct",
                return_value=mock_channel,
            )

            await NonDurableQueueConsumer.get_instance(filter_groups=filter_groups)
            mocked_create_queue.assert_called_once_with(
                mock_channel, filter_groups, False
            )

        async def test_initializes_storage_permissions(
            self,
            consumer,
            mock_queue,
            filter_groups,
            mock_pool_channel,
            expected_scope,
            mocker,
        ):
            mocked_init = mocker.patch.object(
                storage_permissions,
                "initialize",
                return_value=mock_queue,
            )
            await NonDurableQueueConsumer.get_instance(filter_groups=filter_groups)
            mocked_init.assert_called_once_with(expected_scope, filter_groups)

        async def test_doesnt_update_queue_when_no_prev(
            self,
            filter_groups,
            mock_pool_channel,
            mocker,
        ):
            spy = mocker.patch.object(NonDurableQueueConsumer, "update_queue")
            # noinspection PyTypeChecker
            await NonDurableQueueConsumer.get_instance(None, ["new"])
            spy.assert_not_called()

        async def test_raises_error_updating_without_initial(
            self,
            filter_groups,
            mock_pool_channel,
            mocker,
        ):
            spy = mocker.patch.object(NonDurableQueueConsumer, "update_queue")

            with pytest.raises(
                BadRequestError,
                match="Filters cannot be updated before "
                "consuming with initial filters",
            ):
                # noinspection PyTypeChecker
                await NonDurableQueueConsumer.get_instance(None, ["new"], ["old"])

            spy.assert_not_called()

        async def test_returns_instance_with_queue_and_id(
            self,
            mock_queue,
            filter_groups,
            mock_pool_channel,
        ):
            instance = await NonDurableQueueConsumer.get_instance(
                filter_groups=filter_groups
            )
            assert isinstance(instance, NonDurableQueueConsumer)
            assert instance.queue_id == "test-queue-id"
            assert instance.queue == mock_queue
            assert instance.channel == mock_pool_channel

        async def test_releases_channel_on_exception(
            self,
            mock_pool_channel,
            filter_groups,
            mocker,
        ):
            mocker.patch.object(
                RabbitMQBase,
                "_create_queue",
                side_effect=Exception("boom"),
            )
            mock_release = mocker.patch.object(
                rabbitmq_connector,
                "return_channel",
            )

            with pytest.raises(Exception, match="boom"):
                await NonDurableQueueConsumer.get_instance(filter_groups=filter_groups)

            mock_release.assert_called_once_with(mock_pool_channel)

        async def test_releases_channel_on_cancellation(
            self,
            mock_pool_channel,
            filter_groups,
            mocker,
        ):
            # CancelledError is BaseException, not Exception. A bare
            # `except Exception:` would let the cancellation propagate
            # without returning the pool checkout, which is the leak
            # observed on the dev cluster (pool channels stuck at
            # client_count=1, broker consumer_count=0). Verify the
            # channel is returned to the pool on the cancellation path
            # too.
            mocker.patch.object(
                RabbitMQBase,
                "_create_queue",
                side_effect=asyncio.CancelledError(),
            )
            mock_release = mocker.patch.object(
                rabbitmq_connector,
                "return_channel",
            )

            with pytest.raises(asyncio.CancelledError):
                await NonDurableQueueConsumer.get_instance(filter_groups=filter_groups)

            mock_release.assert_called_once_with(mock_pool_channel)


@pytest.mark.asyncio
class TestOneStreamOneCheckout:
    """
    Post-CCM counting contract (consumed by the capacity-surface Task):
    with the ConnectionChannelManager retired, one stream equals one
    direct pool checkout with no multiplexing layer in between. A
    successful ``get_instance`` takes exactly one checkout from the
    NON-DURABLE pool, and one ``cleanup_channel`` returns exactly one.

    The non-durable pin matters because the connector selects the pool
    from the ``durable_pool`` keyword: a stray ``durable_pool=True`` here
    would silently draw non-durable streams out of the durable pool, and
    no other test would catch it (durable consumers pass
    ``durable_pool=True`` on their own side).
    """

    @pytest.fixture(autouse=True)
    def mocked_get_queue(self, mock_queue, mocker):
        return mocker.patch.object(RabbitMQBase, "get_queue", return_value=mock_queue)

    async def test_get_instance_takes_exactly_one_pool_checkout(
        self, mock_queue, mock_pool_channel
    ):
        await NonDurableQueueConsumer.get_instance("test-queue-id")

        rabbitmq_connector.get_channel_direct.assert_called_once()

    async def test_get_instance_acquires_from_the_non_durable_pool(
        self, mock_queue, mock_pool_channel
    ):
        await NonDurableQueueConsumer.get_instance("test-queue-id")

        call = rabbitmq_connector.get_channel_direct.call_args
        assert call.kwargs.get("durable_pool") is not True
        assert True not in call.args

    async def test_cleanup_channel_returns_exactly_one_checkout(
        self, consumer, mock_chan, mock_return_channel
    ):
        await consumer.cleanup_channel()

        mock_return_channel.assert_called_once_with(mock_chan)


@pytest.mark.asyncio
class TestGetInstanceResolvesThePrincipalFirst:
    """
    The principal is resolved before the pool checkout.

    That resolution can still raise -- a cache entry whose stored lifetime
    has lapsed reports as an authentication failure -- and the checkout at
    the top of ``get_instance`` sits outside the ``try`` that releases the
    channel, so a 401 raised between the checkout and the ``try`` would
    strand one non-durable pool channel per refused attach, and a 401
    raised after ``_create_queue`` would leave behind a queue nobody was
    told about. Resolving first is what makes a refused attach cost no
    channel and leave no queue.
    """

    @pytest.fixture
    def lapsed_token(self, mocker, authentication_enabled):
        authentication_enabled(True)
        mocker.patch.object(
            principal_id,
            "principal_id",
            side_effect=AuthenticationError("token expired"),
        )

    async def test_a_lapsed_token_fails_the_attach(
        self, filter_groups, lapsed_token, mock_pool_channel
    ):
        with pytest.raises(AuthenticationError):
            await NonDurableQueueConsumer.get_instance(filter_groups=filter_groups)

    async def test_a_lapsed_token_takes_no_pool_checkout(
        self, filter_groups, lapsed_token, mock_pool_channel
    ):
        with pytest.raises(AuthenticationError):
            await NonDurableQueueConsumer.get_instance(filter_groups=filter_groups)

        rabbitmq_connector.get_channel_direct.assert_not_called()


@pytest.mark.asyncio
class TestQueueName:
    @pytest.fixture
    def test_nd_queue_name(self):
        return "event-queue:test-id:nd"

    @pytest.fixture
    def mock_base_queue_name(self):
        return "event-queue:some-uuid-here"

    @pytest.fixture
    def mock_base_queue_name_method(self, mocker, mock_base_queue_name):
        return mocker.patch.object(
            RabbitMQBase,
            "_queue_name",
            return_value=mock_base_queue_name,
        )

    @pytest.fixture
    def mock_nd_queue_name_method(self, mocker, test_nd_queue_name):
        return mocker.patch.object(
            NonDurableQueueConsumer,
            "_queue_name",
            return_value=test_nd_queue_name,
        )

    @pytest.fixture
    def mock_create_queue_setup(self, mocker, mock_pool_channel, mock_queue):
        mock_exchange = mocker.AsyncMock()

        mock_pool_channel.declare_queue = mocker.AsyncMock(return_value=mock_queue)
        mocker.patch.object(
            NonDurableQueueConsumer.connector,
            "get_exchange",
            return_value=mock_exchange,
        )

        mock_uuid = mocker.patch.object(uuid, "uuid4")
        mock_uuid.return_value = "test-uuid"

        mocker.patch.object(
            ndq_base_module,
            "binding_keys_from_filters",
            return_value=["key1", "key2"],
        )

        return mock_queue

    async def test_queue_name_appends_nd_suffix(
        self,
        mock_base_queue_name_method,
        mock_base_queue_name,
    ):
        result = await NonDurableQueueConsumer._queue_name("some-uuid-here")
        assert result == f"{mock_base_queue_name}:nd"

    async def test_unmocked_name_is_the_prefixed_queue_id_with_the_nd_suffix(self):
        """
        The whole non-durable broker name, composed through the real base
        method rather than a patched one.

        The two tests above stub ``RabbitMQBase._queue_name``, so they pin
        only the suffix and would keep passing if the base composition
        changed underneath them. This is the end-to-end shape: the name
        derives from the queue id alone, with no principal component.
        """
        result = await NonDurableQueueConsumer._queue_name("some-uuid-here")

        assert result == "event-queue:some-uuid-here:nd"

    async def test_unmocked_name_does_not_resolve_a_principal_id(self, mocker):
        """
        Naming a non-durable queue never touches the caller's identity.

        ``principal_id`` is patched where it lives -- the naming path no
        longer imports it, so there is nothing to patch on the consumer
        module.
        """
        resolve = mocker.patch.object(principal_id, "principal_id")

        await NonDurableQueueConsumer._queue_name("some-uuid-here")

        resolve.assert_not_called()

    async def test_max_length_name_stays_under_the_rabbitmq_byte_limit(self):
        """
        The ``:nd`` suffix makes this the longest broker name the shared
        length bound has to cover, so it is the spelling that decides
        whether a maximum-length id is safe: ``event-queue:`` (12) plus 200
        plus ``:nd`` (3) is 215 bytes, inside RabbitMQ's 255-byte limit.
        The durable spelling is guarded in ``test_rabbitmq_base.py``.
        """
        max_queue_id = "a" * MAX_QUEUE_ID_LENGTH

        name = await NonDurableQueueConsumer._queue_name(max_queue_id)

        assert len(name.encode("utf-8")) < 255

    async def test_queue_name_calls_base_class_method(
        self, mock_base_queue_name_method
    ):
        mock_queue_id = "test-uuid"
        await NonDurableQueueConsumer._queue_name(mock_queue_id)
        mock_base_queue_name_method.assert_called_once_with(mock_queue_id)

    async def test_get_queue_uses_overridden_queue_name(
        self,
        mocker,
        mock_pool_channel,
        mock_nd_queue_name_method,
        test_nd_queue_name,
        mock_queue,
    ):
        mock_pool_channel.declare_queue = mocker.AsyncMock(
            return_value=mock_queue,
        )

        result = await NonDurableQueueConsumer.get_queue(mock_pool_channel, "test-id")

        mock_pool_channel.declare_queue.assert_called_once_with(
            test_nd_queue_name,
            passive=True,
            robust=False,
        )
        assert result == mock_queue

    async def test_create_queue_uses_overridden_name(
        self,
        mocker,
        mock_pool_channel,
        filter_groups,
        mock_nd_queue_name_method,
        mock_create_queue_setup,
        test_nd_queue_name,
    ):
        queue_id, queue = await NonDurableQueueConsumer._create_queue(
            mock_pool_channel, filter_groups, False
        )

        mock_pool_channel.declare_queue.assert_called_once_with(
            test_nd_queue_name,
            arguments={"x-expires": mocker.ANY},
            durable=False,
            robust=False,
        )
        assert queue_id == "test-uuid"
        assert queue == mock_create_queue_setup


class TestQueueRef:
    """
    ``queue_ref`` is the stand-in every observability site uses in place of
    ``queue_id``, which on a non-durable consumer IS the reconnect token.
    """

    def test_is_the_redacted_form_of_the_queue_id(self, mock_queue, mock_chan):
        consumer = NonDurableQueueConsumer(mock_queue, SECRET_TOKEN, mock_chan)

        assert consumer.queue_ref == redact_reconnect_token(SECRET_TOKEN)

    def test_does_not_contain_the_queue_id(self, mock_queue, mock_chan):
        consumer = NonDurableQueueConsumer(mock_queue, SECRET_TOKEN, mock_chan)

        assert SECRET_TOKEN not in consumer.queue_ref

    def test_is_stable_across_instances_for_the_same_queue_id(
        self, mock_queue, mock_chan
    ):
        # Correlation across pods and restarts is the point: two consumers
        # for one stream must report the same reference.
        first = NonDurableQueueConsumer(mock_queue, SECRET_TOKEN, mock_chan)
        second = NonDurableQueueConsumer(mock_queue, SECRET_TOKEN, mock_chan)

        assert first.queue_ref == second.queue_ref

    def test_differs_for_a_different_queue_id(self, mock_queue, mock_chan):
        first = NonDurableQueueConsumer(mock_queue, SECRET_TOKEN, mock_chan)
        second = NonDurableQueueConsumer(mock_queue, f"{SECRET_TOKEN}-2", mock_chan)

        assert first.queue_ref != second.queue_ref


@pytest.mark.asyncio
class TestCreateQueueLogHygiene:
    """
    The DEBUG lines a non-durable create writes see the reconnect token
    before the client does.

    This is where the token first exists, and the create logs both the id
    and the broker name derived from it -- so both have to be redacted, and
    the redaction has to survive going through this subclass's
    ``_queue_name`` override, which is what makes the name recognisable as
    non-durable in the first place.
    """

    @pytest.fixture
    def create_records(
        self, caplog, mocker, mock_pool_channel, mock_queue, filter_groups
    ):
        async def _run():
            mock_pool_channel.declare_queue = mocker.AsyncMock(return_value=mock_queue)
            mocker.patch.object(
                NonDurableQueueConsumer.connector, "get_exchange", return_value="ex"
            )

            with caplog.at_level(logging.DEBUG, logger=ndq_base_module.logger.name):
                await NonDurableQueueConsumer._create_queue(
                    mock_pool_channel, filter_groups, False, queue_id=SECRET_TOKEN
                )
            return caplog.records

        return _run

    async def test_a_create_logs_something_to_inspect(self, create_records):
        # Guards the guards below: an empty capture would make them vacuous.
        assert await create_records() != []

    async def test_no_record_names_the_reconnect_token(self, create_records):
        records = await create_records()

        assert [r for r in records if SECRET_TOKEN in r.getMessage()] == []

    async def test_the_id_is_logged_as_the_redacted_reference(self, create_records):
        records = await create_records()
        expected = redact_reconnect_token(SECRET_TOKEN)

        assert any(expected in record.getMessage() for record in records)

    async def test_the_broker_name_is_logged_in_redacted_form(self, create_records):
        # The name keeps its shape -- prefix and `:nd` suffix -- so an
        # operator still reads it as the queue name it is.
        records = await create_records()
        expected = redact_queue_name(f"event-queue:{SECRET_TOKEN}:nd")

        assert any(expected in record.getMessage() for record in records)


@pytest.mark.asyncio
class TestGetQueueDoesNotValidateTheQueueId:
    """
    The non-durable lookup id is a reconnect token, not a durable queue id.

    ``DurableQueueConsumer`` overrides ``get_queue`` to enforce
    ``QUEUE_ID_PATTERN``; the non-durable path deliberately inherits the
    unvalidated base method, so an unrecognised token reaches the broker and
    fails as "queue not found" rather than being rejected as malformed.
    """

    async def test_a_token_outside_the_durable_pattern_reaches_the_broker(
        self, mocker, mock_pool_channel, mock_queue
    ):
        mock_pool_channel.declare_queue = mocker.AsyncMock(return_value=mock_queue)

        await NonDurableQueueConsumer.get_queue(mock_pool_channel, "a" * 201)

        mock_pool_channel.declare_queue.assert_called_once_with(
            f"event-queue:{'a' * 201}:nd",
            passive=True,
            robust=False,
        )


@pytest.mark.asyncio
class TestUpdateQueue:
    @pytest.fixture
    def scoped_consumer(self, consumer, expected_scope):
        """
        The module ``consumer`` fixture builds the consumer directly, so it
        carries no cache scope; ``get_instance`` is what binds one in
        production. Tests that reach the mutation path past the scope guard
        need the scope the real construction path would have set.
        """
        consumer._cache_scope = expected_scope
        return consumer

    async def test_does_nothing_if_filters_unchanged(
        self, consumer, filter_groups, mocker
    ):
        spy = mocker.spy(storage_permissions, "initialize")
        await consumer.update_queue(filter_groups, filter_groups)
        spy.assert_not_called()

    async def test_reports_the_cache_was_not_initialized_if_unchanged(
        self, consumer, filter_groups
    ):
        # The return value is the caller's signal: ``get_instance`` runs its
        # own ``storage_permissions.initialize`` only when this is False, so
        # an unchanged pair must not claim the cache was primed.
        assert await consumer.update_queue(filter_groups, filter_groups) is False

    @pytest.mark.parametrize(
        "new_filters, prev_filters",
        [(["new"], []), ([], ["prev"])],
    )
    async def test_raises_error_if_filters_missing(
        self, consumer, new_filters, prev_filters, mocker
    ):
        spy = mocker.spy(storage_permissions, "initialize")

        with pytest.raises(
            BadRequestError,
            match="Must provide new_filter_groups and "
            "previous_filter_groups when updating "
            "filters",
        ):
            await consumer.update_queue(new_filters, prev_filters)

        spy.assert_not_called()

    async def test_initializes_storage_cache_under_the_consumers_own_scope(
        self, scoped_consumer, expected_scope, mocker
    ):
        # The gRPC mid-stream filter update reaches this method from the
        # servicer, which holds no scope of its own -- so the cache has to
        # be re-initialized under the one the consumer derived at connect.
        mocker.patch.object(scoped_consumer, "_update_bindings")
        spy = mocker.patch.object(storage_permissions, "initialize")
        new_groups = [filter_group()]
        # noinspection PyTypeChecker
        await scoped_consumer.update_queue(new_groups, ["old_groups"])
        spy.assert_called_once_with(expected_scope, new_groups)

    async def test_raises_server_error_if_the_cache_scope_is_not_set(
        self, consumer, mocker
    ):
        # A consumer built outside ``get_instance`` carries no scope, and
        # initializing under whatever scope the ambient context happens to
        # hold would prime the cache for the wrong stream.
        spy = mocker.patch.object(storage_permissions, "initialize")

        with pytest.raises(ServerError, match="Cache scope is not set"):
            # noinspection PyTypeChecker
            await consumer.update_queue(["new"], ["old"])

        spy.assert_not_called()

    async def test_reports_the_cache_was_initialized_when_filters_changed(
        self, scoped_consumer, mocker
    ):
        mocker.patch.object(scoped_consumer, "_update_bindings")
        # noinspection PyTypeChecker
        assert await scoped_consumer.update_queue(["new"], ["old"]) is True

    async def test_updates_bindings_on_the_queue(self, scoped_consumer, mocker):
        spy = mocker.patch.object(scoped_consumer, "_update_bindings")
        # noinspection PyTypeChecker
        await scoped_consumer.update_queue(["b"], ["a"])

        spy.assert_called_once_with(
            scoped_consumer.channel,
            scoped_consumer.queue,
            ["a"],
            ["b"],
        )


# --------------------------------------------------------------------------
# CacheWriter lifecycle (Bee oo / Epic .sg)
# --------------------------------------------------------------------------


class _OrderedRecorder:
    """
    Captures the order in which CacheWriter lifecycle hooks fire so the
    test can assert that writer construction + register + ContextVar set
    all complete BEFORE storage_permissions.initialize is awaited.
    """

    def __init__(self):
        self.events: list[str] = []

    def __call__(self, name: str):
        self.events.append(name)


class _DrainRaisingWriter:
    """
    A CacheWriter whose drain fails, as a Redis outage during teardown
    makes it.

    Shared by both error-path suites below so the two stay a faithful
    stand-in for the same class: it carries ``queue_ref`` as well as
    ``scope_key``, because the drain-failure lines the consumer logs on
    exactly this path name the writer by the reference.
    """

    def __init__(self, scope: CacheScope):
        self.scope_key = scope.key
        self.queue_ref = scope.queue_ref

    def start(self) -> None:
        pass

    async def drain_and_stop(self, timeout: float) -> None:
        raise Exception("redis outage during drain")


@pytest.fixture
def order_recorder():
    return _OrderedRecorder()


@pytest.fixture
def lifecycle_patches(mocker, order_recorder):
    """
    Patch every CacheWriter-side symbol on the consumer module so we can
    drive lifecycle assertions without spawning a real drainer task.
    Returns a namespace exposing the patched mocks.
    """

    created_writers: list = []

    class _FakeWriter:
        def __init__(self, scope: CacheScope):
            self.scope_key = scope.key
            # The real writer exposes this alongside `scope_key`, and the
            # consumer's drain-failure log lines read it rather than the
            # scope. Derived the same way the real one derives it, so the
            # fake cannot quietly diverge into a shape that hides a leak.
            self.queue_ref = scope.queue_ref
            self.start_called = False
            self.drain_calls: list[float] = []
            created_writers.append(self)
            order_recorder("ctor")

        def start(self) -> None:
            self.start_called = True
            order_recorder("start")

        async def drain_and_stop(self, timeout: float) -> None:
            self.drain_calls.append(timeout)

    cache_writer_cls = mocker.patch.object(
        ndq_module, "CacheWriter", side_effect=_FakeWriter
    )

    register = mocker.patch.object(
        ndq_module,
        "register_writer",
        side_effect=lambda *a, **k: order_recorder("register"),
    )
    unregister = mocker.patch.object(ndq_module, "unregister_writer")
    set_active = mocker.patch.object(
        ndq_module,
        "set_active_queue_id",
        side_effect=lambda qid: (order_recorder("set_active") or "TOKEN"),
    )
    reset_active = mocker.patch.object(ndq_module, "reset_active_queue_id")

    async def fake_initialize(*args, **kwargs):
        order_recorder("initialize")

    initialize = mocker.patch.object(
        storage_permissions, "initialize", side_effect=fake_initialize
    )

    class _Namespace:
        pass

    ns = _Namespace()
    ns.CacheWriter = cache_writer_cls
    ns.register_writer = register
    ns.unregister_writer = unregister
    ns.set_active_queue_id = set_active
    ns.reset_active_queue_id = reset_active
    ns.initialize = initialize
    # Every writer the patched class handed out, in construction order, so
    # a test can assert the registry calls carry the WRITER itself rather
    # than something merely shaped like it.
    ns.created_writers = created_writers
    return ns


@pytest.fixture
def redis_enabled(mocker):
    """Enable the redis path for lifecycle tests."""
    mocker.patch.object(_settings_module.settings, "redis_enabled", True)


@pytest.fixture
def redis_disabled(mocker):
    mocker.patch.object(_settings_module.settings, "redis_enabled", False)


@pytest.mark.asyncio
class TestGetInstanceCacheWriterLifecycle:
    @pytest.fixture(autouse=True)
    def get_queue_returns_mock(self, mock_queue, mocker):
        mocker.patch.object(RabbitMQBase, "get_queue", return_value=mock_queue)

    async def test_constructs_writer_with_the_streams_cache_scope(
        self, redis_enabled, lifecycle_patches, expected_scope, mock_pool_channel
    ):
        await NonDurableQueueConsumer.get_instance("test-queue-id")

        lifecycle_patches.CacheWriter.assert_called_once_with(expected_scope)

    async def test_calls_start_on_constructed_writer(
        self, redis_enabled, lifecycle_patches, mock_pool_channel
    ):
        instance = await NonDurableQueueConsumer.get_instance("test-queue-id")

        assert instance._cache_writer.start_called is True

    async def test_registers_the_writer_itself(
        self, redis_enabled, lifecycle_patches, mock_pool_channel
    ):
        # The registry is keyed on the writer's own scope key, so the
        # writer is all `register_writer` takes -- passing a key beside it
        # would make a key/writer mismatch representable.
        instance = await NonDurableQueueConsumer.get_instance("test-queue-id")

        lifecycle_patches.register_writer.assert_called_once_with(
            instance._cache_writer
        )

    async def test_binds_the_scope_key_to_the_context_var(
        self, redis_enabled, lifecycle_patches, expected_scope, mock_pool_channel
    ):
        # What the mutators look the writer up by has to be the scope key,
        # not the queue name: a second principal on this queue binds a
        # different key and must reach its own writer.
        await NonDurableQueueConsumer.get_instance("test-queue-id")

        lifecycle_patches.set_active_queue_id.assert_called_once_with(
            expected_scope.key
        )

    async def test_stores_token_on_consumer(
        self, redis_enabled, lifecycle_patches, mock_pool_channel
    ):
        instance = await NonDurableQueueConsumer.get_instance("test-queue-id")

        assert instance._active_queue_id_token == "TOKEN"

    async def test_writer_lifecycle_completes_before_initialize(
        self, redis_enabled, lifecycle_patches, order_recorder, mock_pool_channel
    ):
        await NonDurableQueueConsumer.get_instance("test-queue-id")

        # ctor, start, register, set_active must all precede initialize so
        # that any mutation triggered by the storage API population emits
        # ops against the right queue.
        assert order_recorder.events == [
            "ctor",
            "start",
            "register",
            "set_active",
            "initialize",
        ]

    async def test_skips_writer_construction_when_redis_disabled(
        self, redis_disabled, lifecycle_patches, mock_pool_channel
    ):
        instance = await NonDurableQueueConsumer.get_instance("test-queue-id")

        lifecycle_patches.CacheWriter.assert_not_called()
        lifecycle_patches.register_writer.assert_not_called()
        lifecycle_patches.set_active_queue_id.assert_not_called()
        assert instance._cache_writer is None
        assert instance._active_queue_id_token is None


class _RegistryWriter:
    """
    A CacheWriter stand-in for the tests that drive the REAL registry.

    Carries exactly what ``register_writer`` / ``unregister_writer`` and the
    consumer read -- ``scope_key``, ``queue_ref``, ``start``,
    ``drain_and_stop`` -- and records whether it was started and drained, so
    a test can tell "registered alongside" from "replaced and drained".
    """

    def __init__(self, scope: CacheScope):
        self.scope_key = scope.key
        self.queue_ref = scope.queue_ref
        self.started = False
        self.drained = False

    def start(self) -> None:
        self.started = True

    async def drain_and_stop(self, timeout: float) -> None:
        self.drained = True


@pytest.fixture
def isolated_writer_registry():
    """
    Snapshot and restore the module-level ``_writer_registry`` around a test
    that registers writers through the real registry functions.
    """
    original = dict(cache_writer_module._writer_registry)
    cache_writer_module._writer_registry.clear()
    try:
        yield cache_writer_module._writer_registry
    finally:
        cache_writer_module._writer_registry.clear()
        cache_writer_module._writer_registry.update(original)


@pytest.fixture
def attach_as(mocker, mock_pool_channel, redis_enabled, isolated_writer_registry):
    """
    Attach to one non-durable queue as a chosen principal, through the real
    registry.

    Only ``CacheWriter`` is faked; ``register_writer``,
    ``unregister_writer`` and ``set_active_queue_id`` run for real, because
    what these tests are about is what the registry does with two writers.
    """
    mocker.patch.object(ndq_module, "CacheWriter", side_effect=_RegistryWriter)

    async def _attach(principal):
        mocker.patch.object(principal_id, "principal_id", return_value=principal)
        return await NonDurableQueueConsumer.get_instance("test-queue-id")

    return _attach


@pytest.mark.asyncio
class TestGetInstanceSecondPrincipalIsolation:
    """
    Issue b.ize: a non-durable queue's id IS its reconnect token, so any
    authenticated principal holding one can attach to that queue. Each is
    authorized per event against its own permissions, but the
    storage-permissions cache behind those checks used to be addressed by
    the broker queue name alone -- so the second principal inherited the
    first's cached directory listings, and registering its writer under the
    same key drained and stopped the first's.

    The namespace is the queue AND the presenting principal, which makes
    the two streams peers: separate scope keys, separate writers, one
    shared ``queue_ref``.
    """

    @pytest.fixture(autouse=True)
    def get_queue_returns_mock(self, mock_queue, mocker):
        mocker.patch.object(RabbitMQBase, "get_queue", return_value=mock_queue)

    @pytest_asyncio.fixture
    async def two_principals(self, attach_as):
        """
        Two writers on one queue, one per principal, in attach order.

        The CONSUMERS are held for the fixture's lifetime, not just their
        writers: a consumer collected mid-test runs ``__del__``, which
        schedules the cleanup that unregisters and drains its writer -- the
        very things these tests are asserting did not happen.
        """
        first = await attach_as(TEST_PRINCIPAL)
        second = await attach_as(SECOND_PRINCIPAL)
        yield first._cache_writer, second._cache_writer
        del first, second

    async def test_the_second_principal_gets_its_own_scope_key(self, two_principals):
        first, second = two_principals

        assert first.scope_key != second.scope_key

    async def test_each_scope_key_carries_its_own_principal_reference(
        self, two_principals
    ):
        first, second = two_principals

        assert (first.scope_key.endswith(TEST_PRINCIPAL_REF)) and (
            second.scope_key.endswith(SECOND_PRINCIPAL_REF)
        )

    async def test_both_principals_report_the_same_queue_ref(self, two_principals):
        # Two views of ONE stream: the exported reference is queue-only, so
        # their metrics and logs still join that stream's spans.
        first, second = two_principals

        assert first.queue_ref == second.queue_ref

    async def test_the_first_principals_writer_stays_reachable(self, two_principals):
        first, second = two_principals

        assert get_writer_for_queue(first.scope_key) is first

    async def test_the_second_principals_writer_is_reachable_too(self, two_principals):
        first, second = two_principals

        assert get_writer_for_queue(second.scope_key) is second

    async def test_attaching_does_not_drain_the_first_principals_writer(
        self, two_principals
    ):
        # The regression this whole change exists for: the second attach
        # used to displace the first's registry entry, which drained and
        # stopped a writer whose stream was still live.
        first, second = two_principals
        for _ in range(5):
            await asyncio.sleep(0)

        assert first.drained is False

    async def test_the_registry_holds_both_writers(
        self, two_principals, isolated_writer_registry
    ):
        first, second = two_principals

        assert set(isolated_writer_registry) == {first.scope_key, second.scope_key}


@pytest.mark.asyncio
class TestGetInstanceSamePrincipalFanOut:
    """
    The supported same-principal fan-out is unaffected: two streams from
    ONE principal still share a scope, so the second registration replaces
    the first and drains it, exactly as it did before scopes existed.
    """

    @pytest.fixture(autouse=True)
    def get_queue_returns_mock(self, mock_queue, mocker):
        mocker.patch.object(RabbitMQBase, "get_queue", return_value=mock_queue)

    @pytest_asyncio.fixture
    async def two_streams(self, attach_as):
        # Both consumers stay alive for the test: see the sibling suite's
        # ``two_principals`` for why that matters.
        first = await attach_as(TEST_PRINCIPAL)
        second = await attach_as(TEST_PRINCIPAL)
        yield first._cache_writer, second._cache_writer
        del first, second

    async def test_both_streams_share_one_scope_key(self, two_streams):
        first, second = two_streams

        assert first.scope_key == second.scope_key

    async def test_the_second_registration_takes_over_the_entry(self, two_streams):
        first, second = two_streams

        assert get_writer_for_queue(second.scope_key) is second

    async def test_the_replaced_writer_is_drained(self, two_streams):
        # Otherwise the replaced writer's drainer task leaks.
        first, second = two_streams
        for _ in range(5):
            await asyncio.sleep(0)

        assert first.drained is True


@pytest.mark.asyncio
class TestGetInstanceWithAuthenticationDisabled:
    """
    With authentication off there is no principal to name, so every stream
    on a queue is the same anonymous caller and they share one scope --
    exactly as they shared one before the scope existed.
    """

    @pytest.fixture(autouse=True)
    def get_queue_returns_mock(self, mock_queue, mocker):
        mocker.patch.object(RabbitMQBase, "get_queue", return_value=mock_queue)

    @pytest.fixture(autouse=True)
    def no_principal(self, mocker):
        mocker.patch.object(principal_id, "principal_id", return_value=None)

    async def test_builds_the_scope_with_the_no_principal_constant(
        self, redis_enabled, lifecycle_patches, mock_queue, mock_pool_channel
    ):
        await NonDurableQueueConsumer.get_instance("test-queue-id")

        lifecycle_patches.CacheWriter.assert_called_once_with(
            CacheScope(mock_queue.name, NO_PRINCIPAL)
        )

    async def test_two_anonymous_streams_share_one_scope_key(self, attach_as):
        first = await attach_as(None)
        second = await attach_as(None)

        assert first._cache_writer.scope_key == second._cache_writer.scope_key


@pytest.mark.asyncio
class TestGetInstanceFilterUpdateLifecycle:
    """
    Bee b.xbb regression: on the filter-update reconnect path
    (queue_id + filter_groups + previous_filter_groups), the writer
    construction + ContextVar bind must complete BEFORE update_queue
    runs — otherwise mutations driven by update_queue (e.g.
    _fetch_dir_contents -> replace_dir Op) silently skip enqueue
    because active_writer() returns None.
    """

    @pytest.fixture(autouse=True)
    def get_queue_returns_mock(self, mock_queue, mocker):
        mocker.patch.object(RabbitMQBase, "get_queue", return_value=mock_queue)

    @pytest.fixture
    def recording_update_queue(self, mocker, order_recorder):
        """
        Install an ``update_queue`` that records when it ran and reports a
        chosen "did I re-initialize the cache?" answer.

        The answer matters to the sequence, not just to ``update_queue``'s
        own contract: ``get_instance`` skips its trailing ``initialize``
        when the update already primed the cache (issue b.czw), so a fake
        that always returned ``None`` would keep producing a trailing
        ``initialize`` that production no longer emits. The same value is
        also what ``get_instance`` records as
        ``connect_time_update_changed`` (issue b.yvu), pinned on its own in
        ``TestGetInstanceRecordsTheConnectTimeUpdateVerdict``.
        """

        def _install(cache_initialized: bool):
            async def _fake_update_queue(self, *_a, **_kw):
                order_recorder("update_queue")
                return cache_initialized

            mocker.patch.object(
                NonDurableQueueConsumer, "update_queue", _fake_update_queue
            )

        return _install

    async def test_writer_active_before_update_queue_runs(
        self,
        redis_enabled,
        lifecycle_patches,
        order_recorder,
        mock_pool_channel,
        recording_update_queue,
    ):
        recording_update_queue(True)

        await NonDurableQueueConsumer.get_instance(
            queue_id="test-queue-id",
            filter_groups=["new"],
            previous_filter_groups=["old"],
        )

        # set_active must precede update_queue — i.e., the writer
        # ContextVar is already bound when update_queue's mutations run.
        # Nothing follows it here: the update re-initialized the cache, so
        # get_instance leaves the storage API alone.
        assert order_recorder.events == [
            "ctor",
            "start",
            "register",
            "set_active",
            "update_queue",
        ]

    async def test_writer_active_before_the_trailing_initialize_runs(
        self,
        redis_enabled,
        lifecycle_patches,
        order_recorder,
        mock_pool_channel,
        recording_update_queue,
    ):
        recording_update_queue(False)

        await NonDurableQueueConsumer.get_instance(
            queue_id="test-queue-id",
            filter_groups=["new"],
            previous_filter_groups=["old"],
        )

        # An update that changed nothing leaves the cache unprimed, so the
        # trailing initialize still runs — and still runs after the bind,
        # which is the b.xbb ordering property on this arm of the path.
        assert order_recorder.events == [
            "ctor",
            "start",
            "register",
            "set_active",
            "update_queue",
            "initialize",
        ]


# Fixed resource ids for the exactly-once suite below. ``filter_group()``
# randomizes the resource id when it is not given one, so a "the client
# re-sent the filters it already had" pair has to name it to really be equal.
_PREVIOUS_RESOURCE_ID = "/proj/assets/before/"
_CHANGED_RESOURCE_ID = "/proj/assets/after/"


@pytest.mark.asyncio
class TestGetInstanceInitializesStorageCacheExactlyOnce:
    """
    Issue b.czw regression: the filter-update reconnect path used to fetch
    the same listing from the Storage API twice — once inside
    ``update_queue`` and once again from ``get_instance``'s trailing
    ``initialize`` call, with identical arguments.

    These drive the REAL ``update_queue`` (only the broker rebind is
    mocked out), because the redundancy lived in how ``get_instance`` and
    ``update_queue`` compose. A suite that stubbed ``update_queue`` would
    pin the stub instead: the stub decides whether the cache was primed,
    which is the very thing under test.

    Both arms are needed. Skipping the trailing call whenever
    ``previous_filter_groups`` was set — the shape the ticket first
    suggested — would leave the cache unprimed on the documented reconnect
    where a client re-sends its current filters, since ``update_queue``
    returns early there without touching the cache.
    """

    @pytest.fixture(autouse=True)
    def get_queue_returns_mock(self, mock_queue, mocker):
        mocker.patch.object(RabbitMQBase, "get_queue", return_value=mock_queue)

    @pytest.fixture(autouse=True)
    def no_broker_rebind(self, mocker):
        """
        Stub the only part of the real ``update_queue`` that needs a broker,
        so everything else in it — including its own ``initialize`` call —
        runs for real.
        """
        return mocker.patch.object(RabbitMQBase, "_update_bindings")

    @pytest.fixture
    def mocked_initialize(self, mocker):
        return mocker.patch.object(storage_permissions, "initialize")

    @pytest.fixture
    def previous_filter_groups(self):
        return [filter_group(resource_id=_PREVIOUS_RESOURCE_ID)]

    @pytest.fixture
    def changed_filter_groups(self):
        return [filter_group(resource_id=_CHANGED_RESOURCE_ID)]

    @pytest.fixture
    def unchanged_filter_groups(self):
        # A separate list object carrying equal content — what a client that
        # re-sends its current filters on reconnect actually produces.
        return [filter_group(resource_id=_PREVIOUS_RESOURCE_ID)]

    async def test_initializes_once_when_the_filters_changed(
        self,
        redis_disabled,
        expected_scope,
        mock_pool_channel,
        mocked_initialize,
        previous_filter_groups,
        changed_filter_groups,
    ):
        await NonDurableQueueConsumer.get_instance(
            queue_id="test-queue-id",
            filter_groups=changed_filter_groups,
            previous_filter_groups=previous_filter_groups,
        )

        mocked_initialize.assert_called_once_with(expected_scope, changed_filter_groups)

    async def test_initializes_once_when_the_filters_are_unchanged(
        self,
        redis_disabled,
        expected_scope,
        mock_pool_channel,
        mocked_initialize,
        previous_filter_groups,
        unchanged_filter_groups,
    ):
        await NonDurableQueueConsumer.get_instance(
            queue_id="test-queue-id",
            filter_groups=unchanged_filter_groups,
            previous_filter_groups=previous_filter_groups,
        )

        mocked_initialize.assert_called_once_with(
            expected_scope, unchanged_filter_groups
        )

    async def test_does_not_rebind_when_the_filters_are_unchanged(
        self,
        redis_disabled,
        mock_pool_channel,
        no_broker_rebind,
        previous_filter_groups,
        unchanged_filter_groups,
    ):
        # Pins WHICH branch the arm above took. Its single ``initialize``
        # call looks the same either way — ``update_queue``'s own call
        # carries those very arguments — so only the absent rebind
        # distinguishes the early return from the changed-filters path.
        await NonDurableQueueConsumer.get_instance(
            queue_id="test-queue-id",
            filter_groups=unchanged_filter_groups,
            previous_filter_groups=previous_filter_groups,
        )

        no_broker_rebind.assert_not_called()

    async def test_rebinds_the_queue_when_the_filters_changed(
        self,
        redis_disabled,
        mock_queue,
        mock_pool_channel,
        no_broker_rebind,
        previous_filter_groups,
        changed_filter_groups,
    ):
        # The control arm: collapsing the two calls into one must not have
        # cost the path its rebind, which is the rest of what update_queue
        # is for.
        await NonDurableQueueConsumer.get_instance(
            queue_id="test-queue-id",
            filter_groups=changed_filter_groups,
            previous_filter_groups=previous_filter_groups,
        )

        no_broker_rebind.assert_called_once_with(
            mock_pool_channel,
            mock_queue,
            previous_filter_groups,
            changed_filter_groups,
        )


@pytest.mark.asyncio
class TestGetInstanceRecordsTheConnectTimeUpdateVerdict:
    """
    Issue b.yvu: ``get_instance`` records what ``update_queue`` said about
    the update the stream arrived with, on ``connect_time_update_changed``.

    That update is applied before the gRPC stream yields anything, so it
    never passes through the servicer's mid-stream loop and gets no
    acknowledgement from there. The servicer reads this attribute instead,
    to acknowledge it on the stream's first response -- so the recorded
    verdict is what a client reconnecting with a token AND an update is
    told about that update.

    It is recorded rather than recomputed on purpose: the "filters are
    unchanged, do nothing" predicate lives in ``update_queue`` and only
    there, and a servicer deriving the equality a second time could
    disagree with the branch that actually ran. These drive the REAL
    ``update_queue`` for the same reason -- only the broker rebind is
    stubbed -- so the recorded value is the verdict production would
    produce, not a stub's idea of it.

    ``None`` is its own state, distinct from ``False``: it means the
    request named no filter set at all -- a reattach carrying a token and
    nothing else -- so there is nothing the servicer could echo back.
    ``True`` covers both connects that DID take the request's set as one to
    establish: the fresh connect, which creates the queue with a binding per
    key derived from that set, and the connect-time update that rebound.
    """

    @pytest.fixture(autouse=True)
    def get_queue_returns_mock(self, mock_queue, mocker):
        mocker.patch.object(RabbitMQBase, "get_queue", return_value=mock_queue)

    @pytest.fixture(autouse=True)
    def no_broker_rebind(self, mocker):
        """
        Stub the only part of the real ``update_queue`` that needs a
        broker, so its verdict is reached the way production reaches it.
        """
        return mocker.patch.object(RabbitMQBase, "_update_bindings")

    @pytest.fixture
    def previous_filter_groups(self):
        return [filter_group(resource_id=_PREVIOUS_RESOURCE_ID)]

    @pytest.fixture
    def changed_filter_groups(self):
        return [filter_group(resource_id=_CHANGED_RESOURCE_ID)]

    @pytest.fixture
    def unchanged_filter_groups(self):
        # A separate list object carrying equal content — what a client
        # that re-sends its current filters on reconnect actually produces.
        return [filter_group(resource_id=_PREVIOUS_RESOURCE_ID)]

    async def test_a_fresh_connect_records_that_it_established_the_requests_set(
        self, redis_disabled, mock_pool_channel, mock_queue, mocker
    ):
        # A fresh connect creates the queue with a binding per key derived
        # from the set the request named, so the set WAS established and the
        # client is entitled to an acknowledgement of it. Recorded as True,
        # the same verdict a connect-time update that rebound reports: the
        # servicer gates its opening acknowledgement on the value not being
        # None, and a fresh connect recording None would leave every new
        # queue's opening set unacknowledged.
        mocker.patch.object(
            RabbitMQBase, "_create_queue", return_value=("test-queue-id", mock_queue)
        )

        consumer = await NonDurableQueueConsumer.get_instance(
            filter_groups=[filter_group(resource_id=_PREVIOUS_RESOURCE_ID)]
        )

        assert consumer.connect_time_update_changed is True

    async def test_a_plain_reconnect_records_no_verdict(
        self, redis_disabled, mock_pool_channel
    ):
        # A token-only reattach asks for no update, so there is nothing to
        # report -- and the servicer sends no acknowledgement for it.
        consumer = await NonDurableQueueConsumer.get_instance(queue_id="test-queue-id")

        assert consumer.connect_time_update_changed is None

    async def test_an_update_that_rebound_the_queue_records_true(
        self,
        redis_disabled,
        mock_pool_channel,
        previous_filter_groups,
        changed_filter_groups,
    ):
        consumer = await NonDurableQueueConsumer.get_instance(
            queue_id="test-queue-id",
            filter_groups=changed_filter_groups,
            previous_filter_groups=previous_filter_groups,
        )

        assert consumer.connect_time_update_changed is True

    async def test_an_update_that_changed_nothing_records_false(
        self,
        redis_disabled,
        mock_pool_channel,
        previous_filter_groups,
        unchanged_filter_groups,
    ):
        # The documented reconnect where a client re-sends the filters it
        # already has. It is False, not None: the request DID name two
        # sets, and the client is entitled to be told they matched.
        consumer = await NonDurableQueueConsumer.get_instance(
            queue_id="test-queue-id",
            filter_groups=unchanged_filter_groups,
            previous_filter_groups=previous_filter_groups,
        )

        assert consumer.connect_time_update_changed is False

    async def test_a_consumer_built_outside_get_instance_records_no_verdict(
        self, mock_queue, mock_chan
    ):
        # The attribute has to exist before ``get_instance`` reaches its
        # update, because the servicer reads it on every stream -- including
        # the ones that never had a connect-time update to record.
        consumer = NonDurableQueueConsumer(mock_queue, "test-queue-id", mock_chan)

        assert consumer.connect_time_update_changed is None


@pytest.mark.asyncio
class TestGetInstanceErrorPath:
    @pytest.fixture(autouse=True)
    def make_initialize_raise(self, mocker, lifecycle_patches):
        async def _boom(*_a, **_kw):
            raise RuntimeError("init boom")

        lifecycle_patches.initialize.side_effect = _boom
        return lifecycle_patches

    @pytest.fixture(autouse=True)
    def get_queue_returns_mock(self, mock_queue, mocker):
        mocker.patch.object(RabbitMQBase, "get_queue", return_value=mock_queue)

    async def test_unregisters_writer_on_error(
        self,
        redis_enabled,
        lifecycle_patches,
        mock_queue,
        mock_pool_channel,
        mock_return_channel,
    ):
        with pytest.raises(RuntimeError, match="init boom"):
            await NonDurableQueueConsumer.get_instance("test-queue-id")

        lifecycle_patches.unregister_writer.assert_called_once_with(
            lifecycle_patches.created_writers[0]
        )

    async def test_drains_writer_with_one_second_timeout_on_error(
        self,
        redis_enabled,
        mocker,
        mock_pool_channel,
        mock_return_channel,
        mock_queue,
    ):
        # Re-patch CacheWriter with a fake whose drain_and_stop captures
        # the timeout argument so we can assert SDD value of 1.0s.
        recorded_timeouts: list[float] = []

        class _CapturingWriter:
            def __init__(self, scope: CacheScope):
                self.scope_key = scope.key

            def start(self) -> None:
                pass

            async def drain_and_stop(self, timeout: float) -> None:
                recorded_timeouts.append(timeout)

        mocker.patch.object(ndq_module, "CacheWriter", side_effect=_CapturingWriter)
        mocker.patch.object(ndq_module, "register_writer")
        mocker.patch.object(ndq_module, "unregister_writer")
        mocker.patch.object(ndq_module, "set_active_queue_id", return_value="TOKEN")
        mocker.patch.object(ndq_module, "reset_active_queue_id")

        async def _boom(*_a, **_kw):
            raise RuntimeError("init boom")

        mocker.patch.object(storage_permissions, "initialize", side_effect=_boom)

        with pytest.raises(RuntimeError, match="init boom"):
            await NonDurableQueueConsumer.get_instance("test-queue-id")

        assert recorded_timeouts == [1.0]

    async def test_resets_context_var_token_on_error(
        self,
        redis_enabled,
        lifecycle_patches,
        mock_pool_channel,
        mock_return_channel,
    ):
        with pytest.raises(RuntimeError, match="init boom"):
            await NonDurableQueueConsumer.get_instance("test-queue-id")

        lifecycle_patches.reset_active_queue_id.assert_called_once_with("TOKEN")

    async def test_releases_channel_on_error(
        self,
        redis_enabled,
        lifecycle_patches,
        mock_pool_channel,
        mock_return_channel,
    ):
        with pytest.raises(RuntimeError, match="init boom"):
            await NonDurableQueueConsumer.get_instance("test-queue-id")

        mock_return_channel.assert_called_once_with(mock_pool_channel)

    async def test_drain_failure_does_not_block_channel_release(
        self,
        redis_enabled,
        lifecycle_patches,
        mock_pool_channel,
        mock_return_channel,
        mocker,
    ):
        # Replace the FakeWriter side_effect with one whose drain_and_stop
        # raises (Redis outage during error-path teardown).
        lifecycle_patches.CacheWriter.side_effect = _DrainRaisingWriter

        with pytest.raises(RuntimeError, match="init boom"):
            await NonDurableQueueConsumer.get_instance("test-queue-id")

        mock_return_channel.assert_called_once_with(mock_pool_channel)


@pytest.mark.asyncio
class TestGetInstanceErrorPathUpdateQueueRaises:
    """
    Bee b.xbb regression: hoisting CacheWriter setup before update_queue
    creates a NEW failure surface — update_queue may raise AFTER the
    writer ContextVar is live. Verify that the error-path teardown still
    unregisters the writer, drains it, resets the ContextVar token, and
    releases the channel, exactly as it does for the pre-existing
    initialize-raises path.
    """

    @pytest.fixture(autouse=True)
    def get_queue_returns_mock(self, mock_queue, mocker):
        mocker.patch.object(RabbitMQBase, "get_queue", return_value=mock_queue)

    @pytest.fixture(autouse=True)
    def make_update_queue_raise(self, mocker):
        async def _boom(self, *_a, **_kw):
            raise RuntimeError("update_queue boom")

        return mocker.patch.object(NonDurableQueueConsumer, "update_queue", _boom)

    async def test_propagates_exception_out_of_get_instance(
        self,
        redis_enabled,
        lifecycle_patches,
        mock_pool_channel,
        mock_return_channel,
    ):
        with pytest.raises(RuntimeError, match="update_queue boom"):
            await NonDurableQueueConsumer.get_instance(
                queue_id="test-queue-id",
                filter_groups=["new"],
                previous_filter_groups=["old"],
            )

    async def test_unregisters_writer_on_error(
        self,
        redis_enabled,
        lifecycle_patches,
        mock_queue,
        mock_pool_channel,
        mock_return_channel,
    ):
        with pytest.raises(RuntimeError, match="update_queue boom"):
            await NonDurableQueueConsumer.get_instance(
                queue_id="test-queue-id",
                filter_groups=["new"],
                previous_filter_groups=["old"],
            )

        lifecycle_patches.unregister_writer.assert_called_once_with(
            lifecycle_patches.created_writers[0]
        )

    async def test_drains_writer_with_one_second_timeout_on_error(
        self,
        redis_enabled,
        mocker,
        mock_pool_channel,
        mock_return_channel,
        mock_queue,
    ):
        # Re-patch CacheWriter with a fake whose drain_and_stop captures
        # the timeout argument so we can assert the SDD value of 1.0s.
        recorded_timeouts: list[float] = []

        class _CapturingWriter:
            def __init__(self, scope: CacheScope):
                self.scope_key = scope.key

            def start(self) -> None:
                pass

            async def drain_and_stop(self, timeout: float) -> None:
                recorded_timeouts.append(timeout)

        mocker.patch.object(ndq_module, "CacheWriter", side_effect=_CapturingWriter)
        mocker.patch.object(ndq_module, "register_writer")
        mocker.patch.object(ndq_module, "unregister_writer")
        mocker.patch.object(ndq_module, "set_active_queue_id", return_value="TOKEN")
        mocker.patch.object(ndq_module, "reset_active_queue_id")

        with pytest.raises(RuntimeError, match="update_queue boom"):
            await NonDurableQueueConsumer.get_instance(
                queue_id="test-queue-id",
                filter_groups=["new"],
                previous_filter_groups=["old"],
            )

        assert recorded_timeouts == [1.0]

    async def test_resets_context_var_token_on_error(
        self,
        redis_enabled,
        lifecycle_patches,
        mock_pool_channel,
        mock_return_channel,
    ):
        with pytest.raises(RuntimeError, match="update_queue boom"):
            await NonDurableQueueConsumer.get_instance(
                queue_id="test-queue-id",
                filter_groups=["new"],
                previous_filter_groups=["old"],
            )

        lifecycle_patches.reset_active_queue_id.assert_called_once_with("TOKEN")

    async def test_releases_channel_on_error(
        self,
        redis_enabled,
        lifecycle_patches,
        mock_pool_channel,
        mock_return_channel,
    ):
        with pytest.raises(RuntimeError, match="update_queue boom"):
            await NonDurableQueueConsumer.get_instance(
                queue_id="test-queue-id",
                filter_groups=["new"],
                previous_filter_groups=["old"],
            )

        mock_return_channel.assert_called_once_with(mock_pool_channel)

    async def test_drain_failure_does_not_block_channel_release(
        self,
        redis_enabled,
        lifecycle_patches,
        mock_pool_channel,
        mock_return_channel,
        mocker,
    ):
        # Replace the FakeWriter side_effect with one whose drain_and_stop
        # raises (Redis outage during error-path teardown). Channel must
        # still be released even though both update_queue and drain raise.
        lifecycle_patches.CacheWriter.side_effect = _DrainRaisingWriter

        with pytest.raises(RuntimeError, match="update_queue boom"):
            await NonDurableQueueConsumer.get_instance(
                queue_id="test-queue-id",
                filter_groups=["new"],
                previous_filter_groups=["old"],
            )

        mock_return_channel.assert_called_once_with(mock_pool_channel)


@pytest.mark.asyncio
class TestConsumeCleanupCacheWriterLifecycle:
    @pytest.fixture
    def consumer_with_writer(self, consumer, mocker):
        """
        Attach a fake CacheWriter + token directly to a consumer so we
        can exercise the consume() finally block without going through
        get_instance.
        """

        class _FakeWriter:
            scope_key = CacheScope("test-queue-name", TEST_PRINCIPAL_REF).key
            # Matches the real writer's property; the cleanup path's
            # drain-failure line names the writer by this, not by the
            # scope key.
            queue_ref = queue_ref_for_name("test-queue-name")
            drain_timeouts: list[float] = []

            async def drain_and_stop(self, timeout: float) -> None:
                _FakeWriter.drain_timeouts.append(timeout)

        # Reset shared list for each parametrization.
        _FakeWriter.drain_timeouts = []

        consumer._cache_writer = _FakeWriter()
        consumer._active_queue_id_token = "TOKEN"
        return consumer

    async def test_drain_called_once_with_one_second_timeout(
        self,
        consumer_with_writer,
        mock_consume,
        mock_return_channel,
        mocker,
    ):
        mocker.patch.object(ndq_module, "unregister_writer")
        mocker.patch.object(ndq_module, "reset_active_queue_id")

        # Capture timeout passed to drain_and_stop. The fixture-attached
        # FakeWriter records into a class-level list; capture before the
        # consumer nulls out self._cache_writer.
        writer = consumer_with_writer._cache_writer

        async for _ in consumer_with_writer.consume():
            continue

        assert writer.__class__.drain_timeouts == [1.0]

    async def test_unregisters_the_writer_itself(
        self,
        consumer_with_writer,
        mock_consume,
        mock_return_channel,
        mocker,
    ):
        # By the writer, not by its key: a same-scope re-attach may already
        # have replaced this entry, and popping by key would remove the
        # live successor instead.
        mocker.patch.object(ndq_module, "unregister_writer")
        mocker.patch.object(ndq_module, "reset_active_queue_id")
        writer = consumer_with_writer._cache_writer

        async for _ in consumer_with_writer.consume():
            continue

        ndq_module.unregister_writer.assert_called_once_with(writer)

    async def test_unregisters_and_resets_after_drain(
        self,
        consumer_with_writer,
        mock_consume,
        mock_return_channel,
        mocker,
    ):
        order: list[str] = []

        async def _drain_capture(timeout):
            order.append("drain")

        consumer_with_writer._cache_writer.drain_and_stop = _drain_capture
        mocker.patch.object(
            ndq_module,
            "unregister_writer",
            side_effect=lambda *_: order.append("unregister"),
        )
        mocker.patch.object(
            ndq_module,
            "reset_active_queue_id",
            side_effect=lambda *_: order.append("reset"),
        )

        async for _ in consumer_with_writer.consume():
            continue

        assert order == ["drain", "unregister", "reset"]

    async def test_passes_one_second_timeout_so_drain_cannot_block_release(
        self,
        consumer_with_writer,
        mock_consume,
        mock_return_channel,
        mocker,
    ):
        """
        The consumer must pass the SDD-mandated 1.0s timeout to
        ``drain_and_stop``. Bounded-timeout enforcement lives inside
        ``CacheWriter`` (covered by Epic 65 tests); here we verify the
        consumer requests the right bound so a hung Redis cannot stall
        channel release indefinitely.
        """
        mocker.patch.object(ndq_module, "unregister_writer")
        mocker.patch.object(ndq_module, "reset_active_queue_id")

        observed_timeouts: list[float] = []

        async def _drain(timeout):
            observed_timeouts.append(timeout)

        consumer_with_writer._cache_writer.drain_and_stop = _drain

        async for _ in consumer_with_writer.consume():
            continue

        assert observed_timeouts == [1.0]
        mock_return_channel.assert_called_once()

    async def test_drain_failure_does_not_prevent_channel_release(
        self,
        consumer_with_writer,
        mock_consume,
        mock_return_channel,
        mocker,
    ):
        mocker.patch.object(ndq_module, "unregister_writer")
        mocker.patch.object(ndq_module, "reset_active_queue_id")

        async def _drain_outage(timeout):
            # Named ``timeout``, because production calls
            # ``drain_and_stop(timeout=...)`` by keyword: a positional-only
            # stand-in raises TypeError instead, and the consumer's blanket
            # ``except Exception`` swallows that just as happily -- so the
            # test would pass without ever exercising the drain failure it
            # claims to pin.
            raise Exception("redis outage")

        consumer_with_writer._cache_writer.drain_and_stop = _drain_outage

        async for _ in consumer_with_writer.consume():
            continue

        mock_return_channel.assert_called_once()

    async def test_no_writer_branch_when_writer_is_none(
        self,
        consumer,
        mock_consume,
        mock_return_channel,
        mocker,
    ):
        """
        When redis is disabled the consumer's _cache_writer stays None;
        cleanup must skip drain/unregister entirely and still release
        the channel.
        """
        unregister_spy = mocker.patch.object(ndq_module, "unregister_writer")
        reset_spy = mocker.patch.object(ndq_module, "reset_active_queue_id")

        assert consumer._cache_writer is None
        assert consumer._active_queue_id_token is None

        async for _ in consumer.consume():
            continue

        unregister_spy.assert_not_called()
        reset_spy.assert_not_called()
        mock_return_channel.assert_called_once()


@pytest.mark.asyncio
class TestConsumeCancellationProtection:
    """
    Regression tests for the asyncio.shield() wrapping around
    cleanup_channel in consume()'s finally block. When the consumer
    task is cancelled mid-iteration, drain_and_stop AND the pool's
    return_channel must both run to completion — otherwise the channel
    leaks back to the pool un-released. asyncio.shield() detaches the cleanup
    coroutine into its own Task so it survives outer-task cancellation
    even under aggressive re-cancellation patterns from gRPC stream
    teardown.
    """

    @pytest.fixture
    def consumer_with_writer(self, consumer):
        class _TrackingWriter:
            scope_key = CacheScope("test-queue-name", TEST_PRINCIPAL_REF).key
            queue_ref = queue_ref_for_name("test-queue-name")

            def __init__(self):
                self.drain_completed = False
                self.drain_started = asyncio.Event()

            async def drain_and_stop(self, timeout: float) -> None:
                self.drain_started.set()
                # Yield to the loop so cancellation has a real chance
                # to interrupt the await if cleanup is not shielded.
                await asyncio.sleep(0.05)
                self.drain_completed = True

        consumer._cache_writer = _TrackingWriter()
        consumer._active_queue_id_token = "TOKEN"
        return consumer

    @pytest.fixture
    def stalling_consume(self, mocker):
        """
        Replace _consume_with_delayed_acks with a generator that yields
        one message then awaits forever, so the consumer's consume()
        async-generator parks at its own ``yield message`` line and the
        driving task is parked mid-iteration when the test cancels it.
        """

        async def _gen(*_args, **_kwargs):
            yield EventMessage({"a": "b"})
            await asyncio.Event().wait()

        mocker.patch.object(
            RabbitMQBase,
            "_consume_with_delayed_acks",
            side_effect=_gen,
        )

    async def test_cleanup_channel_runs_to_completion_when_consumer_task_is_cancelled(
        self,
        consumer_with_writer,
        stalling_consume,
        mock_return_channel,
        mocker,
    ):
        mocker.patch.object(ndq_module, "unregister_writer")
        mocker.patch.object(ndq_module, "reset_active_queue_id")

        writer = consumer_with_writer._cache_writer
        message_received = asyncio.Event()

        async def _drive():
            async for _ in consumer_with_writer.consume():
                # Park here until the test cancels the task; the
                # underlying generator's next iteration awaits forever.
                message_received.set()
                await asyncio.Event().wait()

        task = asyncio.create_task(_drive())

        # Wait until the consumer has yielded its first message and the
        # driver is parked, so cancellation lands during iteration (not
        # before consume() has even started).
        await asyncio.wait_for(message_received.wait(), timeout=1.0)

        # Cancel once to start cleanup, then re-cancel after the drain
        # is in flight — this is the realistic stream-close pattern
        # where a connection drops while teardown is already running.
        # asyncio.shield() must keep cleanup from being abandoned.
        task.cancel()
        await asyncio.wait_for(writer.drain_started.wait(), timeout=1.0)
        task.cancel()

        with pytest.raises(asyncio.CancelledError):
            await asyncio.wait_for(task, timeout=1.0)

        # Give the loop time to finish the (shielded) cleanup. The
        # shielded inner Task may still be running after the awaiter
        # has unwound with CancelledError.
        deadline = asyncio.get_event_loop().time() + 1.0
        while asyncio.get_event_loop().time() < deadline:
            if writer.drain_completed and mock_return_channel.called:
                break
            await asyncio.sleep(0.01)

        assert writer.drain_completed is True
        mock_return_channel.assert_called_once()


# --------------------------------------------------------------------------
# Reporting the storage events dropped against unauthorized entries when the
# stream ends (b.54z)
# --------------------------------------------------------------------------


#: The client folder the unauthorized entry is for.
STORAGE_DIR = "/tenant-x/private-project/new-folder/"


def _seed_unauthorized_entry(cache: StorageCache) -> None:
    """Put a 403 entry with ``DROPPED`` unreported drops into ``cache``."""
    cache[storage_permissions_cache._cache_key(STORAGE_DIR)] = (
        unauthorized_entry_with_drops(STORAGE_DIR)
    )


async def _consume_to_end(consumer: NonDurableQueueConsumer) -> list[EventMessage]:
    return [message async for message in consumer.consume()]


@pytest.mark.asyncio
class TestConsumeReportsDroppedStorageEvents:
    """
    When the stream ends, consume's teardown reports the drops against every
    unauthorized entry still in the stream's storage-permissions cache. A
    reporting failure never stops the channel cleanup that follows it.
    """

    @pytest.fixture
    def consumer_with_drops(self, consumer, expected_scope):
        cache = StorageCache(expected_scope)
        _seed_unauthorized_entry(cache)
        consumer._storage_cache = cache
        return consumer

    @pytest.fixture
    def reporting_fails(self, mocker):
        return mocker.patch.object(
            storage_permissions_cache,
            "report_dropped_events_at_stream_end",
            side_effect=RuntimeError("report failed"),
        )

    async def test_reports_the_drops_at_stream_end(
        self, consumer_with_drops, mock_consume, mock_return_channel, drop_reports
    ):
        await _consume_to_end(consumer_with_drops)

        [record] = drop_reports()
        assert record.dropped_events == DROPPED

    async def test_a_second_teardown_reports_nothing_more(
        self, consumer_with_drops, mock_consume, mock_return_channel, drop_reports
    ):
        await _consume_to_end(consumer_with_drops)
        await _consume_to_end(consumer_with_drops)

        assert len(drop_reports()) == 1

    async def test_reports_before_the_channel_is_released(
        self, consumer_with_drops, mock_consume, mock_return_channel, mocker
    ):
        order: list[str] = []
        mocker.patch.object(
            storage_permissions_cache,
            "report_dropped_events_at_stream_end",
            side_effect=lambda _cache: order.append("report"),
        )
        mock_return_channel.side_effect = lambda _channel: order.append("release")

        await _consume_to_end(consumer_with_drops)

        assert order == ["report", "release"]

    async def test_a_reporting_failure_does_not_raise_out_of_teardown(
        self, consumer_with_drops, mock_consume, mock_return_channel, reporting_fails
    ):
        messages = await _consume_to_end(consumer_with_drops)

        assert len(messages) == 3

    async def test_a_reporting_failure_still_releases_the_channel(
        self, consumer_with_drops, mock_consume, mock_return_channel, reporting_fails
    ):
        await _consume_to_end(consumer_with_drops)

        mock_return_channel.assert_called_once()

    async def test_a_reporting_failure_line_does_not_name_the_reconnect_token(
        self,
        mock_queue,
        mock_chan,
        expected_scope,
        mock_consume,
        mock_return_channel,
        reporting_fails,
        mocker,
    ):
        mock_debug = mocker.patch.object(logger, "debug")
        consumer = NonDurableQueueConsumer(mock_queue, SECRET_TOKEN, mock_chan)
        consumer._storage_cache = StorageCache(expected_scope)

        await _consume_to_end(consumer)

        [failure_line] = mock_debug.call_args_list
        assert SECRET_TOKEN not in str(failure_line)

    async def test_reports_nothing_without_a_storage_cache(
        self, consumer, mock_consume, mock_return_channel, mocker
    ):
        # Storage permissions disabled: get_instance captured no cache.
        report = mocker.patch.object(
            storage_permissions_cache, "report_dropped_events_at_stream_end"
        )

        await _consume_to_end(consumer)

        report.assert_not_called()


@pytest.mark.asyncio
class TestGetInstanceCapturesTheStreamsStorageCache:
    """
    get_instance holds on to the cache the stream's authorization checks
    use, so consume's teardown reaches it even from a context where the
    cache's ContextVar is not set (an async generator closed by the event
    loop's finalizer, for one).
    """

    @pytest.fixture
    def initialize_builds_a_cache_with_drops(self, mocker):
        """
        Make storage-permissions initialization create the stream's cache in
        the ContextVar, as the real one does, and seed it with drops.
        """

        async def _initialize(scope, _filter_groups):
            _seed_unauthorized_entry(await StorageCache.get_instance(scope))

        mocker.patch.object(storage_permissions, "initialize", side_effect=_initialize)

    @pytest.fixture
    def reconnected_consumer(
        self,
        mock_queue,
        mock_pool_channel,
        redis_disabled,
        initialize_builds_a_cache_with_drops,
        mocker,
    ):
        """
        Reconnect through get_instance in its own task, so the ContextVar it
        sets stays in that task's context and out of the test's. Keyword
        arguments are passed through, so a test can make the reconnect a
        filter update.
        """
        mocker.patch.object(RabbitMQBase, "get_queue", return_value=mock_queue)
        # The only part of the real update_queue that needs a broker.
        mocker.patch.object(RabbitMQBase, "_update_bindings")

        async def _run(**filter_update):
            return await asyncio.create_task(
                NonDurableQueueConsumer.get_instance("test-queue-id", **filter_update)
            )

        return _run

    async def test_stream_end_reports_the_drops_in_the_streams_cache(
        self, reconnected_consumer, mock_consume, mock_return_channel, drop_reports
    ):
        consumer = await reconnected_consumer()

        # An empty context: no storage-permissions cache is reachable
        # through the ContextVar while the teardown runs.
        await asyncio.create_task(
            _consume_to_end(consumer), context=contextvars.Context()
        )

        [record] = drop_reports()
        assert record.dropped_events == DROPPED

    async def test_stream_end_reports_the_drops_after_a_filter_update_reconnect(
        self, reconnected_consumer, mock_consume, mock_return_channel, drop_reports
    ):
        # The filters changed, so the real update_queue initializes the cache
        # and get_instance's own initialize call is skipped. The cache is
        # captured on this path too.
        consumer = await reconnected_consumer(
            filter_groups=[filter_group(resource_id=_CHANGED_RESOURCE_ID)],
            previous_filter_groups=[filter_group(resource_id=_PREVIOUS_RESOURCE_ID)],
        )

        await asyncio.create_task(
            _consume_to_end(consumer), context=contextvars.Context()
        )

        [record] = drop_reports()
        assert record.dropped_events == DROPPED
