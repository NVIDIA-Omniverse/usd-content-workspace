# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Named acceptance test for PRD AC-11: a pinned burst is absorbed, not refused.

AC-11 verbatim: "A single pinned in-cluster gRPC connection opens a burst
of streams against a pod already at its soft packing threshold: every
stream in the burst is admitted (absorbed into the overflow band), the pod
reports NOT_SERVING, the autoscaler signal reflects over-threshold load,
and 503s appear only when the same pod is driven to the memory-safe cap."

All four clauses are asserted HERE, in one scenario, rather than being
scattered across the per-surface files -- co-locating them is the point of
a named acceptance test, because the clauses only mean anything together.
The file runs at TWO configurations: a legacy operator row (client_limit 1,
prefetch 500) retained for band-shape coverage, and the shipped PACKED row
(client_limit 8, prefetch 15, soft threshold 800) that Epic 3 turns on.

What the RPC arms prove, and what they do NOT
---------------------------------------------
The RPC classes below (1-3) assert ABSORPTION and REFUSAL against an
explicitly DRIVEN counted load. They do NOT assert count arithmetic, and
the reason is a STATS SEAM, not the ConnectionChannelManager (which Epic 3
Task 1 deleted -- one admitted stream is now one checkout everywhere).
In this file the pod's counted load is SYNTHETIC: ``PodLoad`` drives a
patched ``pool_stats_by_pool()``, while the bursting RPCs check out of a
different, mock-broker pool. So the arms cannot self-count -- the counted
load and the checkouts are two different seams. The synthetic load stands
in for other clients' ordinary load filling the pod, which is what lets the
pinned burst be read as absorption (no 503s) rather than as arithmetic.

The band ARITHMETIC -- exactly N streams admitted, the N+1th refused -- is
asserted at GUARD level in Class 4, where each admitted stream counts as
one checkout. That is the SHIPPED production counting model, post-CCM. The
end-to-end counted==actual proof over REAL pools -- 600 real checkouts
packing onto 75 real channels -- lives in the sibling AC-1 subtask,
``tests/integration_tests/rabbitmq/test_ac1_six_hundred_stream_burst.py``;
this file cross-references it rather than restating it.

Second modelling boundary: the RPC arms also do NOT exercise the library's
overflow ACQUISITION. The mock-broker pool the bursting streams actually
check out of still has free slots, so no limit-ignoring overflow scan ever
runs here. The overflow band's acquisition path is the library's lane
(``notification_common``); this file asserts the service's admission
DECISION above the soft threshold, which is a different thing.

Publishers remain unblocked
---------------------------
Not re-proved here. It is discharged arithmetically in
``tests/app/lib/rabbitmq/test_stream_memory_cap.py`` by the fleet-budget
invariant (cap x ``scaler_max_replicas`` x per-stream bytes <= budget).
There is no broker memory watermark in this harness to provoke -- see the
reasoning already written out in ``test_memory_safe_stream_cap.py``.

"503 + backoff": only the 503 half is assertable
------------------------------------------------
AC-11's "503s" are asserted as ``RESOURCE_EXHAUSTED`` plus the pinned
exhaustion detail. There is deliberately NO backoff assertion: refusal
travels as ``ChannelNotAvailableError``, which carries no ``Retry-After``
or any other wire artifact a client could read as a backoff instruction.
The backoff half of the contract is documented client guidance (Task 6's
prose), so its absence from the assertions below is a decision, not a gap.

