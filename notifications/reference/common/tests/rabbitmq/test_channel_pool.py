# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
from unittest.mock import AsyncMock

import pytest
import pytest_asyncio

from notification_common.config.common_settings import common_settings
from notification_common.errors.errors import ChannelNotAvailableError
from notification_common.rabbitmq.channel_pool import (
    ChannelInfo,
    ChannelPool,
    ChannelPoolConfig,
    DurableChannelPoolConfig,
)

# A per-pool wait short enough that an exhausted pool fails the test fast.
SHORT_WAIT_MS = 50

# The logger the pool emits through.
POOL_LOGGER_NAME = "notification_common.rabbitmq.channel_pool"

# mock_connection, mock_channel_factory, and mock_connection_with_factory live in
# tests/rabbitmq/conftest.py because more than one module in this directory
# needs them. The fixtures below stay file-local on purpose: reset_settings is
# autouse and would change the conditions for the directory's other modules.


@pytest_asyncio.fixture
async def channel_pool(mock_connection_with_factory):
    """Create a ChannelPool instance for testing."""
    return ChannelPool(mock_connection_with_factory)


@pytest_asyncio.fixture(autouse=True)
async def reset_settings():
    """Pin a deliberate, minimal test baseline for each test.

    These are NOT the shipped configuration: the shipped non-durable defaults
    are pool_size 100, client_limit 8, prefetch 15. The values pinned below
    (pool_size 25, min_size 20, prefetch 500, wait_ms 500, client_limit 1) are
    a small, legible baseline chosen so the pool arithmetic in this module is
    easy to reason about (e.g. max_client_capacity == 25 == 25 * 1). Keep
    client_limit at 1 here: assertions like max_client_capacity == 25 are
    load-bearing on it. A test that wants the shipped packing density sets the
    global explicitly.
    """
    original_values = {}
    for attr in [
        "rabbitmq_channel_pool_size",
        "rabbitmq_channel_pool_min_size",
        "rabbitmq_channel_prefetch_count",
        "rabbitmq_channel_wait_ms",
        "rabbitmq_channel_client_limit",
        "rabbitmq_channel_overflow_enabled",
        # The durable globals are only saved and restored, never overridden:
        # TestDurableChannelPoolConfig asserts against whatever they ship as,
        # and mutates them in place, so it needs the restore but not a reset.
        "rabbitmq_durable_channel_pool_size",
        "rabbitmq_durable_channel_pool_min_size",
        "rabbitmq_durable_channel_prefetch_count",
        "rabbitmq_durable_channel_wait_ms",
        "rabbitmq_durable_channel_client_limit",
    ]:
        original_values[attr] = getattr(common_settings, attr)

    # Set test defaults
    common_settings.rabbitmq_channel_pool_size = 25
    common_settings.rabbitmq_channel_pool_min_size = 20
    common_settings.rabbitmq_channel_prefetch_count = 500
    common_settings.rabbitmq_channel_wait_ms = 500
    common_settings.rabbitmq_channel_client_limit = 1
    # Pinned to the shipped default rather than off, so the module's baseline
    # describes production: the non-durable pool is two-tier out of the box.
    # A test that wants today's one-tier semantics turns it off explicitly (see
    # strict_configured_pool). There is deliberately no durable counterpart --
    # DurableChannelPoolConfig hard-overrides the property instead.
    common_settings.rabbitmq_channel_overflow_enabled = True

    yield

    # Restore original values
    for attr, value in original_values.items():
        setattr(common_settings, attr, value)


@pytest_asyncio.fixture
async def reinitialize_pool():
    """Factory fixture to reinitialize a pool with custom settings or config."""

    async def _reinitialize(pool, config=None, **settings):
        """
        Reinitialize the pool with the given global settings or per-pool config.

        Without a config the pool itself is emptied and reinitialized, so the
        given global settings take effect. With a config a fresh pool carrying
        that config is built over the same connection and returned, leaving the
        globals untouched.
        """
        # Set any provided settings
        for key, value in settings.items():
            setattr(common_settings, key, value)

        if config is not None:
            # noinspection PyProtectedMember
            pool = ChannelPool(pool._connection, config)
        else:
            # Clear the pool
            pool._initialized = False
            # noinspection PyProtectedMember
            pool._all_channels.clear()
            # noinspection PyProtectedMember
            while not pool._available_channels.empty():
                # noinspection PyProtectedMember
                pool._available_channels.get_nowait()

        # Reinitialize
        await pool.initialize()
        return pool

    return _reinitialize


@pytest_asyncio.fixture
async def configured_pool(channel_pool, reinitialize_pool):
    """
    Factory fixture for an initialized pool built from an explicit config.

    The globals stay at the defaults pinned by reset_settings, so any value a
    test passes here is honored only if the pool read its own config. Those
    defaults leave rabbitmq_channel_overflow_enabled on, so the pool this
    builds is overflow-enabled and packs past client_limit once every channel
    is open and full. Use strict_configured_pool for a non-durable pool that
    keeps the one-tier semantics, or durable_configured_pool for a durable pool
    that is strict by construction.
    """

    async def _configured_pool(**config_values):
        """Build and initialize a pool with the given per-pool config values."""
        return await reinitialize_pool(
            channel_pool, config=ChannelPoolConfig(**config_values)
        )

    return _configured_pool


@pytest_asyncio.fixture
async def strict_configured_pool(configured_pool):
    """
    Factory fixture for a configured pool with the overflow band turned off.

    Same shape as configured_pool, but the process-global overflow switch is
    off, so the pool refuses with ChannelNotAvailableError once it is full
    rather than packing past client_limit. reset_settings restores the global
    afterwards, so this fixture needs no teardown of its own.
    """

    async def _strict_configured_pool(**config_values):
        """Build and initialize an overflow-disabled pool."""
        common_settings.rabbitmq_channel_overflow_enabled = False
        return await configured_pool(**config_values)

    return _strict_configured_pool


@pytest_asyncio.fixture
async def durable_configured_pool(channel_pool, reinitialize_pool):
    """
    Factory fixture for an initialized pool built from a durable config.

    The durable pool is strict by construction -- DurableChannelPoolConfig
    hard-overrides overflow_enabled to False -- so this builds a pool that
    stays one-tier no matter how the non-durable global is set. Unset values
    follow the durable rabbitmq_durable_channel_* globals, which reset_settings
    saves and restores but never overrides.
    """

    async def _durable_configured_pool(**config_values):
        """Build and initialize a pool with the given durable config values."""
        return await reinitialize_pool(
            channel_pool, config=DurableChannelPoolConfig(**config_values)
        )

    return _durable_configured_pool


@pytest_asyncio.fixture
async def single_channel_pool(channel_pool, reinitialize_pool):
    """A channel pool configured with only 1 channel and 1 client per channel."""
    return await reinitialize_pool(
        channel_pool,
        rabbitmq_channel_pool_min_size=1,
        rabbitmq_channel_pool_size=1,
        rabbitmq_channel_client_limit=1,
    )


@pytest_asyncio.fixture
async def multi_client_single_channel_pool(channel_pool, reinitialize_pool):
    """A channel pool with 1 channel that allows multiple clients per channel."""
    return await reinitialize_pool(
        channel_pool,
        rabbitmq_channel_pool_min_size=1,
        rabbitmq_channel_pool_size=1,
        rabbitmq_channel_client_limit=2,
    )


@pytest_asyncio.fixture
async def small_pool(channel_pool, reinitialize_pool):
    """A small channel pool with 2 channels."""
    return await reinitialize_pool(
        channel_pool,
        rabbitmq_channel_pool_min_size=2,
        rabbitmq_channel_pool_size=2,
        rabbitmq_channel_client_limit=3,
    )


@pytest_asyncio.fixture
async def timeout_pool(channel_pool, reinitialize_pool):
    """
    A strict channel pool with a short timeout for testing timeout scenarios.

    Strictness source: the process-global overflow switch is off, so a full
    pool blocks on the wait tier and then raises instead of packing past
    client_limit. reset_settings restores the global afterwards.
    """
    return await reinitialize_pool(
        channel_pool,
        rabbitmq_channel_pool_min_size=1,
        rabbitmq_channel_pool_size=1,
        rabbitmq_channel_wait_ms=100,
        rabbitmq_channel_overflow_enabled=False,
    )


@pytest.fixture
def channel_collector():
    """Fixture that provides a helper to collect multiple channels."""

    def _collect_channels(pool, count):
        """Collect the specified number of channels from the pool."""

        async def _inner():
            channels = []
            for _ in range(count):
                channel = await pool.get_channel()
                channels.append(channel)
            return channels

        return _inner()

    return _collect_channels


