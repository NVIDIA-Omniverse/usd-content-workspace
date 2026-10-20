# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Acceptance test: all three capacity surfaces agree on the same stream count.

What this file asserts
----------------------
One scenario table -- a single non-durable channel-pool stats report -- is fed
to every surface that answers a capacity question, at packing densities
``client_limit`` 1, 8 and 16 (PRD AC-3). The three surfaces are:

- ``CapacityGuard.has_capacity()`` (``app/lib/rabbitmq/capacity_guard.py``),
  the non-durable admission decision
- ``ChannelPoolHealthServicer.Check()``
  (``app/grpc/channel_pool_health_servicer.py``), the Kubernetes readiness
  verdict
- ``GET /api/v1beta/metrics/channel-pool`` (``app/rest/routes.py``), the
  autoscaler's only capacity input

Every expectation in the table is an independently reasoned literal. No test
here recomputes a surface's formula, so this file cannot go stale into a false
green by drifting along with the code it checks.

One seam, on purpose
--------------------
All three surfaces read ``RabbitMQConnector.pool_stats_by_pool()`` and select
its non-durable entry, so one stub on ONE connector -- the
``rabbitmq_connector`` singleton exported from
``app/lib/rabbitmq/rabbitmq_base.py``, which is the object the REST route
reads -- drives all three. The guard and the servicer are built over that same
singleton rather than over private mocks. Feeding three surfaces from three
mocks would pass even if one of them had quietly started reading a different
connector; feeding them from one cannot.

Why this file exists at all
---------------------------
Each surface already has a per-surface unit file, and all three can stay green
while the three disagree, because none of them ever sees the same input twice.
This file is the only place a disagreement is visible. It is deliberately NOT a
re-run of those tables: idle baselines, ``initialized``/``closed`` lifecycle,
missing entries, missing fields, negative-count clamping and durable-side
admission all live in the per-surface files and are not duplicated here:

- ``tests/app/lib/rabbitmq/test_capacity_guard.py``
- ``tests/app/grpc/test_channel_pool_health_servicer.py``
- ``tests/app/rest/test_routes.py``

Its third standing value is structural: a fourth capacity surface added later
with no test of its own shows up as a surface this table does not drive.

Why it lives in ``tests/app/``
------------------------------
The test-writing guide's "one test file per module under test" rule does not
fit a test that spans ``app/lib/rabbitmq/``, ``app/grpc/`` and ``app/rest/``.
``tests/app/`` is their nearest common ancestor, so the file sits there rather
than being arbitrarily filed under one of the three surfaces it checks.

The three properties the table carries
--------------------------------------
A future editor must not be able to delete a row without noticing what it was
for:

1. **Readiness NOT_SERVING is a steering signal, never a refusal.** Every row
   from ``past-the-reserve-boundary`` upward carries the same disagreement:
   from one stream number readiness answers NOT_SERVING while the guard still
   admits. That is the contract at both ends of the band, for two reasons that
   stack. Below the pool's stream capacity, readiness trips early by the
   reserve to give the autoscaler headroom, while the guard admits against full
   pool capacity so Envoy traffic can use every slot. At and above that
   capacity -- the ``at-pool-capacity`` and ``past-pool-capacity`` rows -- the
   guard admits into the pool's OVERFLOW BAND (Epic 7 Task 2; SDD SR-3.1 as
   amended), because a non-durable consumer stream is pinned to the pod it
   lands on and ``publishNotReadyAddresses: true`` keeps a NOT_SERVING pod
   addressable: refusing there would be a hard client failure that no amount of
   scale-out can rescue (SDD SR-3.2 as amended, PRD FR-11). So the readiness
   column answers "stop steering new work here", never "this pod refuses", and
   the at/past-capacity rows are instances of this property rather than
   exceptions to it. Reading a refusal out of a NOT_SERVING row is the mistake
   this table exists to catch.
2. **The reserve is a flat stream count, never scaled by the packing factor.**
   ``channels_total`` is capacity minus 20 at every density: 80, 780, 1580 --
   not capacity minus 20 x limit. This file's ``RESERVE = 20`` is deliberately
   NOT the shipped default (now 80): pinning against a non-default reserve makes
   the property strictly stronger, since a code path that hardcoded the shipped
   default instead of reading the setting would fail these rows.
