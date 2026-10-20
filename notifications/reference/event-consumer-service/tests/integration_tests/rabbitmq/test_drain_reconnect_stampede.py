# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Class-split acceptance test for Epic 5, Task 4 (Bee b.tj2): the post-drain
reconnect WAVE landing on a SURVIVING (not draining) pod is admitted PER CLASS.

Task 2's draining-refusal short-circuit (SR-6.1) is a different pod and a
different test. When a pod drains for scale-down, the streams it drops
reconnect elsewhere. Some of those reconnects land on a pod that is NOT
draining -- a survivor already carrying its own load -- and the survivor must
handle the wave by the class-aware admission that shipped in Epic 7, not by
class-blind refusal:

- A PINNED reconnect (``re_routable=False``: an unproxied, directly-connected
  gRPC stream) is bound to the pod it lands on for its whole life, so no amount
  of scale-out can rescue it. It is absorbed into the survivor's overflow band
  ABOVE the soft packing threshold, and refused only PAST the memory-safe cap
  (PRD FR-11). Refusing it at the soft threshold would be a hard client failure
  a rebalance cannot fix.
- A RE-ROUTABLE reconnect (``re_routable=True``: an X-Via-Proxy-stamped gRPC
  stream, and every REST/SSE stream) is shed at the SR-3.7 shed threshold --
  which sits AT OR BELOW the cap, inside the overflow band -- so the client's
  retry routes afresh and the wave spreads across the fleet instead of piling
  onto one survivor (PRD FR-12, SDD SR-3.7).

Do NOT reduce this to the pre-split wording "throttled by stream-based
admission past the cap": that is true of PINNED reconnects only. An un-split
test would assert re-routable streams ride to the cap too and would file a
false bug against the shed tier. The split is the whole point of this file.

What this file does and does NOT assert
---------------------------------------
It does NOT re-assert the ``re_routable_shed_threshold()`` arithmetic (the
midpoint formula and the operator-override clamp) -- that is unit-tested in
``tests/app/lib/rabbitmq/test_capacity_guard.py``. This file asserts that the
drain-and-reconnect WAVE exercises that admission correctly PER CLASS against
REAL pool state on a surviving pod: every verdict is read from a real
``ChannelPool``'s ``pool_stats_by_pool()`` (counted==actual), never a synthetic
stats dict, exactly the ``real_pools_and_guard`` idiom from
``test_ac1_six_hundred_stream_burst.py``.

Geometry -- installed, never inherited
--------------------------------------
There is no memory-cap neutralizer under ``tests/integration_tests/`` (that
autouse fixture lives in ``tests/app/conftest.py``), so the memory-safe cap is
LIVE here. Every capacity input is pinned explicitly against a stray developer
``.env`` (``pinned_shipped_config``), and the cap is driven through the COMPUTED
path (override ``None``), matching AC-1. At the shipped geometry the three
admission lines are::

    soft threshold  800   = pool_size 100 x client_limit 8   (packing target)
    shed threshold 2071   = soft + (cap - soft) // 2         (re-routable shed)
    memory-safe cap 3342  = 1024 MiB / (15 x 1800 + 5120) B / 10 replicas

