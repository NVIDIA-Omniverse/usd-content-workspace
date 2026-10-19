# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from concurrent import futures

import grpc
import pytest
from notification_common.config.common_settings import common_settings
from notification_common.config.observability_config import CAPABILITIES_SERVICE_NAME
from notification_common.servers import start_servers
from notification_common.test_tools.any_instance_of import AnyInstanceOf

from app import main
from app.config.settings import settings
from app.grpc.event_aggregation_servicer import EventAggregationServicer
from app.lib.rabbitmq import rabbitmq_connector
from app.main import GrpcServer
from app.rest.rest_server import RestServer

from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2_grpc

EXPECTED_GRPC_SERVICE_NAME = (
    "nvidia.omniverse.notifications.publisher.v1beta.EventPublishingService"
)


@pytest.fixture
def server():
    return grpc.aio.server(futures.ThreadPoolExecutor(max_workers=10))


class TestServer:
    class TestAddServicers:
        @pytest.mark.asyncio
        async def test_adds_event_aggregation_servicer_to_server(self, server, mocker):
            spy = mocker.spy(
                event_publisher_pb2_grpc,
                "add_EventPublishingServiceServicer_to_server",
            )
            main.GrpcServer.add_servicers(server)
            spy.assert_called_once_with(AnyInstanceOf(EventAggregationServicer), server)

    class TestGrpcPort:
        @pytest.mark.asyncio
        async def test_returns_port_from_settings(self, server):
            assert main.GrpcServer.grpc_port() == settings.grpc_port

    class TestServiceName:
        @pytest.mark.asyncio
        async def test_returns_service_name(self):
            service_name = main.GrpcServer.service_name()
            expected_service_name = (
                "nvidia.omniverse.notifications."
                "publisher.v1beta.EventPublishingService"
            )
            assert service_name == expected_service_name


@pytest.mark.asyncio
class TestRun:
    async def test_starts_server_with_rest_enabled(self, mocker):
        """Test that REST server is passed when rest_api_enabled is True."""
        spy = mocker.patch.object(start_servers, "start_servers")
        await main.run()
        spy.assert_called_once_with(rabbitmq_connector, GrpcServer, RestServer)

    async def test_starts_server_with_rest_disabled(self, mocker):
        """Test that REST server is not passed when rest_api_enabled is False."""
        mocker.patch.object(settings, "rest_api_enabled", False)
        spy = mocker.patch.object(start_servers, "start_servers")
        await main.run()
        spy.assert_called_once_with(rabbitmq_connector, GrpcServer, None)


def _grpc_endpoint_names(entry):
    """Flatten the gRPC endpoint service names advertised by the entry."""
    return [
        endpoint.service_name
        for grpc_service in entry.grpc_services
        for endpoint in grpc_service.endpoints
    ]


class TestCapabilitiesServiceEntry:
    @pytest.fixture(autouse=True)
    def enable_grpc_capabilities(self, mocker):
        # Pin the flag on so the capabilities gRPC endpoint is advertised
        # regardless of ambient state under random-order runs.
        mocker.patch.object(common_settings, "grpc_capabilities_enabled", True)

    @pytest.fixture
    def entry(self, mocker):
        # The service name and gRPC endpoints do not vary by REST state, so
        # build the entry on the enabled path and assert the REST-invariant
        # facts once here rather than duplicating them in each nested class.
        mocker.patch.object(settings, "rest_api_enabled", True)
        return GrpcServer.capabilities_service_entry()

    def test_service_name_is_notification_aggregation(self, entry):
        assert entry.service_name == "notification-aggregation"

    def test_advertises_application_then_capabilities_grpc_endpoints(self, entry):
        assert _grpc_endpoint_names(entry) == [
            EXPECTED_GRPC_SERVICE_NAME,
            CAPABILITIES_SERVICE_NAME,
        ]

    class TestWhenRestApiEnabled:
        def test_advertises_exactly_one_rest_service(self, entry):
            assert len(entry.rest_services) == 1

        def test_rest_endpoint_name_is_globally_unique_identity(self, entry):
            endpoint = entry.rest_services[0].endpoints[0]
            assert endpoint.name == "notifications-publisher"

        def test_rest_endpoint_advertises_the_pyproject_api_version(self, entry):
            version = entry.rest_services[0].endpoints[0].versions[0]
            assert version.version == "v1beta"

        def test_rest_version_url_detail_path_is_the_version_prefix(self, entry):
            version = entry.rest_services[0].endpoints[0].versions[0]
            # The advertised path is intentionally the version prefix only: the
            # Discovery client appends the resource route (e.g. /events) from the
            # OpenAPI spec, so this path must not carry a resource suffix.
            assert version.url_detail.path == "/api/v1beta"

    class TestWhenRestApiDisabled:
        @pytest.fixture
        def entry(self, mocker):
            mocker.patch.object(settings, "rest_api_enabled", False)
            return GrpcServer.capabilities_service_entry()

        def test_advertises_no_rest_services(self, entry):
            assert list(entry.rest_services) == []