@pytest.fixture
def exhausted_pool(channel_pool, channel_collector):
    """A factory fixture that creates a pool and exhausts all its channels."""

    async def _create_exhausted_pool(pool_size=None):
        """Create and exhaust a pool with the specified size."""
        if pool_size:
            common_settings.rabbitmq_channel_pool_min_size = pool_size
            common_settings.rabbitmq_channel_pool_size = pool_size
            # Reinitialize
            channel_pool._initialized = False
            channel_pool._all_channels.clear()
            while not channel_pool._available_channels.empty():
                channel_pool._available_channels.get_nowait()
            await channel_pool.initialize()

        # Exhaust all channels
        channels = await channel_collector(channel_pool, pool_size or 20)
        return channel_pool, channels

    return _create_exhausted_pool


@pytest.fixture
def pool_log_messages(real_get_logger, caplog):
    """
    Capture the messages the channel pool logs during the test.

    ``real_get_logger`` restores the stdlib ``logging.getLogger`` so caplog
    captures at all; this hands back a callable returning the messages logged
    so far.
    """
    # DEBUG so the capture spans every level the pool logs at, and the
    # every-line assertions really do see every line.
    caplog.set_level(logging.DEBUG, logger=POOL_LOGGER_NAME)

    def _messages():
        return [
            record.getMessage()
            for record in caplog.records
            if record.name == POOL_LOGGER_NAME
        ]

    return _messages


@pytest.fixture
def mock_channel_info_factory(mock_channel_factory):
    """Factory fixture for creating mock ChannelInfo objects."""

    def _create_channel_info(is_closed=False, client_count=0):
        """Create a mock ChannelInfo with specified properties."""
        mock_channel = mock_channel_factory()
        mock_channel.is_closed = is_closed
        channel_info = ChannelInfo(mock_channel)
        # Set client count by incrementing
        for _ in range(client_count):
            channel_info.increment_clients()
        return channel_info

    return _create_channel_info


# Each per-pool config property and the global setting it falls back to.
CONFIG_PROPERTY_TO_SETTING = [
    ("pool_size", "rabbitmq_channel_pool_size"),
    ("min_size", "rabbitmq_channel_pool_min_size"),
    ("client_limit", "rabbitmq_channel_client_limit"),
    ("prefetch_count", "rabbitmq_channel_prefetch_count"),
    ("wait_ms", "rabbitmq_channel_wait_ms"),
]


class TestChannelPoolConfig:
    """Test the ChannelPoolConfig per-pool configuration object."""

    @pytest.mark.parametrize("prop, setting", CONFIG_PROPERTY_TO_SETTING)
    def test_unset_value_resolves_to_the_global_setting(self, prop, setting):
        config = ChannelPoolConfig()

        assert getattr(config, prop) == getattr(common_settings, setting)

    @pytest.mark.parametrize("prop, setting", CONFIG_PROPERTY_TO_SETTING)
    def test_unset_value_is_bound_late(self, prop, setting):
        """A global changed after construction is still picked up."""
        config = ChannelPoolConfig()

        setattr(common_settings, setting, 77)

        assert getattr(config, prop) == 77

    @pytest.mark.parametrize("prop, setting", CONFIG_PROPERTY_TO_SETTING)
    def test_explicit_value_wins_over_a_later_global_change(self, prop, setting):
        config = ChannelPoolConfig(**{prop: 7})

        setattr(common_settings, setting, 77)

        assert getattr(config, prop) == 7

    def test_sibling_of_an_explicit_value_still_follows_the_global(self):
        """One explicit field does not freeze the rest of the instance."""
        config = ChannelPoolConfig(pool_size=7)

        common_settings.rabbitmq_channel_client_limit = 77

        assert config.client_limit == 77

    @pytest.mark.parametrize("prop, setting", CONFIG_PROPERTY_TO_SETTING)
    def test_explicit_zero_wins_over_the_global(self, prop, setting):
        """An explicit 0 is a value, not an absent value."""
        config = ChannelPoolConfig(**{prop: 0})

        assert getattr(config, prop) == 0

    # overflow_enabled is deliberately absent from CONFIG_PROPERTY_TO_SETTING:
    # the shared parametrizations set the global to 77 and construct with 7 / 0,
    # neither of which means anything for a bool carried by a global-only
    # property. It gets these standalone tests instead.

    @pytest.mark.parametrize("enabled", [True, False])
    def test_overflow_enabled_resolves_to_the_global_setting(self, enabled):
        common_settings.rabbitmq_channel_overflow_enabled = enabled

        assert ChannelPoolConfig().overflow_enabled is enabled

    def test_overflow_enabled_is_bound_late(self):
        """A global flipped after construction is still picked up."""
        config = ChannelPoolConfig()

        common_settings.rabbitmq_channel_overflow_enabled = False

        assert config.overflow_enabled is False

    def test_overflow_enabled_has_no_per_pool_constructor_field(self):
        """
        Global-only by design (SDD SR-1.5 wiring decision 2).

        There is no constructor parameter and no backing field, so a pool can
        never pin its own overflow behavior away from the process global.
        """
        with pytest.raises(TypeError):
            ChannelPoolConfig(overflow_enabled=False)


# Each durable per-pool config property and the durable global it falls back to.
DURABLE_CONFIG_PROPERTY_TO_SETTING = [
    ("pool_size", "rabbitmq_durable_channel_pool_size"),
    ("min_size", "rabbitmq_durable_channel_pool_min_size"),
    ("client_limit", "rabbitmq_durable_channel_client_limit"),
    ("prefetch_count", "rabbitmq_durable_channel_prefetch_count"),
    ("wait_ms", "rabbitmq_durable_channel_wait_ms"),
]


class TestDurableChannelPoolConfig:
    """
    Test the DurableChannelPoolConfig per-pool configuration object.

    Same contract as ChannelPoolConfig, except that every unset value follows
    the durable rabbitmq_durable_channel_* global rather than the non-durable
    rabbitmq_channel_* one that reset_settings pins to its own test values.
    """

    @pytest.mark.parametrize("prop, setting", DURABLE_CONFIG_PROPERTY_TO_SETTING)
    def test_unset_value_resolves_to_the_durable_global_setting(self, prop, setting):
        config = DurableChannelPoolConfig()

        assert getattr(config, prop) == getattr(common_settings, setting)

    @pytest.mark.parametrize("prop, setting", DURABLE_CONFIG_PROPERTY_TO_SETTING)
    def test_unset_value_ignores_the_non_durable_global(self, prop, setting):
        """The durable config never reads the non-durable rabbitmq_channel_*."""
        non_durable_setting = setting.replace("_durable", "")
        setattr(common_settings, non_durable_setting, 8)
        config = DurableChannelPoolConfig()

        assert getattr(config, prop) == getattr(common_settings, setting)

    def test_client_limit_stays_exclusive_when_the_non_durable_limit_packs(self):
        """The headline isolation case: packing non-durable never packs durable."""
        # 8 is now the SHIPPED non-durable client_limit, so this is the
        # production isolation case, not a hypothetical one (the reset_settings
        # baseline pins the global to 1 first, so this is still a real mutation).
        common_settings.rabbitmq_channel_client_limit = 8

        assert DurableChannelPoolConfig().client_limit == 1

    # overflow_enabled is deliberately absent from
    # DURABLE_CONFIG_PROPERTY_TO_SETTING: test_unset_value_ignores_the_non_
    # durable_global derives a durable setting name by string surgery, and there
    # is no rabbitmq_durable_channel_overflow_enabled to derive.

    def test_overflow_stays_off_when_the_non_durable_global_is_on(self):
        """The durable pool never joins the SR-1.5 overflow band."""
        common_settings.rabbitmq_channel_overflow_enabled = True

        assert DurableChannelPoolConfig().overflow_enabled is False

    def test_overflow_cannot_be_turned_on_per_pool(self):
        """No constructor field, so no durable pool can opt itself in."""
        with pytest.raises(TypeError):
            DurableChannelPoolConfig(overflow_enabled=True)

    @pytest.mark.parametrize("prop, setting", DURABLE_CONFIG_PROPERTY_TO_SETTING)
    def test_unset_value_is_bound_late(self, prop, setting):
        """A durable global changed after construction is still picked up."""
        config = DurableChannelPoolConfig()

        setattr(common_settings, setting, 77)

        assert getattr(config, prop) == 77

    @pytest.mark.parametrize("prop, setting", DURABLE_CONFIG_PROPERTY_TO_SETTING)
    def test_explicit_value_wins_over_a_later_durable_global_change(
        self, prop, setting
    ):
        config = DurableChannelPoolConfig(**{prop: 7})

        setattr(common_settings, setting, 77)

        assert getattr(config, prop) == 7

    def test_sibling_of_an_explicit_value_still_follows_the_durable_global(self):
        """One explicit field does not freeze the rest of the instance."""
        config = DurableChannelPoolConfig(pool_size=7)

        common_settings.rabbitmq_durable_channel_client_limit = 77

        assert config.client_limit == 77

    @pytest.mark.parametrize("prop, setting", DURABLE_CONFIG_PROPERTY_TO_SETTING)
    def test_explicit_zero_wins_over_the_durable_global(self, prop, setting):
        """An explicit 0 is a value, not an absent value."""
        config = DurableChannelPoolConfig(**{prop: 0})

        assert getattr(config, prop) == 0

    @pytest.mark.asyncio
    async def test_pool_opens_no_channels_at_the_default_durable_min_size(
        self, mock_connection_with_factory
    ):
        """The durable pool stays cold until a durable checkout needs a channel."""
        pool = ChannelPool(mock_connection_with_factory, DurableChannelPoolConfig())

        await pool.initialize()

        mock_connection_with_factory.channel.assert_not_called()


