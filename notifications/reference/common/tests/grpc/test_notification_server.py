# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from concurrent import futures
from concurrent.futures import ThreadPoolExecutor
from unittest.mock import AsyncMock

import grpc
import pytest
from grpc.aio import Server
from grpc_health.v1.health import aio as health
from grpc_reflection.v1alpha import reflection

from grpc_health.v1 import health_pb2

from nvidia.omniverse.discovery.capabilities.v2alpha import capabilities_pb2

from notification_common.config.common_settings import common_settings
from notification_common.config.observability_config import CAPABILITIES_SERVICE_NAME
from notification_common.grpc import notification_server
from notification_common.grpc.capabilities_servicer import CapabilitiesServicer
from notification_common.grpc.notification_server import (
    LIVENESS_SERVICE,
    NotificationServer,
    _LivenessAwareHealthServicer,
)
from notification_common.interceptors import interceptors
from notification_common.test_tools.any_instance_of import AnyInstanceOf

# (entry present, grpc_capabilities_enabled) -> whether capabilities is active.
# The service is served only when a ServiceEntry is configured AND the flag is on.
CAPABILITIES_ACTIVATION_CASES = [
    pytest.param(True, True, True, id="entry_and_flag_on"),
    pytest.param(True, False, False, id="entry_present_flag_off"),
    pytest.param(False, True, False, id="no_entry_flag_on"),
    pytest.param(False, False, False, id="no_entry_flag_off"),
]


@pytest.fixture
def server():
    return grpc.aio.server(futures.ThreadPoolExecutor(max_workers=10))


@pytest.fixture
def service_name():
    return "test-service"


@pytest.fixture(autouse=True, scope="function")
def mock_grpc_server(server, service_name, mocker):
    mocker.patch.object(NotificationServer, "add_servicers")
    mocker.patch.object(NotificationServer, "service_name", return_value=service_name)
    mocker.patch.object(grpc.aio, "server", return_value=server)
    # noinspection PyProtectedMember
    mocker.patch.object(server, "add_insecure_port")
    mocker.patch.object(server, "start")
    # noinspection PyProtectedMember
    mocker.patch.object(server, "wait_for_termination")


@pytest.fixture
def grpc_port():
    return "test-port"


@pytest.fixture
def mock_interceptors_list(mocker):
    return [mocker.Mock(spec=grpc.aio.ServerInterceptor) for _ in range(3)]


@pytest.fixture(autouse=True)
def mock_get_interceptors(mocker, mock_interceptors_list):
    return mocker.patch.object(
        interceptors, "get_interceptors", return_value=mock_interceptors_list
    )


@pytest.fixture
def capabilities_entry():
    return capabilities_pb2.ServiceEntry(service_name="test-capabilities")


@pytest.fixture
def configure_capabilities(mocker, capabilities_entry):
    """Configure the two inputs to ``_capabilities_active``: the presence of a
    ``ServiceEntry`` and the ``grpc_capabilities_enabled`` flag."""

    def _configure(has_entry, flag_enabled):
        entry = capabilities_entry if has_entry else None
        mocker.patch.object(
            NotificationServer, "capabilities_service_entry", return_value=entry
        )
        mocker.patch.object(common_settings, "grpc_capabilities_enabled", flag_enabled)

    return _configure


class TestStartServer:

    @pytest.mark.asyncio
    async def test_calls_make_server_with_grpc_port_and_interceptors(
        self, server, grpc_port, mocker, mock_interceptors_list
    ):
        spy = mocker.patch.object(NotificationServer, "_make_server")
        mocker.patch.object(NotificationServer, "grpc_port", return_value=grpc_port)
        await NotificationServer.start_server(AsyncMock())
        spy.assert_called_once_with(grpc_port)

    @pytest.mark.asyncio
    async def test_starts_server(self, server, mocker):
        spy = mocker.spy(server, "start")
        await NotificationServer.start_server(AsyncMock())
        spy.assert_called_once_with()

    @pytest.mark.asyncio
    async def test_calls_wait_on_the_asyncio_event(self, server, mocker):
        event = AsyncMock()
        await NotificationServer.start_server(event)
        event.wait.assert_called_once()

    @pytest.mark.asyncio
    async def test_shuts_down_server_when_given_signal(self, server, mocker):
        spy = mocker.spy(NotificationServer, "_shutdown_server")
        await NotificationServer.start_server(AsyncMock())
        spy.assert_called_once_with(server)


