# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Regression pin: overflow does not distort the scaling signal.

What this file asserts
----------------------
Under two-tier admission a loaded pod serves in the OVERFLOW BAND -- the range
between the pool's own stream capacity (the SOFT threshold, a packing/scaling
target that refuses nobody) and the memory-safe stream cap (the hard bound).
The two surfaces that steer the fleet must keep answering in soft-threshold
terms the whole way across that band, and must keep reporting the true load
even above it:

- ``ChannelPoolHealthServicer.Check()``
  (``app/grpc/channel_pool_health_servicer.py``), the Kubernetes readiness
  verdict, whose availability is ``max_client_capacity - total_client_count``
  compared against the reserve.
- ``GET /api/v1beta/metrics/channel-pool`` (``app/rest/routes.py``), the
  autoscaler's only capacity input.

Both were pre-adapted in Epic 2 (negative availability, unclamped in-use) and
this Epic changes neither. This file is the standing pin that they stay
SOFT-threshold-keyed once the memory cap becomes the sole hard refusal.

The regression it exists to catch
---------------------------------
Somebody re-keying either surface onto the memory cap or the re-routable shed
threshold, on the reasoning that the cap is "the real limit" now. That would
make the autoscaler scale against a memory bound instead of against load --
the exact failure the PRD's Decisions section rejects: "a memory-keyed
autoscaler would add pods indefinitely while the broker drowns". A cap-keyed
readiness surface would additionally hold a pod ready past the point where the
autoscaler was supposed to have been given headroom.

The third pre-adapted surface is deliberately absent
----------------------------------------------------
Epic 2 pre-adapted three things; only two are testable here. The pod-deletion
cost sign lives in the autoscaler component (Epic 4), not in this service
repo, so there is nothing in ``app/`` for a third class to drive. The
two-of-three coverage is deliberate, not an omission.

One seam, on purpose
--------------------
Both surfaces read ``RabbitMQConnector.pool_stats_by_pool()`` and select its
non-durable entry, so every scenario publishes ONE stats report on the
``rabbitmq_connector`` singleton exported from ``app/lib/rabbitmq/
rabbitmq_base.py`` -- the object the REST route reads -- and the servicer is
built over that same singleton. Two mocks would let readiness and the metrics
feed drift apart silently; one cannot.

The memory cap is set to a DIFFERENT number than the soft threshold
-------------------------------------------------------------------
``tests/app/conftest.py``'s autouse ``non_binding_stream_memory_cap`` fixture
parks every test in this directory far below both refusing thresholds. This
file overrides it, on purpose, with an explicit cap of 3,342 -- the shipped
default at prefetch 15 -- against a soft threshold of 800. That is what makes
a cap-keyed regression LEGIBLE rather than invisible: with cap and soft equal,
``channels_total`` is the same number either way; at 3,342 against 800 a
re-keyed surface reports 3,262 (cap minus reserve) where the contract says 720.

The override is safe because no surface in this file is the guard. Neither
readiness nor the metrics endpoint reads the cap or the shed threshold at all,
so a cap that binds admission is still structurally unable to move any answer
below. If a reading here changes when the cap changes, that IS the regression.

The four properties the band walk carries
-----------------------------------------
Stated here so a row cannot be deleted without noticing what it was for:

1. ``channels_total`` is **720** in every row -- the soft threshold (800) minus
   the reserve (80) -- and never 3,262 (the cap minus the reserve) and never
   3,342. This is the anti-regression heart of the file.
2. ``channels_in_use`` is the row's stream count VERBATIM at every point in the
   band and past the cap: never clamped down to ``channels_total``, never
   wrapped. The autoscaler has to see the true size of the overflow.
3. Readiness is NOT_SERVING everywhere past ``soft - reserve``, and keeps
   returning a verdict rather than raising once availability goes negative.
   NOT_SERVING is a steering signal, never a refusal -- the pod keeps serving
   its pinned streams.
4. The endpoint answers **200** at every point, asserted explicitly. A
   ``ValidationError`` inside the handler is mapped to a 400 by
   ``error_handling_middleware``, so ``!= 500`` would not catch it, and a
   body-only test would fail with a bare ``KeyError``.

