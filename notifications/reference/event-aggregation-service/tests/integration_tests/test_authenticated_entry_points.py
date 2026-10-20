# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Every authenticated entry point resolves the caller's token before it works.

Nothing about the bearer token is checked until something resolves it to a
principal. That resolution used to be a side effect of stamping
``principal_identity`` onto the published event, which meant a deployment
running with authentication on but the permissions service off published
unauthenticated events and nothing on the request path noticed. ``authenticate``
is now the named step, called first by each of the four entry points -- and from
notification-common 6.12.0 the gRPC interceptor and the REST auth middleware
resolve the token before any handler runs as well, so the service's own call is
answered from ``principal_id``'s per-request cache. This module pins the
resulting behaviour uniformly, at whichever layer produces it:

* a refused token ends the request with UNAUTHENTICATED / 401 and never
  reaches the broker (:class:`TestARefusedTokenIsRefusedEverywhere`);
* the refusal comes *before* every other refusal the entry point can make,
  so a malformed request from an unauthenticated caller is still a 401
  (:class:`TestAuthenticationPrecedesRequestValidation`);
* the entry-point table is the whole surface, so a new RPC or route that
  forgets to authenticate fails a guard here rather than shipping
  (:class:`TestTheEntryPointTableIsComplete`);
* the service resolves the token exactly once per request
  (:class:`TestAnExpiredJwtUnverifiedTokenResolvesOnce`).