class TestMakeServer:
    @pytest.fixture
    def mock_interceptors(self, mocker):
        value = "test-interceptors"
        mocker.patch.object(interceptors, "get_interceptors", return_value=value)
        return value

    @pytest.mark.asyncio
    async def test_makes_aio_server_with_interceptors_and_keepalive_options(
        self, grpc_port, mock_interceptors, mocker
    ):
        spy = mocker.spy(grpc.aio, "server")
        await NotificationServer._make_server(grpc_port)
        expected_options = [
            ("grpc.keepalive_time_ms", common_settings.grpc_keepalive_time_ms),
            (
                "grpc.keepalive_timeout_ms",
                common_settings.grpc_keepalive_timeout_ms,
            ),
            (
                "grpc.keepalive_permit_without_calls",
                1 if common_settings.grpc_keepalive_permit_without_calls else 0,
            ),
        ]
        spy.assert_called_once_with(
            AnyInstanceOf(ThreadPoolExecutor),
            interceptors=mock_interceptors,
            options=expected_options,
        )

    @pytest.mark.asyncio
    async def test_returns_new_aio_server(self, grpc_port):
        assert isinstance(await NotificationServer._make_server(grpc_port), Server)

    @pytest.mark.asyncio
    async def test_adds_servicers_to_server(self, server, grpc_port, mocker):
        spy = mocker.spy(NotificationServer, "add_servicers")
        await NotificationServer._make_server(grpc_port)
        spy.assert_called_once_with(server)

    @pytest.mark.asyncio
    async def test_configures_capabilities(self, server, grpc_port, mocker):
        spy = mocker.spy(NotificationServer, "_configure_capabilities")
        await NotificationServer._make_server(grpc_port)
        spy.assert_called_once_with(server)

    @pytest.mark.asyncio
    async def test_configures_health_servicer(self, server, grpc_port, mocker):
        spy = mocker.spy(NotificationServer, "_configure_health")
        await NotificationServer._make_server(grpc_port)
        spy.assert_called_once_with(server)

    @pytest.mark.asyncio
    async def test_configures_reflection(self, server, grpc_port, mocker):
        spy = mocker.spy(NotificationServer, "_configure_reflection")
        await NotificationServer._make_server(grpc_port)
        spy.assert_called_once_with(server)

    @pytest.mark.asyncio
    async def test_sets_up_server(self, server, grpc_port, mocker):
        spy = mocker.spy(server, "add_insecure_port")
        await NotificationServer._make_server(grpc_port)
        spy.assert_called_once_with(grpc_port)


class TestCreateHealthServicer:
    @pytest.mark.asyncio
    async def test_returns_health_servicer(self):
        result = await NotificationServer.create_health_servicer()
        assert isinstance(result, health.HealthServicer)

    @pytest.mark.asyncio
    async def test_sets_serving_status(self, mocker):
        mock_servicer = mocker.AsyncMock(spec=health.HealthServicer)
        mocker.patch.object(health, "HealthServicer", return_value=mock_servicer)
        await NotificationServer.create_health_servicer()
        mock_servicer.set.assert_called_once_with(
            "", notification_server.HealthCheckResponse.SERVING
        )


