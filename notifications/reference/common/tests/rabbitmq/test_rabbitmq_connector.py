# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from asyncio import Future
from unittest.mock import AsyncMock

import aio_pika
import pytest
import pytest_asyncio

from notification_common.config.common_settings import common_settings
from notification_common.rabbitmq import rabbitmq_connector
from notification_common.rabbitmq.channel_pool import (
    ChannelPool,
    ChannelPoolConfig,
    DurableChannelPoolConfig,
)
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

# A pool size no shipped default equals, so a config that follows it can only
# have picked it up from the global that was changed.
SENTINEL_POOL_SIZE = 4242

# Every common_settings value the per-pool configs read. A test that changes
# any of them goes through preserve_channel_settings.
CHANNEL_SETTING_NAMES = [
    "rabbitmq_channel_pool_size",
    "rabbitmq_channel_pool_min_size",
    "rabbitmq_channel_prefetch_count",
    "rabbitmq_channel_wait_ms",
    "rabbitmq_channel_client_limit",
    "rabbitmq_channel_overflow_enabled",
    "rabbitmq_durable_channel_pool_size",
    "rabbitmq_durable_channel_pool_min_size",
    "rabbitmq_durable_channel_prefetch_count",
    "rabbitmq_durable_channel_wait_ms",
    "rabbitmq_durable_channel_client_limit",
]


@pytest_asyncio.fixture
async def mock_chan(mocker):
    return mocker.patch.object(aio_pika.robust_channel, "RobustChannel", autospec=True)


@pytest_asyncio.fixture(autouse=True)
async def mock_conn(mock_chan, mocker):
    conn = mocker.patch.object(
        aio_pika.robust_connection, "RobustConnection", autospec=True
    )
    conn.channel.return_value = Future()
    conn.channel.return_value.set_result(mock_chan)
    mocker.patch.object(aio_pika, "connect_robust", return_value=conn)

    return conn


@pytest.fixture
def rabbitmq():
    return RabbitMQConnector()


@pytest.fixture
def preserve_channel_settings():
    """
    Restore every channel-pool setting a test changes.

    Deliberately not autouse: the tests that assert the shipped defaults must
    see the real process-global values, not a fixture's substitutes.
    """
    original_values = {
        name: getattr(common_settings, name) for name in CHANNEL_SETTING_NAMES
    }

    yield

    for name, value in original_values.items():
        setattr(common_settings, name, value)


@pytest.fixture
def small_pool_config():
    """A cheap two-channel pool config, so pool stats are easy to read."""
    return ChannelPoolConfig(pool_size=2, min_size=1, client_limit=1)


@pytest.mark.asyncio
class TestConnection:
    @pytest.fixture(autouse=True)
    def setup_connection(self, rabbitmq, mock_conn):
        rabbitmq._connection = mock_conn
        mock_conn.is_closed = False

    async def test_returns_the_connection(self, rabbitmq, mock_conn):
        rabbitmq._connection = mock_conn
        assert await rabbitmq.connection() == mock_conn

    async def test_doesnt_reconnect(self, rabbitmq, mocker):
        spy = mocker.spy(rabbitmq, "connect")
        await rabbitmq.connection()
        spy.assert_not_called()

    class TestWhenConnIsClosed:
        @pytest.fixture(autouse=True)
        def close_connection(self, rabbitmq):
            rabbitmq._connection.is_closed = True

        async def test_returns_a_new_connection(self, rabbitmq, mocker):
            new_conn = AsyncMock()
            mocker.patch.object(aio_pika, "connect_robust", return_value=new_conn)
            assert await rabbitmq.connection() == new_conn

        async def test_it_reconnects(self, rabbitmq, mocker):
            spy = mocker.patch.object(rabbitmq, "connect")
            await rabbitmq.connection()
            spy.assert_called_once()