3. **Durable-pool load is invisible to all three surfaces.** Each
   ``durable-load-changes-nothing`` row carries a fully loaded durable pool and
   expects verdict literals identical to its unloaded twin. This inverts the
   pool-location invariance the pre-Epic version of this file asserted over the
   interim combined view: that property is now false BY DESIGN, because durable
   checkouts no longer consume the non-durable surface's slots. The inversion
   is the decided change, not drift.

Both refusing admission tiers are out of scope here
---------------------------------------------------
Non-durable admission is three-tier and only the outer two tiers refuse: the
memory-safe stream cap is the HARD BOUND and refuses every class, and the
re-routable SHED THRESHOLD inside the overflow band refuses re-routable streams
only. Both are neutralized for this file by the autouse fixture in
``tests/app/conftest.py``, which patches ``max_non_durable_streams_per_pod``
far above every capacity these fixtures can express: that lifts the cap
directly and the shed threshold along with it through the clamp, since the shed
default is the midpoint of the band the cap tops out. Read that fixture's
docstring before changing anything here -- it is where the two-tier
neutralization and its three opt-in patterns are written down.

That is relied on, not reconfigured: configuring either tier per scenario would
encode the cap's budget / prefetch / replica arithmetic, or the shed band's
midpoint, into an acceptance test whose property is pool-bound agreement, and
the prefetch default is still being tuned. Cap-versus-surface and
shed-versus-surface isolation both have a dedicated owner in
``tests/app/test_memory_cap_surface_isolation.py``, which asserts them
differentially.

The neutralization is load-bearing in a way it was not before two-tier
admission. The soft threshold refuses nobody, so with the cap and the shed line
both out of reach there is NO non-durable refusal reachable at any stream count
these fixtures can express -- which is why every ``guard_admits`` in the table
below is True rather than only most of them. The shed line is out of reach
twice over, because ``collect_verdicts`` drives ``has_capacity()`` at its
default class (pinned) and the shed tier refuses only re-routable streams; the
neutralizer is what keeps the table honest if a future editor passes
``re_routable=True``. If a guard row here unexpectedly rejects, one of those
two tiers has started binding -- fix the fixture, do not lower the table's
stream numbers.

What is NOT here, and why that is not lost coverage
---------------------------------------------------
There is no separate "no surface raises past the bound" sweep. The
``past-pool-capacity`` rows already subsume that check: a raise fails them, and
fails them with the offending row id attached rather than as a bare failure
with no scenario named.

