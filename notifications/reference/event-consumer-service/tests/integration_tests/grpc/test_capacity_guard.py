# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
RPC-level proof of the two-surface capacity contract.

Admission is no longer one pool for every endpoint. Each gRPC entry point
gates on the channel pool it actually consumes: ``ConsumeNonDurableEvents``
on the non-durable pool, and the durable / DLQ endpoints
(``ConsumeDurableEvents``, ``ConsumeDurableEventsWithAcks``,
``CreateDurableQueue``, ``DeleteDurableQueue``, ``UpdateDurableQueue``,
``ManageDeadLetteredEvents``) on the durable pool. So there are two
saturation directions, and in each the load-bearing assertion is what is
NOT rejected: a saturated non-durable pool must keep admitting durable
consumers (the 2026-08-11 incident, PRD G-6 / FR-3), and an exhausted
durable pool must keep admitting non-durable consumers.

Both directions are exercised twice over:

- Mock-level (``capacity_guard_rejections``): the patched guard rejects one
  pool selection and admits the other, proving each entry point asks the
  guard about the right pool. This is the wiring, and it covers the whole
  seven-RPC table.
- Real-guard (``real_capacity_guard``): the production
  ``CapacityGuard.raise_if_at_capacity`` runs against patched
  ``pool_stats_by_pool`` output, proving the entry point -> guard -> pool
  stats chain end to end rather than at the mock boundary.