@pytest.mark.asyncio
class TestGetChannel:
    @pytest.fixture(autouse=True)
    def setup_pool(self, rabbitmq, mock_conn):
        rabbitmq._connection = mock_conn
        mock_conn.is_closed = False

        # Mock ChannelPool. Both pools are set, and to distinct mocks, so
        # which pool a channel came from is provable rather than assumed.
        rabbitmq._channel_pool = AsyncMock()
        rabbitmq._durable_channel_pool = AsyncMock()

    async def test_yields_channel_from_pool(self, rabbitmq, mock_chan):
        rabbitmq._channel_pool.get_channel.return_value = "test-channel"

        async with rabbitmq.get_channel() as channel:
            assert channel == "test-channel"

    async def test_returns_channel_to_pool_after_use(self, rabbitmq, mock_chan):
        rabbitmq._channel_pool.get_channel.return_value = mock_chan

        async with rabbitmq.get_channel():
            pass  # Use the channel

        # noinspection PyUnresolvedReferences
        rabbitmq._channel_pool.return_channel.assert_called_once_with(mock_chan)

    async def test_reconnects_if_connection_closed(
        self, rabbitmq, mock_chan, mock_conn, mocker
    ):
        mock_conn.is_closed = True
        spy = mocker.patch.object(rabbitmq, "connect")
        rabbitmq._channel_pool.get_channel.return_value = mock_chan

        async with rabbitmq.get_channel():
            pass

        spy.assert_called_once()

    async def test_reconnects_if_no_connection(self, rabbitmq, mock_chan, mocker):
        rabbitmq._connection = None
        spy = mocker.patch.object(rabbitmq, "connect")
        rabbitmq._channel_pool.get_channel.return_value = mock_chan

        async with rabbitmq.get_channel():
            pass

        spy.assert_called_once()

    async def test_yields_the_non_durable_pools_channel(self, rabbitmq):
        rabbitmq._channel_pool.get_channel.return_value = "non-durable-channel"
        rabbitmq._durable_channel_pool.get_channel.return_value = "durable-channel"

        async with rabbitmq.get_channel() as channel:
            assert channel == "non-durable-channel"

    async def test_doesnt_acquire_from_the_durable_pool(self, rabbitmq, mock_chan):
        rabbitmq._channel_pool.get_channel.return_value = mock_chan

        async with rabbitmq.get_channel():
            pass

        # noinspection PyUnresolvedReferences
        rabbitmq._durable_channel_pool.get_channel.assert_not_called()

    class TestWhenExceptionOccurs:
        async def test_still_returns_channel_to_pool(self, rabbitmq, mock_chan):
            rabbitmq._channel_pool.get_channel.return_value = mock_chan

            with pytest.raises(ValueError):
                async with rabbitmq.get_channel():
                    raise ValueError("Test exception")

            # noinspection PyUnreachableCode,PyUnresolvedReferences
            rabbitmq._channel_pool.return_channel.assert_called_once_with(mock_chan)

    class TestWhenDurablePoolRequested:
        """
        The durable opt-in acquires from the durable pool. The return is
        offered to every pool and lands by ownership (proven on real pools in
        TestReturnChannel), so these assert the owning pool saw the channel.
        """

        async def test_yields_the_durable_pools_channel(self, rabbitmq):
            rabbitmq._channel_pool.get_channel.return_value = "non-durable-channel"
            rabbitmq._durable_channel_pool.get_channel.return_value = "durable-channel"

            async with rabbitmq.get_channel(durable_pool=True) as channel:
                assert channel == "durable-channel"

        async def test_doesnt_acquire_from_the_non_durable_pool(
            self, rabbitmq, mock_chan
        ):
            rabbitmq._durable_channel_pool.get_channel.return_value = mock_chan

            async with rabbitmq.get_channel(durable_pool=True):
                pass

            # noinspection PyUnresolvedReferences
            rabbitmq._channel_pool.get_channel.assert_not_called()

        async def test_returns_channel_to_durable_pool_after_use(
            self, rabbitmq, mock_chan
        ):
            rabbitmq._durable_channel_pool.get_channel.return_value = mock_chan

            async with rabbitmq.get_channel(durable_pool=True):
                pass  # Use the channel

            # noinspection PyUnresolvedReferences
            rabbitmq._durable_channel_pool.return_channel.assert_called_once_with(
                mock_chan
            )

        async def test_returns_channel_to_durable_pool_on_exception(
            self, rabbitmq, mock_chan
        ):
            rabbitmq._durable_channel_pool.get_channel.return_value = mock_chan

            with pytest.raises(ValueError):
                async with rabbitmq.get_channel(durable_pool=True):
                    raise ValueError("Test exception")

            # noinspection PyUnreachableCode,PyUnresolvedReferences
            rabbitmq._durable_channel_pool.return_channel.assert_called_once_with(
                mock_chan
            )

        async def test_reconnects_if_connection_closed(
            self, rabbitmq, mock_chan, mock_conn, mocker
        ):
            mock_conn.is_closed = True
            spy = mocker.patch.object(rabbitmq, "connect")
            rabbitmq._durable_channel_pool.get_channel.return_value = mock_chan

            async with rabbitmq.get_channel(durable_pool=True):
                pass

            spy.assert_called_once()

        async def test_reconnects_if_no_connection(self, rabbitmq, mock_chan, mocker):
            rabbitmq._connection = None
            spy = mocker.patch.object(rabbitmq, "connect")
            rabbitmq._durable_channel_pool.get_channel.return_value = mock_chan

            async with rabbitmq.get_channel(durable_pool=True):
                pass

            spy.assert_called_once()


