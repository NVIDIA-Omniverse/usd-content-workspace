# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import grpc
from grpc_health.v1.health import aio as health
from notification_common.grpc.notification_server import NotificationServer

from app.config import settings
from app.config.app_metadata import api_path_prefix, api_version
from app.grpc.channel_pool_health_servicer import (
    ChannelPoolHealthServicer,
)
from app.grpc.event_consumer_servicer import EventConsumerServicer
from app.lib.rabbitmq import rabbitmq_connector
from nvidia.omniverse.discovery.capabilities.v2alpha import capabilities_pb2
from nvidia.omniverse.notifications.consumer.v1beta import (
    event_consumer_pb2,
    event_consumer_pb2_grpc,
)

# The discovery "type" key clients search by; distinct from the registry "id"
# (event-consumer-service). Matches the static registry entry this endpoint
# replaces.
_DISCOVERY_SERVICE_NAME = "notification-consumer"

# The advertised RestEndpoint.name for this service's single REST API. Kept in
# one place, mirroring _DISCOVERY_SERVICE_NAME. The discovery client requires
# REST endpoint names to be globally unique across all services (it flattens the
# operator's per-ServiceEntry namespacing), so this is a distinct explicit name.
# Note: this is intentionally plural ("notifications-consumer") and differs by
# one letter from the singular _DISCOVERY_SERVICE_NAME ("notification-consumer")
# discovery type key — they are separate surfaces; do not reconcile them.
_REST_ENDPOINT_NAME = "notifications-consumer"


class GrpcServer(NotificationServer):
    @classmethod
    def grpc_port(cls) -> str:
        return settings.settings.grpc_port

    @classmethod
    def add_servicers(cls, server: grpc.aio.Server) -> None:
        event_consumer_pb2_grpc.add_EventConsumerServiceServicer_to_server(
            EventConsumerServicer(), server
        )

    @classmethod
    def service_name(cls) -> str:
        return event_consumer_pb2.DESCRIPTOR.services_by_name[
            "EventConsumerService"
        ].full_name

    @classmethod
    def capabilities_service_entry(cls) -> capabilities_pb2.ServiceEntry:
        return cls.build_capabilities_entry(
            service_name=_DISCOVERY_SERVICE_NAME,
            rest_services=cls._rest_services(),
        )

    @classmethod
    def _rest_services(cls) -> list[capabilities_pb2.RestService]:
        """Advertise the REST API only when this deployment actually serves it,
        so each deployment's advertisement matches what it exposes. This service
        ships a single OpenAPI spec, so it advertises exactly one RestEndpoint at
        the spec's base path (api_path_prefix(), e.g. /api/v1beta), per the
        one-endpoint-per-spec discovery capabilities model."""
        if not settings.settings.rest_api_enabled:
            return []
        return [
            capabilities_pb2.RestService(
                endpoints=[
                    capabilities_pb2.RestEndpoint(
                        name=_REST_ENDPOINT_NAME,
                        versions=[
                            capabilities_pb2.RestVersion(
                                version=api_version(),
                                url_detail=capabilities_pb2.RestVersionDetail(
                                    path=api_path_prefix()
                                ),
                            )
                        ],
                    )
                ]
            )
        ]

    @classmethod
    async def create_health_servicer(cls) -> health.HealthServicer:
        return ChannelPoolHealthServicer(
            rabbitmq_connector,
            settings.settings.readiness_min_available_channels,
        )