@pytest.mark.asyncio
class TestStrictPoolFixtures:
    """
    Prove the two strict pool fixtures while the module default is overflow on.

    These are fixture contracts, not admission behavior: each fixture has to
    hand back an initialized pool that still refuses past its limits even
    though reset_settings pins rabbitmq_channel_overflow_enabled to True. The
    strict admission tests that build on them assert the refusals in detail.
    """

    async def test_strict_configured_pool_is_initialized(self, strict_configured_pool):
        pool = await strict_configured_pool(pool_size=1, min_size=1, client_limit=1)

        assert pool.pool_stats()["initialized"] is True

    async def test_strict_configured_pool_refuses_past_its_limits(
        self, strict_configured_pool
    ):
        pool = await strict_configured_pool(
            pool_size=1, min_size=1, client_limit=1, wait_ms=SHORT_WAIT_MS
        )
        await pool.get_channel()

        with pytest.raises(ChannelNotAvailableError):
            await pool.get_channel()

    async def test_durable_configured_pool_is_initialized(
        self, durable_configured_pool
    ):
        pool = await durable_configured_pool(pool_size=1, min_size=1, client_limit=1)

        assert pool.pool_stats()["initialized"] is True

    async def test_durable_configured_pool_refuses_past_its_limits(
        self, durable_configured_pool
    ):
        """The durable pool stays one-tier with the non-durable global on."""
        common_settings.rabbitmq_channel_overflow_enabled = True
        pool = await durable_configured_pool(
            pool_size=1, min_size=1, client_limit=1, wait_ms=SHORT_WAIT_MS
        )
        await pool.get_channel()

        with pytest.raises(ChannelNotAvailableError):
            await pool.get_channel()


class TestChannelInfo:
    """Test the ChannelInfo helper class."""

    @pytest.fixture
    def mock_channel(self):
        channel = AsyncMock()
        channel.is_closed = False
        return channel

    @pytest.fixture
    def channel_info(self, mock_channel):
        return ChannelInfo(mock_channel)

    @pytest.fixture
    def channel_info_limit_two(self, mock_channel):
        """A ChannelInfo whose config allows 2 clients while the global allows 1."""
        return ChannelInfo(mock_channel, ChannelPoolConfig(client_limit=2))

    def test_init(self, channel_info, mock_channel):
        assert channel_info.channel == mock_channel
        assert channel_info.client_count == 0
        assert channel_info.is_available is True

    def test_increment_clients(self, channel_info):
        channel_info.increment_clients()
        assert channel_info.client_count == 1
        assert channel_info.is_available is False

    def test_increment_clients_multiple_allowed(self, channel_info):
        common_settings.rabbitmq_channel_client_limit = 2
        channel_info.increment_clients()
        assert channel_info.client_count == 1
        assert channel_info.is_available is True

    def test_stays_available_below_an_explicit_client_limit(
        self, channel_info_limit_two
    ):
        channel_info_limit_two.increment_clients()

        assert channel_info_limit_two.is_available is True

    def test_becomes_unavailable_at_an_explicit_client_limit(
        self, channel_info_limit_two
    ):
        channel_info_limit_two.increment_clients()
        channel_info_limit_two.increment_clients()

        assert channel_info_limit_two.is_available is False

    def test_decrement_clients(self, channel_info):
        channel_info.increment_clients()
        result = channel_info.decrement_clients()
        assert result is True
        assert channel_info.client_count == 0
        assert channel_info.is_available is True

    def test_decrement_clients_returns_false_when_already_zero(self, channel_info):
        """Test that decrementing at 0 returns False and doesn't go negative."""
        assert channel_info.client_count == 0
        result = channel_info.decrement_clients()
        assert result is False
        assert channel_info.client_count == 0  # Should not go negative

    def test_is_closed_when_open(self, channel_info, mock_channel):
        mock_channel.is_closed = False
        assert channel_info.is_closed() is False

    def test_is_closed_when_closed(self, channel_info, mock_channel):
        mock_channel.is_closed = True
        assert channel_info.is_closed() is True


class TestChannelPoolInit:
    """Test ChannelPool initialization."""

    def test_init(self, channel_pool, mock_connection_with_factory):
        assert channel_pool._connection == mock_connection_with_factory
        assert isinstance(channel_pool._available_channels, asyncio.Queue)
        assert channel_pool._all_channels == {}
        assert channel_pool._initialized is False
        assert channel_pool._closed is False

    def test_exposes_the_config_it_was_built_with(self, mock_connection_with_factory):
        """Task 2 builds two pools and reads back their limits through .config."""
        config = ChannelPoolConfig(client_limit=4)

        pool = ChannelPool(mock_connection_with_factory, config)

        assert pool.config is config

    def test_exposes_the_name_it_was_built_with(self, mock_connection_with_factory):
        """The name is what tells two pools' log lines apart in a timeline."""
        pool = ChannelPool(
            mock_connection_with_factory, ChannelPoolConfig(), name="durable"
        )

        assert pool.name == "durable"

    def test_name_is_none_when_the_pool_is_unnamed(self, channel_pool):
        assert channel_pool.name is None


@pytest.mark.asyncio
class TestInitialize:
    """Test the initialize method."""

    async def test_creates_min_pool_size_channels(
        self, channel_pool, mock_connection_with_factory
    ):
        common_settings.rabbitmq_channel_pool_min_size = 3

        await channel_pool.initialize()

        assert mock_connection_with_factory.channel.call_count == 3
        assert len(channel_pool._all_channels) == 3
        assert channel_pool._available_channels.qsize() == 3
        assert channel_pool._initialized is True

    async def test_sets_qos_on_channels(
        self, channel_pool, mock_connection_with_factory
    ):
        common_settings.rabbitmq_channel_pool_min_size = 1

        await channel_pool.initialize()

        # Get the created channel from the pool to verify it was called
        channel_info = await channel_pool._available_channels.get()
        # noinspection PyUnresolvedReferences
        channel_info.channel.set_qos.assert_called_with(
            prefetch_count=common_settings.rabbitmq_channel_prefetch_count
        )

    async def test_only_initializes_once(
        self, channel_pool, mock_connection_with_factory
    ):
        common_settings.rabbitmq_channel_pool_min_size = 2

        await channel_pool.initialize()
        await channel_pool.initialize()

        assert mock_connection_with_factory.channel.call_count == 2

    async def test_handles_concurrent_initialization(
        self, channel_pool, mock_connection_with_factory
    ):
        common_settings.rabbitmq_channel_pool_min_size = 2

        # Start multiple concurrent initializations
        tasks = [channel_pool.initialize() for _ in range(3)]
        await asyncio.gather(*tasks)

        # Should only initialize once
        assert mock_connection_with_factory.channel.call_count == 2
        assert channel_pool._initialized is True


