# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Guard tests for the ``make_pool_stats_by_pool`` factory in ``conftest.py``.

The factory is the single stand-in for
``RabbitMQConnector.pool_stats_by_pool()`` shared by every capacity-surface
test (capacity guard, readiness servicer, ``/metrics/channel-pool``, and the
connection channel manager's virtual cap). Two of its properties are load
bearing for those suites and are not visible from any individual surface test,
so they are pinned here: the emitted entries carry the *full* landed
``ChannelPool.pool_stats()`` key set, and successive calls share no mutable
state (several surface tests mutate what they get back, under ``-n6`` and
``pytest-random-order``).

Note the tests that compare against a live ``ChannelPool`` deliberately assert
the key set rather than the values: the factory's default numbers are the
library's defaults, while a local ``.env`` can override the process-global
``rabbitmq_channel_*`` settings a live ``ChannelPool`` would report. Comparing
values would pass in CI and fail on a developer machine. The client-limit
tests below do assert numbers, but only ones the factory derives itself from
its own module-level defaults, which no ``.env`` can move.
"""

from unittest.mock import MagicMock

import pytest

from notification_common.rabbitmq.channel_pool import (
    ChannelPool,
    DurableChannelPoolConfig,
)
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector


@pytest.fixture
def non_durable_stats_keys():
    """The key set a live non-durable ``ChannelPool`` actually reports."""
    return set(ChannelPool(MagicMock()).pool_stats())


@pytest.fixture
def durable_stats_keys():
    """The key set a live durable ``ChannelPool`` actually reports."""
    return set(ChannelPool(MagicMock(), DurableChannelPoolConfig()).pool_stats())


class TestMakePoolStatsByPool:

    def test_keys_off_the_connector_pool_name_constants(self, make_pool_stats_by_pool):
        assert set(make_pool_stats_by_pool()) == {
            RabbitMQConnector.NON_DURABLE_POOL_NAME,
            RabbitMQConnector.DURABLE_POOL_NAME,
        }

    def test_non_durable_entry_carries_the_full_landed_key_set(
        self, make_pool_stats_by_pool, non_durable_stats_keys
    ):
        entry = make_pool_stats_by_pool()[RabbitMQConnector.NON_DURABLE_POOL_NAME]

        assert set(entry) == non_durable_stats_keys

    def test_durable_entry_carries_the_full_landed_key_set(
        self, make_pool_stats_by_pool, durable_stats_keys
    ):
        entry = make_pool_stats_by_pool()[RabbitMQConnector.DURABLE_POOL_NAME]

        assert set(entry) == durable_stats_keys

    def test_defaults_leave_both_pools_idle(self, make_pool_stats_by_pool):
        stats = make_pool_stats_by_pool()

        assert all(
            entry["in_use_channel_count"] == 0 and entry["total_client_count"] == 0
            for entry in stats.values()
        )

    def test_defaults_report_both_pools_initialized_and_open(
        self, make_pool_stats_by_pool
    ):
        stats = make_pool_stats_by_pool()

        assert all(
            entry["initialized"] and not entry["closed"] for entry in stats.values()
        )

    def test_successive_calls_share_no_nested_entry(self, make_pool_stats_by_pool):
        first = make_pool_stats_by_pool()
        second = make_pool_stats_by_pool()

        assert all(first[name] is not second[name] for name in first)

    def test_mutating_one_call_does_not_affect_another(self, make_pool_stats_by_pool):
        first = make_pool_stats_by_pool()
        second = make_pool_stats_by_pool()

        first[RabbitMQConnector.DURABLE_POOL_NAME]["in_use_channel_count"] = 5

        assert second[RabbitMQConnector.DURABLE_POOL_NAME]["in_use_channel_count"] == 0

    def test_shorthands_set_the_durable_usage_keys(self, make_pool_stats_by_pool):
        durable = make_pool_stats_by_pool(durable_in_use=5, durable_clients=5)[
            RabbitMQConnector.DURABLE_POOL_NAME
        ]

        assert (durable["in_use_channel_count"], durable["total_client_count"]) == (
            5,
            5,
        )

    def test_durable_shorthands_leave_the_non_durable_entry_idle(
        self, make_pool_stats_by_pool
    ):
        stats = make_pool_stats_by_pool(durable_in_use=5, durable_clients=5)

        assert (
            stats[RabbitMQConnector.NON_DURABLE_POOL_NAME]["in_use_channel_count"] == 0
        )

    def test_shorthands_set_the_non_durable_usage_keys(self, make_pool_stats_by_pool):
        non_durable = make_pool_stats_by_pool(
            non_durable_in_use=7, non_durable_clients=9
        )[RabbitMQConnector.NON_DURABLE_POOL_NAME]

        assert (
            non_durable["in_use_channel_count"],
            non_durable["total_client_count"],
        ) == (7, 9)

    def test_dict_override_replaces_only_the_named_key(self, make_pool_stats_by_pool):
        default_max = make_pool_stats_by_pool()[
            RabbitMQConnector.NON_DURABLE_POOL_NAME
        ]["max_pool_size"]
        entry = make_pool_stats_by_pool(non_durable={"initialized": False})[
            RabbitMQConnector.NON_DURABLE_POOL_NAME
        ]

        assert entry["initialized"] is False and entry["max_pool_size"] == default_max

    def test_dict_override_composes_with_a_shorthand(self, make_pool_stats_by_pool):
        entry = make_pool_stats_by_pool(
            non_durable={"closed": True}, non_durable_in_use=3
        )[RabbitMQConnector.NON_DURABLE_POOL_NAME]

        assert entry["closed"] is True and entry["in_use_channel_count"] == 3

    def test_rejects_a_key_set_by_both_a_shorthand_and_the_dict(
        self, make_pool_stats_by_pool
    ):
        with pytest.raises(ValueError, match="in_use_channel_count"):
            make_pool_stats_by_pool(
                durable={"in_use_channel_count": 2}, durable_in_use=5
            )

    def test_non_durable_client_limit_sets_the_channel_client_limit(
        self, make_pool_stats_by_pool
    ):
        entry = make_pool_stats_by_pool(non_durable_client_limit=8)[
            RabbitMQConnector.NON_DURABLE_POOL_NAME
        ]

        assert entry["channel_client_limit"] == 8

    def test_non_durable_client_limit_derives_the_max_client_capacity(
        self, make_pool_stats_by_pool
    ):
        entry = make_pool_stats_by_pool(non_durable_client_limit=8)[
            RabbitMQConnector.NON_DURABLE_POOL_NAME
        ]

        assert entry["max_client_capacity"] == 800

    def test_non_durable_client_limit_leaves_the_durable_entry_at_its_defaults(
        self, make_pool_stats_by_pool
    ):
        durable = make_pool_stats_by_pool(non_durable_client_limit=8)[
            RabbitMQConnector.DURABLE_POOL_NAME
        ]

        assert durable == make_pool_stats_by_pool()[RabbitMQConnector.DURABLE_POOL_NAME]

    def test_durable_client_limit_derives_the_durable_max_client_capacity(
        self, make_pool_stats_by_pool
    ):
        entry = make_pool_stats_by_pool(durable_client_limit=8)[
            RabbitMQConnector.DURABLE_POOL_NAME
        ]

        assert (entry["channel_client_limit"], entry["max_client_capacity"]) == (
            8,
            800,
        )

    def test_durable_client_limit_leaves_the_non_durable_entry_at_its_defaults(
        self, make_pool_stats_by_pool
    ):
        non_durable = make_pool_stats_by_pool(durable_client_limit=8)[
            RabbitMQConnector.NON_DURABLE_POOL_NAME
        ]

        assert (
            non_durable
            == make_pool_stats_by_pool()[RabbitMQConnector.NON_DURABLE_POOL_NAME]
        )

    def test_client_limit_composes_with_a_max_pool_size_override(
        self, make_pool_stats_by_pool
    ):
        entry = make_pool_stats_by_pool(
            non_durable={"max_pool_size": 50}, non_durable_client_limit=16
        )[RabbitMQConnector.NON_DURABLE_POOL_NAME]

        assert entry["max_client_capacity"] == 800

    def test_rejects_a_client_limit_shorthand_with_an_explicit_client_limit(
        self, make_pool_stats_by_pool
    ):
        with pytest.raises(ValueError, match="channel_client_limit"):
            make_pool_stats_by_pool(
                non_durable={"channel_client_limit": 4}, non_durable_client_limit=8
            )

    def test_rejects_a_client_limit_shorthand_with_an_explicit_capacity(
        self, make_pool_stats_by_pool
    ):
        with pytest.raises(ValueError, match="max_client_capacity"):
            make_pool_stats_by_pool(
                durable={"max_client_capacity": 400}, durable_client_limit=8
            )

    def test_the_client_limit_conflict_error_names_the_pool(
        self, make_pool_stats_by_pool
    ):
        with pytest.raises(ValueError, match=RabbitMQConnector.NON_DURABLE_POOL_NAME):
            make_pool_stats_by_pool(
                non_durable={"channel_client_limit": 4}, non_durable_client_limit=8
            )
