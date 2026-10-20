# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
RPC-level proof of the memory-safe stream cap (PRD AC-7, SDD SR-3.5).

AC-7 reads: "driving stream count past the configured cap produces 503s
with retry guidance while existing streams keep receiving events and
publishers remain unblocked." Its three clauses are discharged in two
places, deliberately:

1. **503s with retry guidance** and **existing streams keep receiving
   events** -- here, at the RPC level, with the production
   ``CapacityGuard`` deciding admission against crafted pool stats and a
   real configured cap. The gRPC surface signals the 503 as
   ``RESOURCE_EXHAUSTED`` plus the exhaustion detail pinned below; the
   retry-with-backoff *wording* is REST/OpenAPI-only
   (``CHANNEL_POOL_EXHAUSTED_RESPONSE`` in ``app/rest/routes.py``), so it
   is asserted on the REST surface rather than invented here. The REST
   shed row under this same cap lives in
   ``tests/app/rest/test_routes.py`` (``TestStreamMemoryCapRestShed``),
   next to the existing 503-body and OpenAPI-description pins it leans on.
2. **Publishers remain unblocked** -- discharged arithmetically in
   ``tests/app/lib/rabbitmq/test_stream_memory_cap.py``, by the
   fleet-budget invariant ``cap x scaler_max_replicas x per-stream bytes
   <= broker_stream_memory_budget_mib``. The broker's memory high
   watermark -- the thing that blocks publishers -- is a cluster-wide
   alarm no service code path participates in; the service's only lever
   is admission. Proving publishers stay unblocked therefore means
   proving the admitted population cannot reach the watermark, which is
   arithmetic, not an RPC.

No live broker is used here, for the same reason. Provoking the watermark
needs a prod-sized broker filled to ~2.4 GB, while CI runs a plain
``rabbitmq:4.1.3`` container; the ``live_broker_connection`` template in
``tests/integration_tests/rabbitmq/conftest.py`` also skips when the
broker is unreachable, which would put a silent hole in an acceptance
test. The cap is enforced entirely service-side from
``pool_stats_by_pool()``, so a real broker adds no coverage to the
admission decision it would only make slow and flaky.

Scope note: this file is about the cap as the *hard bound* -- under
two-tier admission it is the only thing that turns away a pinned
non-durable stream, so the shed rows below are the acceptance core
rather than one bound among two. The pool's own stream capacity is a
soft packing target that refuses no one; the last class here proves that
at the RPC level, by admitting a stream into a pool that is already
full. The exhaustive tier table -- soft threshold, shed threshold, hard
bound -- and the two-surface (non-durable vs durable pool) contract are
covered in ``tests/integration_tests/grpc/test_capacity_guard.py`` and
``tests/app/lib/rabbitmq/test_capacity_guard.py``.