@pytest.mark.asyncio
class TestGetChannel:
    """Test the get_channel method."""

    @pytest_asyncio.fixture(autouse=True)
    async def setup_initialized_pool(self, channel_pool):
        await channel_pool.initialize()

    async def test_returns_channel_from_pool(self, channel_pool):
        channel = await channel_pool.get_channel()

        assert channel is not None
        assert channel_pool._available_channels.qsize() == 19  # 20 - 1

    async def test_increments_client_count(self, channel_pool):
        channel = await channel_pool.get_channel()
        channel_info = channel_pool._all_channels[channel]

        assert channel_info.client_count == 1

    async def test_distributes_load_across_channels(self, channel_pool):
        common_settings.rabbitmq_channel_client_limit = 3

        # Get multiple channels - should use different channels first
        channel1 = await channel_pool.get_channel()
        channel2 = await channel_pool.get_channel()
        channel3 = await channel_pool.get_channel()

        # Should use different channels to distribute the load
        assert channel1 != channel2 != channel3
        assert channel_pool._available_channels.qsize() == 17  # 20 - 3

    async def test_prefers_channels_with_fewer_clients_when_reuse_needed(
        self, small_pool
    ):
        # Get 2 different channels first
        channel1 = await small_pool.get_channel()
        channel2 = await small_pool.get_channel()
        assert channel1 != channel2

        # Return one channel, so we have: channel1 (1 client), channel2 (1 client)
        await small_pool.return_channel(channel1)

        # The next request should reuse channel1 (which now has 0 clients)
        channel3 = await small_pool.get_channel()
        assert channel3 == channel1

        # Verify channel1 now has 1 client again
        channel_info = small_pool._all_channels[channel1]
        assert channel_info.client_count == 1

    async def test_allows_multiple_clients_per_channel(
        self, multi_client_single_channel_pool
    ):
        # The first request should get the only channel
        channel1 = await multi_client_single_channel_pool.get_channel()

        # Second request should get the same channel since no others available
        channel2 = await multi_client_single_channel_pool.get_channel()

        assert channel1 == channel2
        channel_info = multi_client_single_channel_pool._all_channels[channel1]
        assert channel_info.client_count == 2

    async def test_creates_new_channel_when_pool_not_full(
        self, channel_pool, mock_connection_with_factory, channel_collector
    ):
        # Exhaust initial pool
        await channel_collector(channel_pool, 20)  # min_pool_size

        # Get one more - should create a new channel
        await channel_pool.get_channel()

        assert (
            mock_connection_with_factory.channel.call_count == 21
        )  # 20 initial + 1 new
        assert len(channel_pool._all_channels) == 21

    async def test_raises_error_when_pool_exhausted_and_timeout(self, timeout_pool):
        """The strict pool (overflow global off) still times out and raises."""
        # Get the only available channel
        await timeout_pool.get_channel()

        # Try to get another - should time out
        with pytest.raises(ChannelNotAvailableError):
            await timeout_pool.get_channel()

    async def test_raises_error_when_pool_closed(self, channel_pool):
        channel_pool._closed = True

        with pytest.raises(ChannelNotAvailableError):
            await channel_pool.get_channel()

    async def test_removes_closed_channels_from_pool(
        self, channel_pool, mock_connection_with_factory, mock_channel_factory
    ):
        # Get a channel
        channel = await channel_pool.get_channel()

        # Mock the channel as closed
        # noinspection PyPropertyAccess
        channel.is_closed = True

        # Return it to make it available
        await channel_pool.return_channel(channel)

        # Try to get another channel - closed one should be removed,
        # and the new one created
        another_channel = await channel_pool.get_channel()

        # Should have removed the closed channel and created a new one
        assert another_channel != channel
        assert channel not in channel_pool._all_channels


@pytest.mark.asyncio
class TestReturnChannel:
    """Test the return_channel method."""

    @pytest_asyncio.fixture(autouse=True)
    async def setup_initialized_pool(self, channel_pool):
        await channel_pool.initialize()

    async def test_decrements_client_count(self, channel_pool):
        channel = await channel_pool.get_channel()
        await channel_pool.return_channel(channel)

        channel_info = channel_pool._all_channels[channel]
        assert channel_info.client_count == 0

    async def test_makes_channel_available_again(self, channel_pool):
        channel = await channel_pool.get_channel()
        await channel_pool.return_channel(channel)

        # Should be available again
        assert channel_pool._available_channels.qsize() == 20

    async def test_handles_multiple_clients_per_channel(
        self, multi_client_single_channel_pool
    ):
        channel1 = await multi_client_single_channel_pool.get_channel()
        # Must use the same channel
        channel2 = await multi_client_single_channel_pool.get_channel()

        assert channel1 == channel2

        # Return one client
        await multi_client_single_channel_pool.return_channel(channel1)

        channel_info = multi_client_single_channel_pool._all_channels[channel1]
        assert channel_info.client_count == 1
        assert channel_info.is_available is True

    async def test_ignores_unknown_channels(self, channel_pool):
        unknown_channel = AsyncMock()

        # Should not raise exception
        await channel_pool.return_channel(unknown_channel)

    @pytest_asyncio.fixture
    async def sibling_pools(self, configured_pool):
        """
        Two independently configured pools over the same connection.

        The connection hands out a fresh channel per call, so the two pools
        genuinely own disjoint channel objects - the shape the connector's
        try-each-pool return dispatch runs against.
        """
        pool_a = await configured_pool(
            pool_size=2, min_size=1, client_limit=1, wait_ms=SHORT_WAIT_MS
        )
        pool_b = await configured_pool(
            pool_size=2, min_size=1, client_limit=1, wait_ms=SHORT_WAIT_MS
        )
        return pool_a, pool_b

    @pytest_asyncio.fixture
    async def channel_owned_by_pool_a(self, sibling_pools):
        """A channel legitimately checked out of pool A, not of pool B."""
        pool_a, _ = sibling_pools
        return await pool_a.get_channel()

    async def test_ignores_a_channel_owned_by_a_sibling_pool(
        self, sibling_pools, channel_owned_by_pool_a
    ):
        _, pool_b = sibling_pools

        # Should not raise exception
        await pool_b.return_channel(channel_owned_by_pool_a)

    async def test_leaves_the_sibling_pools_stats_untouched(
        self, sibling_pools, channel_owned_by_pool_a
    ):
        """A foreign return must not inflate the receiving pool's availability."""
        _, pool_b = sibling_pools
        before = pool_b.pool_stats()

        await pool_b.return_channel(channel_owned_by_pool_a)

        assert pool_b.pool_stats() == before

    async def test_a_foreign_return_leaves_the_channel_checked_out_of_its_own_pool(
        self, sibling_pools, channel_owned_by_pool_a
    ):
        pool_a, pool_b = sibling_pools

        await pool_b.return_channel(channel_owned_by_pool_a)

        assert pool_a.pool_stats()["total_client_count"] == 1

    async def test_a_foreign_return_does_not_consume_the_owning_pools_return(
        self, sibling_pools, channel_owned_by_pool_a
    ):
        """The owning pool's own return is still the first one, and it counts."""
        pool_a, pool_b = sibling_pools
        await pool_b.return_channel(channel_owned_by_pool_a)

        await pool_a.return_channel(channel_owned_by_pool_a)

        assert pool_a.pool_stats()["total_client_count"] == 0

    async def test_the_owning_pool_makes_the_channel_available_again_afterwards(
        self, sibling_pools, channel_owned_by_pool_a
    ):
        pool_a, pool_b = sibling_pools
        await pool_b.return_channel(channel_owned_by_pool_a)

        await pool_a.return_channel(channel_owned_by_pool_a)

        assert pool_a.pool_stats()["available_channel_count"] == 1

    async def test_ignores_returns_when_pool_closed(self, channel_pool):
        channel = await channel_pool.get_channel()
        channel_pool._closed = True

        # Should not raise exception
        await channel_pool.return_channel(channel)

    async def test_removes_closed_channels(self, channel_pool):
        channel = await channel_pool.get_channel()
        # noinspection PyPropertyAccess
        channel.is_closed = True

        await channel_pool.return_channel(channel)

        assert channel not in channel_pool._all_channels

    async def test_double_return_does_not_go_negative(self, channel_pool):
        """Test that returning a channel twice doesn't cause negative client count."""
        channel = await channel_pool.get_channel()
        channel_info = channel_pool._all_channels[channel]
        assert channel_info.client_count == 1

        # First return - should decrement to 0
        await channel_pool.return_channel(channel)
        assert channel_info.client_count == 0

        # Second return - should be ignored and not go negative
        await channel_pool.return_channel(channel)
        assert channel_info.client_count == 0  # Should stay at 0, not go negative


@pytest.mark.asyncio
class TestCloseAll:
    """Test the close_all method."""

    @pytest_asyncio.fixture(autouse=True)
    async def setup_initialized_pool(self, channel_pool):
        await channel_pool.initialize()

    async def test_closes_all_channels(self, channel_pool, channel_collector):
        # Get some channels to track
        channels = await channel_collector(channel_pool, 3)

        await channel_pool.close_all()

        # All channels should be closed
        for channel in channels:
            channel.close.assert_called_once()

    async def test_clears_all_data_structures(self, channel_pool):
        await channel_pool.close_all()

        assert len(channel_pool._all_channels) == 0
        assert channel_pool._available_channels.qsize() == 0
        assert channel_pool._closed is True
        assert channel_pool._initialized is False

    async def test_handles_channel_close_errors(
        self, channel_pool, mock_channel_info_factory
    ):
        # Create a channel with close error
        channel_info = mock_channel_info_factory()
        channel_info.channel.close.side_effect = Exception("Close error")
        channel_pool._all_channels[channel_info.channel] = channel_info

        # Should not raise exception
        await channel_pool.close_all()

        assert channel_pool._closed is True


