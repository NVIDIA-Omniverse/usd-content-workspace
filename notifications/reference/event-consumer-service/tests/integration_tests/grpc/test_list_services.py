# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import grpc
import pytest
import pytest_asyncio
from grpc.aio import insecure_channel
from notification_common.config.auth_settings import auth_settings
from notification_common.grpc.capabilities_servicer import CapabilitiesServicer
from notification_common.interceptors import interceptors

from app.grpc.grpc_server import GrpcServer
from nvidia.omniverse.discovery.capabilities.v2alpha import (
    capabilities_service_pb2,
    capabilities_service_pb2_grpc,
)


@pytest_asyncio.fixture
async def capabilities_server():
    """Stand up a real gRPC server that serves the Discovery
    CapabilitiesService behind the production interceptor stack (including the
    AuthInterceptor), mirroring the conftest ``grpc_server`` fixture. Registers
    the servicer from ``GrpcServer.capabilities_service_entry()`` so the
    ListServices response is exactly what this service advertises in production.
    Port 0 lets the OS assign a free port to avoid collisions under parallel
    execution.
    """
    server = grpc.aio.server(interceptors=await interceptors.get_interceptors())
    servicer = CapabilitiesServicer(GrpcServer.capabilities_service_entry())
    capabilities_service_pb2_grpc.add_CapabilitiesServiceServicer_to_server(
        servicer, server
    )
    port = server.add_insecure_port("localhost:0")
    await server.start()
    yield port
    await server.stop(None)


@pytest_asyncio.fixture
async def capabilities_stub(capabilities_server, grpc_host):
    async with insecure_channel(f"{grpc_host}:{capabilities_server}") as channel:
        yield capabilities_service_pb2_grpc.CapabilitiesServiceStub(channel)


class TestListServices:
    @pytest.mark.asyncio
    async def test_succeeds_without_auth_metadata(self, capabilities_stub):
        # Precondition: auth must be enabled so ``get_interceptors()`` actually
        # includes the AuthInterceptor in the server stack. Otherwise the
        # no-metadata call below would pass vacuously (nothing to exempt from)
        # if the ambient baseline ever flipped tokenless.
        assert auth_settings.authentication_enabled()
        # No `metadata=` argument: the AuthInterceptor must exempt ListServices
        # so the unauthenticated discovery scan can reach it. Without the
        # exemption this call would abort UNAUTHENTICATED.
        response = await capabilities_stub.ListServices(
            capabilities_service_pb2.ListServicesRequest()
        )
        assert response.services[0].service_name == "notification-consumer"