@pytest.mark.asyncio
class TestConnect:
    @pytest.fixture
    def first_pool(self):
        """The instance the patched ChannelPool hands out on its 1st call."""
        return AsyncMock()

    @pytest.fixture
    def second_pool(self):
        """The instance the patched ChannelPool hands out on its 2nd call."""
        return AsyncMock()

    @pytest.fixture
    def mock_pool_class(self, mocker, first_pool, second_pool):
        """
        Patch ChannelPool where connect() looks it up, handing out two
        distinguishable instances in construction order.
        """
        pool_class = mocker.patch.object(rabbitmq_connector, "ChannelPool")
        pool_class.side_effect = [first_pool, second_pool]
        return pool_class

    async def test_creates_connection(self, rabbitmq, mock_conn):
        assert rabbitmq._connection is None
        await rabbitmq.connect()
        assert rabbitmq._connection == mock_conn

    async def test_creates_two_channel_pools(self, rabbitmq, mock_pool_class):
        await rabbitmq.connect()

        assert mock_pool_class.call_count == 2

    async def test_creates_both_pools_on_the_same_connection(
        self, rabbitmq, mock_pool_class
    ):
        await rabbitmq.connect()

        connections = [call.args[0] for call in mock_pool_class.call_args_list]
        assert connections == [rabbitmq._connection, rabbitmq._connection]

    async def test_stores_the_non_durable_pool(
        self, rabbitmq, mock_pool_class, first_pool
    ):
        await rabbitmq.connect()

        assert rabbitmq._channel_pool is first_pool

    async def test_stores_the_durable_pool(
        self, rabbitmq, mock_pool_class, second_pool
    ):
        await rabbitmq.connect()

        assert rabbitmq._durable_channel_pool is second_pool

    async def test_the_two_pools_are_different_objects(self, rabbitmq, mock_pool_class):
        await rabbitmq.connect()

        assert rabbitmq._channel_pool is not rabbitmq._durable_channel_pool

    async def test_initializes_the_non_durable_pool(
        self, rabbitmq, mock_pool_class, first_pool
    ):
        await rabbitmq.connect()

        # An uninitialized pool silently skips its min-size pre-creation.
        first_pool.initialize.assert_awaited_once()

    async def test_initializes_the_durable_pool(
        self, rabbitmq, mock_pool_class, second_pool
    ):
        await rabbitmq.connect()

        second_pool.initialize.assert_awaited_once()

    async def test_constructs_the_non_durable_pool_without_a_config(
        self, rabbitmq, mock_pool_class
    ):
        await rabbitmq.connect()

        # The absence of a config argument IS the compatibility proof: the
        # non-durable pool keeps following the process-global settings. Only
        # the log-label name keyword rides along.
        non_durable_call = mock_pool_class.call_args_list[0]
        assert non_durable_call.args == (rabbitmq._connection,)
        assert set(non_durable_call.kwargs) == {"name"}

    async def test_constructs_the_durable_pool_with_a_durable_config(
        self, rabbitmq, mock_pool_class
    ):
        await rabbitmq.connect()

        durable_call = mock_pool_class.call_args_list[1]
        assert isinstance(durable_call.args[1], DurableChannelPoolConfig)

    async def test_names_each_pool_after_its_stats_key(self, rabbitmq, mock_pool_class):
        await rabbitmq.connect()

        # The log label and the pool_stats_by_pool() key are the same string,
        # so a stats reading and an incident timeline name the same pool the
        # same way.
        names = [call.kwargs["name"] for call in mock_pool_class.call_args_list]
        assert names == [
            RabbitMQConnector.NON_DURABLE_POOL_NAME,
            RabbitMQConnector.DURABLE_POOL_NAME,
        ]

    async def test_calls_declare_exchange(self, rabbitmq, mocker):
        spy = mocker.spy(rabbitmq, "declare_exchange")
        await rabbitmq.connect()
        spy.assert_called_once_with()

    class TestPoolConfiguration:
        """
        The limits the two real pools end up with. No class patch here: the
        configs are read back through the real ChannelPool.config property.
        """

        @pytest_asyncio.fixture
        async def connected(self, rabbitmq):
            """A connector holding its two real, initialized pools."""
            await rabbitmq.connect()
            return rabbitmq

        @pytest_asyncio.fixture
        async def connected_after_global_change(
            self, rabbitmq, preserve_channel_settings
        ):
            """
            The same, connected with a non-durable global changed first.

            The value equals no shipped default, so a config carrying it can
            only have followed the global that was changed.
            """
            common_settings.rabbitmq_channel_pool_size = SENTINEL_POOL_SIZE
            await rabbitmq.connect()
            return rabbitmq

        async def test_non_durable_pool_follows_the_non_durable_globals(
            self, connected
        ):
            config = connected._channel_pool.config
            assert (
                config.pool_size,
                config.min_size,
                config.client_limit,
                config.prefetch_count,
                config.wait_ms,
            ) == (
                common_settings.rabbitmq_channel_pool_size,
                common_settings.rabbitmq_channel_pool_min_size,
                common_settings.rabbitmq_channel_client_limit,
                common_settings.rabbitmq_channel_prefetch_count,
                common_settings.rabbitmq_channel_wait_ms,
            )

        async def test_durable_pool_follows_the_durable_globals(self, connected):
            config = connected._durable_channel_pool.config
            assert (
                config.pool_size,
                config.min_size,
                config.client_limit,
                config.prefetch_count,
                config.wait_ms,
            ) == (
                common_settings.rabbitmq_durable_channel_pool_size,
                common_settings.rabbitmq_durable_channel_pool_min_size,
                common_settings.rabbitmq_durable_channel_client_limit,
                common_settings.rabbitmq_durable_channel_prefetch_count,
                common_settings.rabbitmq_durable_channel_wait_ms,
            )

        async def test_durable_checkouts_are_exclusive(self, connected):
            assert connected._durable_channel_pool.config.client_limit == 1

        async def test_non_durable_pool_follows_a_changed_global(
            self, connected_after_global_change
        ):
            config = connected_after_global_change._channel_pool.config
            assert config.pool_size == SENTINEL_POOL_SIZE

        async def test_durable_pool_ignores_a_changed_non_durable_global(
            self, connected_after_global_change
        ):
            config = connected_after_global_change._durable_channel_pool.config
            assert config.pool_size == (
                common_settings.rabbitmq_durable_channel_pool_size
            )

    class TestWhenAlreadyConnected:
        @pytest.fixture(autouse=True)
        def connect(self, mock_conn, rabbitmq):
            rabbitmq._connection = mock_conn
            rabbitmq._channel_pool = AsyncMock()
            rabbitmq._durable_channel_pool = AsyncMock()

            rabbitmq._connection.is_closed = False

        async def test_doesnt_create_connection(self, rabbitmq, mocker):
            spy = mocker.patch.object(aio_pika, "connect_robust")
            await rabbitmq.connect()
            spy.assert_not_called()

        async def test_doesnt_create_channel_pool(self, rabbitmq, mock_pool_class):
            await rabbitmq.connect()

            mock_pool_class.assert_not_called()

        async def test_keeps_the_non_durable_pool(self, rabbitmq, mock_pool_class):
            existing_pool = rabbitmq._channel_pool

            await rabbitmq.connect()

            assert rabbitmq._channel_pool is existing_pool

        async def test_keeps_the_durable_pool(self, rabbitmq, mock_pool_class):
            existing_pool = rabbitmq._durable_channel_pool

            await rabbitmq.connect()

            assert rabbitmq._durable_channel_pool is existing_pool

    class TestWhenOnlyTheNonDurablePoolExists:
        """
        A connector left holding one of the two pools converges on the next
        connect() instead of keeping the missing pool missing.
        """

        @pytest.fixture(autouse=True)
        def partial_state(self, mock_conn, rabbitmq):
            rabbitmq._connection = mock_conn
            rabbitmq._connection.is_closed = False
            rabbitmq._channel_pool = AsyncMock()
            rabbitmq._durable_channel_pool = None

        async def test_creates_exactly_one_pool(self, rabbitmq, mock_pool_class):
            await rabbitmq.connect()

            assert mock_pool_class.call_count == 1

        async def test_assigns_the_new_pool_to_the_durable_attribute(
            self, rabbitmq, mock_pool_class, first_pool
        ):
            await rabbitmq.connect()

            assert rabbitmq._durable_channel_pool is first_pool

        async def test_keeps_the_existing_non_durable_pool(
            self, rabbitmq, mock_pool_class
        ):
            existing_pool = rabbitmq._channel_pool

            await rabbitmq.connect()

            assert rabbitmq._channel_pool is existing_pool

    class TestWhenOnlyTheDurablePoolExists:
        @pytest.fixture(autouse=True)
        def partial_state(self, mock_conn, rabbitmq):
            rabbitmq._connection = mock_conn
            rabbitmq._connection.is_closed = False
            rabbitmq._channel_pool = None
            rabbitmq._durable_channel_pool = AsyncMock()

        async def test_creates_exactly_one_pool(self, rabbitmq, mock_pool_class):
            await rabbitmq.connect()

            assert mock_pool_class.call_count == 1

        async def test_assigns_the_new_pool_to_the_non_durable_attribute(
            self, rabbitmq, mock_pool_class, first_pool
        ):
            await rabbitmq.connect()

            assert rabbitmq._channel_pool is first_pool

        async def test_keeps_the_existing_durable_pool(self, rabbitmq, mock_pool_class):
            existing_pool = rabbitmq._durable_channel_pool

            await rabbitmq.connect()

            assert rabbitmq._durable_channel_pool is existing_pool