Those rows are not modelling a pathology. Under two-tier admission ``in_use``
past the packing product is NORMAL OPERATION: the pool's stream capacity is a
scaling target the guard admits straight through, so a loaded pod sits in the
overflow band by design and every surface has to keep answering coherently
there. The ``at-`` and ``past-pool-capacity`` rows model that designed steady
state, and a reader who takes them for a race or a negative count will draw the
wrong conclusion from the ``guard_admits`` column.
"""

import pytest
from fastapi.testclient import TestClient
from grpc_health.v1 import health_pb2
from typing import NamedTuple

from app.config import settings
from app.grpc.channel_pool_health_servicer import ChannelPoolHealthServicer
from app.lib.rabbitmq.capacity_guard import CapacityGuard
from app.lib.rabbitmq.rabbitmq_base import rabbitmq_connector
from app.rest.rest_server import rest_server
from tests.app.rest.test_routes import METRICS_PATH, METRICS_RESPONSE_KEYS

SERVING = health_pb2.HealthCheckResponse.SERVING
NOT_SERVING = health_pb2.HealthCheckResponse.NOT_SERVING

# The shared factory's default pool size. Stream capacity at any density is
# MAX_POOL_SIZE x client_limit, derived by the factory the way the library
# derives it, so no scenario hand-sets a channel_client_limit /
# max_client_capacity pair.
MAX_POOL_SIZE = 100

# The readiness reserve, in non-durable STREAM slots. Patched in at both of its
# consumption points rather than read from settings: a developer .env can
# override the process-global default, and every expectation below depends on
# this being exactly 20.
RESERVE = 20

# The fully loaded durable pool carried by the durable-load-changes-nothing
# rows: its own packing density, its entire stream budget in use, and all of
# its channels busy. Sized past every non-durable capacity in the table so that
# any surface still summing the two pools is unmistakably wrong.
DURABLE_LOADED_CLIENT_LIMIT = 16
DURABLE_LOADED_STREAMS = 1600
DURABLE_LOADED_CHANNELS = 100


class SurfaceVerdicts(NamedTuple):
    """Every capacity answer the service gives, for one pool-stats report."""

    guard_admits: bool
    readiness_status: int
    channels_in_use: int
    channels_total: int
    utilization_percent: int


def _build_scenario_stats(make_stats, client_limit, streams, durable_loaded):
    """
    Build the one pool-stats report a scenario feeds to all three surfaces.

    The only arithmetic in this file, and it is input bookkeeping rather than
    any surface's formula: a pool holding ``streams`` clients at ``client_limit``
    clients per channel has ``ceil(streams / client_limit)`` channels busy,
    bounded by the channels the pool can hold, with the rest idle.

    On the over-capacity rows that count saturates at ``MAX_POOL_SIZE`` and
    stops tracking the stream count. That is correct for a pool that cannot
    open another channel, no surface reads these keys, and the mismatch is not
    a defect to "fix" -- it is what makes the channel-unit and stream-unit
    answers differ, which is what the table is built to detect.

    :param make_stats: The shared ``make_pool_stats_by_pool`` factory.
    :param client_limit: Clients packed per non-durable channel.
    :param streams: Active non-durable streams (clients).
    :param durable_loaded: True to also load the durable pool to its own limit.
    :returns: A ``pool_stats_by_pool()`` return value.
    """
    busy_channels = min(MAX_POOL_SIZE, (streams + client_limit - 1) // client_limit)
    kwargs = {
        "non_durable": {
            "total_channel_count": MAX_POOL_SIZE,
            "available_channel_count": MAX_POOL_SIZE - busy_channels,
        },
        "non_durable_client_limit": client_limit,
        "non_durable_clients": streams,
        "non_durable_in_use": busy_channels,
    }

    if durable_loaded:
        kwargs.update(
            {
                "durable": {
                    "total_channel_count": MAX_POOL_SIZE,
                    "available_channel_count": 0,
                },
                "durable_client_limit": DURABLE_LOADED_CLIENT_LIMIT,
                "durable_clients": DURABLE_LOADED_STREAMS,
                "durable_in_use": DURABLE_LOADED_CHANNELS,
            }
        )

    return make_stats(**kwargs)


# One row per scenario: three input columns (client_limit, active streams,
# whether the durable pool is loaded) and the whole expected verdict tuple as
# literals. Derivations live in the ids and comments so that no expectation is
# computed here, and no expectation is derived from another.
#
# For a reviewer checking a literal by hand: channels_total is that density's
# stream capacity less the reserve of 20, stated per density group below;
# channels_in_use is the row's stream count verbatim; and utilization_percent
# is those two as a whole-number percentage, floored, and clamped at 100 once
# the streams in use reach channels_total.
SCENARIOS = [
    # --- client_limit 1: 100 pool x limit 1 = 100 streams, less the reserve of
    # 20 gives channels_total 80. The one density where a stream is a channel.
    pytest.param(
        1, 0, False, SurfaceVerdicts(True, SERVING, 0, 80, 0), id="limit-1-idle"
    ),
    pytest.param(
        1,
        50,
        False,
        SurfaceVerdicts(True, SERVING, 50, 80, 62),
        id="limit-1-below-the-reserve-boundary",
    ),
    # Property 3: identical verdicts to the row above, with the durable pool
    # fully loaded.
    pytest.param(
        1,
        50,
        True,
        SurfaceVerdicts(True, SERVING, 50, 80, 62),
        id="limit-1-durable-load-changes-nothing",
    ),
    # Available is exactly the reserve, and readiness compares strictly, so
    # this is still SERVING.
    pytest.param(
        1,
        80,
        False,
        SurfaceVerdicts(True, SERVING, 80, 80, 100),
        id="limit-1-at-the-reserve-boundary",
    ),
    # Property 1: readiness trips while the guard still admits.
    pytest.param(
        1,
        81,
        False,
        SurfaceVerdicts(True, NOT_SERVING, 81, 80, 100),
        id="limit-1-past-the-reserve-boundary",
    ),
    # Property 1 at the far end of the band. The pool's 100 stream slots are
    # the SOFT threshold, and it refuses no one: the guard admits the 101st
    # stream into the overflow band while readiness stays NOT_SERVING (Epic 7
    # Task 2, SDD SR-3.1 as amended). Pre-Epic this row expected a rejection.
    pytest.param(
        1,
        100,
        False,
        SurfaceVerdicts(True, NOT_SERVING, 100, 80, 100),
        id="limit-1-at-pool-capacity",
    ),
    # Five streams into the overflow band, and still admitting: past the soft
    # threshold the next refusal belongs to the hard bound, which is
    # neutralized here (Epic 7 Task 2, SDD SR-3.1 as amended).
    pytest.param(
        1,
        105,
        False,
        SurfaceVerdicts(True, NOT_SERVING, 105, 80, 100),
        id="limit-1-past-pool-capacity",
    ),
    # --- client_limit 8: 100 pool x limit 8 = 800 streams, less the reserve of
    # 20 gives channels_total 780. Property 2: 780, not 800 - 20 x 8.
    pytest.param(
        8, 0, False, SurfaceVerdicts(True, SERVING, 0, 780, 0), id="limit-8-idle"
    ),
    pytest.param(
        8,
        400,
        False,
        SurfaceVerdicts(True, SERVING, 400, 780, 51),
        id="limit-8-below-the-reserve-boundary",
    ),
    # Property 3.
    pytest.param(
        8,
        400,
        True,
        SurfaceVerdicts(True, SERVING, 400, 780, 51),
        id="limit-8-durable-load-changes-nothing",
    ),
    # 780 streams is 98 busy channels, leaving 2 channels idle: a readiness
    # surface still keyed on CHANNELS would report NOT_SERVING here, where the
    # stream-keyed one is SERVING on exactly-the-reserve available streams.
    pytest.param(
        8,
        780,
        False,
        SurfaceVerdicts(True, SERVING, 780, 780, 100),
        id="limit-8-at-the-reserve-boundary",
    ),
    # Property 1.
    pytest.param(
        8,
        781,
        False,
        SurfaceVerdicts(True, NOT_SERVING, 781, 780, 100),
        id="limit-8-past-the-reserve-boundary",
    ),
    # All 100 channels are busy while one stream slot remains: a channel-keyed
    # guard would reject here, where the stream-keyed one admits.
    pytest.param(
        8,
        799,
        False,
        SurfaceVerdicts(True, NOT_SERVING, 799, 780, 100),
        id="limit-8-all-channels-busy-one-stream-slot-left",
    ),
    # Property 1 at the far end of the band, at packing density 8: the 800
    # stream slots are the soft threshold, so the guard admits into the
    # overflow band rather than refusing (Epic 7 Task 2, SDD SR-3.1 as
    # amended). Pre-Epic this row expected a rejection.
    pytest.param(
        8,
        800,
        False,
        SurfaceVerdicts(True, NOT_SERVING, 800, 780, 100),
        id="limit-8-at-pool-capacity",
    ),
    # 100 streams into the overflow band, all 100 channels long since busy,
    # and still admitting: the next refusal is the neutralized hard bound's
    # (Epic 7 Task 2, SDD SR-3.1 as amended).
    pytest.param(
        8,
        900,
        False,
        SurfaceVerdicts(True, NOT_SERVING, 900, 780, 100),
        id="limit-8-past-pool-capacity",
    ),
    # --- client_limit 16: 100 pool x limit 16 = 1600 streams, less the reserve
    # of 20 gives channels_total 1580. Property 2 again, at twice the density.
    pytest.param(
        16, 0, False, SurfaceVerdicts(True, SERVING, 0, 1580, 0), id="limit-16-idle"
    ),
    pytest.param(
        16,
        395,
        False,
        SurfaceVerdicts(True, SERVING, 395, 1580, 25),
        id="limit-16-below-the-reserve-boundary",
    ),
    # Property 3.
    pytest.param(
        16,
        395,
        True,
        SurfaceVerdicts(True, SERVING, 395, 1580, 25),
        id="limit-16-durable-load-changes-nothing",
    ),
    # 1580 streams is 99 busy channels, one channel still idle -- again SERVING
    # on available streams, where a channel-keyed reserve would have tripped.
    pytest.param(
        16,
        1580,
        False,
        SurfaceVerdicts(True, SERVING, 1580, 1580, 100),
        id="limit-16-at-the-reserve-boundary",
    ),
    # Property 1.
    pytest.param(
        16,
        1581,
        False,
        SurfaceVerdicts(True, NOT_SERVING, 1581, 1580, 100),
        id="limit-16-past-the-reserve-boundary",
    ),
    # All 100 channels busy, one stream slot left: the guard admits.
    pytest.param(
        16,
        1599,
        False,
        SurfaceVerdicts(True, NOT_SERVING, 1599, 1580, 100),
        id="limit-16-all-channels-busy-one-stream-slot-left",
    ),
    # Property 1 at the far end of the band, at twice that density: 1600 is
    # the soft threshold, and the guard admits into the overflow band there
    # too (Epic 7 Task 2, SDD SR-3.1 as amended). Pre-Epic this row expected
    # a rejection.
    pytest.param(
        16,
        1600,
        False,
        SurfaceVerdicts(True, NOT_SERVING, 1600, 1580, 100),
        id="limit-16-at-pool-capacity",
    ),
    # The deepest overflow the table expresses -- 100 streams past a soft
    # threshold of 1600 -- and the guard still admits, because only the
    # neutralized hard bound refuses above it (Epic 7 Task 2, SDD SR-3.1 as
    # amended).
    pytest.param(
        16,
        1700,
        False,
        SurfaceVerdicts(True, NOT_SERVING, 1700, 1580, 100),
        id="limit-16-past-pool-capacity",
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
    the endpoint reads this settings field at request time. Neither
    expectation in the table may depend on the process-global default.
    """
    mocker.patch.object(
        settings.settings,
        "readiness_min_available_channels",
        RESERVE,
    )