The rejected side keeps today's contract: ``ChannelNotAvailableError``
surfaces as ``RESOURCE_EXHAUSTED``. The client-visible detail is pinned
below in ``CHANNEL_POOL_EXHAUSTED_DETAIL`` -- gRPC signals retryability
through the code plus that detail; the "retry with exponential backoff"
wording is REST/OpenAPI-only (``CHANNEL_POOL_EXHAUSTED_RESPONSE`` in
``app/rest/routes.py``) and is deliberately not asserted here, because the
gRPC surface does not emit it.
"""

import math
from functools import partial

import grpc
import pytest
import pytest_asyncio
from google.protobuf import json_format
from grpc.aio import AioRpcError

from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings
from app.lib.permissions import permissions
from app.lib.rabbitmq import (
    DurableQueueConsumer,
    capacity_guard,
    rabbitmq_connector,
)
from app.lib.rabbitmq.capacity_guard import CapacityGuard
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ConsumeDurableEventsRequest,
    ConsumeDurableEventsWithAcksRequest,
    ConsumeNonDurableEventsRequest,
    CreateDurableQueueRequest,
    DeleteDurableQueueRequest,
    UpdateDurableQueueRequest,
    UpdateDurableQueueResponse,
)
from tests.integration_tests.conftest import filter_group

RESOURCE_EXHAUSTED = grpc.StatusCode.RESOURCE_EXHAUSTED

# The client-visible detail for a capacity rejection. This is
# ChannelNotAvailableError.user_message, relayed verbatim by
# ExceptionInterceptor via ErrorDetails -- not a message this file invents.
CHANNEL_POOL_EXHAUSTED_DETAIL = "Channel pool is exhausted"

QUEUE_ID = "test-queue-id"
GENERATED_QUEUE_ID = "generated-id"

# The autouse mock_queue_iterator fixture streams three events on every
# durable path, and the non-durable path prefixes them with its
# reconnect-token response.
EXPECTED_DURABLE_EVENT_COUNT = 3
EXPECTED_NON_DURABLE_RESPONSE_COUNT = 4

# Pool geometry for the real-guard tests: 10 channels x 5 clients per
# channel = 50 client (stream) slots, so total_client_count == 50 is a
# fully saturated pool and 0 is an idle one.
POOL_SIZE = 10
CLIENT_LIMIT = 5
POOL_CAPACITY = POOL_SIZE * CLIENT_LIMIT
SATURATED = POOL_CAPACITY
IDLE = 0

# A non-durable stream count well inside the pool's 50 slots, paired with a
# cap set to that same number: the pool bound still has 30 free slots, so a
# refusal at CAPPED can only be the memory cap's doing. Keeping the two equal
# means the scenario sits exactly AT the cap, which is where the hard bound
# refuses (``streams >= hard_bound``).
CAPPED = 20
BINDING_CAP = CAPPED

# A cap far above anything either pool can hold, so it constrains nothing.
# Every test that is not about the cap passes this explicitly rather than
# letting the guard compute a cap from the shipped prefetch default -- that
# computed value (118 streams/pod today) is a number this file does not
# control, and a prefetch tuning would silently change what these tests mean.
NON_BINDING_CAP = 1_000_000


def _pool_stats(client_count):
    """
    Build one pool's ``ChannelPool.pool_stats()`` dict.

    Carries the full key set so it is a faithful stand-in for the real
    call rather than the subset CapacityGuard happens to read today.
    ``tests/app/conftest.py`` has a richer builder for the unit tests, but
    conftest helpers do not reach ``tests/integration_tests/``.
    """
    # Clients pack CLIENT_LIMIT to a channel, so a channel is "in use"
    # once it holds at least one -- the same relationship the real pool
    # reports between its client and channel counts.
    in_use_channels = math.ceil(client_count / CLIENT_LIMIT)

    # ``overflow_enabled`` is False for both pools here, which is what the
    # geometry above already models: clients pack exactly CLIENT_LIMIT to a
    # channel and POOL_CAPACITY is a hard ceiling. Reporting True would
    # describe a pool whose client count may legitimately exceed
    # max_client_capacity, which none of these fixtures produce. No assertion
    # in this file reads the key; it is here so the docstring's "full key set"
    # claim stays true.
    return {
        "min_pool_size": 1,
        "max_pool_size": POOL_SIZE,
        "channel_client_limit": CLIENT_LIMIT,
        "total_channel_count": POOL_SIZE,
        "available_channel_count": POOL_SIZE - in_use_channels,
        "in_use_channel_count": in_use_channels,
        "total_client_count": client_count,
        "max_client_capacity": POOL_CAPACITY,
        "overflow_enabled": False,
        "initialized": True,
        "closed": False,
    }


def _pool_stats_by_pool(non_durable_clients, durable_clients):
    """Build a ``RabbitMQConnector.pool_stats_by_pool()`` return value."""
    return {
        RabbitMQConnector.NON_DURABLE_POOL_NAME: _pool_stats(non_durable_clients),
        RabbitMQConnector.DURABLE_POOL_NAME: _pool_stats(durable_clients),
    }


@pytest_asyncio.fixture
def allow_dlq_permission(mocker):
    """
    Grant the ``inspect-and-replay-all-dlq`` gate so the DLQ RPC can open.

    Mirrors the fixture of the same name in ``test_manage_dlq.py``: the
    autouse ``before_each`` mock only stubs ``permissions._can_or_raise``
    as a plain (non-awaitable) MagicMock, which the real gate coroutine
    cannot be awaited through.
    """
    return mocker.patch.object(
        permissions,
        "raise_if_cant_inspect_and_replay_all_dlq",
        new=mocker.AsyncMock(return_value=True),
    )


@pytest.fixture
def real_capacity_guard(mocker):
    """
    Run the REAL guard against patched pool stats for one test.

    Returns a callable taking per-pool client (stream) counts plus the
    non-durable stream cap to run under. It replaces the autouse
    ``before_each`` patch of ``raise_if_at_capacity`` with the production
    implementation bound to the guard singleton, and patches
    ``pool_stats_by_pool`` on the connector singleton that same guard
    holds -- so the RPC's admission decision is made by real guard
    arithmetic over a real-shaped stats dict, not by a mock's side effect.

    The cap is driven through ``max_non_durable_streams_per_pod``, the
    absolute operator override ``memory_safe_stream_cap()`` returns
    unconditionally; it is read per admission decision, so patching the
    setting takes effect immediately. This is the same mechanism
    ``test_memory_safe_stream_cap.py::real_guard_under_cap`` uses. It is
    stated by every caller because this directory has no autouse cap
    neutralizer: without the patch the guard computes a cap from the
    shipped prefetch, and these tests would silently depend on it.
    """

    def _install(non_durable_clients, durable_clients, stream_cap=NON_BINDING_CAP):
        mocker.patch.object(
            rabbitmq_connector,
            "pool_stats_by_pool",
            return_value=_pool_stats_by_pool(non_durable_clients, durable_clients),
        )
        mocker.patch.object(
            settings.settings, "max_non_durable_streams_per_pod", stream_cap
        )
        mocker.patch.object(
            capacity_guard,
            "raise_if_at_capacity",
            new=partial(CapacityGuard.raise_if_at_capacity, capacity_guard),
        )

    return _install


# Each RPC below is driven by one invoker used in BOTH directions: on the
# admitted side it asserts the RPC's normal outcome, and on the rejected
# side the caller wraps it in pytest.raises. ``send_requests=False`` is the
# rejecting-direction shape for the client-streaming RPCs: every guarded
# endpoint checks capacity BEFORE it reads a request, so no request is
# needed to be rejected -- and sending one races the abort. A client write
# that lands after the server aborted fails on the CLIENT side, with a
# status of grpc.aio's own making (``InvalidStateError("RPC already
# finished")``, or ``AioRpcError(INTERNAL, "Internal error from Core")``
# mid-abort), masking the RESOURCE_EXHAUSTED the server really sent. That
# race is timing-dependent and shows up under worker oversubscription.
# Sending nothing cannot make a rejection test pass for the wrong reason:
# an admitted RPC with an empty request stream just returns, and the
# absent exception fails the test.


async def consume_durable_events(stub, metadata, mocker, dlq, send_requests=True):
    """Consume a durable stream to completion; assert every event arrived."""
    responses = [
        response
        async for response in stub.ConsumeDurableEvents(
            ConsumeDurableEventsRequest(queue_id=QUEUE_ID), metadata=metadata
        )
    ]

    assert len(responses) == EXPECTED_DURABLE_EVENT_COUNT


async def consume_durable_events_with_acks(
    stub, metadata, mocker, dlq, send_requests=True
):
    """Consume a durable with-acks stream; assert every event arrived."""

    def requests():
        if send_requests:
            yield ConsumeDurableEventsWithAcksRequest(queue_id=QUEUE_ID)

    responses = [
        response
        async for response in stub.ConsumeDurableEventsWithAcks(
            requests(), metadata=metadata
        )
    ]

    if not send_requests:
        return

    assert len(responses) == EXPECTED_DURABLE_EVENT_COUNT


async def create_durable_queue(stub, metadata, mocker, dlq, send_requests=True):
    """Create a durable queue; assert the created queue id came back."""
    mocker.patch.object(
        DurableQueueConsumer,
        "create_queue",
        return_value=(GENERATED_QUEUE_ID, True),
    )
    request = CreateDurableQueueRequest()
    request.filter_groups.append(filter_group())

    response = await stub.CreateDurableQueue(request, metadata=metadata)

    assert response.queue_id == GENERATED_QUEUE_ID


async def delete_durable_queue(stub, metadata, mocker, dlq, send_requests=True):
    """Delete a durable queue; assert the delete reached the consumer."""
    spy = mocker.patch.object(DurableQueueConsumer, "delete_queue")

    await stub.DeleteDurableQueue(
        DeleteDurableQueueRequest(queue_id=QUEUE_ID), metadata=metadata
    )

    spy.assert_called_once_with(QUEUE_ID)


async def update_durable_queue(stub, metadata, mocker, dlq, send_requests=True):
    """Update a durable queue's bindings; assert the RPC completed."""
    mocker.patch.object(DurableQueueConsumer, "get_queue", return_value="test-queue")
    mocker.patch.object(DurableQueueConsumer, "update_queue")

    request = UpdateDurableQueueRequest(queue_id=QUEUE_ID)
    request.current_filter_groups.append(filter_group())
    request.new_filter_groups.append(filter_group())

    response = await stub.UpdateDurableQueue(request, metadata=metadata)

    assert isinstance(response, UpdateDurableQueueResponse)