@pytest.mark.asyncio
class TestClose:
    @pytest.fixture(autouse=True)
    def setup_rabbitmq(self, rabbitmq, mock_conn):
        mock_conn.is_closed = False
        rabbitmq._connection = mock_conn

        rabbitmq._channel_pool = AsyncMock()
        rabbitmq._durable_channel_pool = AsyncMock()

    async def test_closes_connection(self, rabbitmq):
        await rabbitmq.close()
        # noinspection PyUnresolvedReferences
        rabbitmq._connection.close.assert_called_once()

    async def test_closes_channel_pool(self, rabbitmq):
        # Need to ensure there's a pool to close
        mock_pool = AsyncMock()
        rabbitmq._channel_pool = mock_pool

        await rabbitmq.close()
        mock_pool.close_all.assert_called_once()

    async def test_closes_durable_channel_pool(self, rabbitmq):
        mock_pool = AsyncMock()
        rabbitmq._durable_channel_pool = mock_pool

        await rabbitmq.close()
        mock_pool.close_all.assert_called_once()

    async def test_clears_channel_pool_reference(self, rabbitmq):
        await rabbitmq.close()
        assert rabbitmq._channel_pool is None

    async def test_clears_durable_channel_pool_reference(self, rabbitmq):
        await rabbitmq.close()
        assert rabbitmq._durable_channel_pool is None

    async def test_doesnt_error_when_called_twice(self, rabbitmq):
        await rabbitmq.close()

        # Both pools are already None on the second pass; should not raise.
        await rabbitmq.close()

    class TestWhenAlreadyClosed:
        @pytest.fixture(autouse=True)
        def close(self, rabbitmq, mock_conn):
            mock_conn.is_closed = True

        async def test_doesnt_close_connection(self, rabbitmq):
            await rabbitmq.close()
            # noinspection PyUnresolvedReferences
            rabbitmq._connection.close.assert_not_called()

    class TestWhenNoChannelPool:
        @pytest.fixture(autouse=True)
        def no_pool(self, rabbitmq):
            rabbitmq._channel_pool = None

        async def test_doesnt_error_on_missing_pool(self, rabbitmq):
            # Should not raise exception
            await rabbitmq.close()

        async def test_still_closes_the_durable_pool(self, rabbitmq):
            durable_pool = rabbitmq._durable_channel_pool

            await rabbitmq.close()

            durable_pool.close_all.assert_called_once()

    class TestWhenNoDurableChannelPool:
        @pytest.fixture(autouse=True)
        def no_durable_pool(self, rabbitmq):
            rabbitmq._durable_channel_pool = None

        async def test_doesnt_error_on_missing_pool(self, rabbitmq):
            # Should not raise exception
            await rabbitmq.close()

        async def test_still_closes_the_non_durable_pool(self, rabbitmq):
            non_durable_pool = rabbitmq._channel_pool

            await rabbitmq.close()

            non_durable_pool.close_all.assert_called_once()

    class TestWhenNoPoolsAtAll:
        @pytest.fixture(autouse=True)
        def no_pools(self, rabbitmq):
            rabbitmq._channel_pool = None
            rabbitmq._durable_channel_pool = None

        async def test_doesnt_error_on_missing_pools(self, rabbitmq):
            # Should not raise exception
            await rabbitmq.close()

        async def test_still_closes_the_connection(self, rabbitmq):
            await rabbitmq.close()

            # noinspection PyUnresolvedReferences
            rabbitmq._connection.close.assert_called_once()