Every client in this file is unstamped and therefore PINNED, which is
what makes each rejection below unambiguously the cap's: the re-routable
shed tier cannot fire on a pinned stream. The RPC-level proof that a
proxy-stamped (re-routable) stream sheds where a pinned one is admitted
is AC-12's, and it is owned by Epic 7 Task 5's named acceptance test,
``tests/integration_tests/grpc/test_ac12_class_shed.py`` -- that row is
deliberately absent here rather than missing.
"""

import asyncio
import json
from dataclasses import dataclass

import grpc
import pytest
from grpc.aio import AioRpcError
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings
from app.lib.models.event_message import EventMessage
from app.lib.rabbitmq import DurableQueueConsumer, capacity_guard
from app.lib.rabbitmq.capacity_guard import CapacityGuard
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ConsumeDurableEventsRequest,
    ConsumeNonDurableEventsRequest,
    CreateDurableQueueRequest,
)
from tests.app.conftest import _make_pool_stats_by_pool
from tests.integration_tests.conftest import (
    _create_raw_mock_messages_gen,
    assert_all_messages_streamed,
    filter_group,
)

RESOURCE_EXHAUSTED = grpc.StatusCode.RESOURCE_EXHAUSTED

# The client-visible detail for a capacity rejection: it is
# ChannelNotAvailableError.user_message, relayed verbatim by the shared
# ExceptionInterceptor. Pinned as a literal so a reword of the error the
# client actually reads fails here. The cap reuses this contract unchanged
# -- a stream turned away at the cap is indistinguishable to the client
# from one turned away by any other capacity refusal, which is the point:
# they all say "try again later".
CHANNEL_POOL_EXHAUSTED_DETAIL = "Channel pool is exhausted"

QUEUE_ID = "test-queue-id"
GENERATED_QUEUE_ID = "generated-id"

# A cap far below the crafted pool's stream capacity, so every rejection
# below is unambiguously the cap's doing and not the pool's.
STREAM_CAP = 5

# The stream capacity of the pools _make_pool_stats_by_pool builds at its
# defaults (max_pool_size 100 x channel_client_limit 1). That capacity is
# the SOFT threshold -- the density the pool is provisioned to pack at --
# so filling it exactly is what puts the next stream in the overflow band.
POOL_STREAM_CAPACITY = 100

# A cap far above that pool capacity, for the row that proves a pod at its
# soft threshold absorbs the next stream instead of rejecting it.
NON_BINDING_CAP = 1_000_000

# The shared mock message generator yields three events on every path; on
# the non-durable path they follow the reconnect-token response.
EXPECTED_EVENT_COUNT = 3


@dataclass
class PodLoad:
    """
    The live stream counts the crafted pool stats are rebuilt from.

    Mutable on purpose: the guard reads ``pool_stats_by_pool()`` afresh on
    every admission decision, so raising ``non_durable`` mid-test models a
    pod filling up between two client requests without re-patching
    anything.

    :param non_durable: Streams currently held on the non-durable pool.
    :param durable: Streams currently held on the durable pool.
    """

    non_durable: int = 0
    durable: int = 0


@pytest.fixture
def real_guard_under_cap(mocker):
    """
    Run the REAL guard under a configured cap for one test.

    Returns a callable taking the cap and the starting per-pool stream
    counts, and returning the :class:`PodLoad` those counts live on.

    The autouse ``before_each`` in ``tests/conftest.py`` patches
    ``capacity_guard.raise_if_at_capacity`` for the whole suite (the
    connector's pools are nulled out there, so a real guard call with no
    pool would fail closed). Rather than undo that patch, this sets its
    ``side_effect`` to the bound method of a real ``CapacityGuard`` built
    over a mock connector -- so the mock relays each call, arguments and
    all, into production admission logic. ``durable_pool=`` therefore
    reaches the real guard, and the patched mock is still available for
    call assertions.

    The cap is driven through ``max_non_durable_streams_per_pod``, the
    absolute operator override AC-7 describes;
    ``memory_safe_stream_cap()`` returns it unconditionally and is read
    per admission decision, so patching the setting takes effect
    immediately.
    """

    def _install(stream_cap, non_durable_clients=0, durable_clients=0):
        load = PodLoad(non_durable=non_durable_clients, durable=durable_clients)

        # spec'd against the real connector so a rename of
        # pool_stats_by_pool fails loudly here instead of silently
        # handing the guard a MagicMock it reads as "no pools".
        connector = mocker.MagicMock(spec=RabbitMQConnector)
        # Not draining: the guard reads ``connector.draining`` on the
        # non-durable path (Epic 5, Task 2 / SR-6.1). It is a plain instance
        # attribute absent from the class spec, so it must be set explicitly or
        # the guard's read raises ``AttributeError``. These rows exercise the
        # cap, not the draining short-circuit.
        connector.draining = False
        connector.pool_stats_by_pool.side_effect = lambda: _make_pool_stats_by_pool(
            non_durable_clients=load.non_durable,
            durable_clients=load.durable,
        )

        mocker.patch.object(
            settings.settings, "max_non_durable_streams_per_pod", stream_cap
        )
        capacity_guard.raise_if_at_capacity.side_effect = CapacityGuard(
            connector=connector
        ).raise_if_at_capacity

        return load

    return _install


def non_durable_requests(send_request=True):
    """
    Build the client-side request stream for ``ConsumeNonDurableEvents``.

    ``send_request=False`` is the rejecting-direction shape: the endpoint
    checks capacity before it reads a request, and a client write that
    lands after the server aborted fails on the CLIENT side with a status
    of grpc-core's own making, masking the RESOURCE_EXHAUSTED the server
    really sent (see ``test_capacity_guard.py`` for the full account of
    that race). Sending nothing cannot make a rejection test pass for the
    wrong reason: an admitted RPC with an empty request stream just
    returns, and the absent exception fails the test.
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


async def consume_durable_events(stub, metadata):
    """Drain a durable consume stream; return every response."""
    return [
        response
        async for response in stub.ConsumeDurableEvents(
            ConsumeDurableEventsRequest(queue_id=QUEUE_ID), metadata=metadata
        )
    ]