class TestConfigureHealth:
    @pytest.mark.asyncio
    async def test_wraps_delegate_in_liveness_aware_servicer(self, server, mocker):
        mock_servicer = mocker.AsyncMock(spec=health.HealthServicer)
        mocker.patch.object(
            NotificationServer, "create_health_servicer", return_value=mock_servicer
        )
        spy = mocker.patch.object(notification_server, "add_HealthServicer_to_server")
        await NotificationServer._configure_health(server)
        registered = spy.call_args[0][0]
        assert isinstance(registered, _LivenessAwareHealthServicer)

    @pytest.mark.asyncio
    async def test_calls_create_health_servicer(self, server, mocker):
        spy = mocker.patch.object(
            NotificationServer,
            "create_health_servicer",
            return_value=mocker.AsyncMock(),
        )
        await NotificationServer._configure_health(server)
        spy.assert_called_once()

    @pytest.mark.asyncio
    async def test_does_not_add_health_servicer_if_health_checking_disabled(
        self, server, mocker
    ):
        spy = mocker.spy(notification_server, "add_HealthServicer_to_server")
        mocker.patch.object(common_settings, "grpc_health_checking_enabled", False)
        await NotificationServer._configure_health(server)
        spy.assert_not_called()

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "has_entry,flag_enabled,active", CAPABILITIES_ACTIVATION_CASES
    )
    async def test_marks_capabilities_serving_only_when_active(
        self, server, mocker, configure_capabilities, has_entry, flag_enabled, active
    ):
        delegate = mocker.AsyncMock(spec=health.HealthServicer)
        mocker.patch.object(
            NotificationServer, "create_health_servicer", return_value=delegate
        )
        configure_capabilities(has_entry, flag_enabled)
        await NotificationServer._configure_health(server)
        marked_capabilities = any(
            call.args[0] == CAPABILITIES_SERVICE_NAME
            for call in delegate.set.call_args_list
        )
        assert marked_capabilities is active

    @pytest.mark.asyncio
    async def test_marks_capabilities_serving_with_serving_status_when_active(
        self, server, mocker, configure_capabilities
    ):
        delegate = mocker.AsyncMock(spec=health.HealthServicer)
        mocker.patch.object(
            NotificationServer, "create_health_servicer", return_value=delegate
        )
        configure_capabilities(has_entry=True, flag_enabled=True)
        await NotificationServer._configure_health(server)
        delegate.set.assert_called_once_with(
            CAPABILITIES_SERVICE_NAME, notification_server.HealthCheckResponse.SERVING
        )


class TestLivenessAwareHealthServicer:
    @pytest.fixture
    def delegate(self, mocker):
        return mocker.AsyncMock(spec=health.HealthServicer)

    @pytest.fixture
    def servicer(self, delegate):
        return _LivenessAwareHealthServicer(delegate)

    @pytest.mark.asyncio
    async def test_liveness_check_returns_serving(self, servicer):
        request = health_pb2.HealthCheckRequest(
            service=LIVENESS_SERVICE,
        )
        response = await servicer.Check(request, None)
        assert response.status == health_pb2.HealthCheckResponse.SERVING

    @pytest.mark.asyncio
    async def test_liveness_check_does_not_call_delegate(self, servicer, delegate):
        request = health_pb2.HealthCheckRequest(
            service=LIVENESS_SERVICE,
        )
        await servicer.Check(request, None)
        delegate.Check.assert_not_called()

    @pytest.mark.asyncio
    async def test_default_check_delegates(self, servicer, delegate):
        request = health_pb2.HealthCheckRequest(service="")
        context = None
        await servicer.Check(request, context)
        delegate.Check.assert_called_once_with(request, context)

    @pytest.mark.asyncio
    async def test_named_service_check_delegates(self, servicer, delegate):
        request = health_pb2.HealthCheckRequest(
            service="some.other.Service",
        )
        await servicer.Check(request, None)
        delegate.Check.assert_called_once_with(request, None)

    @pytest.mark.asyncio
    async def test_watch_delegates(self, servicer, delegate):
        request = health_pb2.HealthCheckRequest(service="")
        context = None
        await servicer.Watch(request, context)
        delegate.Watch.assert_called_once_with(request, context)


