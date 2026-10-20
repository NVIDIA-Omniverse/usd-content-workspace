# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
from abc import ABC, abstractmethod
from collections.abc import Iterable
from concurrent import futures

import grpc
from grpc_health.v1.health import aio as health
from grpc_health.v1 import health_pb2
from grpc_health.v1.health_pb2 import HealthCheckResponse
from grpc_health.v1.health_pb2_grpc import add_HealthServicer_to_server

from grpc_reflection.v1alpha import reflection

from nvidia.omniverse.discovery.capabilities.v2alpha import (
    capabilities_pb2,
    capabilities_service_pb2_grpc,
)

from notification_common.config.common_settings import common_settings
from notification_common.config.metadata import project_name
from notification_common.config.observability_config import CAPABILITIES_SERVICE_NAME
from notification_common.grpc.capabilities_servicer import CapabilitiesServicer
from notification_common.interceptors import interceptors
from notification_common.servers.server_base import ServerBase

logger = logging.getLogger(__name__)


LIVENESS_SERVICE = "liveness"


class _LivenessAwareHealthServicer(health.HealthServicer):
    """
    Wraps a delegate health servicer so that checks for the
    ``"liveness"`` service always return SERVING (as long as
    the server can respond at all).

    All other service names are forwarded to the delegate.
    This lets Kubernetes liveness probes pass even when the
    delegate reports NOT_SERVING due to capacity limits,
    preventing pods from being killed while still actively
    serving clients.
    """

    def __init__(self, delegate: health.HealthServicer):
        super().__init__()
        self._delegate = delegate

    async def Check(self, request, context):
        if request.service == LIVENESS_SERVICE:
            return HealthCheckResponse(
                status=HealthCheckResponse.SERVING,
            )
        return await self._delegate.Check(request, context)

    async def Watch(self, request, context):
        return await self._delegate.Watch(request, context)


