# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
RPC-level proof of PRD AC-12: the shed tier is class-aware.

AC-12 verbatim: "A pod at its shed threshold refuses a new proxied
(``X-Via-Proxy``-stamped) stream with 503/RESOURCE_EXHAUSTED while, in the
same state, admitting a new stream on a pinned direct connection; and the
refused proxied stream succeeds against a less-loaded pod (via proxy retry
policy or client retry)."

The load-bearing property here is END TO END: the admission class is
derived from a real request's wire metadata, through
``EventConsumerServicer._is_proxied``, into the guard's verdict. Nothing
in this file patches ``_is_proxied`` or passes ``re_routable`` by hand --
doing either would test around the plumbing this file exists to prove.
The only things stubbed are the pod's stream counts and the two settings
that place the band.

What this file pins
-------------------
- The differential (``TestTheShedThresholdIsClassAware``): one established
  state, one shared fixture, two outcomes -- the stamped stream is refused
  and the unstamped one streams every event. Both rows share that fixture
  precisely so no reader can suspect they ran against different loads.
- REST/SSE is re-routable by CLASS, not by header (SR-3.7): the same state
  503s a REST stream that carries no stamp at all.
- The shed predicate is INCLUSIVE -- refused at the threshold, admitted one
  stream below it. Task 2's contract fixes the threshold's VALUE but never
  says whether it is ``>=`` or ``>``; ``TestTheRefusedStreamSucceedsOnALessLoadedPod``
  is the RPC-level pin of that predicate.