async def create_durable_queue(stub, metadata, mocker):
    """Create a durable queue; return the response."""
    mocker.patch.object(
        DurableQueueConsumer, "create_queue", return_value=(GENERATED_QUEUE_ID, True)
    )
    request = CreateDurableQueueRequest()
    request.filter_groups.append(filter_group())

    return await stub.CreateDurableQueue(request, metadata=metadata)


class TestStreamsPastTheCapAreShed:
    """
    The cap sheds new non-durable streams before the pool bound binds.

    The pool in every test here has 100 stream slots and the cap is 5, so
    a rejection can only have come from the cap.
    """

    @pytest.mark.asyncio
    async def test_rejects_a_new_stream_at_the_cap(
        self, event_consumer_stub, metadata, real_guard_under_cap
    ):
        real_guard_under_cap(STREAM_CAP, non_durable_clients=STREAM_CAP)

        with pytest.raises(AioRpcError) as exc_info:
            await consume_non_durable_events(
                event_consumer_stub, metadata, send_request=False
            )

        assert exc_info.value.code() == RESOURCE_EXHAUSTED

    @pytest.mark.asyncio
    async def test_shed_stream_is_told_the_channel_pool_is_exhausted(
        self, event_consumer_stub, metadata, real_guard_under_cap
    ):
        # The client-visible half of AC-7's "503s with retry guidance": on
        # gRPC the guidance is carried by the retryable status code plus
        # this detail. The prose form of the guidance is the REST
        # surface's, pinned in tests/app/rest/test_routes.py.
        real_guard_under_cap(STREAM_CAP, non_durable_clients=STREAM_CAP)

        with pytest.raises(AioRpcError) as exc_info:
            await consume_non_durable_events(
                event_consumer_stub, metadata, send_request=False
            )

        assert exc_info.value.details() == CHANNEL_POOL_EXHAUSTED_DETAIL

    @pytest.mark.asyncio
    async def test_admits_a_new_stream_below_the_cap(
        self, event_consumer_stub, metadata, real_guard_under_cap
    ):
        # One slot short of the cap has to land on a real streamed
        # outcome, not merely on the absence of an exception: a cap that
        # rejected everything would still "not raise" if the RPC were
        # never driven to completion.
        real_guard_under_cap(STREAM_CAP, non_durable_clients=STREAM_CAP - 1)

        responses = await consume_non_durable_events(event_consumer_stub, metadata)

        # One acknowledgement: the set the single opening request named,
        # which rides the opening response. This stream sends no mid-stream
        # update.
        assert_all_messages_streamed(
            responses, non_durable=True, expected_acknowledgements=1
        )