Scope boundaries -- deliberate non-duplication
----------------------------------------------
- ``tests/app/rest/test_routes.py`` (``TestChannelPoolMetrics``) owns the
  response model's numeric-bound property: a cap-scale reading (20,000 streams)
  cannot turn the feed non-200. That is availability under an unbounded field;
  this file is signal SEMANTICS across the band. No bare 200-at-cap-scale row
  is re-added here, and that file is not edited.
- ``tests/app/test_capacity_surface_consistency.py`` and
  ``tests/app/test_memory_cap_surface_isolation.py`` own cap/shed ISOLATION,
  asserted differentially (identical answers with a threshold binding versus
  neutralized) and cross-surface AGREEMENT. This file asserts neither
  differentially: it pins the absolute values the soft threshold produces.
  Neither file is edited.
- ``tests/app/grpc/test_channel_pool_health_servicer.py``'s
  ``TestCheckOverCapacity`` already covers negative availability at
  ``client_limit`` 16 on a 75-channel pool (availability -200). This file adds
  the SHIPPED-DEFAULT band shape (soft 800, cap 3,342, availability -2,542),
  which that file does not carry, and pairs readiness with the metrics half over
  the same stats report.
- ``tests/integration_tests/grpc/test_ac11_pinned_burst_absorption.py``
  asserts readiness plus metrics at the packed shipped shape as well. That
  overlap is deliberate and differently motivated: there it is evidence for the
  named acceptance scenario AC-11, driven end-to-end through a real burst;
  here it is the standing unit-level regression pin over a synthetic stats
  report. A future de-duplication pass should delete neither.

