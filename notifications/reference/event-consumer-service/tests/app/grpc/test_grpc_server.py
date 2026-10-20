# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from concurrent import futures

import grpc
import pytest
from grpc_health.v1 import health_pb2
from notification_common.config.common_settings import common_settings
from notification_common.config.observability_config import CAPABILITIES_SERVICE_NAME
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector
from notification_common.test_tools.any_instance_of import AnyInstanceOf

from app.config import settings as _settings_module
from app.grpc.channel_pool_health_servicer import (
    ChannelPoolHealthServicer,
)
from app.grpc.grpc_server import GrpcServer
from app.lib.rabbitmq import rabbitmq_connector
from nvidia.omniverse.notifications.consumer.v1beta import (
    event_consumer_pb2_grpc,
)
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2_grpc import (
    EventConsumerServiceServicer,
)

# The exact discovery type key and gRPC service full name the discovery operator
# indexes on. Asserting the literals (not just GrpcServer.service_name()) guards
# against a rename silently changing what clients search by.
DISCOVERY_SERVICE_NAME = "notification-consumer"
# This service ships a single OpenAPI spec, so it advertises exactly one
# RestEndpoint (globally-unique name) at the spec's bare base path, per the
# one-endpoint-per-spec discovery capabilities model.
REST_ENDPOINT_NAMES = ["notifications-consumer"]
REST_ENDPOINT_PATHS = ["/api/v1beta"]
EVENT_CONSUMER_SERVICE_FULL_NAME = (
    "nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService"
)


@pytest.fixture
def server():
    return grpc.aio.server(futures.ThreadPoolExecutor(max_workers=10))


class TestAddServicers:
    @pytest.mark.asyncio
    async def test_adds_event_aggregation_servicer_to_server(self, server, mocker):
        spy = mocker.spy(
            event_consumer_pb2_grpc,
            "add_EventConsumerServiceServicer_to_server",
        )
        GrpcServer.add_servicers(server)
        spy.assert_called_once_with(AnyInstanceOf(EventConsumerServiceServicer), server)


class TestGrpcPort:
    @pytest.mark.asyncio
    async def test_returns_port_from_settings(self, server):
        assert GrpcServer.grpc_port() == _settings_module.settings.grpc_port


# A readiness reserve deliberately unlike the production default (80), so a
# factory that dropped the setting and let the servicer fall back to a default
# would land on the wrong side of the verdict below.
PATCHED_READINESS_RESERVE = 37

# Pack more than one client per channel, so the non-durable pool's stream
# capacity is not also its channel count and the reserve can only be read in
# stream units.
NON_DURABLE_CLIENT_LIMIT = 5


def _stats_leaving_available_streams(make_pool_stats_by_pool, available_streams):
    """Build a ``pool_stats_by_pool()`` whose non-durable entry leaves exactly
    ``available_streams`` stream slots free. The pool's stream capacity is read
    back off the factory's own entry rather than restated here, so this stays
    correct if the shared defaults are resized."""
    capacity = make_pool_stats_by_pool(
        non_durable_client_limit=NON_DURABLE_CLIENT_LIMIT
    )[RabbitMQConnector.NON_DURABLE_POOL_NAME]["max_client_capacity"]
    return make_pool_stats_by_pool(
        non_durable_client_limit=NON_DURABLE_CLIENT_LIMIT,
        non_durable_clients=capacity - available_streams,
    )


class TestCreateHealthServicer:
    @pytest.mark.asyncio
    async def test_returns_channel_pool_health_servicer(self):
        servicer = await GrpcServer.create_health_servicer()

        assert isinstance(servicer, ChannelPoolHealthServicer)

    @pytest.mark.parametrize(
        "available_streams, expected_status",
        [
            (PATCHED_READINESS_RESERVE, health_pb2.HealthCheckResponse.SERVING),
            (
                PATCHED_READINESS_RESERVE - 1,
                health_pb2.HealthCheckResponse.NOT_SERVING,
            ),
        ],
    )
    @pytest.mark.asyncio
    async def test_readiness_reserve_setting_reaches_the_servicer(
        self,
        mocker,
        make_pool_stats_by_pool,
        available_streams,
        expected_status,
    ):
        # The factory reads the setting when it constructs the servicer, not
        # per-Check, so the patch has to be in place before the factory runs.
        mocker.patch.object(
            _settings_module.settings,
            "readiness_min_available_channels",
            PATCHED_READINESS_RESERVE,
        )
        # The factory hands the servicer the module-level connector singleton,
        # so the stats have to come from that object.
        mocker.patch.object(
            rabbitmq_connector,
            "pool_stats_by_pool",
            return_value=_stats_leaving_available_streams(
                make_pool_stats_by_pool, available_streams
            ),
        )

        servicer = await GrpcServer.create_health_servicer()
        response = await servicer.Check(None, None)

        assert response.status == expected_status