class TestLiveStreamsSurviveTheShed:
    """
    Admission is a per-request gate, not a kill switch on live streams.

    AC-7's second clause. Each test drives the same sequence: open a
    stream while the pod is below the cap, take its first response, then
    fill the pod to the cap and let a second client be turned away. A test
    that opened its stream only after the shed would prove nothing.
    """

    @pytest.fixture
    def held_events(self, mocker):
        """
        Withhold the live stream's events until the test releases them.

        Without this, every event the first stream carries would already
        have been produced before the shed, and "it still delivers" would
        be a claim about the client's receive buffer rather than about the
        server. Holding the source shut until after the rejection means
        the events asserted below were produced by a consumer the shed had
        every opportunity to tear down. The gate is an ``asyncio.Event``
        the sequence sets, so nothing here waits on a clock.

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
    def shed_a_second_stream_mid_flight(
        self, event_consumer_stub, metadata, held_events
    ):
        """
        Run that sequence; return the live call and the second client's error.

        Synchronization is the stream itself: the opening response is read
        before the load is raised, so the stream is provably established
        at the moment the pod fills, with no sleeping involved.
        """

        async def _run(load):
            call = open_non_durable_stream(event_consumer_stub, metadata)

            opening_response = await call.read()
            assert opening_response.reconnect_token

            load.non_durable = STREAM_CAP

            with pytest.raises(AioRpcError) as exc_info:
                await consume_non_durable_events(
                    event_consumer_stub, metadata, send_request=False
                )

            # Only now, after another client has been turned away, does the
            # live stream's broker have anything to deliver.
            held_events.set()

            return call, exc_info.value

        return _run

    @pytest.mark.asyncio
    async def test_second_stream_is_shed_once_the_pod_reaches_the_cap(
        self, real_guard_under_cap, shed_a_second_stream_mid_flight
    ):
        load = real_guard_under_cap(STREAM_CAP, non_durable_clients=STREAM_CAP - 1)

        _call, error = await shed_a_second_stream_mid_flight(load)

        assert error.code() == RESOURCE_EXHAUSTED

    @pytest.mark.asyncio
    async def test_stream_opened_before_the_shed_keeps_delivering_events(
        self, real_guard_under_cap, shed_a_second_stream_mid_flight
    ):
        load = real_guard_under_cap(STREAM_CAP, non_durable_clients=STREAM_CAP - 1)
        call, _error = await shed_a_second_stream_mid_flight(load)

        # The opening reconnect-token response was already read inside the
        # fixture, so what remains is the event payload the client came
        # for -- produced after another client was turned away. Read rather
        # than iterated because grpc.aio forbids mixing the two APIs on one
        # call, and the opening response came from read().
        remaining = [await call.read() for _ in range(EXPECTED_EVENT_COUNT)]

        # A stream the shed had torn down reads back the grpc.aio.EOF
        # sentinel (or raises), so requiring a real event on every read is
        # what makes this "still delivering" rather than "still open".
        assert all(
            response is not grpc.aio.EOF and len(response.events) == 1
            for response in remaining
        )


class TestCapDoesNotShedDurableRpcs:
    """
    The cap bounds non-durable fan-out only.

    The RPC-level twin of the guard unit file's scope test: durable
    clients are gated on the durable pool's own budget, and the handful of
    long-lived durable queues are not the fan-out memory risk the cap
    exists to contain. Both tests assert the RPC's real outcome, so a
    durable call that failed for any reason -- capacity or otherwise --
    fails here rather than passing on the absence of a rejection.
    """

    @pytest.fixture(autouse=True)
    def _pod_at_the_cap(self, real_guard_under_cap):
        return real_guard_under_cap(
            STREAM_CAP, non_durable_clients=STREAM_CAP, durable_clients=0
        )

    @pytest.mark.asyncio
    async def test_consume_durable_events_streams_every_event(
        self, event_consumer_stub, metadata
    ):
        responses = await consume_durable_events(event_consumer_stub, metadata)

        assert_all_messages_streamed(responses)

    @pytest.mark.asyncio
    async def test_create_durable_queue_returns_the_new_queue_id(
        self, event_consumer_stub, metadata, mocker
    ):
        response = await create_durable_queue(event_consumer_stub, metadata, mocker)

        assert response.queue_id == GENERATED_QUEUE_ID


class TestFullPoolUnderANonBindingCapAbsorbsTheBurst:
    """
    A full pool is not a refusal -- the burst overflows into the band.

    The RPC-level proof of the two-tier contract this Epic restores. With
    the cap far above anything the pool can hold, a pod sitting exactly
    at its packing density still admits the next pinned stream: the pool
    bound is a soft scaling threshold, and the stream lands in the pool's
    overflow band (SDD SR-1.5) rather than being turned away. Refusing
    there would be a hard client failure nothing could rescue -- a
    consumer stream is pinned to the pod it landed on for its whole life,
    so scale-out arrives too late to help it (PRD FR-11).

    The pool bound does still refuse in exactly one case: the
    incomputable-cap fallback, where a prefetch of 0 or less leaves the
    per-stream footprint undefined and the pool bound stands in as the
    hard bound. That case is unit-tested in
    ``tests/app/lib/rabbitmq/test_capacity_guard.py``, where the None cap
    is produced through the real settings; it is not reproduced here,
    because reaching it is a settings condition rather than anything an
    RPC can express.

    What this asserts is the admission DECISION. The crafted stats are
    the guard's input only -- the suite nulls the connector's real pools
    out -- so the overflow-band checkout the admitted stream would
    perform in production is the ``ChannelPool``'s own coverage in
    ``notification_common``, not this file's.
    """

    @pytest.mark.asyncio
    async def test_admits_and_streams_when_the_pool_is_full_and_the_cap_is_not(
        self, event_consumer_stub, metadata, real_guard_under_cap
    ):
        # Like the below-the-cap row above, this has to land on a real
        # streamed outcome rather than on the absence of an exception: an
        # RPC that was never driven to completion would "not raise" even
        # if admission had rejected it.
        real_guard_under_cap(NON_BINDING_CAP, non_durable_clients=POOL_STREAM_CAPACITY)

        responses = await consume_non_durable_events(event_consumer_stub, metadata)

        # One acknowledgement: the set the single opening request named,
        # which rides the opening response. This stream sends no mid-stream
        # update.
        assert_all_messages_streamed(
            responses, non_durable=True, expected_acknowledgements=1
        )