Why it lives in ``tests/app/``
------------------------------
The "one test file per module under test" rule does not fit a test that spans
``app/grpc/`` and ``app/rest/``. ``tests/app/`` is their nearest common
ancestor.
"""

import logging
from typing import NamedTuple

import pytest
import pytest_asyncio
from fastapi.testclient import TestClient
from grpc_health.v1 import health_pb2

from app.config import settings
from app.grpc.channel_pool_health_servicer import ChannelPoolHealthServicer
from app.lib.rabbitmq.rabbitmq_base import rabbitmq_connector
from app.rest.rest_server import rest_server
from tests.app.rest.test_routes import METRICS_PATH

SERVING = health_pb2.HealthCheckResponse.SERVING
NOT_SERVING = health_pb2.HealthCheckResponse.NOT_SERVING

SERVICER_LOGGER = "app.grpc.channel_pool_health_servicer"

# The shared factory's default pool size. Stream capacity -- the SOFT threshold
# -- is MAX_POOL_SIZE x client_limit, derived by the factory the way the
# library derives it, so no scenario hand-sets a channel_client_limit /
# max_client_capacity pair.
MAX_POOL_SIZE = 100

# The readiness reserve, in non-durable STREAM slots. Pinned at both of its
# consumption points rather than read from settings: a developer .env can
# override the process-global default, and every expectation below depends on
# this being exactly 80.
RESERVE = 80

# The shipped soft (packing) threshold: MAX_POOL_SIZE 100 x the flipped
# client_limit 8 = 800 stream slots. The band walk runs at this density.
SOFT_THRESHOLD = 800

# The memory-safe stream cap, in streams per pod. 3,342 is the shipped default
# (budget 1024 MiB, prefetch 15, 10 replicas; derivation owned by
# test_stream_memory_cap.py), and -- load-bearing here -- it is a DIFFERENT
# number from the soft threshold of 800, so a surface that re-keyed onto it
# would report channels_total 3,262 instead of 720.
STREAM_CAP = 3342

# The re-routable shed threshold that this cap implies at the shipped shape:
# the midpoint of the band the cap tops out, 800 + (3342 - 800) // 2 = 2071.
# Named so the mid-band row is visibly sitting ON the second refusing threshold
# and still reporting a soft-keyed answer.
SHED_THRESHOLD = 2071


class SignalReading(NamedTuple):
    """Both scaling signals, read from one non-durable pool-stats report."""

    readiness_status: int
    status_code: int
    channels_in_use: int
    channels_total: int
    utilization_percent: int


def _build_stats(make_stats, client_limit, streams):
    """
    Build the one pool-stats report a scenario feeds to both surfaces.

    The only arithmetic in this file, and it is input bookkeeping rather than
    any surface's formula: a pool holding ``streams`` clients at
    ``client_limit`` clients per channel has ``ceil(streams / client_limit)``
    channels busy, bounded by the channels the pool can hold. Neither surface
    reads the channel-unit keys -- they are set so the report is a faithful
    stand-in rather than an idle pool implausibly holding 118 clients.

    :param make_stats: The shared ``make_pool_stats_by_pool`` factory.
    :param client_limit: Clients packed per non-durable channel.
    :param streams: Active non-durable streams (clients).
    :returns: A ``pool_stats_by_pool()`` return value.
    """
    busy_channels = min(MAX_POOL_SIZE, (streams + client_limit - 1) // client_limit)

    return make_stats(
        non_durable={
            "total_channel_count": MAX_POOL_SIZE,
            "available_channel_count": MAX_POOL_SIZE - busy_channels,
        },
        non_durable_client_limit=client_limit,
        non_durable_clients=streams,
        non_durable_in_use=busy_channels,
    )


# One row per point in the band, over the SHIPPED-DEFAULT PACKED shape:
# max_pool_size 100 x client_limit 8 = a soft threshold of 800 stream slots, a
# reserve of 80, and a memory cap of 3,342. The whole expected reading is
# literals; derivations live in the comments so no expectation is computed here
# and none is derived from another.
#
# For a reviewer checking a literal by hand: channels_total is 800 - 80 = 720 in
# every row (property 1); channels_in_use is the row's stream count verbatim
# (property 2); utilization_percent is those two as a floored whole-number
# percentage, saturating at 100; readiness is NOT_SERVING once 800 - streams
# drops below 80, i.e. above 720 streams (property 3); the status code is 200
# throughout (property 4).
BAND_WALK = [
    # Availability 400, comfortably above the reserve. 400 x 100 // 720 = 55.
    pytest.param(
        400,
        SignalReading(SERVING, 200, 400, 720, 55),
        id="below-the-reserve-boundary",
    ),
    # Availability is exactly the reserve and readiness compares strictly, so
    # this is the last SERVING row. 720 x 100 // 720 = 100.
    pytest.param(
        720,
        SignalReading(SERVING, 200, 720, 720, 100),
        id="at-the-reserve-boundary",
    ),
    # One stream further: availability 79 < 80, so the pod stops taking new
    # steered work while still admitting. Utilization would be 100.1 and
    # saturates at the model's le=100.
    pytest.param(
        721,
        SignalReading(NOT_SERVING, 200, 721, 720, 100),
        id="past-the-reserve-boundary",
    ),
    # The soft threshold itself: availability 0. It refuses nobody -- the pod
    # enters the overflow band here rather than turning anyone away -- and the
    # signal simply keeps counting.
    pytest.param(
        800,
        SignalReading(NOT_SERVING, 200, 800, 720, 100),
        id="at-the-soft-threshold",
    ),
    # Mid-band, and exactly on the re-routable shed threshold (2071): the
    # second refusing tier is now binding for re-routable streams, and neither
    # signal moves for it. Availability -1,271.
    pytest.param(
        SHED_THRESHOLD,
        SignalReading(NOT_SERVING, 200, 2071, 720, 100),
        id="mid-band-on-the-shed-threshold",
    ),
    # At the memory cap: availability -2,542. channels_total is still 720, not
    # 3,262 -- the single most likely wrong answer a re-key would produce.
    pytest.param(
        STREAM_CAP,
        SignalReading(NOT_SERVING, 200, 3342, 720, 100),
        id="at-the-memory-cap",
    ),
    # Past the cap. Admission cannot put a stream here, but a report can. The
    # reported count is exactly the pool's checkout count -- one checkout per
    # stream on every surface -- so the ONLY way to land past the cap is an
    # operator lowering the cap under an already loaded pod. Both surfaces have
    # to keep answering, so nothing may assume the cap bounds the count.
    pytest.param(
        3400,
        SignalReading(NOT_SERVING, 200, 3400, 720, 100),
        id="past-the-memory-cap",
    ),
]


@pytest.fixture
def client():
    """
    A test client for the REST surface.

    Rebuilt here because the shared ``client`` fixture lives in
    ``tests/app/rest/conftest.py``, which does not reach this directory.
    """
    return TestClient(rest_server)


@pytest.fixture(autouse=True)
def reserve_in_force(mocker):
    """
    Pin the readiness reserve the metrics endpoint reads, per request.

    The reserve has two consumption points and both must be pinned: the
    servicer takes it as a constructor argument (passed explicitly below), and
    the endpoint reads this settings field at request time. No expectation in
    this file may depend on the process-global default.
    """
    mocker.patch.object(
        settings.settings,
        "readiness_min_available_channels",
        RESERVE,
    )


@pytest.fixture(autouse=True)
def stream_cap_distinct_from_the_soft_threshold(mocker):
    """
    Override the autouse neutralizer with a cap of 118 against a soft 100.

    ``tests/app/conftest.py``'s ``non_binding_stream_memory_cap`` parks this
    directory's tests at a cap of 1,000,000, which would make a cap-keyed
    ``channels_total`` come out as 999,920 -- a failure, but an illegible one,
    and one that only exists because the neutralizer happens to be enormous.
    Pinning the shipped default instead makes the wrong answer the plausible
    3,262 (cap 3,342 minus reserve 80), and keeps the whole file describing a
    configuration an operator actually runs.

    ``mocker`` applies patches in order and the later patch wins, so this
    fixture's value is the one in force. It is pattern (a) from the
    neutralizer's docstring, used for the opposite reason: not to make the cap
    bind something, but to prove it cannot.
    """
    mocker.patch.object(
        settings.settings,
        "max_non_durable_streams_per_pod",
        STREAM_CAP,
    )


@pytest.fixture
def servicer():
    """
    The readiness servicer, over the same connector singleton the route reads.

    The reserve goes in positionally: the constructor's parameter name is this
    file's business only insofar as the value must be pinned.
    """
    return ChannelPoolHealthServicer(rabbitmq_connector, RESERVE)


@pytest.fixture
def apply_pool_stats(mocker):
    """
    Publish one stats report on the single seam both surfaces read.

    The patch targets the connector singleton the REST route imports, which is
    the same object the ``servicer`` fixture is built over, so one stub is the
    only source of capacity numbers in this file.
    """

    def _apply(stats_by_pool):
        mocker.patch.object(
            rabbitmq_connector,
            "pool_stats_by_pool",
            return_value=stats_by_pool,
        )

    return _apply


@pytest.fixture
def read_signals(client, servicer, apply_pool_stats):
    """Return a callable that drives both scaling surfaces over one report."""

    async def _read(stats_by_pool):
        apply_pool_stats(stats_by_pool)

        readiness = await servicer.Check(None, None)
        metrics = client.get(METRICS_PATH)

        # Read defensively: on a non-200 the body carries an error payload, and
        # the reading has to surface as the status-code mismatch this file is
        # asserting rather than as a bare KeyError.
        body = metrics.json() if metrics.status_code == 200 else {}

        return SignalReading(
            readiness_status=readiness.status,
            status_code=metrics.status_code,
            channels_in_use=body.get("channels_in_use"),
            channels_total=body.get("channels_total"),
            utilization_percent=body.get("utilization_percent"),
        )

    return _read


class TestScalingSignalAcrossTheOverflowBand:
    """
    Both scaling signals, walked from below the reserve to past the memory cap.
    """

    @pytest.mark.parametrize("streams, expected", BAND_WALK)
    @pytest.mark.asyncio
    async def test_both_signals_read_as_expected(
        self, read_signals, make_pool_stats_by_pool, streams, expected
    ):
        """
        The whole reading is asserted at once, deliberately.

        The one-assertion-per-test convention yields here for the same reason
        it does in ``test_capacity_surface_consistency.py``: the property is
        that the two surfaces stay coherent with each other over one report,
        and splitting the tuple into an assertion per surface reproduces
        exactly the arrangement that cannot see them disagree.
        """
        stats = _build_stats(make_pool_stats_by_pool, client_limit=8, streams=streams)

        reading = await read_signals(stats)

        assert reading == expected


class TestSoftThresholdMovesTheSignalNotTheCap:
    """
    Move the pool's own stream capacity and both signals follow it.

    The band walk pins the values at the shipped shape (soft 800); these two pin
    what they are keyed ON, by moving the soft threshold away from it -- down to
    100 at ``client_limit`` 1 and up to 1,600 at ``client_limit`` 16. The cap
    stays at 3,342 throughout, so a cap-keyed surface would answer identically
    to the band walk here -- and a shed-keyed one likewise, since the shed line
    is clamped up to the soft threshold and moves for a different reason. Only a
    soft-keyed surface produces the numbers below.
    """

    @pytest.mark.asyncio
    async def test_a_lowered_soft_threshold_moves_both_signals(
        self, read_signals, make_pool_stats_by_pool
    ):
        """
        100 streams: SERVING at the walk's soft 800, NOT_SERVING at soft 100.

        Same stream count as the ``at-the-soft-threshold`` row of the band
        walk, and the opposite readiness verdict, because 100 x 1 = 100 stream
        slots leaves availability 0 against a reserve of 80. channels_total is
        100 - 80 = 20 -- not 720, which is what the walk's soft 800 yields, and
        not 3,262, which the unchanged cap of 3,342 would give a cap-keyed
        surface. Utilization is 100 x 100 // 20 = 500, saturating at 100.
        """
        stats = _build_stats(make_pool_stats_by_pool, client_limit=1, streams=100)

        reading = await read_signals(stats)

        assert reading == SignalReading(NOT_SERVING, 200, 100, 20, 100)

    @pytest.mark.asyncio
    async def test_the_reading_stays_truthful_deep_into_a_packed_band(
        self, read_signals, make_pool_stats_by_pool
    ):
        """
        5,000 streams at a soft threshold of 1,600 (client_limit 16).

        3,400 streams past the soft threshold, with all 100 channels long since
        busy: channels_in_use is 5000 verbatim rather than clamped to 1,520,
        availability is -3,400 so readiness is NOT_SERVING, and the feed still
        answers 200. channels_total is 1,600 - 80 = 1,520 -- soft-keyed, not
        3,262. Utilization saturates at 100.
        """
        stats = _build_stats(make_pool_stats_by_pool, client_limit=16, streams=5000)

        reading = await read_signals(stats)

        assert reading == SignalReading(NOT_SERVING, 200, 5000, 1520, 100)


class TestNegativeAvailabilityIsReportedInTheReadinessLog:
    """
    The over-capacity readiness log at the shipped-default band shape.

    A knowing exception to the test-writing guide's "don't test logging output"
    rule, on the same grounds
    ``tests/app/grpc/test_channel_pool_health_servicer.py`` documents for its
    ``client_limit`` 16 case: this line is an operator-facing signature the
    production diagnostics runbook keys on (the source repository's internal
    runbook, which is not published), and a pod sitting in
    the overflow band is exactly when an operator goes looking for it. What is
    added here is the shipped shape -- soft 800, 3,342 streams in use,
    availability -2,542 -- where that file carries -200 at a density production
    does not run yet. The unit word and the number are asserted, never the
    sentence.
    """

    @pytest.fixture
    def at_the_memory_cap(self, apply_pool_stats, make_pool_stats_by_pool):
        # 800 stream slots of soft capacity against 3,342 active streams (at the
        # hard bound): the pod is full, and availability is 800 - 3342 = -2,542.
        apply_pool_stats(
            _build_stats(
                make_pool_stats_by_pool,
                client_limit=8,
                streams=STREAM_CAP,
            )
        )

    @pytest_asyncio.fixture
    async def readiness_log_messages(self, servicer, at_the_memory_cap, caplog):
        with caplog.at_level(logging.INFO, logger=SERVICER_LOGGER):
            await servicer.Check(None, None)

        return [
            record.getMessage()
            for record in caplog.records
            if record.name == SERVICER_LOGGER and record.levelno == logging.INFO
        ]

    @pytest.mark.asyncio
    async def test_availability_is_reported_in_stream_units(
        self, readiness_log_messages
    ):
        assert any("streams" in message for message in readiness_log_messages)

    @pytest.mark.asyncio
    async def test_the_negative_availability_value_is_reported_unwrapped(
        self, readiness_log_messages
    ):
        assert any("-2542" in message for message in readiness_log_messages)