@pytest.fixture
def apply_pool_stats(mocker):
    """
    Publish one stats report on the single seam all three surfaces read.

    The patch targets the connector singleton the REST route imports, and the
    guard and servicer built below are handed that same object, so one stub is
    the only source of capacity numbers in this file.
    """

    def _apply(stats_by_pool):
        mocker.patch.object(
            rabbitmq_connector,
            "pool_stats_by_pool",
            return_value=stats_by_pool,
        )

    return _apply


@pytest.fixture
def collect_verdicts(client, apply_pool_stats):
    """Return a callable that drives all three surfaces over one stats report."""

    async def _collect(stats_by_pool):
        apply_pool_stats(stats_by_pool)

        guard = CapacityGuard(connector=rabbitmq_connector)
        # The reserve goes in positionally: the constructor's parameter name is
        # this file's business only insofar as the value must be pinned.
        servicer = ChannelPoolHealthServicer(rabbitmq_connector, RESERVE)

        readiness = await servicer.Check(None, None)
        metrics = client.get(METRICS_PATH)

        # A 500 body would otherwise reach the comparison as a KeyError rather
        # than as the surface disagreement this file reports.
        assert metrics.status_code == 200

        body = metrics.json()
        return SurfaceVerdicts(
            guard_admits=guard.has_capacity(),
            readiness_status=readiness.status,
            channels_in_use=body["channels_in_use"],
            channels_total=body["channels_total"],
            utilization_percent=body["utilization_percent"],
        )

    return _collect


