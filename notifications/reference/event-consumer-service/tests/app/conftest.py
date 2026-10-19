# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import copy
import logging
from unittest.mock import MagicMock
import pytest
from opentelemetry import trace

from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings
from app.lib.permissions.storage_permissions import storage_permissions_cache
from app.rest.models.connection_metadata import ConnectionMetadata
from tests.fixtures.dropped_storage_events import DROP_REPORT_MESSAGE

# A per-pod stream cap chosen to sit far above any capacity a test fixture can
# express (the largest is ``max_pool_size`` 100 x ``client_limit`` 16 = 1600),
# so patching it in makes the memory-safe cap structurally unable to bind.
#
# The number has to clear a second, higher bar than the cap alone: it is also
# the TOP of the band ``re_routable_shed_threshold()`` splits, and the shed
# default is the band's midpoint. A cap of ``C`` against a pool capacity of
# ``S`` puts the shed line at ``S + (C - S) // 2``, roughly ``C / 2``, so the
# cap must exceed about twice the largest expressible stream count for the shed
# tier to be unreachable too. 1,000,000 against 1600 gives a shed line of
# 500,800 -- both tiers clear by more than two orders of magnitude.
_NON_BINDING_STREAM_CAP = 1_000_000


@pytest.fixture(autouse=True)
def non_binding_stream_memory_cap(mocker):
    """
    Put every test under ``tests/app/`` below BOTH refusing admission
    thresholds -- the memory-safe stream cap and the re-routable shed
    threshold -- unless the test opts back in.

    WHY. Non-durable admission is three-tier and only the outer two tiers
    refuse: the pool's own stream capacity is a report-only packing/scaling
    target, the shed threshold inside the overflow band refuses RE-ROUTABLE
    streams, and the memory-safe stream cap is the hard bound that refuses
    every class. ``memory_safe_stream_cap()`` divides
    ``broker_stream_memory_budget_mib`` by the worst-case per-stream footprint
    at the configured prefetch and apportions the result across
    ``scaler_max_replicas``; at the shipped configuration (budget 1024 MiB,
    prefetch 15, 10 replicas) that is **3,342 streams per pod**. At the shipped
    prefetch the cap sits ABOVE the stream counts the ``tests/app/`` fixtures
    express -- the ``client_limit`` 8 and 16 tables in
    ``tests/app/lib/rabbitmq/test_capacity_guard.py`` describe pools whose
    ``max_client_capacity`` is 800/1600 and walk stream counts up to and just
    past those -- so at the shipped shape the computed cap would NOT bind on
    those rows. But the cap is a function of the ambient prefetch, budget and
    replica divisor: an untracked ``.env``, a lower budget, or a prefetch raised
    back toward 500 moves it under those capacities (at prefetch 500 it is 118),
    which inverts the band and collapses the shed threshold onto the hard bound
    at 118. So the fixture cannot assume the cap is out of reach and must pin it
    there.

    That the cap CAN land inside the fixture range is a test-configuration
    artifact of the 800/1600 fixture tables, not a production regression: at the
    shipped ``channel_client_limit`` 8 the computed cap of 3,342 does not bind.
    So the load-bearing reason to retain this fixture is no longer "the cap
    would bind" -- it is the stricter guarantee that a test which does NOT opt
    in is never silently answering a cap question or a shed question, whatever
    the ambient prefetch. Pinning every test below both thresholds keeps each
    surface test exercising the bound it is actually about. Note the direction:
    a test parked below both thresholds ADMITS,
    including above the pool's stream capacity, because the pool bound refuses
    on the non-durable path only under the incomputable-cap fallback, which
    this fixture's absolute override rules out.

    HOW IT WORKS. ``max_non_durable_streams_per_pod`` is the operator override
    for the hard bound, and ``memory_safe_stream_cap()`` returns it
    unconditionally before reading the prefetch at all. Setting it above every
    fixture capacity therefore neutralizes the hard bound and keeps the
    unbounded-prefetch warn-once branch out of unrelated tests.

    The shed tier needs no patch of its own: it is neutralized VIA THE CLAMP,
    because ``re_routable_shed_threshold()`` takes that same cap as the top of
    the band and ``max_re_routable_streams_per_pod`` ships unset, so the
    threshold is the band's midpoint -- 500,800 at the largest fixture capacity
    (1600), against a cap of 1,000,000. The clamp's LOWER end is the pool's
    stream capacity, not the cap, so this lift only holds while
    ``max_re_routable_streams_per_pod`` is ``None``: a test that sets it to an
    absolute value gets a shed line at ``max(override, max_client_capacity)``,
    which this fixture does not raise. That is pattern (c) below, and it is
    deliberate -- it is what makes an in-band shed test possible at all.

    HOW TO OPT BACK IN. A test that wants a threshold to matter re-patches the
    relevant attribute inside the test (or in its own fixture): ``mocker``
    applies patches in order and the later patch wins, so a per-test
    ``mocker.patch.object(settings.settings,
    "max_non_durable_streams_per_pod", 5)`` overrides this one for that test.
    Three patterns are in use:

    - (a) Exercise a *binding absolute cap*: patch
      ``settings.settings.max_non_durable_streams_per_pod`` to a small value
      (the cap tests in ``test_capacity_guard.py`` and the surface-isolation
      tests do this). Refuses every class at that value.
    - (b) Exercise the *computed cap path*: patch
      ``settings.settings.max_non_durable_streams_per_pod`` to ``None`` and set
      ``settings.settings.broker_stream_memory_budget_mib`` /
      ``common_settings.rabbitmq_channel_prefetch_count`` /
      ``settings.settings.scaler_max_replicas`` explicitly. This is what
      ``tests/app/lib/rabbitmq/test_stream_memory_cap.py`` does, including for
      the pin on the shipped-default cap value (3,342 at prefetch 15) -- that
      pin is only meaningful because it patches the override back to ``None``,
      so this fixture cannot mask a regression in the default computation.
    - (c) Exercise a *binding shed threshold with the cap still non-binding*:
      patch ``settings.settings.max_re_routable_streams_per_pod`` to the stream
      count the shed line should land on and leave this fixture's cap in place.
      The clamp raises anything at or below the pool's ``max_client_capacity``
      to that capacity, so aim at or above it. This is the only way to test the
      class shed in isolation -- re-routable streams refuse from the shed line
      up while pinned ones keep admitting, since the hard bound stays out of
      reach.
    """
    mocker.patch.object(
        settings.settings,
        "max_non_durable_streams_per_pod",
        _NON_BINDING_STREAM_CAP,
    )