async def manage_dead_lettered_events(stub, metadata, mocker, dlq, send_requests=True):
    """
    Browse the DLQ; assert one event per dead-lettered message arrived.

    This one never writes, in either direction: browse is server-initiated,
    so the opening request ``test_manage_dlq.py`` sends (it needs the
    delivery tags before it can write an action) is not needed to prove
    admission.
    """
    call = stub.ManageDeadLetteredEvents(metadata=metadata)

    browse = []
    while len(browse) < len(dlq.messages):
        response = json_format.MessageToDict(
            await call.read(), preserving_proto_field_name=True
        )
        assert "dead_lettered_event" in response
        browse.append(response["dead_lettered_event"])

    call.cancel()

    assert len(browse) == len(dlq.messages)


async def consume_non_durable_events(stub, metadata, mocker, dlq, send_requests=True):
    """Consume a non-durable stream; assert the token plus every event."""

    def requests():
        if send_requests:
            request = ConsumeNonDurableEventsRequest()
            request.filter_groups.append(filter_group(event_type="e1"))
            yield request

    responses = [
        response
        async for response in stub.ConsumeNonDurableEvents(
            requests(), metadata=metadata
        )
    ]

    if not send_requests:
        return

    assert len(responses) == EXPECTED_NON_DURABLE_RESPONSE_COUNT
    assert responses[0].reconnect_token