def _grpc_endpoint_names(entry):
    # There is exactly one GrpcService representing this server; flatten its
    # endpoint service names in advertised order.
    return [
        endpoint.service_name
        for grpc_service in entry.grpc_services
        for endpoint in grpc_service.endpoints
    ]


def _rest_version_paths(entry):
    return [
        version.url_detail.path
        for rest_service in entry.rest_services
        for endpoint in rest_service.endpoints
        for version in endpoint.versions
    ]


class TestCapabilitiesServiceEntry:
    @pytest.fixture(autouse=True)
    def capabilities_enabled(self, mocker):
        """Pin ``grpc_capabilities_enabled`` on so the CapabilitiesService gRPC
        endpoint is advertised deterministically regardless of ambient default
        or ``--random-order`` interleaving."""
        mocker.patch.object(common_settings, "grpc_capabilities_enabled", True)

    @pytest.fixture
    def rest_disabled(self, mocker):
        mocker.patch.object(_settings_module.settings, "rest_api_enabled", False)

    @pytest.fixture
    def rest_enabled(self, mocker):
        mocker.patch.object(_settings_module.settings, "rest_api_enabled", True)

    def test_service_name_is_discovery_type_key(self, rest_disabled):
        entry = GrpcServer.capabilities_service_entry()
        assert entry.service_name == DISCOVERY_SERVICE_NAME

    def test_advertises_event_consumer_and_capabilities_grpc_endpoints(
        self, rest_disabled
    ):
        entry = GrpcServer.capabilities_service_entry()
        assert _grpc_endpoint_names(entry) == [
            EVENT_CONSUMER_SERVICE_FULL_NAME,
            CAPABILITIES_SERVICE_NAME,
        ]

    def test_single_grpc_service_represents_the_server(self, rest_disabled):
        entry = GrpcServer.capabilities_service_entry()
        assert len(entry.grpc_services) == 1

    def test_no_rest_services_when_rest_api_disabled(self, rest_disabled):
        entry = GrpcServer.capabilities_service_entry()
        assert list(entry.rest_services) == []

    def test_advertises_single_rest_service_when_rest_api_enabled(self, rest_enabled):
        entry = GrpcServer.capabilities_service_entry()
        assert len(entry.rest_services) == 1

    def test_advertises_single_rest_endpoint_for_the_spec(self, rest_enabled):
        entry = GrpcServer.capabilities_service_entry()
        names = [ep.name for ep in entry.rest_services[0].endpoints]
        assert names == REST_ENDPOINT_NAMES

    def test_each_rest_endpoint_advertises_v1beta_version(self, rest_enabled):
        entry = GrpcServer.capabilities_service_entry()
        versions = [ep.versions[0].version for ep in entry.rest_services[0].endpoints]
        assert versions == ["v1beta"]

    def test_rest_endpoint_advertises_the_spec_base_path(self, rest_enabled):
        entry = GrpcServer.capabilities_service_entry()
        assert _rest_version_paths(entry) == REST_ENDPOINT_PATHS

    def test_hidden_v1_alias_is_not_advertised(self, rest_enabled):
        # The service serves an unversioned /api/v1 alias, but discovery should
        # only advertise canonical /api/v1beta paths, never the /api/v1 alias.
        entry = GrpcServer.capabilities_service_entry()
        paths = _rest_version_paths(entry)
        assert all(path.startswith("/api/v1beta") for path in paths)
        assert not any(path.startswith("/api/v1/") for path in paths)
