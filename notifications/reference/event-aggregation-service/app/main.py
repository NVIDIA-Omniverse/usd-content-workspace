# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import tomllib
from pathlib import Path

from notification_common.grpc.notification_server import NotificationServer
from notification_common.servers import start_servers

from app.config.settings import settings
from app.grpc.event_aggregation_servicer import EventAggregationServicer
from app.lib.rabbitmq import rabbitmq_connector

from nvidia.omniverse.discovery.capabilities.v2alpha import capabilities_pb2
from nvidia.omniverse.notifications.publisher.v1beta import (
    event_publisher_pb2_grpc,
    event_publisher_pb2,
)


class GrpcServer(NotificationServer):
    @classmethod
    def grpc_port(cls) -> str:
        return settings.grpc_port

    @classmethod
    def add_servicers(cls, server) -> None:
        event_publisher_pb2_grpc.add_EventPublishingServiceServicer_to_server(
            EventAggregationServicer(), server
        )

    @classmethod
    def service_name(cls) -> str:
        return event_publisher_pb2.DESCRIPTOR.services_by_name[
            "EventPublishingService"
        ].full_name

    @classmethod
    def capabilities_service_entry(cls) -> capabilities_pb2.ServiceEntry:
        rest_services = []
        if settings.rest_api_enabled:
            api_version = tomllib.loads(Path("pyproject.toml").read_text())["tool"][
                "app"
            ]["api_version"]
            rest_services.append(
                capabilities_pb2.RestService(
                    endpoints=[
                        capabilities_pb2.RestEndpoint(
                            # RestEndpoint.name is a globally-unique identity
                            # across the Discovery namespace, so it names this
                            # publisher API rather than a bare resource. The path
                            # is the version prefix only because the client
                            # appends the resource route from the OpenAPI spec.
                            name="notifications-publisher",
                            versions=[
                                capabilities_pb2.RestVersion(
                                    version=api_version,
                                    url_detail=capabilities_pb2.RestVersionDetail(
                                        path=f"/api/{api_version}"
                                    ),
                                )
                            ],
                        )
                    ]
                )
            )

        return cls.build_capabilities_entry(
            service_name="notification-aggregation",
            rest_services=rest_services,
        )


async def run() -> None:
    from app.rest.rest_server import RestServer

    rest_server_class = RestServer if settings.rest_api_enabled else None

    await start_servers.start_servers(rabbitmq_connector, GrpcServer, rest_server_class)


if __name__ == "__main__":
    asyncio.run(run())