class TestPoolStats:
    """Test the pool_stats method."""

    def test_returns_correct_stats_when_empty(self, channel_pool):
        stats = channel_pool.pool_stats()

        expected = {
            "min_pool_size": common_settings.rabbitmq_channel_pool_min_size,
            "max_pool_size": common_settings.rabbitmq_channel_pool_size,
            "channel_client_limit": common_settings.rabbitmq_channel_client_limit,
            "total_channel_count": 0,
            "available_channel_count": 0,
            "in_use_channel_count": 0,
            "total_client_count": 0,
            "max_client_capacity": (
                common_settings.rabbitmq_channel_pool_size
                * common_settings.rabbitmq_channel_client_limit
            ),
            "overflow_enabled": common_settings.rabbitmq_channel_overflow_enabled,
            "initialized": False,
            "closed": False,
        }
        assert stats == expected

    def test_reports_overflow_disabled_for_a_durable_pool(
        self, mock_connection_with_factory
    ):
        """The durable pool advertises its strictness to readiness callers."""
        common_settings.rabbitmq_channel_overflow_enabled = True
        pool = ChannelPool(mock_connection_with_factory, DurableChannelPoolConfig())

        assert pool.pool_stats()["overflow_enabled"] is False

    @pytest.mark.asyncio
    async def test_returns_correct_stats_after_initialization(self, channel_pool):
        await channel_pool.initialize()

        stats = channel_pool.pool_stats()

        assert stats["total_channel_count"] == 20
        assert stats["available_channel_count"] == 20
        assert stats["in_use_channel_count"] == 0
        assert stats["total_client_count"] == 0
        assert stats["max_client_capacity"] == 25  # 25 * 1 (client_limit=1)
        assert stats["initialized"] is True
        assert stats["closed"] is False

    @pytest.mark.asyncio
    async def test_returns_correct_stats_with_channels_in_use(self, channel_pool):
        await channel_pool.initialize()

        # Get some channels
        await channel_pool.get_channel()
        await channel_pool.get_channel()

        stats = channel_pool.pool_stats()

        assert stats["total_channel_count"] == 20
        assert stats["available_channel_count"] == 18
        assert stats["in_use_channel_count"] == 2
        assert stats["total_client_count"] == 2

    @pytest.mark.asyncio
    async def test_returns_correct_stats_after_close(self, channel_pool):
        await channel_pool.initialize()
        await channel_pool.close_all()

        stats = channel_pool.pool_stats()

        assert stats["total_channel_count"] == 0
        assert stats["available_channel_count"] == 0
        assert stats["in_use_channel_count"] == 0
        assert stats["total_client_count"] == 0
        assert stats["initialized"] is False
        assert stats["closed"] is True

    @pytest.mark.asyncio
    async def test_returns_correct_client_count_with_multiple_clients_per_channel(
        self, reinitialize_pool, channel_pool
    ):
        """Test that total_client_count tracks all clients, not just channels."""
        pool = await reinitialize_pool(
            channel_pool,
            rabbitmq_channel_client_limit=3,
            rabbitmq_channel_pool_min_size=1,
            rabbitmq_channel_pool_size=2,
        )

        # Get 3 connections that will share channels
        await pool.get_channel()
        await pool.get_channel()
        await pool.get_channel()

        stats = pool.pool_stats()

        assert stats["total_client_count"] == 3
        # max_client_capacity = 2 pools * 3 clients per channel = 6
        assert stats["max_client_capacity"] == 6


@pytest.mark.asyncio
class TestGetBestAvailableChannel:
    """Test the _get_best_available_channel private method."""

    @pytest_asyncio.fixture(autouse=True)
    async def setup_initialized_pool(self, channel_pool):
        await channel_pool.initialize()

    async def test_returns_available_channel(self, channel_pool):
        channel_info = await channel_pool._get_best_available_channel()

        assert isinstance(channel_info, ChannelInfo)
        assert not channel_info.is_closed()

    async def test_creates_new_channel_when_none_available(
        self, channel_pool, mock_connection_with_factory
    ):
        # Exhaust available channels by giving them all max clients
        common_settings.rabbitmq_channel_client_limit = 1

        # Get all channels - _get_best_available_channel now atomically increments
        # client count, so each channel is at limit (1) after the call
        channels = []
        for _ in range(20):
            channel_info = await channel_pool._get_best_available_channel()
            channels.append(channel_info)

        # Get one more - should create a new channel since all others are at limit
        await channel_pool._get_best_available_channel()

        assert mock_connection_with_factory.channel.call_count == 21
        assert len(channel_pool._all_channels) == 21

    async def test_skips_closed_channels(self, channel_pool, mock_channel_factory):
        # Get a channel and mark it as closed
        channel_info = await channel_pool._get_best_available_channel()
        # noinspection PyPropertyAccess
        channel_info.channel.is_closed = True

        # Put it back in the available queue
        await channel_pool._available_channels.put(channel_info)

        # Get the best available should skip the closed one and create a new one
        new_channel_info = await channel_pool._get_best_available_channel()

        assert new_channel_info != channel_info
        assert not new_channel_info.is_closed()


@pytest.mark.asyncio
class TestCreateChannelInfo:
    """Test the _create_channel_info private method."""

    async def test_creates_channel_and_sets_qos(
        self, channel_pool, mock_connection_with_factory
    ):
        channel_info = await channel_pool._create_channel_info()

        assert isinstance(channel_info, ChannelInfo)
        assert channel_info.channel is not None
        # noinspection PyUnresolvedReferences
        channel_info.channel.set_qos.assert_called_with(
            prefetch_count=common_settings.rabbitmq_channel_prefetch_count
        )

    async def test_adds_channel_to_all_channels(self, channel_pool):
        channel_info = await channel_pool._create_channel_info()

        assert channel_info.channel in channel_pool._all_channels
        assert channel_pool._all_channels[channel_info.channel] == channel_info


@pytest.mark.asyncio
class TestRemoveChannelInfo:
    """Test the _remove_channel_info private method."""

    @pytest_asyncio.fixture
    async def channel_info_in_pool(self, channel_pool, mock_channel_info_factory):
        channel_info = mock_channel_info_factory()
        channel_pool._all_channels[channel_info.channel] = channel_info
        return channel_info

    async def test_removes_from_all_channels(self, channel_pool, channel_info_in_pool):
        await channel_pool._remove_channel_info(channel_info_in_pool)

        assert channel_info_in_pool.channel not in channel_pool._all_channels

    async def test_closes_channel(self, channel_pool, channel_info_in_pool):
        await channel_pool._remove_channel_info(channel_info_in_pool)

        # noinspection PyUnresolvedReferences
        channel_info_in_pool.channel.close.assert_called_once()

    async def test_handles_channel_not_in_pool(
        self, channel_pool, mock_channel_info_factory
    ):
        channel_info = mock_channel_info_factory()

        # Should not raise exception
        await channel_pool._remove_channel_info(channel_info)


@pytest.mark.asyncio
class TestCloseChannelInfo:
    """Test the _close_channel_info private method."""

    async def test_closes_open_channel(self, channel_pool, mock_channel_info_factory):
        channel_info = mock_channel_info_factory()

        await channel_pool._close_channel_info(channel_info)

        channel_info.channel.close.assert_called_once()

    async def test_skips_closed_channel(self, channel_pool, mock_channel_info_factory):
        channel_info = mock_channel_info_factory(is_closed=True)

        await channel_pool._close_channel_info(channel_info)

        channel_info.channel.close.assert_not_called()

    async def test_handles_close_errors(self, channel_pool, mock_channel_info_factory):
        channel_info = mock_channel_info_factory()
        channel_info.channel.close.side_effect = Exception("Close error")

        # Should not raise exception
        await channel_pool._close_channel_info(channel_info)


