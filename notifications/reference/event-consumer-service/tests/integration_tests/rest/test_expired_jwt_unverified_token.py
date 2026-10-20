# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
The REST half of the expired-``jwtUnverified``-token regression.

The gRPC half, and the full account of what the regression was, live in
``tests/integration_tests/grpc/test_expired_jwt_unverified_token.py``. The
two transports reach the same in-handler resolutions by different routes --
a gRPC interceptor there, an ASGI middleware here -- so the REST surface
gets its own end-to-end pin rather than inheriting the gRPC one: this
service's REST app is built by ``create_app`` with its own excluded-path
list, and whether the middleware resolved the token before the route ran is
exactly what the second resolution on each of these two paths depends on.

Both paths the Issue names are covered: the durable create, and the
non-durable attach that reaches ``get_instance`` with a reconnect token.

Driven through ``TestClient`` against the real app, so the middleware in
play is the production one.
"""

import pytest

from app.lib.rabbitmq import NonDurableQueueConsumer
from tests.fixtures.expired_jwt_unverified_token import (
    make_expired_jwt_unverified_token,
)
from tests.fixtures.reconnect_token import RECONNECT_TOKEN
from tests.integration_tests.conftest import CREATED_QUEUE_ID
from tests.integration_tests.rest.conftest import parse_sse_response

DURABLE_QUEUE_PATH = "/api/v1beta/queues/durable"

NON_DURABLE_STREAM_PATH = "/api/v1beta/events/stream"


@pytest.fixture(autouse=True)
def mock_principal_id():
    """
    Opt out of the suite-wide principal stub, as the gRPC half does.

    Under ``tests/conftest.py``'s autouse stub nothing in this module would
    touch the real resolver, the real cache, or the real decoder, and every
    assertion below would pass whatever the library does with an expired
    token.
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
def expired_token_headers():
    """Present an already-expired token the way a REST client does."""
    return {"Authorization": f"Bearer {make_expired_jwt_unverified_token()}"}


@pytest.mark.asyncio
class TestDurableCreateUnderAnExpiredJwtUnverifiedToken:
    """
    The REST durable-create resolves the token three times: the auth
    middleware, the route's ``Depends(authenticate)``, and ``create_queue``
    itself, which needs the principal for the create log line.
    """

    async def test_the_create_succeeds(
        self, client, create_queue_payload, expired_token_headers, durable_broker
    ):
        response = client.post(
            DURABLE_QUEUE_PATH, json=create_queue_payload, headers=expired_token_headers
        )

        assert response.status_code == 200

    async def test_the_queue_is_actually_declared(
        self, client, create_queue_payload, expired_token_headers, durable_broker
    ):
        response = client.post(
            DURABLE_QUEUE_PATH, json=create_queue_payload, headers=expired_token_headers
        )

        assert response.json()["queue_id"] == CREATED_QUEUE_ID

    async def test_the_request_decodes_the_token_once(
        self,
        client,
        create_queue_payload,
        expired_token_headers,
        durable_broker,
        decodes,
    ):
        client.post(
            DURABLE_QUEUE_PATH, json=create_queue_payload, headers=expired_token_headers
        )

        assert decodes.call_count == 1

    async def test_a_lapsed_cache_lifetime_returns_401(
        self,
        client,
        create_queue_payload,
        expired_token_headers,
        durable_broker,
        lapsed_cache_lifetime,
    ):
        """
        Vacuity control, and a tripwire for notification_common b.3z5 item
        1 -- see the ``lapsed_cache_lifetime`` fixture. The refusal is not
        behaviour this service wants pinned: when the library re-resolves
        on a stale entry this arm starts failing, and that is the signal to
        revisit it.
        """
        response = client.post(
            DURABLE_QUEUE_PATH, json=create_queue_payload, headers=expired_token_headers
        )

        assert response.status_code == 401

    async def test_a_token_that_does_not_decode_is_still_refused(
        self, client, create_queue_payload, durable_broker
    ):
        # The discrimination check: the middleware really is resolving the
        # presented token, so the successes above are attributable to the
        # resolution accepting an expired one.
        response = client.post(
            DURABLE_QUEUE_PATH,
            json=create_queue_payload,
            headers={"Authorization": "Bearer not-a-jwt"},
        )

        assert response.status_code == 401