The shed line sits strictly inside ``(soft, cap]``, which is exactly what makes
a pinned-only band above it exist -- this file asserts that relationship rather
than assuming it, but reads the concrete line values from the shipped functions
so it tracks a geometry change instead of a frozen literal.
"""

from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock

import pytest
import pytest_asyncio

from notification_common.config.common_settings import CommonSettings, common_settings
from notification_common.errors.errors import ChannelNotAvailableError
from notification_common.rabbitmq.channel_pool import (
    ChannelPool,
    ChannelPoolConfig,
    DurableChannelPoolConfig,
)
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings
from app.grpc.event_consumer_servicer import EventConsumerServicer
from app.lib.rabbitmq.capacity_guard import (
    CapacityGuard,
    re_routable_shed_threshold,
)
from app.lib.rabbitmq.stream_memory_cap import memory_safe_stream_cap
from app.rest import routes

# --- Module constants: each a literal with its derivation in a comment ----

# The shipped non-durable pool geometry (common_settings defaults).
POOL_SIZE = 100  # rabbitmq_channel_pool_size
POOL_MIN_SIZE = 75  # rabbitmq_channel_pool_min_size
CLIENT_LIMIT = 8  # rabbitmq_channel_client_limit
PREFETCH = 15  # rabbitmq_channel_prefetch_count

# The soft packing threshold: pool_size 100 x client_limit 8 = 800 streams/pod.
# The pool reports this as max_client_capacity; the guard admits ABOVE it into
# the overflow band, so it refuses no one.
SOFT_THRESHOLD = 800

# The memory-safe cap inputs and the cap they compute to. Derivation, checked by
# hand and NOT recomputed from production code (same figure as AC-1):
#   budget 1024 MiB / (15 x 1800 + 5120) B = 33,429 fleet / 10 replicas = 3,342
BUDGET_MIB = 1024
MAX_REPLICAS = 10
CAP = 3_342

NON_DURABLE = RabbitMQConnector.NON_DURABLE_POOL_NAME
DURABLE = RabbitMQConnector.DURABLE_POOL_NAME

# The X-Via-Proxy admission-class stamp the chart injects on the proxied gRPC
# route. Its presence marks a stream re-routable (SDD SR-3.7); its absence marks
# the pinned default. Read from the servicer so a rename fails here loudly.
PROXY_HEADER = EventConsumerServicer._PROXY_HEADER


def _fresh_mock_channel():
    """A fresh mock channel, as a real connection hands out per ``channel()``."""
    channel = AsyncMock()
    channel.is_closed = False
    channel.set_qos = AsyncMock()
    channel.close = AsyncMock()
    return channel


def _mock_connection():
    """
    A mock connection handing out a FRESH channel per ``channel()`` call.

    The pool grows by calling ``connection.channel()``; a connection that
    returned one shared channel would collapse the packing this file relies on
    to reach the overflow band. Mirrors the ``_mock_connection`` idiom in
    ``test_ac1_six_hundred_stream_burst.py``.
    """
    connection = MagicMock()
    connection.is_closed = False
    connection.channel = AsyncMock(side_effect=lambda: _fresh_mock_channel())
    return connection


def _grpc_context(*, proxied: bool):
    """
    A mock gRPC servicer context carrying (or not) the X-Via-Proxy stamp.

    ``EventConsumerServicer._is_proxied`` reads
    ``dict(context.invocation_metadata())``, so the metadata is the only field
    that matters. A proxied context stamps the header; an unproxied one omits
    it, which is the pinned default.
    """
    context = MagicMock()
    metadata = [(PROXY_HEADER, "true")] if proxied else []
    context.invocation_metadata = MagicMock(return_value=metadata)
    return context


@pytest.fixture
def pinned_shipped_config(mocker):
    """
    Pin the shipped non-durable geometry and the memory-cap inputs.

    Pools built with a bare ``ChannelPoolConfig()`` late-bind to
    ``common_settings`` on every read, and the cap reads ``settings`` on every
    call, so a developer's untracked ``.env`` (which on this box still carries
    the pre-Epic-3 ``rabbitmq_channel_client_limit=1`` /
    ``prefetch_count=500``) would re-seed both. Every input the three admission
    lines ride on is pinned here explicitly, the "installed, never inherited"
    posture AC-1 takes.

    The cap runs through the COMPUTED path (override ``None``), and the shed
    override is ``None`` too so the shed line is the live-band midpoint rather
    than a hand-set number. Overflow is pinned on: the survivor only reaches its
    overflow band because the non-durable pool admits past ``client_limit`` once
    all ``pool_size`` channels are full.
    """
    mocker.patch.object(common_settings, "rabbitmq_channel_pool_size", POOL_SIZE)
    mocker.patch.object(
        common_settings, "rabbitmq_channel_pool_min_size", POOL_MIN_SIZE
    )
    mocker.patch.object(common_settings, "rabbitmq_channel_client_limit", CLIENT_LIMIT)
    mocker.patch.object(common_settings, "rabbitmq_channel_prefetch_count", PREFETCH)
    mocker.patch.object(common_settings, "rabbitmq_channel_overflow_enabled", True)

    # Computed-cap path: override None, pin the three inputs that compute 3342.
    mocker.patch.object(settings.settings, "max_non_durable_streams_per_pod", None)
    mocker.patch.object(
        settings.settings, "broker_stream_memory_budget_mib", BUDGET_MIB
    )
    mocker.patch.object(settings.settings, "scaler_max_replicas", MAX_REPLICAS)

    # Shed override None -> midpoint of the live band, the shipped default.
    mocker.patch.object(settings.settings, "max_re_routable_streams_per_pod", None)


@pytest_asyncio.fixture
async def surviving_pod(pinned_shipped_config, mocker):
    """
    Build a surviving-pod factory: a real non-durable pool filled to a
    band-relative stream count, plus a real ``CapacityGuard`` reading its live
    stats.

    The factory takes a callable ``fill(soft, shed, cap) -> int`` so each test
    fills the pool to the exact class boundary it exercises, from the SHIPPED
    line values (read off the real pool's ``max_client_capacity`` for ``soft``,
    and off ``memory_safe_stream_cap`` / ``re_routable_shed_threshold`` for
    ``cap`` / ``shed``) rather than frozen literals.

    The connector is a ``spec``'d ``MagicMock`` whose ``pool_stats_by_pool``
    rebuilds from the live pools on every call, so the guard's verdict is
    counted==actual. ``spec=RabbitMQConnector`` makes a rename of that method
    fail loudly here. ``draining = False`` is set explicitly: this is a
    SURVIVING pod, so the guard must fall through to the class-aware tiers, not
    the Task 2 draining short-circuit (which would refuse class-blind).
    """
    created = []

    async def _make(fill):
        non_durable = ChannelPool(
            _mock_connection(), ChannelPoolConfig(), name=NON_DURABLE
        )
        durable = ChannelPool(
            _mock_connection(), DurableChannelPoolConfig(), name=DURABLE
        )
        await non_durable.initialize()
        await durable.initialize()
        created.append((non_durable, durable))

        soft = non_durable.pool_stats()["max_client_capacity"]
        cap = memory_safe_stream_cap()
        shed = re_routable_shed_threshold(soft, cap)
        count = fill(soft, shed, cap)

        for _unused_stream in range(count):
            await non_durable.get_channel()

        connector = mocker.MagicMock(spec=RabbitMQConnector)
        connector.draining = False
        connector.pool_stats_by_pool.side_effect = lambda: {
            NON_DURABLE: non_durable.pool_stats(),
            DURABLE: durable.pool_stats(),
        }
        guard = CapacityGuard(connector=connector)

        return SimpleNamespace(
            non_durable=non_durable,
            durable=durable,
            guard=guard,
            soft=soft,
            shed=shed,
            cap=cap,
        )

    try:
        yield _make
    finally:
        for non_durable, durable in created:
            await non_durable.close_all()
            await durable.close_all()


class TestShippedGeometryAnchor:
    """
    Pin the geometry the three admission lines ride on, so a defaults or
    cap-input drift that silently moved a boundary fails HERE rather than
    quietly making a boundary test vacuous.

    The four packing constants are read from ``model_fields[...].default`` (not a
    resolved ``settings`` instance) because a stray ``.env`` re-seeds the live
    vars on reload -- the hazard recorded in ``tests/app/config/test_settings.py``.
    """

    def test_pool_size_matches_the_declared_default(self):
        assert (
            POOL_SIZE
            == CommonSettings.model_fields["rabbitmq_channel_pool_size"].default
        )

    def test_client_limit_matches_the_declared_default(self):
        assert (
            CLIENT_LIMIT
            == CommonSettings.model_fields["rabbitmq_channel_client_limit"].default
        )

    def test_prefetch_matches_the_declared_default(self):
        assert (
            PREFETCH
            == CommonSettings.model_fields["rabbitmq_channel_prefetch_count"].default
        )

    def test_the_computed_cap_is_the_shipped_memory_safe_value(
        self, pinned_shipped_config
    ):
        # The wave must not exceed this. Driven through the computed path so the
        # file exercises the real prefetch-15 arithmetic, not a hand-set number.
        assert memory_safe_stream_cap() == CAP

    def test_the_shed_line_sits_within_the_band_above_the_soft_threshold(
        self, pinned_shipped_config
    ):
        # The band relationship the pinned-only reserve depends on, ASSERTED not
        # assumed: soft < shed <= cap. If shed collapsed onto the cap there would
        # be no pinned band and (a) would be untestable; if it fell to the soft
        # threshold re-routable traffic would shed at the packing product.
        shed = re_routable_shed_threshold(SOFT_THRESHOLD, CAP)

        assert SOFT_THRESHOLD < shed <= CAP


class TestReservedOverflowBandOnASurvivingPod:
    """
    At the shed line, the two classes diverge on REAL pool state: the pinned
    reconnect is absorbed, the re-routable one is shed. This one filled state
    IS the SR-3.7 reserved-band contract -- proving (a) and (c) at the exact
    boundary where they split.
    """

    @pytest_asyncio.fixture
    async def pod_at_shed(self, surviving_pod):
        pod = await surviving_pod(lambda soft, shed, cap: shed)
        return pod

    @pytest.mark.asyncio
    async def test_the_pod_really_holds_streams_in_the_overflow_band(self, pod_at_shed):
        # Non-vacuity: the survivor is genuinely above the 800 soft threshold
        # (in the overflow band) and below the 3342 cap, read from real state.
        streams = pod_at_shed.non_durable.pool_stats()["total_client_count"]

        assert pod_at_shed.soft < streams == pod_at_shed.shed < pod_at_shed.cap

    @pytest.mark.asyncio
    async def test_a_pinned_reconnect_is_absorbed_into_the_overflow_band(
        self, pod_at_shed
    ):
        # (a) A pinned reconnect above the soft threshold and below the cap is
        # admitted -- proving pinned is NOT refused at the soft packing
        # threshold and rides into the band reserved for it.
        pod_at_shed.guard.raise_if_at_capacity(re_routable=False)

    @pytest.mark.asyncio
    async def test_a_re_routable_reconnect_is_shed_at_the_same_state(self, pod_at_shed):
        # (c) At the identical real pool state, a re-routable reconnect is shed
        # (refused) so the client retries and re-routes -- spreading the wave.
        with pytest.raises(ChannelNotAvailableError):
            pod_at_shed.guard.raise_if_at_capacity(re_routable=True)


class TestReRoutableAdmittedBelowTheShedThreshold:
    """
    (d) Below the shed line the re-routable class is still admitted: the shed
    tier engages only inside the reserved band, not across the whole overflow
    region. The tight boundary (one stream below the shed line) is what makes
    this the shed line's lower edge rather than a loose "somewhere below" claim.
    """

    @pytest_asyncio.fixture
    async def pod_below_shed(self, surviving_pod):
        pod = await surviving_pod(lambda soft, shed, cap: shed - 1)
        return pod

    @pytest.mark.asyncio
    async def test_a_re_routable_reconnect_just_below_the_shed_line_is_admitted(
        self, pod_below_shed
    ):
        streams = pod_below_shed.non_durable.pool_stats()["total_client_count"]
        assert streams == pod_below_shed.shed - 1

        pod_below_shed.guard.raise_if_at_capacity(re_routable=True)


class TestPinnedRefusedAtTheMemorySafeCap:
    """
    (b) The pinned band has a ceiling: AT the memory-safe cap every class
    refuses, pinned included. This is the drain invariant's teeth -- the
    survivor stops absorbing before it can cross the broker's memory high
    watermark and block publishers for the whole fleet (SR-3.5).
    """

    @pytest_asyncio.fixture
    async def pod_at_cap(self, surviving_pod):
        pod = await surviving_pod(lambda soft, shed, cap: cap)
        return pod

    @pytest.mark.asyncio
    async def test_the_pod_is_filled_exactly_to_the_cap(self, pod_at_cap):
        # Non-vacuity: real pool state sits AT the hard bound, not merely near it.
        streams = pod_at_cap.non_durable.pool_stats()["total_client_count"]
        assert streams == pod_at_cap.cap

    @pytest.mark.asyncio
    async def test_a_pinned_reconnect_at_the_cap_is_refused(self, pod_at_cap):
        with pytest.raises(ChannelNotAvailableError):
            pod_at_cap.guard.raise_if_at_capacity(re_routable=False)

    @pytest.mark.asyncio
    async def test_a_re_routable_reconnect_at_the_cap_is_also_refused(self, pod_at_cap):
        # The hard bound is class-blind; re-routable is refused here too (it was
        # already shed far below, at the shed line).
        with pytest.raises(ChannelNotAvailableError):
            pod_at_cap.guard.raise_if_at_capacity(re_routable=True)


class TestDrainWaveStaysUnderTheCap:
    """
    The drain invariant end to end: no admitted-stream sequence drives the
    survivor's ``total_client_count`` above the memory-safe cap. Pinned is the
    class that rides highest (it is never shed below the cap), so gating a
    PINNED wave at the cap is the strongest form of "the wave cannot push a
    survivor past the cap".
    """

    # A few slots below the cap, so the wave admits a bounded number and then
    # the guard closes the gate exactly at the cap.
    GAP = 5

    @pytest_asyncio.fixture
    async def pod_just_below_cap(self, surviving_pod):
        pod = await surviving_pod(lambda soft, shed, cap: cap - self.GAP)
        return pod

    @pytest.mark.asyncio
    async def test_the_wave_fills_the_gap_and_then_the_gate_closes_at_the_cap(
        self, pod_just_below_cap
    ):
        pod = pod_just_below_cap
        non_durable, guard = pod.non_durable, pod.guard

        # A reconnect wave larger than the remaining headroom. Each arrival is
        # admitted only if the guard says so, and every admitted checkout is
        # a REAL pool checkout that moves live state the next guard read sees.
        admitted = 0
        for _unused_reconnect in range(self.GAP + 7):
            if guard.has_capacity(re_routable=False):
                await non_durable.get_channel()
                admitted += 1
                # The invariant, checked on every single admission: no admitted
                # stream ever pushed the survivor above the cap.
                assert non_durable.pool_stats()["total_client_count"] <= pod.cap

        # Exactly the headroom was admitted; the surplus reconnects were refused.
        assert admitted == self.GAP
        # And the survivor came to rest precisely at the cap, not past it.
        assert non_durable.pool_stats()["total_client_count"] == pod.cap


class TestReconnectClassificationAtEntryPoints:
    """
    The wave's class is decided at the consume ENTRY POINTS, and this ties that
    classification to real admission on a surviving pod filled to the shed line
    -- the one state where the class actually changes the verdict.

    - gRPC ``ConsumeNonDurableEvents`` classifies via
      ``_is_proxied(context)``: an X-Via-Proxy-stamped request is re-routable
      (shed here), an unstamped one is the pinned default (admitted here).
    - REST/SSE ``_check_non_durable_capacity`` is UNCONDITIONALLY re-routable
      (shed here), never consulting the stamp.

    The real guard is patched into ``app.rest.routes`` so the REST dependency
    decides against the same live pool state, not a stub.
    """

    @pytest_asyncio.fixture
    async def pod_at_shed(self, surviving_pod):
        pod = await surviving_pod(lambda soft, shed, cap: shed)
        return pod

    @pytest.mark.asyncio
    async def test_an_unproxied_grpc_reconnect_is_classified_pinned_and_admitted(
        self, pod_at_shed
    ):
        # Reproduces the servicer's entry-point expression (line 144):
        #   raise_if_at_capacity(re_routable=self._is_proxied(context))
        context = _grpc_context(proxied=False)
        re_routable = EventConsumerServicer._is_proxied(context)

        assert re_routable is False
        # Pinned at the shed line rides into the reserved band: admitted.
        pod_at_shed.guard.raise_if_at_capacity(re_routable=re_routable)

    @pytest.mark.asyncio
    async def test_a_proxied_grpc_reconnect_is_classified_re_routable_and_shed(
        self, pod_at_shed
    ):
        context = _grpc_context(proxied=True)
        re_routable = EventConsumerServicer._is_proxied(context)

        assert re_routable is True
        with pytest.raises(ChannelNotAvailableError):
            pod_at_shed.guard.raise_if_at_capacity(re_routable=re_routable)

    @pytest.mark.asyncio
    async def test_a_rest_reconnect_is_unconditionally_re_routable_and_shed(
        self, pod_at_shed, mocker
    ):
        # The REST dependency calls the module-level guard with re_routable=True
        # regardless of any stamp; point that name at the real survivor guard.
        mocker.patch.object(routes, "capacity_guard", pod_at_shed.guard)

        with pytest.raises(ChannelNotAvailableError):
            routes._check_non_durable_capacity()