# Every RPC gated on the durable pool. ManageDeadLetteredEvents is here
# because dead-letter browse borrows a channel from the durable pool.
DURABLE_POOL_RPCS = [
    pytest.param(consume_durable_events, id="ConsumeDurableEvents"),
    pytest.param(consume_durable_events_with_acks, id="ConsumeDurableEventsWithAcks"),
    pytest.param(create_durable_queue, id="CreateDurableQueue"),
    pytest.param(delete_durable_queue, id="DeleteDurableQueue"),
    pytest.param(update_durable_queue, id="UpdateDurableQueue"),
    pytest.param(manage_dead_lettered_events, id="ManageDeadLetteredEvents"),
]


class TestNonDurablePoolSaturated:
    """
    The guard refuses the non-durable direction; the durable one has room.

    This is the incident shape: a fan-out of non-durable streams drives the
    pod past what it may admit, and only the non-durable entry point may
    503. What produces that refusal is deliberately left to the mocked
    guard here -- since Epic 7 the non-durable pool's own bound is a soft
    packing target that refuses no one, and the hard refusal comes from the
    memory cap (see ``TestRealGuardAgainstPoolStats``). These tests are
    about which pool each entry point asks about, so they are insensitive
    to which tier said no.
    """

    @pytest.mark.asyncio
    async def test_rejects_non_durable_consumer(
        self, event_consumer_stub, metadata, capacity_guard_rejections, mocker
    ):
        capacity_guard_rejections(non_durable_rejects=True)

        with pytest.raises(AioRpcError) as exc_info:
            await consume_non_durable_events(
                event_consumer_stub, metadata, mocker, None, send_requests=False
            )

        assert exc_info.value.code() == RESOURCE_EXHAUSTED

    @pytest.mark.asyncio
    async def test_rejection_details_report_the_exhausted_pool(
        self, event_consumer_stub, metadata, capacity_guard_rejections, mocker
    ):
        capacity_guard_rejections(non_durable_rejects=True)

        with pytest.raises(AioRpcError) as exc_info:
            await consume_non_durable_events(
                event_consumer_stub, metadata, mocker, None, send_requests=False
            )

        assert exc_info.value.details() == CHANNEL_POOL_EXHAUSTED_DETAIL

    @pytest.mark.asyncio
    async def test_non_durable_fan_out_does_not_reject_durable_consumer(
        self, event_consumer_stub, metadata, capacity_guard_rejections, mocker
    ):
        # The 2026-08-11 regression, named for the incident it encodes: a
        # desktop client's scene load saturates the non-durable pool, and a
        # durable consumer must still stream every event (PRD G-6 / FR-3).
        guard = capacity_guard_rejections(non_durable_rejects=True)

        await consume_durable_events(event_consumer_stub, metadata, mocker, None)

        guard.assert_called_once_with(durable_pool=True)

    @pytest.mark.parametrize("invoke_rpc", DURABLE_POOL_RPCS)
    @pytest.mark.asyncio
    async def test_durable_rpc_is_admitted(
        self,
        invoke_rpc,
        event_consumer_stub,
        metadata,
        capacity_guard_rejections,
        mock_dlq_browse,
        allow_dlq_permission,
        mocker,
    ):
        # Each invoke_rpc asserts its own normal outcome, so a durable RPC
        # that failed for any reason -- capacity or otherwise -- fails here
        # rather than passing on the absence of an exception. The guard
        # assertion pins WHY it was admitted: it asked about the durable
        # pool, not that it skipped the check.
        guard = capacity_guard_rejections(non_durable_rejects=True)

        await invoke_rpc(event_consumer_stub, metadata, mocker, mock_dlq_browse)

        guard.assert_called_once_with(durable_pool=True)