@pytest.fixture
def mock_tracer(mocker):
    tracer = MagicMock()
    mocker.patch.object(trace, "get_tracer", return_value=tracer)
    return tracer


@pytest.fixture
def mock_span():
    return MagicMock()


@pytest.fixture
def connection_metadata():
    return ConnectionMetadata("/api/v1/events/stream")


@pytest.fixture
def drop_reports(caplog):
    """
    Capture the storage-permissions cache logger from DEBUG up, and return a
    function that lists the drop-report lines captured so far (see
    ``tests/fixtures/dropped_storage_events.py``).
    """
    caplog.set_level(logging.DEBUG, logger=storage_permissions_cache.logger.name)
    return lambda: [r for r in caplog.records if r.getMessage() == DROP_REPORT_MESSAGE]


# An idle, freshly initialized non-durable pool as ``ChannelPool.pool_stats()``
# reports it, at a deliberately minimal ONE-STREAM-PER-CHANNEL baseline chosen
# so surface tests read easily -- this is NOT the shipped configuration. The
# SHIPPED non-durable defaults are ``rabbitmq_channel_client_limit=8`` (so
# ``max_client_capacity`` is 800) and ``rabbitmq_channel_prefetch_count=15``;
# those shipped numbers are pinned in
# ``tests/app/test_packing_defaults_contract.py``, and a test that cares about
# packing passes ``non_durable_client_limit=8`` to the factory below.
# ``rabbitmq_channel_pool_size=100`` and ``rabbitmq_channel_pool_min_size=75``
# below ARE real shipped values (unchanged by the packing flip).
# ``initialize()`` pre-opens ``min_size`` channels, so an idle pool holds 75
# channels with all 75 available, and ``max_client_capacity`` is
# ``pool_size * client_limit`` (100 x 1 at this baseline). ``overflow_enabled``
# reports the process-global ``rabbitmq_channel_overflow_enabled``, which ships
# True; the non-durable pool takes no per-pool override of it (it is global-only
# by design), so a real non-durable pool reports True at the shipped default.
_DEFAULT_NON_DURABLE_POOL_STATS = {
    "min_pool_size": 75,
    "max_pool_size": 100,
    "channel_client_limit": 1,
    "total_channel_count": 75,
    "available_channel_count": 75,
    "in_use_channel_count": 0,
    "total_client_count": 0,
    "max_client_capacity": 100,
    "overflow_enabled": True,
    "initialized": True,
    "closed": False,
}