@pytest.mark.asyncio
class TestPerPoolConfiguration:
    """
    Test that a pool honors its own config instead of the process globals.

    Every value used here differs from the global pinned by reset_settings
    (pool size 25, min size 20, prefetch 500, wait 500 ms, client limit 1), so
    a test passes only if the pool read its own config.
    """

    async def test_creates_min_size_channels_from_the_instance_config(
        self, configured_pool, mock_connection_with_factory
    ):
        await configured_pool(min_size=3)

        assert mock_connection_with_factory.channel.call_count == 3

    async def test_reports_the_instance_min_size(self, configured_pool):
        pool = await configured_pool(min_size=3)

        assert pool.pool_stats()["min_pool_size"] == 3

    async def test_applies_the_instance_prefetch_count_to_new_channels(
        self, configured_pool
    ):
        pool = await configured_pool(min_size=1, prefetch_count=15)

        channel = await pool.get_channel()

        channel.set_qos.assert_called_with(prefetch_count=15)

    async def test_packs_clients_onto_one_channel_up_to_the_instance_limit(
        self, configured_pool
    ):
        pool = await configured_pool(pool_size=1, min_size=1, client_limit=4)

        channels = [await pool.get_channel() for _ in range(4)]

        assert channels.count(channels[0]) == 4

    async def test_counts_every_client_sharing_the_channel(self, configured_pool):
        pool = await configured_pool(pool_size=1, min_size=1, client_limit=4)

        for _ in range(4):
            await pool.get_channel()

        assert pool.pool_stats()["total_client_count"] == 4

    async def test_reports_the_instance_client_limit(self, configured_pool):
        pool = await configured_pool(pool_size=1, min_size=1, client_limit=4)

        assert pool.pool_stats()["channel_client_limit"] == 4

    async def test_raises_once_the_instance_client_limit_is_reached(
        self, strict_configured_pool
    ):
        """
        The instance client limit is a hard bound while overflow is off.

        Strictness source: the process-global overflow switch, which
        strict_configured_pool turns off. A durable config would work too but
        would change which globals the unset values fall back to, breaking this
        class's "every value differs from the global" premise. The
        overflow-enabled counterpart is asserted by TestOverflowAdmission.
        """
        pool = await strict_configured_pool(
            pool_size=1, min_size=1, client_limit=4, wait_ms=SHORT_WAIT_MS
        )
        for _ in range(4):
            await pool.get_channel()

        with pytest.raises(ChannelNotAvailableError):
            await pool.get_channel()

    async def test_creates_no_more_channels_than_the_instance_pool_size(
        self, configured_pool, mock_connection_with_factory
    ):
        pool = await configured_pool(
            pool_size=2, min_size=1, client_limit=1, wait_ms=SHORT_WAIT_MS
        )

        await pool.get_channel()
        await pool.get_channel()

        assert mock_connection_with_factory.channel.call_count == 2

    async def test_raises_once_the_instance_pool_size_is_exhausted(
        self, strict_configured_pool
    ):
        """
        The instance pool size is a hard bound while overflow is off.

        Strictness source: the process-global overflow switch, which
        strict_configured_pool turns off, for the same reason as
        test_raises_once_the_instance_client_limit_is_reached -- a durable
        config would move the fallbacks this class relies on.
        """
        pool = await strict_configured_pool(
            pool_size=2, min_size=1, client_limit=1, wait_ms=SHORT_WAIT_MS
        )
        await pool.get_channel()
        await pool.get_channel()

        with pytest.raises(ChannelNotAvailableError):
            await pool.get_channel()

    async def test_reports_the_instance_pool_size(self, configured_pool):
        pool = await configured_pool(pool_size=2, min_size=1)

        assert pool.pool_stats()["max_pool_size"] == 2

    async def test_instance_wait_expires_while_the_global_wait_is_still_running(
        self, strict_configured_pool
    ):
        """
        A short instance wait wins over a long global wait.

        Strictness source: the process-global overflow switch, off via
        strict_configured_pool. The blocking-wait tier is unreachable on an
        overflow-enabled pool -- overflow admits immediately whenever any open
        channel exists -- so there is deliberately no overflow twin of this
        test. It must not use a durable config either: the assertion compares
        the instance wait against rabbitmq_channel_wait_ms, and a durable
        config falls back to rabbitmq_durable_channel_wait_ms instead.
        """
        common_settings.rabbitmq_channel_wait_ms = 5000
        pool = await strict_configured_pool(
            pool_size=1, min_size=1, client_limit=1, wait_ms=SHORT_WAIT_MS
        )
        await pool.get_channel()

        start = asyncio.get_running_loop().time()
        with pytest.raises(ChannelNotAvailableError):
            await pool.get_channel()

        assert asyncio.get_running_loop().time() - start < 1.0

    async def test_instance_wait_outlives_a_shorter_global_wait(
        self, strict_configured_pool
    ):
        """
        A long instance wait wins over a 1 ms global wait.

        Strictness source: the process-global overflow switch, off via
        strict_configured_pool. Without it this test passes vacuously -- an
        overflow-enabled pool admits the pending checkout straight away, so the
        wait path it exists to exercise is never entered. Same reason as its
        sibling for not using a durable config: the comparison is against
        rabbitmq_channel_wait_ms.
        """
        common_settings.rabbitmq_channel_wait_ms = 1
        pool = await strict_configured_pool(
            pool_size=1, min_size=1, client_limit=1, wait_ms=500
        )
        held = await pool.get_channel()

        pending = asyncio.create_task(pool.get_channel())
        await asyncio.sleep(0.05)
        await pool.return_channel(held)

        assert await pending is held

    async def test_derives_max_client_capacity_from_the_instance_config(
        self, configured_pool
    ):
        pool = await configured_pool(pool_size=2, min_size=1, client_limit=4)

        assert pool.pool_stats()["max_client_capacity"] == 8