class TestCapacitySurfacesAgree:
    """
    Every capacity surface, over one stats report, at three packing densities.
    """

    @pytest.mark.parametrize(
        "client_limit, streams, durable_loaded, expected", SCENARIOS
    )
    @pytest.mark.asyncio
    async def test_all_surfaces_answer_the_scenario_as_expected(
        self,
        collect_verdicts,
        make_pool_stats_by_pool,
        client_limit,
        streams,
        durable_loaded,
        expected,
    ):
        """
        The whole verdict tuple is asserted at once, deliberately.

        The one-assertion-per-test convention yields here: cross-surface
        agreement IS the property under test, and splitting it into one
        assertion per surface reproduces exactly the per-surface arrangement
        that cannot see a disagreement in the first place.
        """
        stats = _build_scenario_stats(
            make_pool_stats_by_pool, client_limit, streams, durable_loaded
        )

        verdicts = await collect_verdicts(stats)

        assert verdicts == expected


class TestAutoscalerProtocol:
    """
    The metrics response body is a protocol the autoscaler reads by name.

    Renaming a key or adding a fourth breaks the scaler's parsing, so the key
    set is pinned in the Epic's acceptance test as well as in the endpoint's
    own file. The overlap with ``tests/app/rest/test_routes.py`` is intended,
    and cannot drift, because the key set is imported from there rather than
    re-declared here.

    The names stay wrong-by-noun on purpose: the values are stream slots, not
    channels, and renaming them would break a running autoscaler across a
    rollout. The rename is tracked separately; see the deferral note in
    ``tests/app/test_memory_cap_surface_isolation.py``.
    """

    def test_response_contains_exactly_the_autoscaler_fields(
        self, client, apply_pool_stats, make_pool_stats_by_pool
    ):
        apply_pool_stats(
            _build_scenario_stats(
                make_pool_stats_by_pool,
                client_limit=8,
                streams=400,
                durable_loaded=False,
            )
        )

        response = client.get(METRICS_PATH)

        assert set(response.json()) == METRICS_RESPONSE_KEYS