@pytest.fixture
def attach_headers(expired_token_headers):
    """
    Reattach to an existing non-durable queue the way a browser does.

    The token rides in ``Last-Event-ID``, which is the carrier an
    ``EventSource`` uses on its automatic reconnect and the one the
    deprecated query parameter points clients at. Either carrier reaches
    the same ``get_instance`` call, so the header is the one worth pinning.
    """
    return {**expired_token_headers, "Last-Event-ID": RECONNECT_TOKEN}


@pytest.mark.asyncio
class TestNonDurableAttachUnderAnExpiredJwtUnverifiedToken:
    """
    The SSE attach resolves the token three times too: the auth middleware,
    the route's ``Depends(authenticate)``, and ``get_instance``, which needs
    the principal as the value half of the stream's ``CacheScope``.

    That third resolution is the same call the gRPC half already pins, but
    it is reached here through the ASGI middleware and this route's own
    dependency chain -- and it is the one that used to raise, so the REST
    surface gets its own pin rather than inheriting the gRPC one.
    """

    async def test_the_attach_succeeds(
        self, client, attach_headers, non_durable_broker
    ):
        response = client.get(NON_DURABLE_STREAM_PATH, headers=attach_headers)

        assert response.status_code == 200

    async def test_the_stream_opens_on_the_reconnected_queue(
        self, client, attach_headers, non_durable_broker
    ):
        # The 200 above has to be a stream that actually attached, not a
        # request refused before it: the resolution that used to raise sat
        # ahead of the queue lookup, so a stream that opens on the presented
        # token is one whose resolution succeeded.
        response = client.get(NON_DURABLE_STREAM_PATH, headers=attach_headers)

        connected_event = parse_sse_response(response.content.decode("utf-8"))[0]
        assert connected_event["data"]["reconnect_token"] == RECONNECT_TOKEN

    async def test_the_request_decodes_the_token_once(
        self, client, attach_headers, non_durable_broker, decodes
    ):
        client.get(NON_DURABLE_STREAM_PATH, headers=attach_headers)

        assert decodes.call_count == 1

    async def test_a_lapsed_cache_lifetime_returns_401(
        self, client, attach_headers, non_durable_broker, lapsed_cache_lifetime
    ):
        """
        Vacuity control, and a tripwire for notification_common b.3z5 item
        1 -- see the ``lapsed_cache_lifetime`` fixture. The refusal is not
        behaviour this service wants pinned: when the library re-resolves
        on a stale entry this arm starts failing, and that is the signal to
        revisit it.
        """
        response = client.get(NON_DURABLE_STREAM_PATH, headers=attach_headers)

        assert response.status_code == 401

    async def test_a_lapsed_cache_lifetime_takes_no_pool_checkout(
        self, client, attach_headers, non_durable_broker, lapsed_cache_lifetime
    ):
        """
        Vacuity control and b.3z5 tripwire, as above -- see the
        ``lapsed_cache_lifetime`` fixture.
        """
        # Under this fixture the raise lands at the route's
        # Depends(authenticate), the second resolution -- the middleware's
        # resolution stored an expires_at one second in the past, so the
        # cached read authenticate() performs raises above the handler's own
        # get_instance call. This pins only "no side effect before any
        # refusal" (no channel taken, no queue left behind), not the
        # consumer-side ordering. For that ordering see
        # tests/app/lib/rabbitmq/test_non_durable_queue_consumer.py's
        # TestGetInstanceResolvesThePrincipalFirst.
        client.get(NON_DURABLE_STREAM_PATH, headers=attach_headers)

        NonDurableQueueConsumer.connector.get_channel_direct.assert_not_called()

    async def test_a_token_that_does_not_decode_is_still_refused(
        self, client, non_durable_broker
    ):
        # The discrimination check, as on the durable half above.
        response = client.get(
            NON_DURABLE_STREAM_PATH,
            headers={
                "Authorization": "Bearer not-a-jwt",
                "Last-Event-ID": RECONNECT_TOKEN,
            },
        )

        assert response.status_code == 401