class TestPrepareToShutdown:
    @pytest.fixture(autouse=True)
    def reset(self, rabbitmq, mock_conn, mock_chan):
        rabbitmq.closing = False
        yield
        rabbitmq.closing = False

    def test_sets_closing_to_true(self, rabbitmq):
        assert rabbitmq.closing is False
        rabbitmq.prepare_to_shutdown()
        assert rabbitmq.closing is True


@pytest.mark.asyncio
class TestDeclareExchange:
    """
    The declaration borrows its channel from one explicitly-assigned pool:
    the non-durable one, so the durable pool stays at zero open channels
    until a durable checkout actually needs one.
    """

    @pytest.fixture(autouse=True)
    def setup_pools(self, rabbitmq, mock_chan):
        rabbitmq._channel_pool = AsyncMock()
        rabbitmq._channel_pool.get_channel.return_value = mock_chan
        rabbitmq._durable_channel_pool = AsyncMock()
        rabbitmq._durable_channel_pool.get_channel.return_value = mock_chan

    async def test_declares_topic_exchange(self, rabbitmq, mock_chan):
        await rabbitmq.declare_exchange()

        # noinspection PyUnresolvedReferences
        rabbitmq._channel_pool.get_channel.assert_called_once()
        mock_chan.declare_exchange.assert_called_once_with(
            "events", type="topic", durable=True
        )
        # noinspection PyUnresolvedReferences
        rabbitmq._channel_pool.return_channel.assert_called_once_with(mock_chan)

    async def test_doesnt_acquire_from_the_durable_pool(self, rabbitmq):
        await rabbitmq.declare_exchange()

        # noinspection PyUnresolvedReferences
        rabbitmq._durable_channel_pool.get_channel.assert_not_called()

    async def test_doesnt_return_a_channel_to_the_durable_pool(self, rabbitmq):
        await rabbitmq.declare_exchange()

        # noinspection PyUnresolvedReferences
        rabbitmq._durable_channel_pool.return_channel.assert_not_called()

    async def test_handles_no_pool(self, rabbitmq):
        rabbitmq._channel_pool = None
        # Should not raise exception
        await rabbitmq.declare_exchange()

    async def test_handles_no_pools_at_all(self, rabbitmq):
        rabbitmq._channel_pool = None
        rabbitmq._durable_channel_pool = None
        # Should not raise exception
        await rabbitmq.declare_exchange()


@pytest.mark.asyncio
class TestGetChannelDirect:
    @pytest.fixture(autouse=True)
    def setup_pool(self, rabbitmq, mock_conn):
        rabbitmq._connection = mock_conn
        mock_conn.is_closed = False

        # Mock ChannelPool. Both pools are set, and to distinct mocks, so
        # which pool a channel came from is provable rather than assumed.
        rabbitmq._channel_pool = AsyncMock()
        rabbitmq._durable_channel_pool = AsyncMock()

    async def test_returns_channel_from_pool(self, rabbitmq, mock_chan):
        rabbitmq._channel_pool.get_channel.return_value = "test-channel"

        result = await rabbitmq.get_channel_direct()

        assert result == "test-channel"
        # noinspection PyUnresolvedReferences
        rabbitmq._channel_pool.get_channel.assert_called_once()

    async def test_returns_the_non_durable_pools_channel(self, rabbitmq):
        rabbitmq._channel_pool.get_channel.return_value = "non-durable-channel"
        rabbitmq._durable_channel_pool.get_channel.return_value = "durable-channel"

        result = await rabbitmq.get_channel_direct()

        assert result == "non-durable-channel"

    async def test_doesnt_acquire_from_the_durable_pool(self, rabbitmq, mock_chan):
        rabbitmq._channel_pool.get_channel.return_value = mock_chan

        await rabbitmq.get_channel_direct()

        # noinspection PyUnresolvedReferences
        rabbitmq._durable_channel_pool.get_channel.assert_not_called()

    async def test_reconnects_if_connection_closed(
        self, rabbitmq, mock_chan, mock_conn, mocker
    ):
        mock_conn.is_closed = True
        spy = mocker.patch.object(rabbitmq, "connect")
        rabbitmq._channel_pool.get_channel.return_value = mock_chan

        result = await rabbitmq.get_channel_direct()

        spy.assert_called_once()
        assert result == mock_chan

    async def test_reconnects_if_no_connection(self, rabbitmq, mock_chan, mocker):
        rabbitmq._connection = None
        spy = mocker.patch.object(rabbitmq, "connect")
        rabbitmq._channel_pool.get_channel.return_value = mock_chan

        result = await rabbitmq.get_channel_direct()

        spy.assert_called_once()
        assert result == mock_chan

    class TestWhenDurablePoolRequested:
        async def test_returns_the_durable_pools_channel(self, rabbitmq):
            rabbitmq._channel_pool.get_channel.return_value = "non-durable-channel"
            rabbitmq._durable_channel_pool.get_channel.return_value = "durable-channel"

            result = await rabbitmq.get_channel_direct(durable_pool=True)

            assert result == "durable-channel"

        async def test_doesnt_acquire_from_the_non_durable_pool(
            self, rabbitmq, mock_chan
        ):
            rabbitmq._durable_channel_pool.get_channel.return_value = mock_chan

            await rabbitmq.get_channel_direct(durable_pool=True)

            # noinspection PyUnresolvedReferences
            rabbitmq._channel_pool.get_channel.assert_not_called()

        async def test_reconnects_if_connection_closed(
            self, rabbitmq, mock_chan, mock_conn, mocker
        ):
            mock_conn.is_closed = True
            spy = mocker.patch.object(rabbitmq, "connect")
            rabbitmq._durable_channel_pool.get_channel.return_value = mock_chan

            result = await rabbitmq.get_channel_direct(durable_pool=True)

            spy.assert_called_once()
            assert result == mock_chan

        async def test_reconnects_if_no_connection(self, rabbitmq, mock_chan, mocker):
            rabbitmq._connection = None
            spy = mocker.patch.object(rabbitmq, "connect")
            rabbitmq._durable_channel_pool.get_channel.return_value = mock_chan

            result = await rabbitmq.get_channel_direct(durable_pool=True)

            spy.assert_called_once()
            assert result == mock_chan