# An idle durable pool, per ``DurableChannelPoolConfig``:
# ``rabbitmq_durable_channel_pool_size=100``,
# ``rabbitmq_durable_channel_pool_min_size=0``,
# ``rabbitmq_durable_channel_client_limit=1``. ``min_size`` 0 means the pool
# opens no channels until a durable checkout needs one, so an idle durable pool
# reports zero total/available channels even though it is initialized.
# ``overflow_enabled`` is False unconditionally: ``DurableChannelPoolConfig``
# hard overrides the property rather than reading
# ``rabbitmq_channel_overflow_enabled``, so the durable pool stays strict no
# matter how the process-global flag is set.
_DEFAULT_DURABLE_POOL_STATS = {
    "min_pool_size": 0,
    "max_pool_size": 100,
    "channel_client_limit": 1,
    "total_channel_count": 0,
    "available_channel_count": 0,
    "in_use_channel_count": 0,
    "total_client_count": 0,
    "max_client_capacity": 100,
    "overflow_enabled": False,
    "initialized": True,
    "closed": False,
}


def _conflict_error(key, pool_label):
    """Build the shared shorthand-versus-override-dict conflict error."""
    return ValueError(
        f"'{key}' for the '{pool_label}' pool was set both by a "
        f"shorthand keyword and by the override dict; pass it exactly "
        f"one way."
    )


def _apply_pool_overrides(
    defaults, overrides, shorthands, pool_label, client_limit=None
):
    """
    Build one per-pool entry from the defaults plus a test's overrides.

    :param defaults: The module-level default stats dict for this pool; deep
        copied, never mutated.
    :param overrides: The caller's per-key override dict, or None.
    :param shorthands: ``{key: value}`` for the shorthand kwargs the caller
        passed; entries with a None value are treated as not passed.
    :param pool_label: The pool name, used in the conflict error message.
    :param client_limit: The per-channel client limit to size this entry to, or
        None to leave the default sizing alone. Unlike the ``shorthands``
        entries this sets *two* keys: it writes ``channel_client_limit`` and
        derives ``max_client_capacity`` as ``max_pool_size * client_limit``
        from the entry's effective ``max_pool_size`` -- that is, after any
        ``max_pool_size`` in ``overrides`` has been merged in. The derivation
        mirrors ``ChannelPool.pool_stats()`` in the library
        (``notification_common/rabbitmq/channel_pool.py:377``:
        ``max_client_capacity = self._config.pool_size *
        self._config.client_limit``), so the pair can never be incoherent.
    :returns: An independent stats dict for this pool.
    :raises ValueError: If a key is set both by a shorthand and by the
        override dict.
    """
    entry = copy.deepcopy(defaults)

    if overrides:
        entry.update(overrides)

    if client_limit is not None:
        for derived_key in ("channel_client_limit", "max_client_capacity"):
            if overrides and derived_key in overrides:
                raise _conflict_error(derived_key, pool_label)
        entry["channel_client_limit"] = client_limit
        entry["max_client_capacity"] = entry["max_pool_size"] * client_limit

    for key, value in shorthands.items():
        if value is None:
            continue
        if overrides and key in overrides:
            raise _conflict_error(key, pool_label)
        entry[key] = value

    return entry


