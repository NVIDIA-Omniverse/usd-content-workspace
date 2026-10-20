# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Negative space: the refusing admission thresholds reach ADMISSION, nothing else.

The contract
------------
Non-durable admission is three-tier and only the outer two tiers refuse. Both
of them are enforced inside ``CapacityGuard.has_capacity()`` on the non-durable
path and read nowhere else in the service:

- ``max_non_durable_streams_per_pod`` (SDD SR-3.5) is the HARD bound, and since
  Epic 7 it is the SOLE hard admission threshold on the non-durable path: the
  pool's own stream capacity underneath it is a report-only packing and scaling
  target that refuses nobody, so streams past it land in the pool's overflow
  band and the cap is the only thing that ends admission for every class.
- ``max_re_routable_streams_per_pod`` (SDD SR-3.7, PRD FR-12) is the SHED
  threshold inside that overflow band. It refuses RE-ROUTABLE streams only --
  ones a client retry can land on another pod -- reserving the rest of the band
  for pinned streams, which scale-out cannot rescue.

The Epic's other two capacity surfaces must see neither:

- **Readiness** (``ChannelPoolHealthServicer``, Epic 2 Task 2) is a POOL
  capacity signal. If either threshold bounded it, a small configured value
  would take pods NOT_SERVING while their pools still had room -- shrinking the
  fleet in response to a memory backstop, or to a threshold scoped to one
  stream CLASS, rather than to load.
- **``/metrics/channel-pool``** (Epic 2 Task 3) is the autoscaler's only
  capacity input, and its three response keys are a protocol the scaler reads
  by name. If either threshold moved a value -- or added a fourth field -- the
  scaler would be reacting to a bound it knows nothing about, and the shape
  change would break its parsing.

Both failures are production-only for both settings: they need a configured
threshold below what the pool would otherwise allow, which no other test
exercises. Only this file says the two stay out.

The file is named for the cap, which it was built for. The shed threshold
joined it in Epic 7 Task 2 because it has the same hazard profile, the same
enforcement point and the same two surfaces to stay out of; splitting it into a
near-identical second file would have duplicated the fixtures rather than the
coverage.

Why the assertions are differential
-----------------------------------
Each test drives one surface TWICE over identical pool stats -- once with the
threshold under test neutralized and once with it configured to reject at those
stream numbers -- and asserts the two answers are identical. Nothing here
re-derives either surface's formula, so the file keeps its meaning when those
formulas change and cannot go stale into a false green. ``TestCapIsRejecting``
and ``TestShedThresholdIsRejecting`` are the anti-vacuity guards: each proves,
through the guard itself, that the crafted scenarios really are ones that
threshold rejects, so an "identical" result below is a real negative and not an
artifact of a threshold that never bound.

One asymmetry is called out where it lands, in
``TestReadinessIgnoresTheShedThreshold``: a shed-rejecting scenario is inside
the overflow band by construction, so readiness answers NOT_SERVING on both
sides of that differential and an absolute test carries the readiness half.

Threshold observability is deferred, not forgotten
--------------------------------------------------
Exporting either threshold -- a gauge, a shed counter, a cap or shed field on
any surface -- belongs to ticket ``b.3c9``, which owns channel-pool
observability. Today their only externally visible signals are the log lines
``CapacityGuard`` emits when it refuses: a WARNING at the cap, an INFO at the
shed threshold. A future change that adds a reading derived from either to the
readiness verdict or the metrics body will fail here; that is this file
working, and the answer is ``b.3c9``, not a wider assertion.