@pytest.mark.asyncio
class TestOverflowAdmission:
    """
    Test the SR-1.5 overflow band: what a full overflow-enabled pool admits.

    reset_settings pins rabbitmq_channel_overflow_enabled on, so every pool
    configured_pool builds here is two-tier. Up to the point where every
    channel is open and carrying client_limit clients the limit is honored as
    written; past it the limit stops being a refusal and becomes a packing
    target, because a pinned gRPC stream cannot be rescued by autoscaling and
    has nowhere else to go. The negative arms use strict_configured_pool and
    durable_configured_pool to pin that the refusal is still exactly where it
    was wherever the band does not apply.

    Every pool built here carries wait_ms=SHORT_WAIT_MS, so an implementation
    that wrongly falls through to the blocking tier fails the test fast instead
    of hanging it. No test here waits on that tier deliberately.
    """

    @pytest_asyncio.fixture
    async def full_overflow_pool(self, configured_pool):
        """A one-channel pool already carrying its full client_limit of 2."""
        pool = await configured_pool(
            pool_size=1, min_size=1, client_limit=2, wait_ms=SHORT_WAIT_MS
        )
        for _ in range(2):
            await pool.get_channel()
        return pool

    @pytest_asyncio.fixture
    async def overflowed_pool(self, full_overflow_pool):
        """The same pool pushed one client into the band: 3 clients, limit 2."""
        await full_overflow_pool.get_channel()
        return full_overflow_pool

    @pytest_asyncio.fixture
    async def evenly_filled_pool(self, configured_pool):
        """Two channels, both open and both at the client_limit of 2."""
        pool = await configured_pool(
            pool_size=2, min_size=2, client_limit=2, wait_ms=SHORT_WAIT_MS
        )
        for _ in range(4):
            await pool.get_channel()
        return pool

    async def test_admits_past_the_client_limit_when_the_pool_is_full(
        self, full_overflow_pool
    ):
        """The headline SR-1.5 case: a full pool packs instead of refusing."""
        # noinspection PyProtectedMember
        filled_channel = next(iter(full_overflow_pool._all_channels))

        overflowed_channel = await full_overflow_pool.get_channel()

        assert overflowed_channel is filled_channel

    async def test_counts_the_overflow_client(self, full_overflow_pool):
        await full_overflow_pool.get_channel()

        assert full_overflow_pool.pool_stats()["total_client_count"] == 3

    async def test_reports_the_packing_target_unchanged_while_overflowed(
        self, full_overflow_pool
    ):
        """
        The autoscaler/guard protocol Task 2 consumes.

        max_client_capacity stays pool_size * client_limit -- the soft
        threshold where the pool stops growing and starts packing -- while
        total_client_count is allowed to run past it. Reporting the in-use
        count as the new capacity instead would hide the band from the feed.
        """
        await full_overflow_pool.get_channel()

        stats = full_overflow_pool.pool_stats()
        assert (stats["max_client_capacity"], stats["total_client_count"]) == (2, 3)

    async def test_logs_the_overflow_admission(
        self, full_overflow_pool, pool_log_messages
    ):
        """Entering the band is only reconstructable from a timeline if logged."""
        await full_overflow_pool.get_channel()

        assert any(
            "admitting client past the channel client limit" in message
            for message in pool_log_messages()
        )

    async def test_creates_a_new_channel_instead_of_overflowing_while_the_pool_can_grow(
        self, configured_pool
    ):
        """
        Overflow sits behind the growth tier, so growth always wins.

        The first checkout puts channel A at its client_limit of 1 while the
        pool may still open a second channel, so the second checkout has to
        grow rather than pack onto A. Fails if the overflow scan is placed
        ahead of the growth tier without a fullness re-check.
        """
        pool = await configured_pool(
            pool_size=2, min_size=1, client_limit=1, wait_ms=SHORT_WAIT_MS
        )
        first_channel = await pool.get_channel()

        second_channel = await pool.get_channel()

        assert second_channel is not first_channel
        # noinspection PyProtectedMember
        assert pool._all_channels[first_channel].client_count == 1

    async def test_opens_every_channel_the_pool_is_allowed_before_overflowing(
        self, configured_pool, mock_connection_with_factory
    ):
        """
        The pool_size arm of the same ordering: growth is exhausted first.

        Three checkouts on a pool_size of 3 with a client_limit of 1 must open
        three distinct channels; only the fourth, with nowhere left to grow,
        may pack onto one of them.
        """
        pool = await configured_pool(
            pool_size=3, min_size=1, client_limit=1, wait_ms=SHORT_WAIT_MS
        )
        channels = [await pool.get_channel() for _ in range(3)]
        assert mock_connection_with_factory.channel.call_count == 3
        assert len(set(channels)) == 3

        overflowed_channel = await pool.get_channel()

        assert mock_connection_with_factory.channel.call_count == 3
        assert overflowed_channel in channels

    async def test_packs_the_overflow_client_onto_the_least_loaded_channel(
        self, evenly_filled_pool
    ):
        """A first-match overflow scan would leave [2, 4] rather than [3, 3]."""
        await evenly_filled_pool.get_channel()
        await evenly_filled_pool.get_channel()

        # noinspection PyProtectedMember
        counts = sorted(
            info.client_count for info in evenly_filled_pool._all_channels.values()
        )
        assert counts == [3, 3]

    async def test_the_second_overflow_client_does_not_pile_onto_the_first_channel(
        self, evenly_filled_pool
    ):
        first_overflow = await evenly_filled_pool.get_channel()

        second_overflow = await evenly_filled_pool.get_channel()

        assert second_overflow is not first_overflow

    async def test_raises_when_overflow_is_disabled_and_the_pool_is_full(
        self, strict_configured_pool
    ):
        """The same full-pool state still refuses with the global switched off."""
        pool = await strict_configured_pool(
            pool_size=1, min_size=1, client_limit=2, wait_ms=SHORT_WAIT_MS
        )
        for _ in range(2):
            await pool.get_channel()

        with pytest.raises(ChannelNotAvailableError):
            await pool.get_channel()

    async def test_durable_config_pool_raises_when_full_even_with_the_global_on(
        self, durable_configured_pool
    ):
        """The durable-stays-strict contract Tasks 2 and 5 assert against."""
        common_settings.rabbitmq_channel_overflow_enabled = True
        pool = await durable_configured_pool(
            pool_size=1, min_size=1, client_limit=1, wait_ms=SHORT_WAIT_MS
        )
        await pool.get_channel()

        with pytest.raises(ChannelNotAvailableError):
            await pool.get_channel()

    async def test_does_not_hand_out_a_closed_channel_when_overflowing(
        self, configured_pool
    ):
        """
        The overflow scan drops the limit predicate, not the is-open one.

        The channel is closed while still checked out, so it never passes back
        through the queue where closed channels are reaped; the scan's own
        is_closed check is the only thing between the caller and a dead
        channel. With no open channel left to pack onto, the pool refuses.
        """
        pool = await configured_pool(
            pool_size=1, min_size=1, client_limit=1, wait_ms=SHORT_WAIT_MS
        )
        channel = await pool.get_channel()
        # noinspection PyPropertyAccess
        channel.is_closed = True

        with pytest.raises(ChannelNotAvailableError):
            await pool.get_channel()

    async def test_raises_on_a_zero_sized_pool_even_with_overflow_on(
        self, configured_pool
    ):
        """Overflow needs an open channel to pack onto; this pool has none."""
        pool = await configured_pool(pool_size=0, min_size=0, wait_ms=SHORT_WAIT_MS)

        with pytest.raises(ChannelNotAvailableError):
            await pool.get_channel()

    async def test_an_overflowed_channel_becomes_available_once_every_client_returns(
        self, overflowed_pool
    ):
        """
        Overflow must not strand capacity in the return path.

        return_channel requeues only at 0 clients, and the three returns here
        cross the client_limit boundary on the way down, so a channel that was
        pushed past the limit still has to come back exactly once.
        """
        # noinspection PyProtectedMember
        channel = next(iter(overflowed_pool._all_channels))
        for _ in range(3):
            await overflowed_pool.return_channel(channel)

        assert overflowed_pool.pool_stats()["available_channel_count"] == 1

    async def test_an_overflowed_channel_is_not_requeued_while_clients_remain(
        self, overflowed_pool
    ):
        """Dropping back to the limit is not the same as being free."""
        # noinspection PyProtectedMember
        channel = next(iter(overflowed_pool._all_channels))

        await overflowed_pool.return_channel(channel)

        assert overflowed_pool.pool_stats()["available_channel_count"] == 0


@pytest.mark.asyncio
class TestDefaultConfiguration:
    """Test that a pool built without a config still follows the globals."""

    @pytest.mark.parametrize(
        "stat, setting",
        [
            ("min_pool_size", "rabbitmq_channel_pool_min_size"),
            ("max_pool_size", "rabbitmq_channel_pool_size"),
            ("channel_client_limit", "rabbitmq_channel_client_limit"),
        ],
    )
    async def test_reports_the_global_setting(self, channel_pool, stat, setting):
        assert channel_pool.pool_stats()[stat] == getattr(common_settings, setting)

    async def test_derives_max_client_capacity_from_the_globals(self, channel_pool):
        expected = (
            common_settings.rabbitmq_channel_pool_size
            * common_settings.rabbitmq_channel_client_limit
        )

        assert channel_pool.pool_stats()["max_client_capacity"] == expected

    async def test_picks_up_a_global_changed_after_construction(self, channel_pool):
        """The no-config path is bound late, not snapshotted at construction."""
        common_settings.rabbitmq_channel_client_limit = 4

        assert channel_pool.pool_stats()["channel_client_limit"] == 4


@pytest.mark.asyncio
class TestIntegration:
    """Integration tests for the channel pool."""

    async def test_concurrent_get_and_return(self, channel_pool):
        """Test concurrent access to the pool."""
        await channel_pool.initialize()

        async def worker():
            channel = await channel_pool.get_channel()
            await asyncio.sleep(0.01)  # Simulate work
            await channel_pool.return_channel(channel)
            return channel

        # Run multiple workers concurrently
        tasks = [worker() for _ in range(10)]
        channels = await asyncio.gather(*tasks)

        # All should complete successfully
        assert len(channels) == 10
        # Should have 20 available channels (all returned)
        assert channel_pool._available_channels.qsize() == 20

    async def test_concurrent_get_respects_client_limit(
        self, reinitialize_pool, channel_pool
    ):
        """
        Test that concurrent get_channel calls respect client limit.

        This pins the race condition fix for the *strict* pool specifically:
        with the overflow band off, multiple concurrent coroutines must not
        exceed channel_client_limit, so only 2 of the 5 racers get in.
        Strictness source: the process-global overflow switch, set off below.
        The overflow-enabled twin -- where every racer is admitted -- is
        test_concurrent_get_admits_every_client_when_overflow_is_on, below.
        """
        pool = await reinitialize_pool(
            channel_pool,
            rabbitmq_channel_client_limit=2,
            rabbitmq_channel_pool_min_size=1,
            rabbitmq_channel_pool_size=1,
            rabbitmq_channel_overflow_enabled=False,
        )

        # Get channels concurrently - with only 1 channel and limit of 2,
        # only 2 clients should be able to get the channel
        async def get_channel_task():
            try:
                return await pool.get_channel()
            except Exception:
                return None

        # Run 5 concurrent requests with short timeout
        common_settings.rabbitmq_channel_wait_ms = 100
        tasks = [get_channel_task() for _ in range(5)]
        results = await asyncio.gather(*tasks)

        # Count successful channel acquisitions
        successful = [r for r in results if r is not None]

        # Should only have 2 successful acquisitions (client limit)
        assert len(successful) == 2

        # Verify the channel was used by exactly 2 clients
        channel_info = list(pool._all_channels.values())[0]
        assert channel_info.client_count == 2

    async def test_concurrent_get_admits_every_client_when_overflow_is_on(
        self, reinitialize_pool, channel_pool
    ):
        """
        The overflow twin of test_concurrent_get_respects_client_limit.

        The same 1-channel, limit-2 pool and the same 5 racers, but with the
        overflow band on none of them is refused: the three that arrive after
        the limit pack onto the only channel there is.
        """
        pool = await reinitialize_pool(
            channel_pool,
            rabbitmq_channel_client_limit=2,
            rabbitmq_channel_pool_min_size=1,
            rabbitmq_channel_pool_size=1,
            rabbitmq_channel_wait_ms=SHORT_WAIT_MS,
        )

        async def get_channel_task():
            try:
                return await pool.get_channel()
            except ChannelNotAvailableError:
                return None

        results = await asyncio.gather(*[get_channel_task() for _ in range(5)])

        assert all(result is not None for result in results)
        # noinspection PyProtectedMember
        assert list(pool._all_channels.values())[0].client_count == 5

    async def test_concurrent_overflow_packs_least_loaded_first(
        self, reinitialize_pool, channel_pool
    ):
        """
        Concurrent overflow still lands evenly, because selection is locked.

        Two already-full channels take four concurrent overflow acquisitions.
        Each one selects the least-loaded channel and increments it while
        holding the pool lock, so no two racers can both see the same channel
        as least loaded and pile onto it.
        """
        pool = await reinitialize_pool(
            channel_pool,
            rabbitmq_channel_client_limit=2,
            rabbitmq_channel_pool_min_size=2,
            rabbitmq_channel_pool_size=2,
            rabbitmq_channel_wait_ms=SHORT_WAIT_MS,
        )
        for _ in range(4):
            await pool.get_channel()

        await asyncio.gather(*[pool.get_channel() for _ in range(4)])

        # noinspection PyProtectedMember
        counts = sorted(info.client_count for info in pool._all_channels.values())
        assert counts == [4, 4]

    async def test_pool_expansion_under_load(
        self,
        reinitialize_pool,
        channel_pool,
        mock_connection_with_factory,
        channel_collector,
    ):
        """Test that pool expands under load."""
        pool = await reinitialize_pool(
            channel_pool,
            rabbitmq_channel_pool_min_size=2,
            rabbitmq_channel_pool_size=5,
        )

        # Get more channels than initial size
        await channel_collector(pool, 4)

        assert len(pool._all_channels) == 4

        # 2 initial ones + 2 new
        assert mock_connection_with_factory.channel.call_count == 4

    async def test_multiple_clients_per_channel_workflow(
        self, reinitialize_pool, channel_pool
    ):
        """Test workflow with multiple clients per channel when forced by pool size."""
        pool = await reinitialize_pool(
            channel_pool,
            rabbitmq_channel_client_limit=3,
            rabbitmq_channel_pool_min_size=1,
            rabbitmq_channel_pool_size=1,
        )

        # All requests must use the same channel since only 1 exists
        channel1 = await pool.get_channel()
        channel2 = await pool.get_channel()
        channel3 = await pool.get_channel()

        assert channel1 == channel2 == channel3

        # Return them one by one
        await pool.return_channel(channel1)
        await pool.return_channel(channel2)

        # Should still be in use
        stats = pool.pool_stats()
        assert stats["in_use_channel_count"] == 1

        # Return the last one
        await pool.return_channel(channel3)

        # Now should be available again
        stats = pool.pool_stats()
        assert stats["available_channel_count"] == 1