def _make_pool_stats_by_pool(
    *,
    non_durable=None,
    durable=None,
    non_durable_in_use=None,
    durable_in_use=None,
    non_durable_clients=None,
    durable_clients=None,
    non_durable_client_limit=None,
    durable_client_limit=None,
):
    """
    Build a ``RabbitMQConnector.pool_stats_by_pool()`` return value.

    The connector keeps one channel pool per queue type and reports them
    separately, so this returns one full per-pool entry each. It is a faithful
    stand-in for the real call and nothing more: what each capacity surface
    reads out of it -- which pool it looks at, and how it combines or ignores
    the other -- is that surface's own contract, pinned in that surface's test
    file rather than described here.

    Defaults are both pools idle at the module's baseline values (see
    ``_DEFAULT_NON_DURABLE_POOL_STATS`` / ``_DEFAULT_DURABLE_POOL_STATS``; the
    non-durable baseline is a one-stream-per-channel test baseline, not the
    shipped packing density), and
    each entry carries the full ``ChannelPool.pool_stats()`` key set so the
    result is a faithful stand-in rather than a subset. Every call deep copies
    the defaults, so a test may mutate what it gets back without leaking into
    another test.

    A test that wants "the durable pool is busy" varies only the durable
    entry's usage keys -- ``make_pool_stats_by_pool(durable_in_use=5,
    durable_clients=5)`` -- and one that wants a differently sized non-durable
    pool passes ``non_durable_client_limit=8`` rather than hand-setting
    ``channel_client_limit`` and ``max_client_capacity`` as a pair.

    :param non_durable: Per-key overrides merged into the non-durable entry.
    :param durable: Per-key overrides merged into the durable entry.
    :param non_durable_in_use: Shorthand for the non-durable entry's
        ``in_use_channel_count``.
    :param durable_in_use: Shorthand for the durable entry's
        ``in_use_channel_count``.
    :param non_durable_clients: Shorthand for the non-durable entry's
        ``total_client_count``.
    :param durable_clients: Shorthand for the durable entry's
        ``total_client_count``.
    :param non_durable_client_limit: Sizes the non-durable entry to this
        per-channel client limit, setting ``channel_client_limit`` and
        deriving ``max_client_capacity`` from the entry's effective
        ``max_pool_size`` the way the library does (see
        ``_apply_pool_overrides``). Composes with a ``max_pool_size`` override.
    :param durable_client_limit: The same for the durable entry.
    :returns: ``{NON_DURABLE_POOL_NAME: {...}, DURABLE_POOL_NAME: {...}}``,
        keyed off the connector's class constants so a library rename fails
        loudly here instead of silently yielding entries no surface can find.
    :raises ValueError: If a shorthand and the matching override dict both set
        the same key. The shorthands are sugar for the dimensions the surface
        tests vary, not a second precedence layer -- setting a key both ways is
        a test-authoring mistake, so it is rejected rather than silently
        resolved.
    """
    return {
        RabbitMQConnector.NON_DURABLE_POOL_NAME: _apply_pool_overrides(
            _DEFAULT_NON_DURABLE_POOL_STATS,
            non_durable,
            {
                "in_use_channel_count": non_durable_in_use,
                "total_client_count": non_durable_clients,
            },
            RabbitMQConnector.NON_DURABLE_POOL_NAME,
            client_limit=non_durable_client_limit,
        ),
        RabbitMQConnector.DURABLE_POOL_NAME: _apply_pool_overrides(
            _DEFAULT_DURABLE_POOL_STATS,
            durable,
            {
                "in_use_channel_count": durable_in_use,
                "total_client_count": durable_clients,
            },
            RabbitMQConnector.DURABLE_POOL_NAME,
            client_limit=durable_client_limit,
        ),
    }


@pytest.fixture
def make_pool_stats_by_pool():
    """
    Factory fixture returning ``_make_pool_stats_by_pool``.

    Lets a capacity-surface test build a ``pool_stats_by_pool()`` return value
    -- both pools idle by default -- and vary either pool's usage, sizing, or
    any individual stat without hand-rolling a two-pool dict. See
    ``_make_pool_stats_by_pool`` for the parameters; what a given surface reads
    out of the result is pinned in that surface's own test file.
    """
    return _make_pool_stats_by_pool
