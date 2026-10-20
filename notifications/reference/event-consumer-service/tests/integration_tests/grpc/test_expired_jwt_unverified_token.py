# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
An expired bearer token under the ``jwtUnverified`` source.

``jwtUnverified`` is the opt-in ``principal_source`` for a deployment in which
every path to the pods validates the token first: the upstream component
validates it and the service only decodes the payload to read the principal
claim, deliberately without judging ``exp``. A request
carrying an expired-but-upstream-validated token is therefore one the
service is supposed to serve.

It did not serve one. A request resolves the token more than once -- the
shared transport layer resolves it before the handler runs, the entry point
calls ``authenticate``, and the durable-create and non-durable-attach paths
resolve it again to name the principal they need as a value -- and every
resolution after the first is answered from the per-request cache, whose
hit test re-reads the stored expiry. While that stored expiry was the
token's own ``exp``, the first resolution succeeded and the next one raised
``AuthenticationError("Token expired")``, so these two paths answered
UNAUTHENTICATED for a token the mode had just accepted. The shared library
now stores a derived cache lifetime instead, so the later resolutions
succeed.

These tests drive the real gRPC server behind the production interceptor
stack with a genuinely expired token, so the resolution the transport
performs is the real one and the in-handler resolutions read the real
cache. They assert the request succeeds and that the whole request cost one
decode, which is the property the callers' comments rely on when they
resolve the principal a second and third time.
"""

import grpc
import pytest
from grpc.aio import AioRpcError

from app.lib.rabbitmq import DurableQueueConsumer, NonDurableQueueConsumer
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ConsumeNonDurableEventsRequest,
    CreateDurableQueueRequest,
)
from tests.fixtures.expired_jwt_unverified_token import (
    make_expired_jwt_unverified_token,
)
from tests.fixtures.reconnect_token import RECONNECT_TOKEN
from tests.integration_tests.conftest import CREATED_QUEUE_ID, filter_group


@pytest.fixture(autouse=True)
def mock_principal_id():
    """
    Opt out of the suite-wide principal stub.

    ``tests/conftest.py`` patches ``principal_id.principal_id`` for every
    test, which answers from a stand-in and never touches the cache or the
    decoder. Under it every assertion in this file would pass whatever the
    library does with an expired token, so the whole module runs against
    the real resolver instead. Overriding the fixture by name is what
    retires it here: pytest resolves the autouse fixture to this
    definition, so nothing patches the resolver.
    """


@pytest.fixture(autouse=True)
def jwt_unverified_source(jwt_unverified_source):
    """
    Make the shared ``jwtUnverified`` selection apply to every test here.

    The fixture in ``tests/integration_tests/conftest.py`` is opt-in, so it
    perturbs no other integration test; this same-named override is what
    turns it on for this module, with pytest handing the shared one in as
    the argument.
    """
    return jwt_unverified_source


@pytest.fixture
def expired_token():
    return make_expired_jwt_unverified_token()


@pytest.fixture
def metadata(expired_token):
    """
    Present the expired token the way a client does.

    Overrides the integration-suite ``metadata`` fixture, whose token is an
    opaque stand-in that no real decoder could read.
    """
    return [("authorization", f"Bearer {expired_token}")]


@pytest.fixture
def create_durable_queue_request():
    request = CreateDurableQueueRequest()
    request.filter_groups.append(filter_group())
    return request


@pytest.mark.asyncio
class TestDurableCreateUnderAnExpiredJwtUnverifiedToken:
    """
    ``CreateDurableQueue`` resolves the token three times: the interceptor,
    ``authenticate``, and ``create_queue``'s own ``_resolve_principal``,
    which needs the principal as the attribution on the create log line.
    """

    async def test_the_create_succeeds(
        self,
        event_consumer_stub,
        metadata,
        durable_broker,
        create_durable_queue_request,
    ):
        response = await event_consumer_stub.CreateDurableQueue(
            create_durable_queue_request, metadata=metadata
        )

        assert response.created is True

    async def test_the_queue_is_actually_declared(
        self,
        event_consumer_stub,
        metadata,
        durable_broker,
        create_durable_queue_request,
    ):
        # The success above has to be a create that reached the broker, not
        # a request refused before it: the resolution that used to raise sat
        # ahead of every side effect in create_queue.
        response = await event_consumer_stub.CreateDurableQueue(
            create_durable_queue_request, metadata=metadata
        )

        assert response.queue_id == CREATED_QUEUE_ID

    async def test_the_request_decodes_the_token_once(
        self,
        event_consumer_stub,
        metadata,
        durable_broker,
        decodes,
        create_durable_queue_request,
    ):
        await event_consumer_stub.CreateDurableQueue(
            create_durable_queue_request, metadata=metadata
        )

        assert decodes.call_count == 1

    async def test_a_lapsed_cache_lifetime_fails_the_create(
        self,
        event_consumer_stub,
        metadata,
        durable_broker,
        lapsed_cache_lifetime,
        create_durable_queue_request,
    ):
        """
        Vacuity control, and a tripwire for notification_common b.3z5 item
        1 -- see the ``lapsed_cache_lifetime`` fixture. The refusal is not
        behaviour this service wants pinned: when the library re-resolves
        on a stale entry this arm starts failing, and that is the signal to
        revisit it.
        """
        with pytest.raises(AioRpcError) as exc_info:
            await event_consumer_stub.CreateDurableQueue(
                create_durable_queue_request, metadata=metadata
            )

        assert exc_info.value.code() == grpc.StatusCode.UNAUTHENTICATED

    async def test_a_lapsed_cache_lifetime_declares_no_queue(
        self,
        event_consumer_stub,
        metadata,
        durable_broker,
        lapsed_cache_lifetime,
        create_durable_queue_request,
    ):
        """
        Vacuity control and b.3z5 tripwire, as above -- see the
        ``lapsed_cache_lifetime`` fixture.
        """
        # Under this fixture the raise lands at the entry point's
        # authenticate(): the transport's resolution stored an expires_at
        # one second in the past, so the SECOND resolution -- the cached
        # read authenticate() performs -- is the one that raises, above the
        # servicer method's own create_queue call. This assertion therefore
        # pins only "no side effect before any refusal", not the
        # consumer-side ordering. That ordering is pinned by
        # tests/app/lib/rabbitmq/test_durable_queue_consumer.py's
        # TestCreateQueueResolvesThePrincipalFirst and TestResolvePrincipal.
        with pytest.raises(AioRpcError):
            await event_consumer_stub.CreateDurableQueue(
                create_durable_queue_request, metadata=metadata
            )

        DurableQueueConsumer._create_queue.assert_not_called()

    async def test_a_token_that_does_not_decode_is_still_refused(
        self, event_consumer_stub, durable_broker, create_durable_queue_request
    ):
        # The discrimination check for every assertion above: the transport
        # really is resolving the presented token, so a request that
        # succeeds does so because the resolution accepted the token rather
        # than because nothing resolved it.
        with pytest.raises(AioRpcError) as exc_info:
            await event_consumer_stub.CreateDurableQueue(
                create_durable_queue_request,
                metadata=[("authorization", "Bearer not-a-jwt")],
            )

        assert exc_info.value.code() == grpc.StatusCode.UNAUTHENTICATED


@pytest.mark.asyncio
class TestTokenResolutionOutranksACapacityRefusal:
    """
    On a saturated pod, a caller whose token does not resolve is refused
    UNAUTHENTICATED rather than RESOURCE_EXHAUSTED.

    The guard is the first statement in every guarded servicer method, but
    the interceptor's resolution runs above the method, so it -- not the
    guard -- answers a request whose token is unusable. That is the
    recorded decision (``ConsumeNonDurableEvents``' guard rationale, and the
    SDD): such a request fails identically on every pod, so the proxy's
    retryOn resource-exhausted policy re-routing it would spend fleet retry
    budget during a saturation incident to reach the same refusal, and an
    unauthenticated caller does not get told the pod is saturated.

    Only the ordering between the two refusals is pinned here. That the
    guard stays ahead of everything inside the method is pinned by the
    admission-ordering and trailers-only tests.
    """

    @pytest.fixture(autouse=True)
    def saturated_durable_pool(self, capacity_guard_rejections):
        return capacity_guard_rejections(durable_rejects=True)

    async def test_an_unresolvable_token_is_refused_unauthenticated(
        self, event_consumer_stub, durable_broker, create_durable_queue_request
    ):
        with pytest.raises(AioRpcError) as exc_info:
            await event_consumer_stub.CreateDurableQueue(
                create_durable_queue_request,
                metadata=[("authorization", "Bearer not-a-jwt")],
            )

        assert exc_info.value.code() == grpc.StatusCode.UNAUTHENTICATED

    async def test_a_resolvable_token_still_gets_the_capacity_status(
        self,
        event_consumer_stub,
        metadata,
        durable_broker,
        create_durable_queue_request,
    ):
        # The other half of the decision: the caller the retry policy is
        # for still gets the status it keys on -- including this one, whose
        # token is expired and resolves anyway.
        with pytest.raises(AioRpcError) as exc_info:
            await event_consumer_stub.CreateDurableQueue(
                create_durable_queue_request, metadata=metadata
            )

        assert exc_info.value.code() == grpc.StatusCode.RESOURCE_EXHAUSTED


@pytest.fixture
def attach_request_stream():
    """The first request of a stream reattaching with a reconnect token."""

    def requests():
        request = ConsumeNonDurableEventsRequest()
        request.reconnect_token = RECONNECT_TOKEN
        yield request

    return requests


@pytest.mark.asyncio
class TestNonDurableAttachUnderAnExpiredJwtUnverifiedToken:
    """
    ``ConsumeNonDurableEvents`` resolves the token three times too: the
    interceptor, ``authenticate``, and ``get_instance``, which needs the
    principal as the value half of the stream's ``CacheScope``.
    """

    async def test_the_attach_succeeds(
        self, event_consumer_stub, metadata, non_durable_broker, attach_request_stream
    ):
        responses = [
            response
            async for response in event_consumer_stub.ConsumeNonDurableEvents(
                attach_request_stream(), metadata=metadata
            )
        ]

        assert responses[0].reconnect_token == RECONNECT_TOKEN

    async def test_the_request_decodes_the_token_once(
        self,
        event_consumer_stub,
        metadata,
        non_durable_broker,
        attach_request_stream,
        decodes,
    ):
        async for _ in event_consumer_stub.ConsumeNonDurableEvents(
            attach_request_stream(), metadata=metadata
        ):
            pass

        assert decodes.call_count == 1

    async def test_a_lapsed_cache_lifetime_fails_the_attach(
        self,
        event_consumer_stub,
        metadata,
        non_durable_broker,
        lapsed_cache_lifetime,
        attach_request_stream,
    ):
        """
        Vacuity control, and a tripwire for notification_common b.3z5 item
        1 -- see the ``lapsed_cache_lifetime`` fixture. The refusal is not
        behaviour this service wants pinned: when the library re-resolves
        on a stale entry this arm starts failing, and that is the signal to
        revisit it.
        """
        with pytest.raises(AioRpcError) as exc_info:
            async for _ in event_consumer_stub.ConsumeNonDurableEvents(
                attach_request_stream(), metadata=metadata
            ):
                pass

        assert exc_info.value.code() == grpc.StatusCode.UNAUTHENTICATED

    async def test_a_lapsed_cache_lifetime_takes_no_pool_checkout(
        self,
        event_consumer_stub,
        metadata,
        non_durable_broker,
        lapsed_cache_lifetime,
        attach_request_stream,
    ):
        """
        Vacuity control and b.3z5 tripwire, as above -- see the
        ``lapsed_cache_lifetime`` fixture.
        """
        # As on the durable half: under this fixture the raise lands at the
        # entry point's authenticate(), the second resolution, above the
        # servicer method's own get_instance call -- so this pins only "no
        # side effect before any refusal" (no channel taken, no queue left
        # behind), not the consumer-side ordering. For that ordering see
        # tests/app/lib/rabbitmq/test_non_durable_queue_consumer.py's
        # TestGetInstanceResolvesThePrincipalFirst.
        with pytest.raises(AioRpcError):
            async for _ in event_consumer_stub.ConsumeNonDurableEvents(
                attach_request_stream(), metadata=metadata
            ):
                pass

        NonDurableQueueConsumer.connector.get_channel_direct.assert_not_called()

    async def test_a_token_that_does_not_decode_is_still_refused(
        self, event_consumer_stub, non_durable_broker, attach_request_stream
    ):
        # The discrimination check, as on the durable half above.
        with pytest.raises(AioRpcError) as exc_info:
            async for _ in event_consumer_stub.ConsumeNonDurableEvents(
                attach_request_stream(),
                metadata=[("authorization", "Bearer not-a-jwt")],
            ):
                pass

        assert exc_info.value.code() == grpc.StatusCode.UNAUTHENTICATED