What this file does NOT prove
-----------------------------
That Envoy re-dispatches the refused stream. The "less-loaded pod" clause
is modelled sequentially -- the refusal, then the IDENTICAL stamped request
against a pod whose load has fallen one stream below the shed threshold --
so what is proven is that the refusal is a per-request capacity verdict
that a fresh load-balancer selection can clear, not that Envoy performs
that selection. It is worth being precise about the gap: Contour sets no
retry-host predicate, so on retry Envoy may re-select the very host that
refused (under weighted least request it may even prefer it, the refusal
having freed that host's outstanding-request count). The re-dispatch is a
proxy behaviour, not a service behaviour.

A second gRPC server is deliberately NOT stood up to fake two pods: both
servers would share the one process-global patched
``capacity_guard.raise_if_at_capacity``, so the "two pods" would be
indistinguishable and the test would be theatre.

Two facts complete AC-12's claim and live elsewhere, on purpose:

- The wire-shape precondition -- the refusal must arrive as a trailers-only
  response, which is all Envoy inspects to decide retryability -- is pinned
  by this Task's trailers-only test,
  ``tests/integration_tests/grpc/test_capacity_refusal_is_trailers_only.py``.
- The chart-side ``retryPolicy`` render (``retryOn`` including
  resource-exhausted) is pinned by Task 4 in
  ``tests/helm/test_http_proxy_streaming_template.py``.

Deliberately not here (Task 2's, at unit level)
-----------------------------------------------
- The shed INFO log line ("shedding re-routable stream") --
  ``tests/app/lib/rabbitmq/test_capacity_guard.py``.
- The threshold RESOLVER: the unset-means-midpoint default, the clamp into
  ``[soft, hard]``, and the degenerate / inverted-band cases -- same file.
  This file pins its band explicitly rather than relying on the midpoint,
  so a change to the default cannot silently move what these rows mean.
- The incomputable-cap fallback. There the pool bound IS the soft
  threshold, the band is degenerate, and the shed tier collapses onto the
  hard bound -- structurally unreachable. A fallback-path shed row cannot
  pass and is absent rather than missing.
"""

from dataclasses import dataclass

import grpc
import pytest
import pytest_asyncio
from fastapi.testclient import TestClient
from grpc.aio import AioRpcError
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings
from app.lib.rabbitmq import capacity_guard
from app.lib.rabbitmq.capacity_guard import CapacityGuard
from app.rest.rest_server import rest_server
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ConsumeNonDurableEventsRequest,
)
from tests.app.conftest import _make_pool_stats_by_pool
from tests.integration_tests.conftest import (
    assert_all_messages_streamed,
    filter_group,
)

RESOURCE_EXHAUSTED = grpc.StatusCode.RESOURCE_EXHAUSTED

# The client-visible detail for a capacity rejection: it is
# ChannelNotAvailableError.user_message, relayed verbatim by the shared
# ExceptionInterceptor. A stream shed by the CLASS tier is indistinguishable
# to the client from one turned away by any other capacity refusal -- which
# is the point, since both mean "try again".
CHANNEL_POOL_EXHAUSTED_DETAIL = "Channel pool is exhausted"

# Spelled as the wire literal rather than imported from
# EventConsumerServicer._PROXY_HEADER: the header name IS the contract
# between the chart's HTTPProxy and this service, so a rename of the
# constant that does not change the wire must fail here, loudly.
PROXY_STAMP = ("x-via-proxy", "true")

NON_DURABLE_STREAM_PATH = "/api/v1beta/events/stream"
AUTH_HEADER = {"Authorization": "Bearer test-token"}

# ``max_pool_size`` of the pool ``_make_pool_stats_by_pool`` builds. The soft
# threshold -- the pool's own stream capacity, and the packing density it is
# provisioned for -- is this times the per-channel client limit.
POOL_SIZE = 100


@dataclass(frozen=True)
class AdmissionBand:
    """
    One placement of the three-tier band, as a parametrizable configuration.

    Every number this file depends on lives here rather than inline, so
    Epic 3's packed-defaults re-run ADDS a configuration below instead of
    editing arithmetic scattered through the rows.

    :param client_limit: The non-durable pool's per-channel client limit;
        with ``POOL_SIZE`` it fixes the soft threshold.
    :param hard_cap: The memory-safe stream cap, driven through the
        ``max_non_durable_streams_per_pod`` operator override so it is a
        number this file controls rather than one the shipped prefetch
        computes.
    :param shed_threshold: The stream count at which re-routable streams
        are shed, driven through ``max_re_routable_streams_per_pod``. Set
        explicitly and strictly inside the band, so no row here rides on
        the unset-means-midpoint default (covered by test_capacity_guard.py's
        ``TestReRoutableShedThreshold``).
    """

    client_limit: int
    hard_cap: int
    shed_threshold: int

    @property
    def soft_threshold(self) -> int:
        return POOL_SIZE * self.client_limit


# The FACTORY geometry: a pool of 100 channels at one client each, so the soft
# threshold is 100 -- the same number the AC-11 file pins as SOFT_THRESHOLD,
# from the same _make_pool_stats_by_pool defaults. This is the factory baseline
# the crafted rows drive absolute overrides against, NOT the shipped packing
# density (client_limit 8, soft 800 as of Epic 3).
FACTORY_CLIENT_LIMIT = 1
FACTORY_SOFT_THRESHOLD = POOL_SIZE * FACTORY_CLIENT_LIMIT

# The hard bound at the LEGACY prefetch (500). OWNED by
# tests/app/lib/rabbitmq/test_stream_memory_cap.py, which pins the derivation;
# restated here as a literal with its one-line derivation and deliberately NOT
# recomputed from production code:
#   budget 1024 MiB / (500 x 1800 + 5120) B = 1,186 fleet / 10 replicas = 118
# It goes in as the absolute operator override so no row here can be perturbed
# by an ambient prefetch -- the shipped prefetch is now 15, so this legacy 118
# is driven through the override precisely so no row rides on the shipped cap.
FACTORY_HARD_CAP = 118

# The shed line, placed strictly inside the 18-stream band. This is a CHOICE,
# not a derivation: it is set explicitly through the operator override so the
# rows below say what they mean at a glance, and so a change to the
# unset-means-midpoint default (which would also land on 109 here) cannot
# silently move them. That default -- including the width-1 band case -- is
# covered by test_capacity_guard.py's TestReRoutableShedThreshold
# (test_a_width_1_band_sheds_at_the_soft_threshold_when_unset).
FACTORY_SHED_THRESHOLD = 109

# The SHIPPED PACKED geometry (Epic 3, t1.tj2.xn): client_limit 8, prefetch 15.
# Each value is a LITERAL with its one-line derivation, not recomputed from
# production code -- the same convention the FACTORY_* block above follows.
PACKED_CLIENT_LIMIT = 8

# The one product this file already derives, matching the AdmissionBand
# soft_threshold property: pool_size 100 x client_limit 8 = 800.
PACKED_SOFT_THRESHOLD = POOL_SIZE * PACKED_CLIENT_LIMIT

# The memory-safe stream cap at the shipped prefetch 15. As with
# FACTORY_HARD_CAP this is driven in through the absolute
# max_non_durable_streams_per_pod override, so this row does NOT exercise the
# computed-cap path; that path at prefetch 15 is covered by the AC-11 packed
# row (test_ac11_pinned_burst_absorption.py) and by
# tests/app/lib/rabbitmq/test_stream_memory_cap.py. Derivation:
#   budget 1024 MiB / (15 x 1800 + 5120) B = 33,429 fleet / 10 replicas = 3,342
PACKED_HARD_CAP = 3_342

# The shed line for the packed row, set EXPLICITLY through the
# max_re_routable_streams_per_pod override as this file's convention requires
# (see FACTORY_SHED_THRESHOLD). It happens to coincide with the
# unset-means-midpoint default at this band, and that coincidence is
# DELIBERATE: it makes the row model the shipped posture. The row still does
# not RIDE on the default -- it pins the value -- so a change to the resolver's
# midpoint arithmetic cannot silently move it; the resolver itself is
# unit-tested in tests/app/lib/rabbitmq/test_capacity_guard.py. Derivation:
#   soft 800 + (cap 3342 - soft 800) // 2 = 800 + 1271 = 2071
PACKED_SHED_THRESHOLD = 2_071

BAND_CONFIGURATIONS = [
    pytest.param(
        AdmissionBand(
            client_limit=FACTORY_CLIENT_LIMIT,
            hard_cap=FACTORY_HARD_CAP,
            shed_threshold=FACTORY_SHED_THRESHOLD,
        ),
        id=(
            f"soft-{FACTORY_SOFT_THRESHOLD}"
            f"-shed-{FACTORY_SHED_THRESHOLD}"
            f"-cap-{FACTORY_HARD_CAP}"
        ),
    ),
    pytest.param(
        AdmissionBand(
            client_limit=PACKED_CLIENT_LIMIT,
            hard_cap=PACKED_HARD_CAP,
            shed_threshold=PACKED_SHED_THRESHOLD,
        ),
        id=(
            f"soft-{PACKED_SOFT_THRESHOLD}"
            f"-shed-{PACKED_SHED_THRESHOLD}"
            f"-cap-{PACKED_HARD_CAP}"
        ),
    ),
]


@pytest.fixture(params=BAND_CONFIGURATIONS)
def band(request):
    """The band configuration under test; every row runs against each."""
    return request.param


@dataclass
class PodLoad:
    """
    The live stream count the crafted pool stats are rebuilt from.

    Mutable on purpose: the guard reads ``pool_stats_by_pool()`` afresh on
    every admission decision, so lowering ``non_durable`` between two
    requests models a pod draining -- or, for AC-12's last clause, models
    the same request meeting a less-loaded pod -- without re-patching
    anything.
    """

    non_durable: int = 0


@pytest.fixture
def pod_holding(band, mocker):
    """
    Run the REAL guard over the singleton, at a chosen load, for one test.

    Returns a callable taking the pod's non-durable stream count and
    returning the mutable :class:`PodLoad` it lives on.

    Rolled locally rather than shared with the AC-11 file: the two are
    separate acceptance files precisely so they can execute in parallel,
    and a common conftest would couple them.

    The autouse ``before_each`` in ``tests/conftest.py`` patches
    ``capacity_guard.raise_if_at_capacity`` for the whole suite (the
    connector's pools are nulled out there, so a real guard call with no
    pool would fail closed). Rather than undo that patch, this sets its
    ``side_effect`` to the bound method of a real ``CapacityGuard``, so
    the mock relays every call -- arguments and all -- into production
    admission logic. That relay is what makes ``re_routable=`` reach the
    real guard from the servicer, and it covers the REST surface too:
    ``app/rest/routes.py`` gates on this same singleton.
    """

    # A band whose shed line is not strictly inside it would make every row
    # here mean something other than what its name says, so an incoherent
    # configuration fails at setup rather than by a confusing assertion.
    assert band.soft_threshold < band.shed_threshold < band.hard_cap

    def _hold(streams):
        load = PodLoad(non_durable=streams)

        # spec'd against the real connector so a rename of
        # pool_stats_by_pool fails loudly here instead of silently handing
        # the guard a MagicMock it reads as "no pools".
        connector = mocker.MagicMock(spec=RabbitMQConnector)
        # Not draining: the guard reads ``connector.draining`` on the
        # non-durable path (Epic 5, Task 2 / SR-6.1). It is a plain instance
        # attribute absent from the class spec, so it must be set explicitly or
        # the guard's read raises ``AttributeError``. This scenario drives the
        # class-aware shed threshold, not the draining short-circuit.
        connector.draining = False
        connector.pool_stats_by_pool.side_effect = lambda: _make_pool_stats_by_pool(
            non_durable_clients=load.non_durable,
            non_durable_client_limit=band.client_limit,
        )

        mocker.patch.object(
            settings.settings, "max_non_durable_streams_per_pod", band.hard_cap
        )
        mocker.patch.object(
            settings.settings,
            "max_re_routable_streams_per_pod",
            band.shed_threshold,
        )
        capacity_guard.raise_if_at_capacity.side_effect = CapacityGuard(
            connector=connector
        ).raise_if_at_capacity

        return load

    return _hold


@pytest.fixture
def rest_client():
    """
    A test client for the REST surface.

    Built here because the shared ``client`` fixtures live in
    ``tests/app/rest/conftest.py`` and ``tests/integration_tests/rest/``,
    neither of which reaches this directory.
    """
    return TestClient(rest_server)


@pytest.fixture
def proxy_stamped_metadata(metadata):
    """The gRPC call metadata a request forwarded by the HTTPProxy carries."""
    return [*metadata, PROXY_STAMP]


def _non_durable_requests(send_request=True):
    """
    Build the client-side request stream for ``ConsumeNonDurableEvents``.

    ``send_request=False`` is the rejecting-direction shape, for the
    reason recorded in ``test_capacity_guard.py``: the endpoint checks
    capacity before it reads a request, and a client write that lands
    after the server aborted fails on the CLIENT side with a status of
    grpc-core's own making, masking the RESOURCE_EXHAUSTED the server
    really sent. Sending nothing cannot make a rejection row pass for the
    wrong reason -- an admitted RPC with an empty request stream just
    returns, and the absent exception fails the test.
    """

    def requests():
        if send_request:
            request = ConsumeNonDurableEventsRequest()
            request.filter_groups.append(filter_group(event_type="e1"))
            yield request

    return requests()


async def consume_non_durable_events(stub, call_metadata, send_request=True):
    """Drain a non-durable consume stream; return every response."""
    return [
        response
        async for response in stub.ConsumeNonDurableEvents(
            _non_durable_requests(send_request), metadata=call_metadata
        )
    ]


class TestTheShedThresholdIsClassAware:
    """
    AC-12's differential: one state, two classes, two outcomes.

    The pod sits at exactly the shed threshold -- inside the overflow band,
    well below the hard cap -- so nothing here can be the cap's doing. A
    stamped stream is re-routable and must be shed; an unstamped one is
    pinned to this pod, which no amount of scale-out can rescue, and must
    be admitted all the way to the cap (PRD FR-11 / FR-12, SDD SR-3.7).

    Every row runs off the single autouse fixture below, so the refusal and
    the admission are provably answers to the same question.
    """

    @pytest.fixture(autouse=True)
    def pod_at_the_shed_threshold(self, band, pod_holding):
        return pod_holding(band.shed_threshold)

    @pytest.mark.asyncio
    async def test_stamped_stream_is_refused(
        self, event_consumer_stub, proxy_stamped_metadata
    ):
        with pytest.raises(AioRpcError) as exc_info:
            await consume_non_durable_events(
                event_consumer_stub, proxy_stamped_metadata, send_request=False
            )

        assert exc_info.value.code() == RESOURCE_EXHAUSTED

    @pytest.mark.asyncio
    async def test_stamped_refusal_reports_channel_pool_exhaustion(
        self, event_consumer_stub, proxy_stamped_metadata
    ):
        # AC-12 names the client-visible verdict, not just the fact of a
        # rejection: on gRPC that is the retryable status code plus this
        # detail. The prose retry guidance is the REST surface's and is
        # pinned in tests/app/rest/test_routes.py.
        with pytest.raises(AioRpcError) as exc_info:
            await consume_non_durable_events(
                event_consumer_stub, proxy_stamped_metadata, send_request=False
            )

        assert exc_info.value.details() == CHANNEL_POOL_EXHAUSTED_DETAIL

    @pytest.mark.asyncio
    async def test_unstamped_stream_is_admitted_and_streams_every_event(
        self, event_consumer_stub, metadata
    ):
        # The half of the differential that carries the weight, and it has
        # to land on a real streamed outcome rather than on the absence of
        # an exception: an RPC that was never driven to completion would
        # "not raise" even if admission had rejected it.
        responses = await consume_non_durable_events(event_consumer_stub, metadata)

        # One acknowledgement: the set the single opening request named,
        # which rides the opening response. This stream sends no mid-stream
        # update.
        assert_all_messages_streamed(
            responses, non_durable=True, expected_acknowledgements=1
        )

    def test_rest_stream_is_refused_without_carrying_the_stamp(self, rest_client):
        # REST/SSE is re-routable by CLASS: _check_non_durable_capacity
        # passes re_routable=True as a literal and deliberately never reads
        # the stamp, because the chart stamps only the gRPC
        # ConsumeNonDurableEvents route. This request sends no stamp and is
        # shed anyway, in the very state that just admitted an unstamped
        # gRPC stream -- the two rows together are what makes the REST
        # classification a decision rather than an accident.
        #
        # Task 2 already pins REST classification at unit level
        # (tests/app/rest/test_routes.py::
        # TestRestStreamsAreUnconditionallyReRoutable, which also proves
        # sending the stamp changes nothing). The overlap is deliberate:
        # this is the NAMED ACCEPTANCE row for AC-12's REST surface, run
        # against the same established state as the gRPC rows.
        response = rest_client.get(NON_DURABLE_STREAM_PATH, headers=AUTH_HEADER)

        assert response.status_code == 503


class TestRestAdmissionBelowTheShedThreshold:
    """
    Anti-vacuity for the REST row above: one stream lower, REST is admitted.

    Without this, a regression that made the REST stream route 503 for any
    reason at all -- a broken dependency, a missing fixture in this
    directory -- would leave the acceptance row above green while proving
    nothing about the shed tier. Asserted on a real 200 rather than on
    "not 503".
    """

    def test_rest_stream_one_stream_below_the_threshold_is_admitted(
        self, band, pod_holding, rest_client
    ):
        pod_holding(band.shed_threshold - 1)

        response = rest_client.get(NON_DURABLE_STREAM_PATH, headers=AUTH_HEADER)

        assert response.status_code == 200


class TestTheRefusedStreamSucceedsOnALessLoadedPod:
    """
    AC-12's last clause, and the pin of the shed PREDICATE.

    The sequence is: refuse the stamped stream at the shed threshold, drop
    the pod's load by exactly one stream, and re-issue the IDENTICAL
    request -- same metadata, same filters. It streams. So the refusal was
    a per-request verdict about this pod's load at that instant, which a
    fresh load-balancer selection onto a less-loaded pod clears; see the
    module docstring for why that is not the same as proving Envoy makes
    that selection.

    Asserting both sides one stream apart is also the only place the
    predicate itself is pinned at the RPC level: Task 2's contract fixes
    the threshold's value but never says whether the comparison is ``>=``
    or ``>``. Refused AT the threshold and admitted one below it says it is
    ``>=``.
    """

    @pytest_asyncio.fixture
    async def refusal_then_readmission(
        self, event_consumer_stub, proxy_stamped_metadata, band, pod_holding
    ):
        """
        Run that sequence; return the refusal's error and the retry's
        responses.

        Both halves come out of one fixture so the two rows below cannot
        drift apart on which request, which metadata, or which load each
        was talking about.
        """
        load = pod_holding(band.shed_threshold)

        with pytest.raises(AioRpcError) as exc_info:
            await consume_non_durable_events(
                event_consumer_stub, proxy_stamped_metadata, send_request=False
            )

        # The less-loaded pod, modelled as the same pod one stream below
        # the shed line. The guard re-reads the stats on the next
        # admission decision, so nothing is re-patched here.
        load.non_durable = band.shed_threshold - 1

        responses = await consume_non_durable_events(
            event_consumer_stub, proxy_stamped_metadata
        )

        return exc_info.value, responses

    @pytest.mark.asyncio
    async def test_the_stamped_stream_is_refused_at_the_shed_threshold(
        self, refusal_then_readmission
    ):
        error, _responses = refusal_then_readmission

        assert error.code() == RESOURCE_EXHAUSTED

    @pytest.mark.asyncio
    async def test_the_identical_request_streams_every_event_one_stream_below(
        self, refusal_then_readmission
    ):
        _error, responses = refusal_then_readmission

        # One acknowledgement: the set the single opening request named,
        # which rides the opening response. This stream sends no mid-stream
        # update.
        assert_all_messages_streamed(
            responses, non_durable=True, expected_acknowledgements=1
        )