class TestDurablePoolExhausted:
    """
    The durable pool is full; the non-durable direction has room.

    The mirror image: every durable / DLQ entry point 503s, and the
    non-durable consumer is untouched. The durable pool's bound really is
    a hard refusal -- durable admission stayed single-tier through Epic 7 --
    but as above these tests drive a mocked guard, so they pin pool
    selection rather than the arithmetic behind it.
    """

    @pytest.mark.parametrize("invoke_rpc", DURABLE_POOL_RPCS)
    @pytest.mark.asyncio
    async def test_rejects_durable_rpc(
        self,
        invoke_rpc,
        event_consumer_stub,
        metadata,
        capacity_guard_rejections,
        mock_dlq_browse,
        allow_dlq_permission,
        mocker,
    ):
        capacity_guard_rejections(durable_rejects=True)

        with pytest.raises(AioRpcError) as exc_info:
            await invoke_rpc(
                event_consumer_stub,
                metadata,
                mocker,
                mock_dlq_browse,
                send_requests=False,
            )

        assert exc_info.value.code() == RESOURCE_EXHAUSTED

    @pytest.mark.asyncio
    async def test_rejection_details_report_the_exhausted_pool(
        self, event_consumer_stub, metadata, capacity_guard_rejections, mocker
    ):
        capacity_guard_rejections(durable_rejects=True)

        with pytest.raises(AioRpcError) as exc_info:
            await create_durable_queue(event_consumer_stub, metadata, mocker, None)

        assert exc_info.value.details() == CHANNEL_POOL_EXHAUSTED_DETAIL

    @pytest.mark.asyncio
    async def test_non_durable_consumer_is_admitted(
        self, event_consumer_stub, metadata, capacity_guard_rejections, mocker
    ):
        guard = capacity_guard_rejections(durable_rejects=True)

        await consume_non_durable_events(event_consumer_stub, metadata, mocker, None)

        # The absent durable_pool= is what this row is about: the non-durable
        # entry point asked the guard about the NON-durable pool, so an
        # exhausted durable pool could not have reached it. Asserted this way
        # rather than with assert_called_once_with() because Epic 7 gave
        # raise_if_at_capacity a second keyword (the stream class); that
        # keyword is the shed tier's to pin, and spelling it out here would
        # make this pool-selection row fail on a change it says nothing about.
        assert guard.call_count == 1
        assert "durable_pool" not in guard.call_args.kwargs