@pytest.mark.asyncio
class TestReturnChannel:
    @pytest.fixture(autouse=True)
    def setup_pool(self, rabbitmq, mock_conn):
        rabbitmq._connection = mock_conn
        mock_conn.is_closed = False

        # Mock ChannelPool. Both pools are set, and to distinct mocks, so a
        # return can be asserted positively against the owning pool.
        rabbitmq._channel_pool = AsyncMock()
        rabbitmq._durable_channel_pool = AsyncMock()

    async def test_returns_channel_to_pool(self, rabbitmq, mock_chan):
        await rabbitmq.return_channel(mock_chan)

        # noinspection PyUnresolvedReferences
        rabbitmq._channel_pool.return_channel.assert_called_once_with(mock_chan)

    async def test_offers_the_channel_to_the_durable_pool_too(
        self, rabbitmq, mock_chan
    ):
        # Every pool is offered the channel; the one that owns it keeps it and
        # the other ignores it, so the caller never restates where it borrowed.
        await rabbitmq.return_channel(mock_chan)

        # noinspection PyUnresolvedReferences
        rabbitmq._durable_channel_pool.return_channel.assert_called_once_with(mock_chan)

    async def test_handles_no_channel_pool(self, rabbitmq, mock_chan):
        rabbitmq._channel_pool = None

        # Should not raise exception
        await rabbitmq.return_channel(mock_chan)

    async def test_handles_no_durable_channel_pool(self, rabbitmq, mock_chan):
        rabbitmq._durable_channel_pool = None

        # Should not raise exception
        await rabbitmq.return_channel(mock_chan)

    async def test_handles_no_pools_at_all(self, rabbitmq, mock_chan):
        rabbitmq._channel_pool = None
        rabbitmq._durable_channel_pool = None

        # Should not raise exception
        await rabbitmq.return_channel(mock_chan)

    class TestWithRealPools:
        """
        Ownership dispatch, proven on two real ChannelPools over one
        connection: a channel returned through the connector lands back in
        the pool that handed it out, and never in the other one.
        """

        @pytest_asyncio.fixture(autouse=True)
        async def real_pools(
            self, rabbitmq, mock_connection_with_factory, small_pool_config
        ):
            non_durable = ChannelPool(
                mock_connection_with_factory,
                small_pool_config,
                name=RabbitMQConnector.NON_DURABLE_POOL_NAME,
            )
            durable = ChannelPool(
                mock_connection_with_factory,
                small_pool_config,
                name=RabbitMQConnector.DURABLE_POOL_NAME,
            )
            await non_durable.initialize()
            await durable.initialize()

            rabbitmq._channel_pool = non_durable
            rabbitmq._durable_channel_pool = durable

        @pytest_asyncio.fixture
        async def busy_pools(self, rabbitmq):
            """
            One channel checked out of each pool.

            Both pools hold a client, so a return credited to the wrong pool
            moves stats that must not move, rather than moving nothing.
            """
            non_durable_channel = await rabbitmq._channel_pool.get_channel()
            durable_channel = await rabbitmq._durable_channel_pool.get_channel()
            return non_durable_channel, durable_channel

        @pytest.fixture
        def non_durable_channel(self, busy_pools):
            return busy_pools[0]

        @pytest.fixture
        def durable_channel(self, busy_pools):
            return busy_pools[1]

        async def test_non_durable_channel_frees_its_client_slot(
            self, rabbitmq, non_durable_channel
        ):
            await rabbitmq.return_channel(non_durable_channel)

            assert rabbitmq._channel_pool.pool_stats()["total_client_count"] == 0

        async def test_non_durable_channel_becomes_available_again(
            self, rabbitmq, non_durable_channel
        ):
            checked_out = rabbitmq._channel_pool.pool_stats()["available_channel_count"]

            await rabbitmq.return_channel(non_durable_channel)

            returned = rabbitmq._channel_pool.pool_stats()["available_channel_count"]
            assert returned == checked_out + 1

        async def test_non_durable_return_leaves_the_durable_pool_alone(
            self, rabbitmq, non_durable_channel
        ):
            before = rabbitmq._durable_channel_pool.pool_stats()

            await rabbitmq.return_channel(non_durable_channel)

            assert rabbitmq._durable_channel_pool.pool_stats() == before

        async def test_durable_channel_frees_its_client_slot(
            self, rabbitmq, durable_channel
        ):
            await rabbitmq.return_channel(durable_channel)

            durable_stats = rabbitmq._durable_channel_pool.pool_stats()
            assert durable_stats["total_client_count"] == 0

        async def test_durable_channel_becomes_available_again(
            self, rabbitmq, durable_channel
        ):
            checked_out = rabbitmq._durable_channel_pool.pool_stats()[
                "available_channel_count"
            ]

            await rabbitmq.return_channel(durable_channel)

            returned = rabbitmq._durable_channel_pool.pool_stats()[
                "available_channel_count"
            ]
            assert returned == checked_out + 1

        async def test_durable_return_leaves_the_non_durable_pool_alone(
            self, rabbitmq, durable_channel
        ):
            before = rabbitmq._channel_pool.pool_stats()

            await rabbitmq.return_channel(durable_channel)

            assert rabbitmq._channel_pool.pool_stats() == before

        async def test_foreign_channel_doesnt_raise(self, rabbitmq, busy_pools):
            # Owned by neither pool: silently dropped rather than
            # mis-credited to whichever pool was tried first.
            await rabbitmq.return_channel(AsyncMock())

        async def test_foreign_channel_leaves_the_non_durable_pool_alone(
            self, rabbitmq, busy_pools
        ):
            before = rabbitmq._channel_pool.pool_stats()

            await rabbitmq.return_channel(AsyncMock())

            assert rabbitmq._channel_pool.pool_stats() == before

        async def test_foreign_channel_leaves_the_durable_pool_alone(
            self, rabbitmq, busy_pools
        ):
            before = rabbitmq._durable_channel_pool.pool_stats()

            await rabbitmq.return_channel(AsyncMock())

            assert rabbitmq._durable_channel_pool.pool_stats() == before


