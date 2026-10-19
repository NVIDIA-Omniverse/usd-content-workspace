# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from contextlib import asynccontextmanager

import grpc
import pytest
import pytest_asyncio
from grpc.aio import AioRpcError, insecure_channel

from app.main import GrpcServer

from nvidia.omniverse.discovery.capabilities.v2alpha import (
    capabilities_service_pb2,
    capabilities_service_pb2_grpc,
)
from nvidia.omniverse.notifications.publisher.v1beta import (
    event_publisher_pb2,
    event_publisher_pb2_grpc,
)

pytestmark = pytest.mark.asyncio

# A dedicated port so this server does not collide with the EventPublishingService
# `grpc_server` fixture (localhost:50055) from the top-level conftest.
CAPABILITIES_GRPC_PORT = "50056"


@pytest_asyncio.fixture
async def capabilities_server():
    # Build the real production server assembly (application servicer, auth
    # interceptor stack, and the Discovery CapabilitiesService) so the auth
    # exemption is exercised end to end.
    server = await GrpcServer._make_server(f"localhost:{CAPABILITIES_GRPC_PORT}")
    await server.start()
    yield server
    await server.stop(None)


@asynccontextmanager
async def _channel(grpc_host):
    async with insecure_channel(f"{grpc_host}:{CAPABILITIES_GRPC_PORT}") as c:
        yield c


@pytest_asyncio.fixture
async def capabilities_stub(capabilities_server, grpc_host):
    async with _channel(grpc_host) as c:
        yield capabilities_service_pb2_grpc.CapabilitiesServiceStub(c)


@pytest_asyncio.fixture
async def event_publishing_stub(capabilities_server, grpc_host):
    # A stub for the non-exempt EventPublishingService on the same production
    # server assembly, used to prove the AuthInterceptor is actually enforcing
    # auth (so the ListServices exemption cannot pass for the wrong reason).
    async with _channel(grpc_host) as c:
        yield event_publisher_pb2_grpc.EventPublishingServiceStub(c)


class TestPublishEvent:
    async def test_rejects_call_without_auth_metadata(self, event_publishing_stub):
        # Negative control: a non-exempt RPC with no authorization metadata must
        # be rejected by the AuthInterceptor with UNAUTHENTICATED.
        with pytest.raises(AioRpcError) as exc_info:
            await event_publishing_stub.PublishEvent(
                event_publisher_pb2.PublishEventRequest()
            )
        assert exc_info.value.code() == grpc.StatusCode.UNAUTHENTICATED


class TestListServices:
    async def test_succeeds_without_auth_metadata(self, capabilities_stub):
        # No authorization metadata is supplied; the RPC completing at all
        # proves the ListServices auth exemption works end to end.
        response = await capabilities_stub.ListServices(
            capabilities_service_pb2.ListServicesRequest()
        )
        assert len(response.services) == 1

    async def test_advertises_notification_aggregation_without_auth_metadata(
        self, capabilities_stub
    ):
        response = await capabilities_stub.ListServices(
            capabilities_service_pb2.ListServicesRequest()
        )
        assert response.services[0].service_name == "notification-aggregation"