"""

import json
from types import SimpleNamespace
from typing import Callable, NamedTuple

import grpc
import jwt
import pytest
from grpc.aio import AioRpcError
from notification_common.auth import principal_id
from notification_common.auth.principal_id import principal_id as resolve_principal
from notification_common.auth.principal_source import PrincipalSource
from notification_common.config.auth_settings import auth_settings
from notification_common.errors.errors import AuthenticationError

from app.lib.auth import authenticate as authenticate_module
from app.lib.auth.authenticate import authenticate
from app.lib.rabbitmq import publisher
from app.rest import routes as routes_module

from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2

from nvidia.omniverse.notifications.publisher.v1beta.event_publisher_pb2 import (
    BatchPublishEventsRequest,
    PublishEventRequest,
)

#: The message the refused resolution raises with, matched on the wire.
REFUSAL_MESSAGE = "Invalid or expired JWT"

#: The principal claimed by the expired token the regression guard sends.
EXPIRED_TOKEN_PRINCIPAL = "expired-principal"

#: Signing material for that token. Nothing verifies the signature in this
#: mode, but PyJWT warns below 32 bytes and the warning is only noise here.
EXPIRED_TOKEN_SECRET = "a-secret-long-enough-to-keep-pyjwt-quiet"

#: An `exp` far enough in the past to be expired under any clock, including
#: the frozen one the shared event fixtures install.
LONG_EXPIRED = 1

#: The gRPC service whose RPC list the completeness guard reads.
GRPC_SERVICE_NAME = "EventPublishingService"

#: Every route on the REST router that authenticates, by the path the router
#: knows it as (the app mounts the router under ``/api/v1beta``). ``/health``
#: is deliberately absent: it is reachable without a token by design.
AUTHENTICATED_ROUTES = {"/events", "/events/batch"}


class Outcome(NamedTuple):
    """What the caller sees: a gRPC status code or an HTTP status, and detail."""

    status: object
    detail: str | None


class EntryPoint(NamedTuple):
    """
    One authenticated entry point, described so a single table can drive both
    transports.

    ``publisher_method`` names the broker call that stands for "the request
    did the thing the caller asked for", so a refusal test can assert it was
    never reached. ``call`` runs the entry point to its normal outcome and
    normalises the result; ``refused`` and ``accepted`` are that transport's
    spellings of 401 and 200.
    """

    publisher_method: str
    call: Callable
    refused: object
    accepted: object


class Rig:
    """
    Access to the fixtures an entry point needs, by name.

    The table's drivers span two transports and each row uses only half of
    what the table as a whole needs, so the rig resolves a fixture on
    attribute access rather than listing every one of them in every
    signature.
    """

    def __init__(self, request):
        self._request = request

    def __getattr__(self, name):
        return self._request.getfixturevalue(name)


async def _grpc_outcome(rpc) -> Outcome:
    """Await a gRPC call and report its status rather than raising."""
    try:
        await rpc
    except AioRpcError as error:
        return Outcome(error.code(), error.details())

    return Outcome(grpc.StatusCode.OK, None)


def _rest_outcome(response) -> Outcome:
    return Outcome(response.status_code, response.text)


async def _grpc_publish_event(rig: Rig) -> Outcome:
    return await _grpc_outcome(
        rig.event_aggregation_stub.PublishEvent(
            PublishEventRequest(event=rig.mocked_event), metadata=rig.metadata
        )
    )


async def _grpc_batch_publish_events(rig: Rig) -> Outcome:
    return await _grpc_outcome(
        rig.event_aggregation_stub.BatchPublishEvents(
            BatchPublishEventsRequest(events=[rig.mocked_event]), metadata=rig.metadata
        )
    )


async def _rest_publish_event(rig: Rig) -> Outcome:
    return _rest_outcome(
        rig.client.post(
            "/api/v1beta/events",
            json={"event": rig.sample_event},
            headers=rig.auth_headers,
        )
    )


async def _rest_batch_publish_events(rig: Rig) -> Outcome:
    return _rest_outcome(
        rig.client.post(
            "/api/v1beta/events/batch",
            json={"events": [rig.sample_event]},
            headers=rig.auth_headers,
        )
    )


# Keyed by the RPC name the proto descriptor uses, so the completeness guard
# below can compare the table against the generated service definition.
GRPC_ENTRY_POINTS = {
    "PublishEvent": EntryPoint(
        publisher_method="publish",
        call=_grpc_publish_event,
        refused=grpc.StatusCode.UNAUTHENTICATED,
        accepted=grpc.StatusCode.OK,
    ),
    "BatchPublishEvents": EntryPoint(
        publisher_method="batch_publish",
        call=_grpc_batch_publish_events,
        refused=grpc.StatusCode.UNAUTHENTICATED,
        accepted=grpc.StatusCode.OK,
    ),
}

REST_ENTRY_POINTS = {
    "POST /api/v1beta/events": EntryPoint(
        publisher_method="publish",
        call=_rest_publish_event,
        refused=401,
        accepted=200,
    ),
    "POST /api/v1beta/events/batch": EntryPoint(
        publisher_method="batch_publish",
        call=_rest_batch_publish_events,
        refused=401,
        accepted=200,
    ),
}

ENTRY_POINTS = [
    pytest.param(entry_point, id=f"grpc {name}")
    for name, entry_point in GRPC_ENTRY_POINTS.items()
] + [
    pytest.param(entry_point, id=f"rest {name}")
    for name, entry_point in REST_ENTRY_POINTS.items()
]


@pytest.fixture
def rig(request, event_aggregation_stub):
    """
    Build the rig for whichever entry point the row under test drives.

    ``event_aggregation_stub`` is requested up front rather than resolved on
    demand because it is an async fixture, and pytest cannot set one up from
    inside an already-running event loop. Everything else the drivers reach
    for is synchronous and resolves lazily.
    """
    return Rig(request)


@pytest.fixture
def broker(mocker, successful_result):
    """
    Stand in for the message broker on both publish paths.

    Both are patched for every row so a test can assert the *other* path was
    not reached either, and the returned mocks are what "the request did its
    work" is read off.
    """
    return {
        "publish": mocker.patch.object(
            publisher, "publish", autospec=True, return_value=successful_result
        ),
        "batch_publish": mocker.patch.object(
            publisher,
            "batch_publish",
            autospec=True,
            return_value=[successful_result],
        ),
    }


@pytest.fixture
def refuse_the_token(mocker):
    """Make the principal resolution refuse the caller's token.

    No ``autospec``: the integration ``conftest``'s autouse ``setup_auth``
    has already replaced this attribute, and ``mock`` refuses to build a spec
    from an attribute that is itself a mock. The unit tests in
    ``tests/app/lib/auth/test_authenticate.py`` patch the same callable with
    ``autospec=True``, so signature drift is still caught.
    """
    return mocker.patch.object(
        principal_id,
        "principal_id",
        side_effect=AuthenticationError(REFUSAL_MESSAGE),
    )


@pytest.mark.asyncio
class TestARefusedTokenIsRefusedEverywhere:
    @pytest.mark.parametrize("entry_point", ENTRY_POINTS)
    async def test_refuses_the_request(
        self, entry_point, rig, refuse_the_token, broker
    ):
        outcome = await entry_point.call(rig)

        assert outcome.status == entry_point.refused

    @pytest.mark.parametrize("entry_point", ENTRY_POINTS)
    async def test_never_reaches_the_broker(
        self, entry_point, rig, refuse_the_token, broker
    ):
        # The ordering pin. Without it ``await authenticate()`` could sit at
        # the bottom of every handler -- the event already published -- and
        # the status assertion above would still pass, because the refusal
        # reaches the caller either way.
        await entry_point.call(rig)

        broker[entry_point.publisher_method].assert_not_called()


@pytest.mark.asyncio
class TestAuthenticationPrecedesRequestValidation:
    """
    An unauthenticated caller gets 401, not a critique of their request.

    Each entry point can refuse a request for reasons of its own -- an empty
    batch, a body that does not match the schema. Those refusals describe the
    request, so answering one to a caller whose token was never accepted both
    leaks the shape of the API and hides the real reason. Authentication is
    therefore the first statement in every handler, and these rows pin the
    resulting precedence.
    """

    async def test_grpc_empty_batch_is_unauthenticated_not_invalid_argument(
        self, event_aggregation_stub, metadata, refuse_the_token, broker
    ):
        outcome = await _grpc_outcome(
            event_aggregation_stub.BatchPublishEvents(
                BatchPublishEventsRequest(), metadata=metadata
            )
        )

        assert outcome.status == grpc.StatusCode.UNAUTHENTICATED

    async def test_rest_empty_batch_is_401_not_400(
        self, client, auth_headers, refuse_the_token, broker
    ):
        response = client.post(
            "/api/v1beta/events/batch", json={"events": []}, headers=auth_headers
        )

        assert response.status_code == 401

    async def test_rest_schema_invalid_body_is_401_not_422(
        self, client, auth_headers, refuse_the_token, broker
    ):
        # The auth middleware refuses the token before the route is reached at
        # all; even if it did not, FastAPI solves the handler's dependencies
        # before it validates the body parameters, so the dependency's refusal
        # would still win over the schema's.
        response = client.post(
            "/api/v1beta/events",
            json={"event": {"event_type": "test.event"}},
            headers=auth_headers,
        )

        assert response.status_code == 401

    async def test_rest_unparseable_body_never_reaches_the_broker(
        self, post_raw_body, auth_headers, refuse_the_token, broker
    ):
        # The pin-independent half of the pair: the sibling row below pins the
        # status itself at 401, while this one asserts the consequence that
        # holds whatever status a refusal carries -- an unauthenticated caller
        # publishes nothing. Kept as its own row so any future re-litigation of
        # the status code cannot take the broker-side guarantee down with it.
        post_raw_body("/api/v1beta/events", b"{not json", auth_headers)

        broker["publish"].assert_not_called()

    async def test_rest_unparseable_body_is_401(
        self, post_raw_body, auth_headers, refuse_the_token, broker
    ):
        # The last request shape that used to escape the precedence rule, now
        # inside it. A body that is not JSON at all is rejected while the
        # request is being read, which used to happen before the route
        # dependency that refuses the token could run -- so this one shape
        # answered 422 to an unauthenticated caller and was documented as a
        # residual. From notification-common 6.12.0 the auth middleware
        # resolves the token itself, ahead of the body being read at all, so
        # the refusal now wins here too and the carve-out is closed.
        #
        # Pinned at 401 rather than relaxed to "401 or 422": accepting both
        # would stop this row from noticing if the middleware ever went back
        # to extracting the token without resolving it.
        response = post_raw_body("/api/v1beta/events", b"{not json", auth_headers)

        assert response.status_code == 401


class TestTheEntryPointTableIsComplete:
    """
    The table above is the whole authenticated surface.

    Every test in this module is only as good as the table it iterates, so an
    entry point added without a row would be silently unpinned. These read the
    surface off the generated service descriptor and off the router rather
    than off a list this file maintains by hand.
    """

    def test_every_rpc_has_a_row(self):
        service = event_publisher_pb2.DESCRIPTOR.services_by_name[GRPC_SERVICE_NAME]

        assert {method.name for method in service.methods} == set(GRPC_ENTRY_POINTS)

    def test_exactly_the_publish_routes_depend_on_authenticate(self):
        # ``route.dependant.dependencies`` and not ``route.dependencies``: the
        # latter holds only the decorator's ``Depends(http_bearer)``, so the
        # same assertion against it would pass without any route resolving a
        # principal at all.
        #
        # Set equality rather than "no route is unauthenticated", which passes
        # vacuously if the publish routes ever leave this router -- moved to a
        # versioned sub-router, split into a second module. This form fails in
        # both directions: a new route that forgets to authenticate, and an
        # authenticated route that goes missing. ``/health``'s absence from
        # the expected set is the carve-out; that ``/health`` still exists and
        # answers without a token is pinned by
        # ``tests/integration_tests/rest/test_health.py``, which asks for it
        # over the wire with no Authorization header.
        authenticated = {
            route.path
            for route in routes_module.router.routes
            if authenticate
            in [dependency.call for dependency in route.dependant.dependencies]
        }

        assert authenticated == AUTHENTICATED_ROUTES


@pytest.mark.asyncio
class TestAnExpiredJwtUnverifiedTokenResolvesOnce:
    """
    Regression guard for the double-resolution defect.

    Under ``principal_source=jwtUnverified`` an expired token is accepted by
    design: the upstream validator is the trust anchor and this mode makes no
    expiry judgement. The first cut of the fix resolved the principal twice
    per request, and the second resolution was answered from
    ``principal_id``'s per-request cache -- which, at the time, held the
    *token's* own ``exp``. Every expired token therefore published fine on the
    first resolution and was refused with "Token expired" on the second, on
    every entry point.

    Two things had to change and both are pinned here: the library now caches
    a derived lifetime rather than the token's expiry (its own tests pin that
    directly), and the service resolves exactly once. The count is the
    load-bearing assertion -- with the library fixed, a second resolution
    succeeds, so a status-only guard would pass on the defect it exists to
    catch.
    """

    @pytest.fixture
    def expired_token(self):
        return jwt.encode(
            {"sub": EXPIRED_TOKEN_PRINCIPAL, "exp": LONG_EXPIRED},
            EXPIRED_TOKEN_SECRET,
            algorithm="HS256",
        )

    @pytest.fixture
    def mock_auth_token(self, expired_token):
        """Send the expired token instead of the suite's opaque placeholder.

        Overriding the shared fixture re-points both ``metadata`` (gRPC) and
        ``auth_headers`` (REST) at it, so every row in the table carries it.
        """
        return expired_token

    @pytest.fixture(autouse=True)
    def jwt_unverified_source(self, mocker):
        """Select the principal source that accepts an expired token."""
        mocker.patch.object(
            auth_settings, "principal_source", PrincipalSource.JWT_UNVERIFIED
        )

    @pytest.fixture
    def service_resolutions(self, mocker):
        """
        Count the resolutions *this service* performs, per request.

        The real resolution is left in place and merely wrapped, because what
        the guard is checking is the behaviour of the real decoder and the
        real per-request cache; a mock in their place would pin nothing.

        The wrapper is installed by rebinding the ``principal_id`` name inside
        ``app.lib.auth.authenticate`` rather than on the shared
        ``notification_common.auth.principal_id`` module, so the count is the
        service's own resolutions and not the library's. That distinction is
        what makes the count meaningful at any pin: a release whose
        interceptor and middleware resolve the token before a handler runs
        adds a library-side resolution to every request, which would turn a
        module-level count into a pin-specific number, while the property
        under test -- the service resolves once -- is unchanged.
        """
        resolutions = []

        async def _counting_principal_id():
            resolutions.append(1)
            return await resolve_principal()

        mocker.patch.object(
            authenticate_module,
            "principal_id",
            SimpleNamespace(principal_id=_counting_principal_id),
        )

        return resolutions

    @pytest.mark.parametrize("entry_point", ENTRY_POINTS)
    async def test_accepts_the_expired_token(
        self, entry_point, rig, service_resolutions, broker
    ):
        outcome = await entry_point.call(rig)

        assert outcome.status == entry_point.accepted

    @pytest.mark.parametrize("entry_point", ENTRY_POINTS)
    async def test_stamps_the_principal_from_the_expired_token(
        self, entry_point, rig, service_resolutions, broker
    ):
        await entry_point.call(rig)

        published = broker[entry_point.publisher_method].call_args[0][0]
        published_event = (
            published if entry_point.publisher_method == "publish" else published[0]
        )

        assert published_event.principal_identity == EXPIRED_TOKEN_PRINCIPAL

    @pytest.mark.parametrize("entry_point", ENTRY_POINTS)
    async def test_resolves_the_principal_exactly_once(
        self, entry_point, rig, service_resolutions, broker
    ):
        await entry_point.call(rig)

        assert service_resolutions == [1]


@pytest.mark.asyncio
class TestTheRefusalIsReportedToTheCaller:
    """The refusal carries the resolver's message, not a generic 401 body."""

    async def test_grpc_reports_the_refusal_message(
        self, rig, refuse_the_token, broker
    ):
        outcome = await _grpc_publish_event(rig)

        assert REFUSAL_MESSAGE in outcome.detail

    async def test_rest_reports_the_refusal_message(
        self, client, auth_headers, sample_event, refuse_the_token, broker
    ):
        response = client.post(
            "/api/v1beta/events",
            json={"event": sample_event},
            headers=auth_headers,
        )

        assert json.loads(response.text)["detail"] == REFUSAL_MESSAGE