@pytest.mark.asyncio
class TestTwoPoolIsolation:
    """
    Test that two differently configured pools over one connection don't leak.

    This is the shape Task 2 builds, now literally: pool A is a durable pool
    and pool B a non-durable one, on the same connection, each with its own
    limits. Because the module pins the non-durable overflow global on, pool B
    is two-tier while pool A is strict by construction, so the pair also pins
    that the overflow band does not cross the pool boundary.
    """

    @pytest_asyncio.fixture
    async def exhausted_strict_pool(self, durable_configured_pool):
        """
        Pool A: a durable pool, one channel, one client, already taken.

        Strictness source: DurableChannelPoolConfig, which hard-overrides
        overflow_enabled to False. That holds while the process-global
        non-durable switch stays on (reset_settings pins it True), so pool B
        below is genuinely two-tier and the pair proves durable-stays-strict
        rather than mere pool-boundedness. Every value the sibling tests read
        is explicit here, so the durable globals never come into play.
        """
        pool = await durable_configured_pool(
            pool_size=1, min_size=1, client_limit=1, wait_ms=SHORT_WAIT_MS
        )
        await pool.get_channel()
        return pool

    @pytest_asyncio.fixture
    async def packing_pool(self, configured_pool):
        """Pool B: one channel shared by up to four clients."""
        return await configured_pool(
            pool_size=1, min_size=1, client_limit=4, wait_ms=SHORT_WAIT_MS
        )

    async def test_second_pool_packs_clients_while_the_first_is_exhausted(
        self, exhausted_strict_pool, packing_pool
    ):
        channels = [await packing_pool.get_channel() for _ in range(4)]

        assert channels.count(channels[0]) == 4

    async def test_first_pool_keeps_its_own_stricter_limit(
        self, exhausted_strict_pool, packing_pool
    ):
        """
        The durable pool refuses while the non-durable one is packing.

        Strictness source: DurableChannelPoolConfig on pool A -- the
        process-global overflow switch is on for this test, so a refusal here
        proves the durable pool ignores it.
        """
        await packing_pool.get_channel()

        with pytest.raises(ChannelNotAvailableError):
            await exhausted_strict_pool.get_channel()

    async def test_each_pool_reports_its_own_client_limit(
        self, exhausted_strict_pool, packing_pool
    ):
        limits = (
            exhausted_strict_pool.pool_stats()["channel_client_limit"],
            packing_pool.pool_stats()["channel_client_limit"],
        )

        assert limits == (1, 4)

    async def test_each_pool_reports_its_own_max_client_capacity(
        self, exhausted_strict_pool, packing_pool
    ):
        capacities = (
            exhausted_strict_pool.pool_stats()["max_client_capacity"],
            packing_pool.pool_stats()["max_client_capacity"],
        )

        assert capacities == (1, 4)

    async def test_each_pool_counts_only_its_own_channels(
        self, exhausted_strict_pool, packing_pool
    ):
        """Two channels exist on the connection, one owned by each pool."""
        counts = (
            exhausted_strict_pool.pool_stats()["total_channel_count"],
            packing_pool.pool_stats()["total_channel_count"],
        )

        assert counts == (1, 1)

    async def test_each_pool_counts_only_its_own_clients(
        self, exhausted_strict_pool, packing_pool
    ):
        await packing_pool.get_channel()
        await packing_pool.get_channel()

        counts = (
            exhausted_strict_pool.pool_stats()["total_client_count"],
            packing_pool.pool_stats()["total_client_count"],
        )

        assert counts == (1, 2)


@pytest.mark.asyncio
class TestPoolNameLogPrefix:
    """
    Test that a named pool labels its log lines and an unnamed one does not.

    A process running two pools is only reconstructable from an incident
    timeline if each pool's lines say which pool emitted them. An unnamed pool
    must keep logging exactly the messages operators already grep for.
    """

    @pytest_asyncio.fixture
    async def named_pool(self, mock_connection_with_factory):
        return ChannelPool(
            mock_connection_with_factory, ChannelPoolConfig(min_size=1), name="durable"
        )

    @pytest_asyncio.fixture
    async def unnamed_pool(self, mock_connection_with_factory):
        return ChannelPool(mock_connection_with_factory, ChannelPoolConfig(min_size=1))

    async def test_named_pool_prefixes_its_initialize_lines(
        self, named_pool, pool_log_messages
    ):
        await named_pool.initialize()

        assert "[durable] Channel pool initialized" in pool_log_messages()

    async def test_named_pool_prefixes_its_close_lines(
        self, named_pool, pool_log_messages
    ):
        await named_pool.initialize()

        await named_pool.close_all()

        assert "[durable] All channels closed" in pool_log_messages()

    async def test_named_pool_prefixes_every_line_it_emits(
        self, named_pool, pool_log_messages
    ):
        await named_pool.initialize()
        await named_pool.close_all()

        assert all(message.startswith("[durable] ") for message in pool_log_messages())

    async def test_named_pool_prefixes_its_double_return_warning(
        self, named_pool, pool_log_messages
    ):
        """The line most worth attributing to a pool is the bug warning."""
        await named_pool.initialize()
        channel = await named_pool.get_channel()
        await named_pool.return_channel(channel)

        await named_pool.return_channel(channel)

        assert any(
            message.startswith("[durable] Attempted to return a channel")
            for message in pool_log_messages()
        )

    async def test_unnamed_pool_initialize_lines_are_unchanged(
        self, unnamed_pool, pool_log_messages
    ):
        await unnamed_pool.initialize()

        assert "Channel pool initialized" in pool_log_messages()

    async def test_unnamed_pool_close_lines_are_unchanged(
        self, unnamed_pool, pool_log_messages
    ):
        await unnamed_pool.initialize()

        await unnamed_pool.close_all()

        assert "All channels closed" in pool_log_messages()

    async def test_unnamed_pool_emits_no_bracket_prefix_at_all(
        self, unnamed_pool, pool_log_messages
    ):
        await unnamed_pool.initialize()
        await unnamed_pool.close_all()

        assert not any(message.startswith("[") for message in pool_log_messages())