@pytest.mark.asyncio
class TestGetTemporaryChannel:
    """
    Tests for get_temporary_channel, which creates a channel directly from
    the connection, bypassing the pool. This is useful for operations that
    may leave the channel with stale cache (e.g., queue deletion).
    """

    @pytest.fixture
    def temp_mock_chan(self, mocker):
        """A separate mock channel for temporary channel tests."""
        chan = mocker.MagicMock()
        chan.is_closed = False
        chan.close = AsyncMock()
        return chan

    @pytest.fixture(autouse=True)
    def setup_connection(self, rabbitmq, mock_conn, temp_mock_chan):
        rabbitmq._connection = mock_conn
        mock_conn.is_closed = False
        mock_conn.channel = AsyncMock(return_value=temp_mock_chan)

    async def test_creates_channel_from_connection(
        self, rabbitmq, mock_conn, temp_mock_chan
    ):
        async with rabbitmq.get_temporary_channel() as channel:
            assert channel == temp_mock_chan

        mock_conn.channel.assert_called_once()

    async def test_closes_channel_after_use(self, rabbitmq, temp_mock_chan):
        temp_mock_chan.is_closed = False

        async with rabbitmq.get_temporary_channel():
            pass

        temp_mock_chan.close.assert_called_once()

    async def test_does_not_use_pool(self, rabbitmq):
        mock_pool = AsyncMock()
        rabbitmq._channel_pool = mock_pool

        async with rabbitmq.get_temporary_channel():
            pass

        mock_pool.get_channel.assert_not_called()
        mock_pool.return_channel.assert_not_called()

    async def test_does_not_use_durable_pool(self, rabbitmq):
        mock_pool = AsyncMock()
        rabbitmq._durable_channel_pool = mock_pool

        async with rabbitmq.get_temporary_channel():
            pass

        mock_pool.get_channel.assert_not_called()
        mock_pool.return_channel.assert_not_called()

    async def test_reconnects_if_connection_closed(
        self, rabbitmq, mock_conn, temp_mock_chan, mocker
    ):
        mock_conn.is_closed = True
        spy = mocker.patch.object(rabbitmq, "connect")

        async with rabbitmq.get_temporary_channel():
            pass

        spy.assert_called_once()

    async def test_reconnects_if_no_connection(
        self, rabbitmq, mock_conn, temp_mock_chan, mocker
    ):
        rabbitmq._connection = None

        # connect() will set up the connection with our mock
        async def fake_connect():
            rabbitmq._connection = mock_conn
            mock_conn.is_closed = False

        spy = mocker.patch.object(rabbitmq, "connect", side_effect=fake_connect)

        async with rabbitmq.get_temporary_channel():
            pass

        spy.assert_called_once()

    async def test_closes_channel_even_on_exception(self, rabbitmq, temp_mock_chan):
        temp_mock_chan.is_closed = False

        with pytest.raises(ValueError):
            async with rabbitmq.get_temporary_channel():
                raise ValueError("Test exception")

        temp_mock_chan.close.assert_called_once()

    async def test_closes_channel_even_when_is_closed_is_true(
        self, rabbitmq, temp_mock_chan
    ):
        # is_closed=True at finally time does not mean "no cleanup
        # needed": RobustChannel auto-reopens after a broker-side
        # close, and is_closed can flip True transiently while a new
        # underlying AMQP channel is being opened by _on_close's
        # restore() callback. Calling close() unconditionally clears
        # the restore signal and prevents the auto-reopen from
        # producing an orphan channel at the broker.
        temp_mock_chan.is_closed = True

        async with rabbitmq.get_temporary_channel():
            pass

        temp_mock_chan.close.assert_called_once()

    async def test_swallows_exceptions_from_close(self, rabbitmq, temp_mock_chan):
        # A double-close or a channel that's already gone server-side
        # may raise; the finally must not propagate that.
        temp_mock_chan.is_closed = False
        temp_mock_chan.close.side_effect = RuntimeError("already closed")

        # Should not raise.
        async with rabbitmq.get_temporary_channel():
            pass

        temp_mock_chan.close.assert_called_once()


@pytest.mark.asyncio
class TestGetExchange:
    async def test_gets_exchange_from_channel(self, rabbitmq, mock_chan):
        mock_exchange = AsyncMock()
        mock_chan.get_exchange.return_value = mock_exchange

        result = await rabbitmq.get_exchange(mock_chan)

        mock_chan.get_exchange.assert_called_once_with("events")
        assert result == mock_exchange


@pytest.fixture
def non_durable_stats():
    """The non-durable pool's stats, distinguishable from the durable pool's."""
    return {"total_client_count": 5}