class NotificationServer(ServerBase, ABC):
    @classmethod
    @abstractmethod
    def add_servicers(cls, server: grpc.aio.Server) -> None:
        pass

    @classmethod
    @abstractmethod
    def service_name(cls) -> str:
        pass

    @classmethod
    @abstractmethod
    def grpc_port(cls) -> str:
        pass

    @classmethod
    def capabilities_service_entry(cls) -> capabilities_pb2.ServiceEntry | None:
        """Return the ``ServiceEntry`` advertised via the Discovery
        CapabilitiesService, or ``None`` to disable capabilities.

        Subclasses override this to opt in to discovery by returning a
        static entry describing their gRPC and REST APIs. The default of
        ``None`` keeps the capabilities service off, so existing servers
        are unaffected.

        Subclasses may hand-author the full ``ServiceEntry`` here, or call
        :meth:`build_capabilities_entry` to auto-derive the gRPC portion
        from this server's own exposed gRPC service name(s).
        """
        return None

    @classmethod
    def build_capabilities_entry(
        cls,
        *,
        service_name: str,
        rest_services: Iterable[capabilities_pb2.RestService] = (),
    ) -> capabilities_pb2.ServiceEntry:
        """Build a ``ServiceEntry`` whose gRPC portion is auto-derived from
        this server's own exposed gRPC service name(s).

        This is an optional convenience for ``capabilities_service_entry``
        overrides so a subclass supplies only the discovery type key and its
        REST resources, rather than hand-listing every gRPC service name.

        :param service_name: The discovery type key advertised as
            ``ServiceEntry.service_name`` (e.g. ``"notification-consumer"``).
            The library stays value-agnostic; the caller supplies it.
        :param rest_services: REST transport details, passed through to
            ``ServiceEntry.rest_services`` unchanged.
        :returns: A fully-populated ``ServiceEntry``.
        """
        # The exposed application gRPC service, plus the capabilities service
        # itself so the discovery operator sees it too. These are derived
        # DIRECTLY from cls.service_name() / CAPABILITIES_SERVICE_NAME rather
        # than from the reflection list in _configure_reflection, so building
        # an entry never silently depends on reflection being enabled. The
        # reflection/health meta-services are infrastructure, not advertised
        # application capabilities, so they are deliberately omitted.
        grpc_service_names = [cls.service_name()]
        # Guard on the raw grpc_capabilities_enabled flag, NOT on
        # _capabilities_active(): that predicate calls
        # capabilities_service_entry(), which is exactly the override this
        # helper is invoked from, so calling it here would recurse forever.
        # In this context the entry is non-None by construction, so
        # _capabilities_active() reduces to the flag alone.
        if common_settings.grpc_capabilities_enabled:
            grpc_service_names.append(CAPABILITIES_SERVICE_NAME)

        # One GrpcService represents this single gRPC server; its endpoints
        # carry one GrpcEndpoint per exposed gRPC service, matching the proto
        # comment "A gRPC server entry listing the gRPC services it exposes."
        # Ordering is deterministic (application service, then capabilities).
        grpc_service = capabilities_pb2.GrpcService(
            endpoints=[
                capabilities_pb2.GrpcEndpoint(service_name=name)
                for name in grpc_service_names
            ]
        )

        return capabilities_pb2.ServiceEntry(
            service_name=service_name,
            grpc_services=[grpc_service],
            rest_services=list(rest_services),
        )

    @classmethod
    def _capabilities_active(cls) -> bool:
        """Whether the Discovery CapabilitiesService should be served.

        A single predicate shared by registration, reflection, and health
        so the three sites cannot disagree (registered-but-not-reflected,
        health-marked-but-not-registered, etc.).
        """
        return (
            common_settings.grpc_capabilities_enabled
            and cls.capabilities_service_entry() is not None
        )

    @classmethod
    async def start_server(cls, shutdown_event: asyncio.Event) -> None:
        grpc_port = cls.grpc_port()

        server = await cls._make_server(grpc_port)
        await server.start()

        logger.info(f"{project_name()} - gRPC server started on port {grpc_port}")

        await shutdown_event.wait()

        # stop server with a grace period
        await cls._shutdown_server(server)

        logger.info("gRPC server shutdown complete")

    @classmethod
    async def create_health_servicer(cls) -> health.HealthServicer:
        """Override this method to provide a custom health servicer."""
        health_servicer = health.HealthServicer()
        await health_servicer.set("", HealthCheckResponse.SERVING)
        return health_servicer

    @classmethod
    async def _configure_health(cls, server: grpc.aio.Server):
        if not common_settings.grpc_health_checking_enabled:
            return
        delegate = await cls.create_health_servicer()
        if cls._capabilities_active():
            await delegate.set(CAPABILITIES_SERVICE_NAME, HealthCheckResponse.SERVING)
        servicer = _LivenessAwareHealthServicer(delegate)
        add_HealthServicer_to_server(servicer, server)

    @classmethod
    def _configure_reflection(cls, server: grpc.aio.Server):
        if not common_settings.grpc_reflection_enabled:
            return
        svcs = [reflection.SERVICE_NAME, cls.service_name()]
        if common_settings.grpc_health_checking_enabled:
            svcs.append(health_pb2.DESCRIPTOR.services_by_name["Health"].full_name)
        if cls._capabilities_active():
            svcs.append(CAPABILITIES_SERVICE_NAME)
        reflection.enable_server_reflection(svcs, server)

    @classmethod
    def _configure_capabilities(cls, server: grpc.aio.Server):
        if not cls._capabilities_active():
            return
        servicer = CapabilitiesServicer(cls.capabilities_service_entry())
        capabilities_service_pb2_grpc.add_CapabilitiesServiceServicer_to_server(
            servicer, server
        )

    @classmethod
    async def _make_server(cls, grpc_port: str) -> grpc.aio.Server:
        options = [
            ("grpc.keepalive_time_ms", common_settings.grpc_keepalive_time_ms),
            ("grpc.keepalive_timeout_ms", common_settings.grpc_keepalive_timeout_ms),
            (
                "grpc.keepalive_permit_without_calls",
                1 if common_settings.grpc_keepalive_permit_without_calls else 0,
            ),
        ]

        server = grpc.aio.server(
            futures.ThreadPoolExecutor(max_workers=10),
            interceptors=await interceptors.get_interceptors(),
            options=options,
        )

        cls.add_servicers(server)
        cls._configure_capabilities(server)
        await cls._configure_health(server)
        cls._configure_reflection(server)
        server.add_insecure_port(grpc_port)

        return server

    @classmethod
    async def _shutdown_server(cls, server) -> None:
        logger.info("gRPC server shutting down with five second grace period...")
        await server.stop(grace=5)  # Graceful shutdown with a 5-second timeout