class TestRealGuardAgainstPoolStats:
    """
    Both directions again, with the production guard in the loop.

    The mock-level tables above prove each entry point selects the right
    pool; these prove the selection actually lands on that pool's stats
    and that the guard's own arithmetic decides the outcome.

    The two directions are driven through DIFFERENT constraints, because
    the guard no longer applies one bound to both. Durable admission is
    single-tier and is driven through the durable pool's own stream bound.
    Non-durable admission is three-tier, and the only tier that hard-refuses
    a pinned stream is the memory cap -- so the non-durable direction is
    driven through the cap, and a row that filled the non-durable POOL and
    expected a refusal would now be admitted. Every test states its cap
    explicitly: the ones about the cap set a binding one, the rest set a
    non-binding one so nothing here rides on the shipped prefetch default.
    """

    @pytest.mark.asyncio
    async def test_rejects_non_durable_consumer_at_the_memory_cap(
        self, event_consumer_stub, metadata, real_capacity_guard, mocker
    ):
        # DECIDED BEHAVIOR CHANGE (Epic 7, Task 2): this row used to fill the
        # non-durable POOL and expect a refusal. The pool bound is now a soft
        # packing/scaling threshold that refuses no one -- streams above it
        # land in the overflow band -- and the memory cap is the hard refusal
        # (SR-3.1 as amended, PRD FR-11). So the refusal is driven through the
        # cap, at CAPPED streams against 50 pool slots: 30 slots are still
        # free, which is what makes the cap the only thing that can explain
        # this rejection.
        real_capacity_guard(
            non_durable_clients=CAPPED,
            durable_clients=IDLE,
            stream_cap=BINDING_CAP,
        )

        with pytest.raises(AioRpcError) as exc_info:
            await consume_non_durable_events(
                event_consumer_stub, metadata, mocker, None, send_requests=False
            )

        assert exc_info.value.code() == RESOURCE_EXHAUSTED

    @pytest.mark.asyncio
    async def test_admits_durable_consumer_when_non_durable_streams_are_capped(
        self, event_consumer_stub, metadata, real_capacity_guard, mocker
    ):
        # The RPC-level twin of the 2026-08-11 incident regression, and a
        # stronger form of it than a full non-durable pool: the non-durable
        # side is at its HARD refusal (the row above proves this same load
        # turns a non-durable consumer away), and the durable consumer must
        # still stream every event. consume_durable_events asserts that
        # outcome, so a durable call that failed for any reason fails here
        # rather than passing on an absent exception.
        real_capacity_guard(
            non_durable_clients=CAPPED,
            durable_clients=IDLE,
            stream_cap=BINDING_CAP,
        )

        await consume_durable_events(event_consumer_stub, metadata, mocker, None)

    @pytest.mark.asyncio
    async def test_rejects_durable_rpc_when_durable_pool_is_full(
        self, event_consumer_stub, metadata, real_capacity_guard, mocker
    ):
        # Unchanged by Epic 7: durable admission is byte-for-byte what it
        # was, still a strict refusal at the durable pool's own bound. The
        # non-binding cap is stated so the durable direction cannot be
        # confused with a cap effect -- the cap never reads the durable pool.
        real_capacity_guard(
            non_durable_clients=IDLE,
            durable_clients=SATURATED,
            stream_cap=NON_BINDING_CAP,
        )

        with pytest.raises(AioRpcError) as exc_info:
            await delete_durable_queue(event_consumer_stub, metadata, mocker, None)

        assert exc_info.value.code() == RESOURCE_EXHAUSTED

    @pytest.mark.asyncio
    async def test_admits_non_durable_consumer_when_durable_pool_is_full(
        self, event_consumer_stub, metadata, real_capacity_guard, mocker
    ):
        # The mirror admit direction, under a cap that constrains nothing:
        # an exhausted durable pool must not reach the non-durable path, and
        # with the cap non-binding the only way this could reject is the
        # wrong pool's stats being read.
        real_capacity_guard(
            non_durable_clients=IDLE,
            durable_clients=SATURATED,
            stream_cap=NON_BINDING_CAP,
        )

        await consume_non_durable_events(event_consumer_stub, metadata, mocker, None)