class TestConfigureReflection:
    def test_adds_all_service_names_for_reflection(self, server, mocker, service_name):
        spy = mocker.spy(reflection, "enable_server_reflection")
        NotificationServer._configure_reflection(server)
        expected_services = [
            reflection.SERVICE_NAME,
            service_name,
            "grpc.health.v1.Health",
        ]
        spy.assert_called_once_with(expected_services, server)

    def test_does_not_add_health_service_if_health_checking_disabled(
        self, server, mocker, service_name
    ):
        spy = mocker.spy(reflection, "enable_server_reflection")
        mocker.patch.object(common_settings, "grpc_health_checking_enabled", False)
        NotificationServer._configure_reflection(server)
        expected_services = [
            reflection.SERVICE_NAME,
            service_name,
        ]
        spy.assert_called_once_with(expected_services, server)

    def test_does_not_add_reflection_if_reflection_disabled(self, server, mocker):
        spy = mocker.spy(reflection, "enable_server_reflection")
        mocker.patch.object(common_settings, "grpc_reflection_enabled", False)
        NotificationServer._configure_reflection(server)
        spy.assert_not_called()

    @pytest.mark.parametrize(
        "has_entry,flag_enabled,active", CAPABILITIES_ACTIVATION_CASES
    )
    def test_reflects_capabilities_service_only_when_active(
        self, server, mocker, configure_capabilities, has_entry, flag_enabled, active
    ):
        configure_capabilities(has_entry, flag_enabled)
        spy = mocker.spy(reflection, "enable_server_reflection")
        NotificationServer._configure_reflection(server)
        reflected_services = spy.call_args[0][0]
        assert (CAPABILITIES_SERVICE_NAME in reflected_services) is active


class TestCapabilitiesServiceEntry:
    def test_defaults_to_none(self):
        # Opting out of discovery is the default; subclasses override to opt in.
        assert NotificationServer.capabilities_service_entry() is None


class TestBuildCapabilitiesEntry:
    DISCOVERY_KEY = "notification-consumer"

    @pytest.fixture
    def capabilities_enabled(self, mocker):
        """Toggle ``grpc_capabilities_enabled`` explicitly so the outcome does
        not depend on the ambient default under ``--random-order``."""

        def _set(enabled):
            mocker.patch.object(common_settings, "grpc_capabilities_enabled", enabled)

        return _set

    @staticmethod
    def _endpoint_names(entry):
        # There is exactly one GrpcService; flatten its endpoint service names.
        return [
            endpoint.service_name
            for grpc_service in entry.grpc_services
            for endpoint in grpc_service.endpoints
        ]

    def test_service_name_is_the_supplied_discovery_key(self, capabilities_enabled):
        capabilities_enabled(True)
        entry = NotificationServer.build_capabilities_entry(
            service_name=self.DISCOVERY_KEY
        )
        assert entry.service_name == self.DISCOVERY_KEY

    def test_only_application_service_remains_when_flag_disabled(
        self, capabilities_enabled, service_name
    ):
        capabilities_enabled(False)
        entry = NotificationServer.build_capabilities_entry(
            service_name=self.DISCOVERY_KEY
        )
        assert self._endpoint_names(entry) == [service_name]

    def test_deterministic_ordering_application_then_capabilities(
        self, capabilities_enabled, service_name
    ):
        capabilities_enabled(True)
        entry = NotificationServer.build_capabilities_entry(
            service_name=self.DISCOVERY_KEY
        )
        assert self._endpoint_names(entry) == [
            service_name,
            CAPABILITIES_SERVICE_NAME,
        ]

    def test_single_grpc_service_represents_the_server(self, capabilities_enabled):
        capabilities_enabled(True)
        entry = NotificationServer.build_capabilities_entry(
            service_name=self.DISCOVERY_KEY
        )
        assert len(entry.grpc_services) == 1

    def test_rest_services_are_passed_through(self, capabilities_enabled):
        capabilities_enabled(True)
        rest_service = capabilities_pb2.RestService(
            endpoints=[capabilities_pb2.RestEndpoint(name="events")]
        )
        entry = NotificationServer.build_capabilities_entry(
            service_name=self.DISCOVERY_KEY, rest_services=[rest_service]
        )
        assert list(entry.rest_services) == [rest_service]

    def test_rest_services_default_to_empty(self, capabilities_enabled):
        capabilities_enabled(True)
        entry = NotificationServer.build_capabilities_entry(
            service_name=self.DISCOVERY_KEY
        )
        assert list(entry.rest_services) == []

    @pytest.mark.parametrize("reflection_enabled", [True, False])
    def test_derivation_is_independent_of_reflection_flag(
        self, capabilities_enabled, mocker, reflection_enabled, service_name
    ):
        capabilities_enabled(True)
        mocker.patch.object(
            common_settings, "grpc_reflection_enabled", reflection_enabled
        )
        entry = NotificationServer.build_capabilities_entry(
            service_name=self.DISCOVERY_KEY
        )
        assert self._endpoint_names(entry) == [
            service_name,
            CAPABILITIES_SERVICE_NAME,
        ]

    def test_reflection_meta_service_name_not_included(self, capabilities_enabled):
        capabilities_enabled(True)
        entry = NotificationServer.build_capabilities_entry(
            service_name=self.DISCOVERY_KEY
        )
        assert reflection.SERVICE_NAME not in self._endpoint_names(entry)

    def test_health_meta_service_name_not_included(self, capabilities_enabled):
        capabilities_enabled(True)
        entry = NotificationServer.build_capabilities_entry(
            service_name=self.DISCOVERY_KEY
        )
        health_name = health_pb2.DESCRIPTOR.services_by_name["Health"].full_name
        assert health_name not in self._endpoint_names(entry)