@pytest.fixture
def durable_stats():
    """The durable pool's stats, distinguishable from the non-durable pool's."""
    return {"total_client_count": 3}


@pytest.fixture
def stats_pools(rabbitmq, mocker, non_durable_stats, durable_stats):
    """
    Both pools set to MagicMocks with distinguishable stats.

    MagicMock, not AsyncMock: pool_stats() is synchronous, and an AsyncMock
    would hand back a coroutine that passes an identity assertion vacuously.
    """
    rabbitmq._channel_pool = mocker.MagicMock()
    rabbitmq._channel_pool.pool_stats.return_value = non_durable_stats
    rabbitmq._durable_channel_pool = mocker.MagicMock()
    rabbitmq._durable_channel_pool.pool_stats.return_value = durable_stats


class TestPoolStats:
    def test_returns_stats_from_channel_pool(self, rabbitmq, mocker):
        mock_pool = mocker.MagicMock()
        mock_pool.pool_stats.return_value = {"total_client_count": 5}
        rabbitmq._channel_pool = mock_pool

        result = rabbitmq.pool_stats()

        assert result == {"total_client_count": 5}
        mock_pool.pool_stats.assert_called_once()

    def test_returns_empty_dict_when_no_pool(self, rabbitmq):
        rabbitmq._channel_pool = None

        result = rabbitmq.pool_stats()

        assert result == {}

    class TestWhenBothPoolsExist:
        def test_reports_the_non_durable_pool(
            self, rabbitmq, stats_pools, non_durable_stats
        ):
            # The flat shape is the compatibility guarantee: capacity
            # surfaces reading it by key keep seeing non-durable capacity.
            assert rabbitmq.pool_stats() == non_durable_stats

        def test_doesnt_read_the_durable_pool(self, rabbitmq, stats_pools):
            rabbitmq.pool_stats()

            # noinspection PyUnresolvedReferences
            rabbitmq._durable_channel_pool.pool_stats.assert_not_called()


class TestPoolStatsByPool:
    def test_pool_names_are_the_documented_keys(self):
        # Asserted once here; every other test references the constants.
        assert (
            RabbitMQConnector.NON_DURABLE_POOL_NAME,
            RabbitMQConnector.DURABLE_POOL_NAME,
        ) == ("non_durable", "durable")

    def test_returns_empty_dict_when_no_pools(self, rabbitmq):
        rabbitmq._channel_pool = None
        rabbitmq._durable_channel_pool = None

        assert rabbitmq.pool_stats_by_pool() == {}

    class TestWhenBothPoolsExist:
        def test_keys_are_exactly_the_two_pool_names(self, rabbitmq, stats_pools):
            assert set(rabbitmq.pool_stats_by_pool()) == {
                RabbitMQConnector.NON_DURABLE_POOL_NAME,
                RabbitMQConnector.DURABLE_POOL_NAME,
            }

        def test_reports_the_non_durable_pools_own_stats(
            self, rabbitmq, stats_pools, non_durable_stats
        ):
            result = rabbitmq.pool_stats_by_pool()

            assert result[RabbitMQConnector.NON_DURABLE_POOL_NAME] == non_durable_stats

        def test_reports_the_durable_pools_own_stats(
            self, rabbitmq, stats_pools, durable_stats
        ):
            result = rabbitmq.pool_stats_by_pool()

            assert result[RabbitMQConnector.DURABLE_POOL_NAME] == durable_stats

        def test_doesnt_aggregate_the_two_pools(
            self, rabbitmq, stats_pools, non_durable_stats, durable_stats
        ):
            # 5 + 3 must appear nowhere: capacity surfaces key on the
            # non-durable pool alone, so a summed value would be a lie.
            summed = (
                non_durable_stats["total_client_count"]
                + durable_stats["total_client_count"]
            )
            reported = [
                value
                for pool_stats in rabbitmq.pool_stats_by_pool().values()
                for value in pool_stats.values()
            ]
            assert summed not in reported

        def test_reads_the_non_durable_pool_once(self, rabbitmq, stats_pools):
            rabbitmq.pool_stats_by_pool()

            # noinspection PyUnresolvedReferences
            rabbitmq._channel_pool.pool_stats.assert_called_once()

        def test_reads_the_durable_pool_once(self, rabbitmq, stats_pools):
            rabbitmq.pool_stats_by_pool()

            # noinspection PyUnresolvedReferences
            rabbitmq._durable_channel_pool.pool_stats.assert_called_once()

        def test_agrees_with_pool_stats(self, rabbitmq, stats_pools):
            by_pool = rabbitmq.pool_stats_by_pool()

            assert (
                by_pool[RabbitMQConnector.NON_DURABLE_POOL_NAME]
                == rabbitmq.pool_stats()
            )

    class TestWhenOnlyTheNonDurablePoolExists:
        @pytest.fixture(autouse=True)
        def one_pool(self, rabbitmq, stats_pools):
            rabbitmq._durable_channel_pool = None

        def test_reports_only_the_present_pool(self, rabbitmq, non_durable_stats):
            assert rabbitmq.pool_stats_by_pool() == {
                RabbitMQConnector.NON_DURABLE_POOL_NAME: non_durable_stats
            }

    class TestWhenOnlyTheDurablePoolExists:
        @pytest.fixture(autouse=True)
        def one_pool(self, rabbitmq, stats_pools):
            rabbitmq._channel_pool = None

        def test_reports_only_the_present_pool(self, rabbitmq, durable_stats):
            assert rabbitmq.pool_stats_by_pool() == {
                RabbitMQConnector.DURABLE_POOL_NAME: durable_stats
            }