Capacity numbers are installed, never inherited
-----------------------------------------------
There is no autouse cap neutralizer under ``tests/integration_tests/``
(unlike ``tests/app/conftest.py``), so every capacity number in this file is
one this file installs explicitly. Each is a literal with its derivation in
a comment; no test here recomputes a surface's formula or re-derives the cap.
"""

import asyncio
import json
from dataclasses import dataclass

import grpc
import pytest
from fastapi.testclient import TestClient
from grpc.aio import AioRpcError
from grpc_health.v1 import health_pb2
from notification_common.config.common_settings import common_settings
from notification_common.errors.errors import ChannelNotAvailableError

from app.config import settings
from app.grpc.channel_pool_health_servicer import ChannelPoolHealthServicer
from app.lib.models.event_message import EventMessage
from app.lib.rabbitmq import capacity_guard
from app.lib.rabbitmq.capacity_guard import CapacityGuard
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase, rabbitmq_connector
from app.rest.rest_server import rest_server
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ConsumeNonDurableEventsRequest,
)
from tests.app.conftest import _make_pool_stats_by_pool
from tests.app.rest.test_routes import METRICS_PATH
from tests.integration_tests.conftest import (
    _create_raw_mock_messages_gen,
    assert_all_messages_streamed,
    filter_group,
)

RESOURCE_EXHAUSTED = grpc.StatusCode.RESOURCE_EXHAUSTED
NOT_SERVING = health_pb2.HealthCheckResponse.NOT_SERVING
SERVING = health_pb2.HealthCheckResponse.SERVING

# The client-visible detail for a capacity refusal: ChannelNotAvailableError's
# user_message, relayed verbatim by the shared ExceptionInterceptor. Pinned as
# a literal so a reword of what the client actually reads fails here.
CHANNEL_POOL_EXHAUSTED_DETAIL = "Channel pool is exhausted"

# --- The hard bound, at both configurations ------------------------------
# Both caps are OWNED by tests/app/lib/rabbitmq/test_stream_memory_cap.py
# (CAP_AT_SHIPPED_PREFETCH / CAP_AT_LOW_PREFETCH); that file pins the
# derivation. Restated here as literals with a one-line derivation, and
# deliberately NOT recomputed from production code.
#   budget 1024 MiB / (500 x 1800 + 5120) B = 1,186 fleet / 10 replicas = 118
CAP_AT_LEGACY_PREFETCH = 118
#   budget 1024 MiB / (15 x 1800 + 5120) B = 33,429 fleet / 10 replicas = 3,342
CAP_AT_PACKED_PREFETCH = 3_342

# The apportioning inputs the computed cap reads (app/config/settings.py
# defaults), pinned explicitly so no expectation here depends on an
# untracked local .env.
LEGACY_PREFETCH = 500
PACKED_PREFETCH = 15
BUDGET_MIB = 1024
MAX_REPLICAS = 10

# The PRD's measured worst-case desktop client (AC-1 / G-1): one scene load
# opens ~600 streams on ONE connection. At the shipped packed defaults this
# 600 fits inside the overflow band of 2,542 AND, separately, sits BELOW the
# soft threshold of 800 when a pod starts empty -- so on an idle pod it never
# reaches the band at all. The AC-11 scenario deliberately starts the pod AT
# its soft threshold, which is what pushes the same burst into the band.
WORST_CASE_CLIENT_BURST = 600

# Utilization is clamped at 100 once streams in use reach channels_total, so
# every count in the band reports exactly 100.
SATURATED_UTILIZATION_PERCENT = 100

# How many concurrent in-process RPCs the RPC arms open at each configuration.
# Kept small and INDEPENDENT of ``burst`` (which the guard walk uses): the
# 600-stream claim is carried by the guard walk in Class 4 and by the AC-1
# file's real-pool checkouts, and opening 600 concurrent in-process RPCs here
# would be a different, much slower test that proves nothing extra.
RPC_BURST = 18

# The shared mock message generator yields three events on every path; on the
# non-durable path they follow the reconnect-token response.
EXPECTED_EVENT_COUNT = 3


@dataclass(frozen=True)
class CapacityConfiguration:
    """
    One capacity configuration this acceptance test runs at, end to end.

    Expressed as data rather than as inline patches so the same clauses run at
    both the legacy operator geometry and the shipped packed defaults without
    editing arithmetic. Every value is a LITERAL with its derivation in the row
    comment -- the file's rule (see the module docstring) is that no test
    recomputes a surface's formula.

    :param client_limit: The non-durable pool's per-channel client limit,
        passed to ``_make_pool_stats_by_pool`` so ``max_client_capacity`` (the
        soft threshold) is derived exactly as the library derives it.
    :param soft_threshold: The non-durable pool's own stream capacity
        (``max_pool_size`` 100 x ``client_limit``) -- the packing density the
        pool is provisioned for, and the load AC-11's burst opens against.
    :param absolute_cap: The ``max_non_durable_streams_per_pod`` operator
        override to install, or None to drive the COMPUTED cap path. None is
        what makes the packed row a real test of prefetch 15: an absolute cap
        would short-circuit the formula before it read the prefetch at all.
    :param prefetch: The library prefetch the computed cap is derived from.
    :param cap: The hard bound this configuration lands on, as a literal.
    :param band: ``cap`` less the soft threshold, as a literal.
    :param burst: The burst size the guard walk asserts absorption of.
    :param rpc_burst: How many concurrent RPCs the RPC arms open at this
        configuration -- small and independent of ``burst``.
    :param readiness_reserve: The ``readiness_min_available_channels`` reserve
        this configuration installs, in stream slots.
    :param autoscaler_channels_total: The usable capacity the metrics surface
        reports, ``soft_threshold - readiness_reserve``, as a literal.
    :param quiet_load: A counted load well below the reserve boundary, used as
        the readiness SERVING control.
    """

    client_limit: int
    soft_threshold: int
    absolute_cap: int | None
    prefetch: int
    cap: int
    band: int
    burst: int
    rpc_burst: int
    readiness_reserve: int
    autoscaler_channels_total: int
    quiet_load: int


# The LEGACY operator configuration: a pool of 100 channels at one client each,
# so the soft threshold is 100. This is NOT today's shipped defaults -- it is a
# non-default operator geometry retained because its narrow, legible band
# (cap 118, band 18) keeps the band-shape coverage this file has always had.
# The cap goes in as the absolute override so the RPC arms cannot be perturbed
# by an ambient prefetch. Derivations:
#   soft threshold = pool_size 100 x client_limit 1 = 100
#   cap 118 (legacy prefetch 500, see CAP_AT_LEGACY_PREFETCH)
#   band = 118 - 100 = 18
#   channels_total = soft 100 - reserve 20 = 80
#   reserve 20 is chosen so the reserve boundary stays legible against a soft
#   threshold of 100 (100 - 20 = 80 usable); it is NOT a shipped value.
#   quiet_load 50: available 100 - 50 = 50 >= reserve 20 -> SERVING.
LEGACY_CONFIGURATION = CapacityConfiguration(
    client_limit=1,
    soft_threshold=100,
    absolute_cap=CAP_AT_LEGACY_PREFETCH,
    prefetch=LEGACY_PREFETCH,
    cap=CAP_AT_LEGACY_PREFETCH,
    band=18,
    burst=18,
    rpc_burst=RPC_BURST,
    readiness_reserve=20,
    autoscaler_channels_total=80,
    quiet_load=50,
)

# The SHIPPED PACKED configuration Epic 3 turns on: client_limit 8, prefetch
# 15, so the soft threshold is 800 and the computed cap is 3342. The cap is
# COMPUTED (override None) from prefetch 15, which is the only way this row
# really tests the prefetch-15 configuration rather than a hand-set number.
# The cap formula does not read ``client_limit``, so the old soft-100/
# prefetch-15 hybrid row is deleted rather than kept -- the computed-cap path
# is fully covered here. Derivations:
#   soft threshold = pool_size 100 x client_limit 8 = 800
#   cap 3342 (packed prefetch 15, see CAP_AT_PACKED_PREFETCH)
#   band = 3342 - 800 = 2542
#   channels_total = soft 800 - reserve 80 = 720 (the shipped reserve is 80)
#   quiet_load 400: available 800 - 400 = 400 >= reserve 80 -> SERVING.
#   burst 600: the PRD's measured worst-case desktop client (AC-1 / G-1),
#   well inside the 2542-wide band.
PACKED_CONFIGURATION = CapacityConfiguration(
    client_limit=8,
    soft_threshold=800,
    absolute_cap=None,
    prefetch=PACKED_PREFETCH,
    cap=CAP_AT_PACKED_PREFETCH,
    band=2_542,
    burst=WORST_CASE_CLIENT_BURST,
    rpc_burst=RPC_BURST,
    readiness_reserve=80,
    autoscaler_channels_total=720,
    quiet_load=400,
)

CONFIGURATIONS = [
    pytest.param(LEGACY_CONFIGURATION, id="legacy-soft-100-prefetch-500-cap-118"),
    pytest.param(
        PACKED_CONFIGURATION, id="shipped-packed-soft-800-prefetch-15-cap-3342"
    ),
]

# The (configuration, in-band counted load) pairs the scaling-surface tests run
# at: the bottom of each band (the soft threshold) and its top (the cap). Both
# are in the overflow band, so an in-use count ABOVE the reported capacity is
# the designed steady state, not a defect.
IN_BAND_CASES = [
    pytest.param(
        LEGACY_CONFIGURATION,
        LEGACY_CONFIGURATION.soft_threshold,
        id="legacy-at-the-soft-threshold",
    ),
    pytest.param(
        LEGACY_CONFIGURATION,
        LEGACY_CONFIGURATION.cap,
        id="legacy-at-the-memory-safe-cap",
    ),
    pytest.param(
        PACKED_CONFIGURATION,
        PACKED_CONFIGURATION.soft_threshold,
        id="packed-at-the-soft-threshold",
    ),
    pytest.param(
        PACKED_CONFIGURATION,
        PACKED_CONFIGURATION.cap,
        id="packed-at-the-memory-safe-cap",
    ),
]


@dataclass
class PodLoad:
    """
    The counted (checked-out) stream load the crafted pool stats report.

    Mutable on purpose: every surface reads ``pool_stats_by_pool()`` afresh,
    so raising ``non_durable`` between two client requests models a pod
    filling up mid-flight without re-patching anything.

    This is the load the RPC arms drive SEPARATELY from the pinned burst: it
    stands in for the other clients' ordinary load filling the pod, which is
    what lets the pinned burst below be read as absorption rather than as
    count arithmetic (see the module docstring on the synthetic stats seam).
    Post-CCM one admitted stream is one checkout, so at GUARD level (Class 4)
    this load counts one-for-one; the RPC arms cannot self-count only because
    their checkouts land on a different, mock-broker pool.

    :param non_durable: Streams currently counted on the non-durable pool.
    :param durable: Streams currently counted on the durable pool.
    """

    non_durable: int = 0
    durable: int = 0


@pytest.fixture
def pod_at_load(mocker):
    """
    Install one capacity configuration and one counted load, on ONE seam.

    Returns a callable taking a :class:`CapacityConfiguration` and a starting
    counted stream count, and returning ``(load, guard)``.

    Self-contained by design: both existing capacity files roll their own
    guard fixture (``real_capacity_guard`` in ``test_capacity_guard.py``,
    ``real_guard_under_cap`` in ``test_memory_safe_stream_cap.py``), and
    following that precedent instead of adding a shared
    ``tests/integration_tests/grpc/conftest.py`` is what lets this Subtask and
    the AC-12 one execute in parallel without touching a common file.

    One difference from those two: ``pool_stats_by_pool`` is patched on the
    ``rabbitmq_connector`` SINGLETON rather than on a private MagicMock, and
    the real ``CapacityGuard`` is bound to that same singleton -- so ONE stats
    seam feeds all three surfaces (guard, readiness, metrics). That is the
    design proven in ``tests/app/test_capacity_surface_consistency.py``:
    feeding three surfaces from three mocks would pass even if one had quietly
    started reading a different connector.

    The autouse ``before_each`` in ``tests/conftest.py`` patches
    ``capacity_guard.raise_if_at_capacity`` for the whole suite (the
    connector's pools are nulled there, so a real guard call with no pool
    would fail closed). Rather than undo that patch, its ``side_effect`` is
    set to a real guard's bound method, so the mock relays each call --
    arguments and all, including the ``re_routable`` class the servicer
    derives from ``_is_proxied`` -- into production admission logic.

    The non-durable pool is sized to ``configuration.client_limit`` so
    ``max_client_capacity`` (the soft threshold) is derived by the library's
    own formula, and the readiness reserve is patched to the configuration's
    own value rather than a module constant.
    """

    def _install(configuration, counted_streams):
        load = PodLoad(non_durable=counted_streams)

        mocker.patch.object(
            rabbitmq_connector,
            "pool_stats_by_pool",
            side_effect=lambda: _make_pool_stats_by_pool(
                non_durable_clients=load.non_durable,
                durable_clients=load.durable,
                non_durable_client_limit=configuration.client_limit,
            ),
        )

        mocker.patch.object(
            settings.settings,
            "max_non_durable_streams_per_pod",
            configuration.absolute_cap,
        )
        # Pin the shed setting AT this configuration's cap. The burst below is
        # a direct, unstamped, pinned connection, so Task 2's class shed can
        # never apply to it (``re_routable`` defaults False) -- but pinning the
        # setting per configuration keeps this file's verdicts from silently
        # depending on the shed default, which sits at the MIDPOINT of the band
        # and would otherwise move with the cap. Setting it at the cap is the
        # documented "shed tier disabled" posture.
        mocker.patch.object(
            settings.settings,
            "max_re_routable_streams_per_pod",
            configuration.cap,
        )
        # The three computed-cap inputs, pinned so the packed configuration
        # derives 3,342 from real settings rather than from the ambient
        # environment.
        mocker.patch.object(
            common_settings, "rabbitmq_channel_prefetch_count", configuration.prefetch
        )
        mocker.patch.object(
            settings.settings, "broker_stream_memory_budget_mib", BUDGET_MIB
        )
        mocker.patch.object(settings.settings, "scaler_max_replicas", MAX_REPLICAS)
        mocker.patch.object(
            settings.settings,
            "readiness_min_available_channels",
            configuration.readiness_reserve,
        )

        guard = CapacityGuard(connector=rabbitmq_connector)
        capacity_guard.raise_if_at_capacity.side_effect = guard.raise_if_at_capacity

        return load, guard

    return _install


@pytest.fixture(params=CONFIGURATIONS)
def pod_at_the_soft_threshold(request, pod_at_load):
    """
    A pod whose counted load sits exactly on the soft threshold -- the pod
    state AC-11 opens its burst against -- at each configuration.

    Parametrized over ``CONFIGURATIONS`` via ``@pytest.fixture(params=...)``,
    the idiom the AC-12 file uses at its ``band`` fixture, so the two
    acceptance files stay stylistically aligned. Returns
    ``(configuration, load, guard)`` so the RPC arms can read the row's own
    ``rpc_burst`` / ``cap``.
    """
    configuration = request.param
    load, guard = pod_at_load(configuration, configuration.soft_threshold)
    return configuration, load, guard


@pytest.fixture
def client():
    """
    A test client for the REST surface.

    Built here because the shared ``client`` fixture lives in
    ``tests/app/rest/conftest.py``, which does not reach this directory.
    """
    return TestClient(rest_server)


def non_durable_requests(send_request=True):
    """
    Build the client-side request stream for ``ConsumeNonDurableEvents``.

    ``send_request=False`` is the rejecting-direction shape (the convention
    established in ``test_memory_safe_stream_cap.py``): the endpoint checks
    capacity before it reads a request, and a client write landing after the
    server aborted fails on the CLIENT side with a status of grpc-core's own
    making, masking the RESOURCE_EXHAUSTED the server really sent. Sending
    nothing cannot make a rejection test pass for the wrong reason -- an
    admitted RPC with an empty request stream just returns, and the absent
    exception fails the test.
    """

    def requests():
        if send_request:
            request = ConsumeNonDurableEventsRequest()
            request.filter_groups.append(filter_group(event_type="e1"))
            yield request

    return requests()


def open_non_durable_stream(stub, metadata):
    """Start a non-durable consume call without draining it."""
    return stub.ConsumeNonDurableEvents(non_durable_requests(), metadata=metadata)


async def consume_non_durable_events(stub, metadata, send_request=True):
    """Drain a non-durable consume stream; return every response."""
    return [
        response
        async for response in stub.ConsumeNonDurableEvents(
            non_durable_requests(send_request), metadata=metadata
        )
    ]


def admit_streams(guard, load, count):
    """
    Ask the real guard to admit ``count`` streams, one at a time.

    Each admission raises the counted load by one, so the walk SELF-COUNTS:
    stream N+1 is decided against a pod holding N more streams than when the
    walk began. That is the shipped counting model -- one admitted stream, one
    checkout -- and it is the only place in this file where band arithmetic is
    asserted (see the module docstring on why the RPC arms cannot carry it).

    :param guard: A real ``CapacityGuard`` bound to the shared stats seam.
    :param load: The :class:`PodLoad` those stats are rebuilt from.
    :param count: How many admissions to attempt before giving up.
    :returns: How many streams were admitted before the first refusal.
    """
    admitted = 0

    for _unused_step in range(count):
        try:
            guard.raise_if_at_capacity()
        except ChannelNotAvailableError:
            break

        admitted += 1
        load.non_durable += 1

    return admitted


class TestPinnedBurstIsAbsorbed:
    """
    AC-11 clause 1: every stream in the burst is admitted.

    The counted load sits exactly ON the soft threshold with the cap above it,
    and an ``rpc_burst`` of concurrent ``ConsumeNonDurableEvents`` calls is
    opened on ONE gRPC channel -- the ``event_consumer_stub`` fixture IS a
    single pinned connection, so concurrent RPCs on that one stub are precisely
    the pinned burst AC-11 describes. Opening one channel per stream instead
    would test something else entirely.

    Runs at both configurations. Every stream is asserted on a REAL streamed
    outcome rather than on the absence of an exception: an RPC never driven to
    completion would "not raise" even if admission had rejected it.
    """

    @pytest.mark.asyncio
    async def test_every_stream_in_the_burst_streams_its_events(
        self, event_consumer_stub, metadata, pod_at_the_soft_threshold
    ):
        configuration, _load, _guard = pod_at_the_soft_threshold

        bursts = [
            consume_non_durable_events(event_consumer_stub, metadata)
            for _unused_stream in range(configuration.rpc_burst)
        ]

        results = await asyncio.gather(*bursts)

        for responses in results:
            # One acknowledgement per stream: the set its single opening
            # request named, which rides that stream's opening response.
            # These streams send no mid-stream update.
            assert_all_messages_streamed(
                responses, non_durable=True, expected_acknowledgements=1
            )

    @pytest.mark.parametrize("configuration", CONFIGURATIONS)
    @pytest.mark.asyncio
    async def test_the_same_request_at_the_cap_is_refused(
        self, event_consumer_stub, metadata, pod_at_load, configuration
    ):
        # The non-vacuity control for the row above: the identical request,
        # against the identical configuration, IS refused once the counted
        # load reaches the cap. Without it "all admitted" could just as well
        # be a guard that never bound at all.
        pod_at_load(configuration, configuration.cap)

        with pytest.raises(AioRpcError) as exc_info:
            await consume_non_durable_events(
                event_consumer_stub, metadata, send_request=False
            )

        assert exc_info.value.code() == RESOURCE_EXHAUSTED


class TestSamePodDrivenToTheMemoryCap:
    """
    AC-11 clause 4: 503s appear only when the pod reaches the memory-safe cap.

    The sequence is one pod's life: a stream is opened while the counted load
    is in the band and provably established (its opening response is read),
    the load then rises to the cap, and the NEXT stream on that SAME pinned
    connection is refused -- while the established one keeps delivering.

    Runs at both configurations. The withheld-events technique is borrowed
    from ``TestLiveStreamsSurviveTheShed``: the surviving stream's events are
    held shut until after the refusal, so what it delivers was provably
    PRODUCED after another client was turned away, rather than sitting in the
    client's receive buffer since before. Nothing here sleeps -- the gate is
    an ``asyncio.Event`` and the synchronization is the stream itself.
    """

    @pytest.fixture
    def held_events(self, mocker):
        """
        Withhold the live stream's events until the test releases them.

        Overrides the autouse ``mock_queue_iterator`` patch of the same
        method, yielding the same three messages from the same shared
        generator so the payloads cannot drift from the rest of the suite.
        """
        release = asyncio.Event()

        async def _held_consume(_unused_queue_obj, _unused_authz_check, **_kwargs):
            await release.wait()
            async for raw_message in _create_raw_mock_messages_gen():
                yield EventMessage(json.loads(raw_message.body))

        mocker.patch.object(
            RabbitMQBase, "_consume_with_delayed_acks", side_effect=_held_consume
        )

        return release

    @pytest.fixture
    def drive_to_the_cap_mid_flight(
        self, event_consumer_stub, metadata, held_events, pod_at_the_soft_threshold
    ):
        """Run that sequence; return the live call and the refused client's
        error."""

        async def _run():
            configuration, load, _guard = pod_at_the_soft_threshold

            call = open_non_durable_stream(event_consumer_stub, metadata)

            opening_response = await call.read()
            assert opening_response.reconnect_token

            # The same pod, now driven to the memory-safe cap.
            load.non_durable = configuration.cap

            with pytest.raises(AioRpcError) as exc_info:
                await consume_non_durable_events(
                    event_consumer_stub, metadata, send_request=False
                )

            # Only now, after the refusal, does the live stream's broker have
            # anything to deliver.
            held_events.set()

            return call, exc_info.value

        return _run

    @pytest.mark.asyncio
    async def test_the_next_stream_is_refused_with_resource_exhausted(
        self, drive_to_the_cap_mid_flight
    ):
        _call, error = await drive_to_the_cap_mid_flight()

        assert error.code() == RESOURCE_EXHAUSTED

    @pytest.mark.asyncio
    async def test_the_refused_stream_is_told_the_channel_pool_is_exhausted(
        self, drive_to_the_cap_mid_flight
    ):
        # The client-visible half of AC-11's "503s": on gRPC the guidance is
        # carried by the retryable status code plus this detail. The
        # backoff wording is documented client guidance with no wire
        # artifact -- see the module docstring.
        _call, error = await drive_to_the_cap_mid_flight()

        assert error.details() == CHANNEL_POOL_EXHAUSTED_DETAIL

    @pytest.mark.asyncio
    async def test_the_stream_opened_before_the_cap_keeps_delivering(
        self, drive_to_the_cap_mid_flight
    ):
        call, _error = await drive_to_the_cap_mid_flight()

        # The opening reconnect-token response was already read inside the
        # fixture, so what remains is the event payload the client came for --
        # produced after another client was turned away. Read rather than
        # iterated because grpc.aio forbids mixing the two APIs on one call.
        remaining = [await call.read() for _unused_event in range(EXPECTED_EVENT_COUNT)]

        # A stream the cap had torn down reads back the grpc.aio.EOF sentinel
        # (or raises), so requiring a real event on every read is what makes
        # this "still delivering" rather than "still open".
        assert all(
            response is not grpc.aio.EOF and len(response.events) == 1
            for response in remaining
        )


class TestScalingSurfacesDuringTheBurst:
    """
    AC-11 clauses 2 and 3: NOT_SERVING, and a truthfully over-threshold signal.

    Asserted in the SAME scenario and over the SAME stats seam as the
    admission clauses above, which is the whole point of a named acceptance
    test: a pod that absorbs a burst while reporting itself healthy, or while
    under-reporting its load to the autoscaler, would satisfy the admission
    clauses and still be the outage AC-11 exists to prevent.

    Runs at both configurations. Both stream counts below are in the overflow
    band -- its bottom (the soft threshold) and its top (the cap) -- so an
    in-use count ABOVE the reported capacity is the designed steady state, not
    a defect.
    """

    @pytest.mark.parametrize("configuration,counted_streams", IN_BAND_CASES)
    @pytest.mark.asyncio
    async def test_readiness_is_not_serving(
        self, pod_at_load, configuration, counted_streams
    ):
        # Available streams are soft - counted, which is 0 at the soft
        # threshold and negative at the cap -- both below the reserve, so
        # readiness is NOT_SERVING across the WHOLE band. Check is async and
        # returns a HealthCheckResponse, so the status is read off the awaited
        # response.
        pod_at_load(configuration, counted_streams)
        servicer = ChannelPoolHealthServicer(
            rabbitmq_connector, configuration.readiness_reserve
        )

        readiness = await servicer.Check(None, None)

        assert readiness.status == NOT_SERVING

    @pytest.mark.parametrize("configuration", CONFIGURATIONS)
    @pytest.mark.asyncio
    async def test_readiness_is_serving_on_a_quiet_pod(
        self, pod_at_load, configuration
    ):
        # The non-vacuity control the readiness rows above need, and the only
        # surface here that has one to need: the servicer fails CLOSED, so a
        # seam that was never wired -- the suite nulls the connector's real
        # pools out -- would answer NOT_SERVING for the wrong reason and the
        # rows above would pass without ever seeing a stream count. A SERVING
        # answer can only come from stats this fixture published.
        pod_at_load(configuration, configuration.quiet_load)
        servicer = ChannelPoolHealthServicer(
            rabbitmq_connector, configuration.readiness_reserve
        )

        readiness = await servicer.Check(None, None)

        assert readiness.status == SERVING

    @pytest.mark.parametrize("configuration,counted_streams", IN_BAND_CASES)
    def test_metrics_report_a_truthful_over_capacity_reading(
        self, client, pod_at_load, configuration, counted_streams
    ):
        # channels_total is the soft threshold less the reserve
        # (autoscaler_channels_total) and channels_in_use is the counted load
        # verbatim, so a load in the band against that total IS the truthful
        # over-capacity reading the autoscaler must see. utilization_percent is
        # clamped at 100. The whole body is asserted at once because the
        # over-capacity reading is the RELATIONSHIP between the fields, not any
        # one of them.
        pod_at_load(configuration, counted_streams)

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        assert response.json() == {
            "utilization_percent": SATURATED_UTILIZATION_PERCENT,
            "channels_in_use": counted_streams,
            "channels_total": configuration.autoscaler_channels_total,
        }


class TestBandArithmeticAtBothConfigurations:
    """
    The band absorbs exactly ``cap - soft`` streams, at BOTH configurations.

    Asserted at guard level, where the walk self-counts (one admitted stream,
    one checkout) -- the shipped counting model, post-CCM. The RPC arms above
    cannot carry this claim because this file's counted load is a synthetic
    stats seam separate from the arms' checkouts; see the module docstring, and
    ``tests/integration_tests/rabbitmq/test_ac1_six_hundred_stream_burst.py``
    for the counted==actual proof over real pools.

    The walk drives the guard at its DEFAULT class, which is pinned
    (``re_routable`` defaults False), so the shed tier is irrelevant to every
    verdict here regardless of where its threshold sits. The fixture pins the
    shed setting at each configuration's cap anyway, so the two configurations
    cannot differ by an unpinned midpoint.
    """

    @pytest.mark.parametrize("configuration", CONFIGURATIONS)
    def test_the_burst_is_fully_absorbed(self, pod_at_load, configuration):
        # Legacy: the whole 18-stream band. Packed: the PRD's measured
        # worst-case desktop client, 600 streams from one scene load (AC-1 /
        # G-1), absorbed starting FROM the soft threshold (800 -> 1400, inside
        # the 2542-wide band) -- the strongest packed-defaults AC-11 claim in
        # the file.
        load, guard = pod_at_load(configuration, configuration.soft_threshold)

        admitted = admit_streams(guard, load, configuration.burst)

        assert admitted == configuration.burst

    @pytest.mark.parametrize("configuration", CONFIGURATIONS)
    def test_exactly_the_band_is_admitted_before_the_first_refusal(
        self, pod_at_load, configuration
    ):
        # Walked one step PAST the band: if the guard never bound, the extra
        # step is admitted and this fails on the count rather than hanging.
        load, guard = pod_at_load(configuration, configuration.soft_threshold)

        admitted = admit_streams(guard, load, configuration.band + 1)

        assert admitted == configuration.band

    @pytest.mark.parametrize("configuration", CONFIGURATIONS)
    def test_the_stream_past_the_band_is_refused(self, pod_at_load, configuration):
        # The N+1th stream, stated directly rather than inferred from the walk
        # stopping: at the cap even a PINNED stream is refused, because the
        # memory-safe cap is the hard bound and refuses every class.
        _load, guard = pod_at_load(configuration, configuration.cap)

        with pytest.raises(ChannelNotAvailableError):
            guard.raise_if_at_capacity()