class TestCapabilitiesActive:
    @pytest.mark.parametrize(
        "has_entry,flag_enabled,active", CAPABILITIES_ACTIVATION_CASES
    )
    def test_is_active_only_when_entry_present_and_flag_enabled(
        self, mocker, configure_capabilities, has_entry, flag_enabled, active
    ):
        configure_capabilities(has_entry, flag_enabled)
        assert NotificationServer._capabilities_active() is active


class TestConfigureCapabilities:
    @pytest.fixture
    def spy_add_servicer(self, mocker):
        return mocker.patch.object(
            notification_server.capabilities_service_pb2_grpc,
            "add_CapabilitiesServiceServicer_to_server",
        )

    @pytest.mark.parametrize(
        "has_entry,flag_enabled,active", CAPABILITIES_ACTIVATION_CASES
    )
    def test_registers_servicer_only_when_active(
        self,
        server,
        spy_add_servicer,
        configure_capabilities,
        has_entry,
        flag_enabled,
        active,
    ):
        configure_capabilities(has_entry, flag_enabled)
        NotificationServer._configure_capabilities(server)
        assert spy_add_servicer.called is active

    def test_registers_capabilities_servicer_when_active(
        self, server, spy_add_servicer, configure_capabilities
    ):
        configure_capabilities(has_entry=True, flag_enabled=True)
        NotificationServer._configure_capabilities(server)
        registered = spy_add_servicer.call_args[0][0]
        assert isinstance(registered, CapabilitiesServicer)

    def test_registered_servicer_carries_the_configured_entry(
        self, server, spy_add_servicer, configure_capabilities, capabilities_entry
    ):
        configure_capabilities(has_entry=True, flag_enabled=True)
        NotificationServer._configure_capabilities(server)
        registered = spy_add_servicer.call_args[0][0]
        assert registered._service_entry is capabilities_entry

    def test_registers_servicer_on_the_given_server_when_active(
        self, server, spy_add_servicer, configure_capabilities
    ):
        configure_capabilities(has_entry=True, flag_enabled=True)
        NotificationServer._configure_capabilities(server)
        assert spy_add_servicer.call_args[0][1] is server


class TestShutdownServer:
    @pytest.mark.asyncio
    async def test_stops_server_with_five_second_grace_period(self, server, mocker):
        spy = mocker.patch.object(server, "stop")
        await NotificationServer._shutdown_server(server)
        spy.assert_called_once_with(grace=5)