This file is deliberately NOT additions to
``tests/app/grpc/test_channel_pool_health_servicer.py`` or
``tests/app/rest/test_routes.py`` (whose ``TestChannelPoolMetrics`` belongs to
Epic 2 Task 3 and is fenced off), and it is not
``tests/app/test_capacity_surface_consistency.py``, which is the Epic-level
cross-surface acceptance table -- it was parked for Epic 2 Task 5 when this
file was written and has since landed, most recently re-cut by Epic 7 Task 2.
That table asserts the three surfaces AGREE on one stats report with both
thresholds neutralized; this file asserts the two thresholds never reach the
other two surfaces at all. Bare "Task N" references do not survive a second
Epic, so every cross-Task pointer in this file names its Epic.
"""

import pytest
from fastapi.testclient import TestClient
from grpc_health.v1 import health_pb2

from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings
from app.grpc.channel_pool_health_servicer import ChannelPoolHealthServicer
from app.lib.rabbitmq.capacity_guard import CapacityGuard
from app.rest.rest_server import rest_server
from tests.app.rest import test_routes
from tests.app.rest.test_routes import METRICS_PATH, METRICS_RESPONSE_KEYS

# The endpoint's path and its full response-key set are the autoscaler protocol
# Epic 2 Task 3 owns. They are imported rather than re-declared so a protocol
# change lands in one place instead of drifting between two files.
patch_pool_stats = test_routes.patch_pool_stats

NON_DURABLE = RabbitMQConnector.NON_DURABLE_POOL_NAME

# Far below every scenario's active-stream count, so the cap rejects in all of
# them. Small enough that it is also below every pool's stream capacity, which
# is what makes it a bound only the cap could impose.
REJECTING_CAP = 5

# The readiness reserve, in non-durable stream slots.
MIN_AVAILABLE_STREAMS = 10

# Scenarios the cap rejects, spanning both readiness verdicts and both packing
# densities. The non-durable pool is the fixture default of 100 channels, so
# stream capacity is 100 at a client limit of 1 and 800 at a limit of 8; every
# active-stream count below leaves the pool itself with room, and readiness
# reserve headroom is what separates the first row from the second.
CAP_REJECTED_SCENARIOS = [
    # 60 of 100 stream slots: 40 available, above the reserve of 10.
    pytest.param(1, 60, id="limit1-60-streams-pool-has-room"),
    # 95 of 100: 5 available, below the reserve -- readiness trips here for a
    # reason that has nothing to do with the cap.
    pytest.param(1, 95, id="limit1-95-streams-inside-the-reserve"),
    # 390 of 800: the packed path, where a stream is not a channel.
    pytest.param(8, 390, id="limit8-390-streams-packed"),
]

# The one scenario the metrics tests run, picked from the table above: packed,
# so the endpoint is exercised on the shipped stream-unit path rather than the
# limit-1 case where streams and channels coincide.
METRICS_CLIENT_LIMIT = 8
METRICS_ACTIVE_STREAMS = 390

# Scenarios the SHED threshold rejects, as ``(client_limit, shed_threshold,
# active_streams)``. Unlike the cap, the shed tier lives entirely INSIDE the
# overflow band, and ``re_routable_shed_threshold()`` clamps the operator
# override up to the pool's ``max_client_capacity``. So a shed line only binds
# when it is set ABOVE that capacity, and only rejects at stream counts above
# the line again -- which is opt-in pattern (c) in ``tests/app/conftest.py``,
# and the reason these numbers look bigger than the cap table's. Both rows sit
# far below the neutralized cap that fixture leaves in place, so the refusal
# they produce can only be the shed tier's.
#
# The channel-unit keys are left at the factory default here, as everywhere
# else in this file: no surface under test reads them.
SHED_REJECTED_SCENARIOS = [
    # Pool stream capacity 100, so a shed line at 120 is inside the overflow
    # band and 130 active streams is 10 past it.
    pytest.param(1, 120, 130, id="limit1-130-streams-past-a-shed-line-of-120"),
    # The packed path, where a stream is not a channel: capacity 800, shed
    # line 900, 950 active streams.
    pytest.param(8, 900, 950, id="limit8-950-streams-past-a-shed-line-of-900"),
]

# A shed setting BELOW the pool's stream capacity. The clamp discards it -- the
# real shed line is the capacity, so nothing is actually shed -- but a surface
# that read the setting raw would honor it, which is the leak this value is
# here to catch.
SUB_CAPACITY_SHED_SETTING = 5

# The one shed scenario the metrics tests run, taken from the packed row above.
METRICS_SHED_CLIENT_LIMIT = 8
METRICS_SHED_THRESHOLD = 900
METRICS_SHED_ACTIVE_STREAMS = 950


@pytest.fixture
def client():
    return TestClient(rest_server)


@pytest.fixture
def mock_connector(mocker):
    connector = mocker.Mock(spec=RabbitMQConnector)
    # The guard reads ``connector.draining`` on the non-durable path (Epic 5,
    # Task 2 / SR-6.1). ``draining`` is a plain instance attribute on
    # ``RabbitMQConnector`` (set in ``__init__``), so it is absent from the
    # class spec and accessing it would raise ``AttributeError`` here. Pin the
    # not-draining default so these cap/shed-surface rows exercise the tier
    # they are about rather than the draining short-circuit.
    connector.draining = False
    return connector


@pytest.fixture
def set_pool_stats(mock_connector):
    """
    Publish one stats result on BOTH connector accessors.

    The same anti-vacuity measure ``test_channel_pool_health_servicer.py`` uses:
    ``Mock(spec=RabbitMQConnector)`` hands back a truthy ``Mock`` from any
    accessor left unstubbed, so driving only one of them could pass without the
    surface ever seeing these numbers.
    """

    def _set(stats_by_pool):
        mock_connector.pool_stats_by_pool.return_value = stats_by_pool
        mock_connector.pool_stats.return_value = stats_by_pool.get(NON_DURABLE) or {}
        return stats_by_pool

    return _set


@pytest.fixture
def apply_rejecting_cap(mocker):
    """
    Return a callable that switches the memory-safe cap ON, mid-test.

    ``tests/app/conftest.py`` neutralizes the cap for every test under
    ``tests/app/`` by patching this same attribute to a non-binding value; a
    later patch of the same attribute wins, which is the documented opt-in.
    Applying it partway through a test lets one test observe a surface with the
    cap off and then on, over identical stats.
    """

    def _apply():
        mocker.patch.object(
            settings.settings,
            "max_non_durable_streams_per_pod",
            REJECTING_CAP,
        )

    return _apply


@pytest.fixture
def apply_shed_threshold(mocker):
    """
    Return a callable that switches the re-routable shed threshold ON, mid-test.

    ``max_re_routable_streams_per_pod`` ships unset, and while it is unset the
    shed line sits at the midpoint of the band the neutralized cap tops out --
    far past any stream count these fixtures express. Setting it puts the line
    where the caller asks, subject to the clamp, which is opt-in pattern (c) in
    ``tests/app/conftest.py``. Applying it partway through a test lets one test
    observe a surface with the shed tier out of reach and then binding, over
    identical stats.

    The cap is deliberately NOT touched: it stays at the autouse fixture's
    non-binding value throughout, so anything these tests observe is the shed
    tier's doing and not the hard bound's.
    """

    def _apply(shed_threshold):
        mocker.patch.object(
            settings.settings,
            "max_re_routable_streams_per_pod",
            shed_threshold,
        )

    return _apply


class TestCapIsRejecting:
    """
    Precondition: every scenario below IS one the cap rejects.

    Without this, the identical-answer assertions in the rest of the file would
    still pass if the cap stopped binding altogether -- a cap that never
    rejects trivially fails to leak. Asserting rejection through the guard's
    own public entry point, rather than by re-deriving the cap, keeps that
    premise honest for both of the surfaces that follow.
    """

    @pytest.mark.parametrize("client_limit, active_streams", CAP_REJECTED_SCENARIOS)
    def test_the_guard_admits_without_the_cap_and_sheds_with_it(
        self,
        mock_connector,
        set_pool_stats,
        make_pool_stats_by_pool,
        apply_rejecting_cap,
        client_limit,
        active_streams,
    ):
        guard = CapacityGuard(connector=mock_connector)
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=client_limit,
                non_durable_clients=active_streams,
            )
        )

        admitted_uncapped = guard.has_capacity()
        apply_rejecting_cap()
        admitted_capped = guard.has_capacity()

        assert (admitted_uncapped, admitted_capped) == (True, False)


class TestReadinessIgnoresTheCap:
    """Readiness answers the same with the cap rejecting as without it."""

    @pytest.fixture
    def servicer(self, mock_connector):
        return ChannelPoolHealthServicer(
            mock_connector, min_available_streams=MIN_AVAILABLE_STREAMS
        )

    @pytest.mark.parametrize("client_limit, active_streams", CAP_REJECTED_SCENARIOS)
    @pytest.mark.asyncio
    async def test_verdict_is_identical_under_a_rejecting_cap(
        self,
        servicer,
        set_pool_stats,
        make_pool_stats_by_pool,
        apply_rejecting_cap,
        client_limit,
        active_streams,
    ):
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=client_limit,
                non_durable_clients=active_streams,
            )
        )

        uncapped = await servicer.Check(None, None)
        apply_rejecting_cap()
        capped = await servicer.Check(None, None)

        assert capped.status == uncapped.status

    @pytest.mark.asyncio
    async def test_a_capped_out_pod_with_pool_headroom_still_serves(
        self,
        servicer,
        set_pool_stats,
        make_pool_stats_by_pool,
        apply_rejecting_cap,
    ):
        """
        The failure mode in its own right, stated absolutely.

        The differential test above would still pass if BOTH verdicts became
        NOT_SERVING for some unrelated reason. This one pins the direction that
        matters operationally: a pod shedding on the cap has pool headroom by
        construction, so it must stay in the Service. Taking it out would
        remove capacity the pool still has and hand the fleet a readiness
        signal driven by a memory backstop.
        """
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=1,
                non_durable_clients=60,
            )
        )
        apply_rejecting_cap()

        response = await servicer.Check(None, None)

        assert response.status == health_pb2.HealthCheckResponse.SERVING


class TestChannelPoolMetricsIgnoreTheCap:
    """
    The autoscaler's feed is byte-identical with the cap rejecting.

    Both the values and the key SET are asserted: a cap-derived fourth field
    would be a protocol change to a surface this Task promised not to touch
    (Epic 2 Task 1's ``t3.tj2.nm.53.up`` fences ``TestChannelPoolMetrics``
    off), and cap observability is ``b.3c9``'s.
    """

    @pytest.fixture
    def responses(
        self,
        client,
        patch_pool_stats,
        make_pool_stats_by_pool,
        apply_rejecting_cap,
    ):
        """Return ``(uncapped, capped)`` responses over identical pool stats."""
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=METRICS_CLIENT_LIMIT,
                non_durable_clients=METRICS_ACTIVE_STREAMS,
            )
        )

        uncapped = client.get(METRICS_PATH)
        apply_rejecting_cap()
        capped = client.get(METRICS_PATH)

        # A pair of 500s would compare equal too, so anchor the premise here
        # rather than letting an error body masquerade as agreement.
        assert (uncapped.status_code, capped.status_code) == (200, 200)

        return uncapped, capped

    def test_response_body_is_identical_under_a_rejecting_cap(self, responses):
        uncapped, capped = responses

        assert capped.json() == uncapped.json()

    def test_response_key_set_is_the_unchanged_autoscaler_protocol(self, responses):
        uncapped, capped = responses

        assert set(capped.json()) == set(uncapped.json()) == METRICS_RESPONSE_KEYS


class TestShedThresholdIsRejecting:
    """
    Precondition: every shed scenario below IS one the shed tier rejects.

    The same anti-vacuity role ``TestCapIsRejecting`` plays for the cap, and it
    has more to prove here, because the shed tier can fail to bind in two ways
    rather than one: the clamp can raise a configured line back to the pool's
    stream capacity, and the neutralized cap can refuse first. Both tests below
    assert through the guard's own public entry point rather than by re-deriving
    the threshold.
    """

    @pytest.mark.parametrize(
        "client_limit, shed_threshold, active_streams", SHED_REJECTED_SCENARIOS
    )
    def test_the_guard_admits_a_re_routable_stream_until_the_shed_line_is_set(
        self,
        mock_connector,
        set_pool_stats,
        make_pool_stats_by_pool,
        apply_shed_threshold,
        client_limit,
        shed_threshold,
        active_streams,
    ):
        guard = CapacityGuard(connector=mock_connector)
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=client_limit,
                non_durable_clients=active_streams,
            )
        )

        admitted_unshed = guard.has_capacity(re_routable=True)
        apply_shed_threshold(shed_threshold)
        admitted_shedding = guard.has_capacity(re_routable=True)

        assert (admitted_unshed, admitted_shedding) == (True, False)

    @pytest.mark.parametrize(
        "client_limit, shed_threshold, active_streams", SHED_REJECTED_SCENARIOS
    )
    def test_a_pinned_stream_still_admits_at_the_same_stream_numbers(
        self,
        mock_connector,
        set_pool_stats,
        make_pool_stats_by_pool,
        apply_shed_threshold,
        client_limit,
        shed_threshold,
        active_streams,
    ):
        """
        The refusal above is the SHED tier's, not some other bound's.

        Without this, a rejection produced by the hard bound -- or by any
        future threshold that refuses class-blind -- would satisfy the
        precondition test and let the differentials below claim a leak-free
        result they never earned. The shed tier is class-scoped by
        construction, so the same stats and the same configured line must still
        admit a pinned stream.
        """
        guard = CapacityGuard(connector=mock_connector)
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=client_limit,
                non_durable_clients=active_streams,
            )
        )
        apply_shed_threshold(shed_threshold)

        assert guard.has_capacity() is True


class TestReadinessIgnoresTheShedThreshold:
    """
    Readiness answers the same with the shed line rejecting as without it.

    The differential is weaker here than it is for the cap, and deliberately so
    rather than by oversight: a shed-rejecting scenario sits inside the overflow
    band by construction -- the clamp puts the shed line at or above the pool's
    stream capacity -- and any stream count past that capacity is already past
    the readiness reserve. So readiness answers NOT_SERVING on both sides of
    the differential, for a pool reason that has nothing to do with the shed
    setting, and a leak would not move it. There is no arrangement of these
    numbers that avoids that.

    ``test_a_pod_with_pool_headroom_serves_under_a_small_shed_setting`` is what
    actually carries the readiness half. The differential still earns its place
    as the regression pin for the direction that IS reachable: readiness must
    not start SERVING, or start reading a shed-derived field, when the shed
    line is configured.
    """

    @pytest.fixture
    def servicer(self, mock_connector):
        return ChannelPoolHealthServicer(
            mock_connector, min_available_streams=MIN_AVAILABLE_STREAMS
        )

    @pytest.mark.parametrize(
        "client_limit, shed_threshold, active_streams", SHED_REJECTED_SCENARIOS
    )
    @pytest.mark.asyncio
    async def test_verdict_is_identical_under_a_rejecting_shed_threshold(
        self,
        servicer,
        set_pool_stats,
        make_pool_stats_by_pool,
        apply_shed_threshold,
        client_limit,
        shed_threshold,
        active_streams,
    ):
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=client_limit,
                non_durable_clients=active_streams,
            )
        )

        unshed = await servicer.Check(None, None)
        apply_shed_threshold(shed_threshold)
        shedding = await servicer.Check(None, None)

        assert shedding.status == unshed.status

    @pytest.mark.asyncio
    async def test_a_pod_with_pool_headroom_serves_under_a_small_shed_setting(
        self,
        servicer,
        set_pool_stats,
        make_pool_stats_by_pool,
        apply_shed_threshold,
    ):
        """
        The operationally dangerous leak, stated absolutely.

        A shed setting of 5 against a pool holding 60 of 100 stream slots is
        exactly the shape that would fire if readiness ever read
        ``max_re_routable_streams_per_pod`` raw. The clamp means nothing is
        actually shed at these numbers, so this scenario is intentionally NOT
        one ``TestShedThresholdIsRejecting`` covers -- what it pins is that a
        setting scoped to ONE stream class, configured well below the pool's
        capacity, cannot take a pod with pool headroom out of the Service. A
        leak here would shrink the fleet on a re-routable-stream policy while
        every pod still had room for pinned ones.
        """
        set_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=1,
                non_durable_clients=60,
            )
        )
        apply_shed_threshold(SUB_CAPACITY_SHED_SETTING)

        response = await servicer.Check(None, None)

        assert response.status == health_pb2.HealthCheckResponse.SERVING


class TestChannelPoolMetricsIgnoreTheShedThreshold:
    """
    The autoscaler's feed is byte-identical with the shed line rejecting.

    The strongest half of the shed isolation, and the one the readiness
    differential above cannot supply. Every value in the body is a real number
    here, and the configured shed line (900) differs from all three of them, so
    a surface that leaked it would move the body rather than coincide with it.
    A leak would steer the autoscaler off a threshold that governs one stream
    class, on a signal that is supposed to describe pool load.
    """

    @pytest.fixture
    def responses(
        self,
        client,
        patch_pool_stats,
        make_pool_stats_by_pool,
        apply_shed_threshold,
    ):
        """Return ``(unshed, shedding)`` responses over identical pool stats."""
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=METRICS_SHED_CLIENT_LIMIT,
                non_durable_clients=METRICS_SHED_ACTIVE_STREAMS,
            )
        )

        unshed = client.get(METRICS_PATH)
        apply_shed_threshold(METRICS_SHED_THRESHOLD)
        shedding = client.get(METRICS_PATH)

        # A pair of 500s would compare equal too, so anchor the premise here
        # rather than letting an error body masquerade as agreement.
        assert (unshed.status_code, shedding.status_code) == (200, 200)

        return unshed, shedding

    def test_response_body_is_identical_under_a_rejecting_shed_threshold(
        self, responses
    ):
        unshed, shedding = responses

        assert shedding.json() == unshed.json()

    def test_response_key_set_is_the_unchanged_autoscaler_protocol(self, responses):
        unshed, shedding = responses

        assert set(shedding.json()) == set(unshed.json()) == METRICS_RESPONSE_KEYS
