# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging
from unittest.mock import MagicMock

from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.lib.observability.channel_pool_metrics import ChannelPoolMetrics
from app.lib.rabbitmq.rabbitmq_base import rabbitmq_connector

NON_DURABLE = RabbitMQConnector.NON_DURABLE_POOL_NAME
DURABLE = RabbitMQConnector.DURABLE_POOL_NAME

# Every gauge the singleton registers, in the order the constructor creates
# them. Pinning the whole set -- names and count -- is what catches a gauge
# dropped, renamed, or added without a corresponding test.
EXPECTED_GAUGE_NAMES = [
    "channel_pool.total_channel_count",
    "channel_pool.in_use_channel_count",
    "channel_pool.available_channel_count",
    "channel_pool.total_client_count",
    "channel_pool.max_pool_size",
    "channel_pool.channel_client_limit",
    "channel_pool.max_client_capacity",
]


def _observations(stat_key):
    """
    Run one stat's observable-gauge callback and return its observations.

    The callback reads the connector's live per-pool stats at collection time,
    so a test patches ``rabbitmq_connector.pool_stats_by_pool`` first and then
    calls this. The ``CallbackOptions`` argument the collector would pass is
    irrelevant to this callback, so a bare mock stands in for it.

    :param stat_key: The pool-stats key the gauge reports.
    :returns: A list of ``(value, attributes)`` pairs, one per observation.
    """
    callback = ChannelPoolMetrics._make_callback(stat_key)
    return [(o.value, o.attributes) for o in callback(MagicMock())]


class TestInit:

    def test_registers_the_expected_gauge_names_in_order(self, mock_meter):
        ChannelPoolMetrics()

        names = [
            call.args[0] for call in mock_meter.create_observable_gauge.call_args_list
        ]
        assert names == EXPECTED_GAUGE_NAMES

    def test_each_gauge_is_wired_to_its_own_stat_key(self, mock_meter, mocker):
        # Pin the name<->callback binding the constructor actually wires, not
        # just the names. Each gauge name is "channel_pool.<stat_key>"; give
        # every stat key a DISTINCT value so a gauge wired to the wrong key
        # (a copy-paste slip across the 7 near-identical blocks) reports the
        # wrong number and fails here.
        stats = {
            name.split(".", 1)[1]: i for i, name in enumerate(EXPECTED_GAUGE_NAMES)
        }
        mocker.patch.object(
            rabbitmq_connector, "pool_stats_by_pool", return_value={NON_DURABLE: stats}
        )

        ChannelPoolMetrics()

        for call in mock_meter.create_observable_gauge.call_args_list:
            name = call.args[0]
            stat_key = name.split(".", 1)[1]
            callback = call.kwargs["callbacks"][0]
            observations = [(o.value, o.attributes) for o in callback(MagicMock())]
            assert observations == [(stats[stat_key], {"pool": NON_DURABLE})]

    def test_does_not_register_when_metrics_disabled(
        self, telemetry_disabled, mock_meter
    ):
        ChannelPoolMetrics()

        mock_meter.create_observable_gauge.assert_not_called()

    def test_does_not_register_when_only_tracing_enabled(
        self, tracing_only, mock_meter
    ):
        # Metrics must gate on metrics_enabled, not tracing_enabled.
        ChannelPoolMetrics()

        mock_meter.create_observable_gauge.assert_not_called()


class TestCallbackYieldsPerPool:
    """
    Each gauge's callback reports one Observation per pool, carrying that pool's
    value for the gauge's stat under the ``pool`` attribute.
    """

    def test_yields_one_observation_per_pool_with_the_pool_attribute(self, mocker):
        # Two pools present with distinct values, so a callback that read the
        # wrong key, dropped a pool, or mislabelled one fails here.
        mocker.patch.object(
            rabbitmq_connector,
            "pool_stats_by_pool",
            return_value={
                NON_DURABLE: {"total_channel_count": 12},
                DURABLE: {"total_channel_count": 34},
            },
        )

        assert _observations("total_channel_count") == [
            (12, {"pool": NON_DURABLE}),
            (34, {"pool": DURABLE}),
        ]

    def test_reads_the_stat_key_the_callback_was_built_for(self, mocker):
        # The same stats carry two keys with different values; the callback
        # built for one must report that one, not the other.
        mocker.patch.object(
            rabbitmq_connector,
            "pool_stats_by_pool",
            return_value={
                NON_DURABLE: {"total_client_count": 7, "max_client_capacity": 800},
            },
        )

        assert _observations("max_client_capacity") == [(800, {"pool": NON_DURABLE})]


class TestCallbackIsDefensive:
    """
    A raising callback drops the entire scrape, so every lookup degrades to
    "emit nothing" rather than raising. The pre-``connect()`` empty snapshot and
    a stats dict missing a key are the two shapes the connector really produces.
    """

    def test_yields_nothing_when_stats_are_empty(self, mocker):
        # ``pool_stats_by_pool()`` returns ``{}`` before ``connect()``.
        mocker.patch.object(rabbitmq_connector, "pool_stats_by_pool", return_value={})

        assert _observations("total_channel_count") == []

    def test_yields_nothing_when_pool_stats_is_none(self, mocker):
        # The ``or {}`` guard also absorbs a None snapshot without raising.
        mocker.patch.object(rabbitmq_connector, "pool_stats_by_pool", return_value=None)

        assert _observations("total_channel_count") == []

    def test_skips_a_pool_whose_stats_omit_the_key(self, mocker):
        # One pool carries the key and one does not: only the pool that has it
        # is reported, and the missing one is silently skipped.
        mocker.patch.object(
            rabbitmq_connector,
            "pool_stats_by_pool",
            return_value={
                NON_DURABLE: {"total_channel_count": 5},
                DURABLE: {},
            },
        )

        assert _observations("total_channel_count") == [(5, {"pool": NON_DURABLE})]

    def test_yields_nothing_when_no_pool_carries_the_key(self, mocker):
        mocker.patch.object(
            rabbitmq_connector,
            "pool_stats_by_pool",
            return_value={NON_DURABLE: {}, DURABLE: {}},
        )

        assert _observations("total_channel_count") == []

    def test_yields_nothing_when_the_stats_read_raises(self, mocker, caplog):
        # The stats read runs on the exporter's collection thread and can race a
        # concurrent channel checkout/return mutating the pool dicts on the
        # event-loop thread, surfacing as a RuntimeError ("dictionary changed
        # size during iteration"). The backstop must swallow it and yield
        # nothing -- a propagating error drops the ENTIRE scrape -- and log the
        # skip at debug only.
        mocker.patch.object(
            rabbitmq_connector,
            "pool_stats_by_pool",
            side_effect=RuntimeError("dictionary changed size during iteration"),
        )

        with caplog.at_level(logging.DEBUG):
            assert _observations("total_channel_count") == []

        assert any(record.levelno == logging.DEBUG for record in caplog.records)
