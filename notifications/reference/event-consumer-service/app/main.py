# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
import os

logger = logging.getLogger(__name__)


async def run():
    """Run the main service."""
    # Import app modules here to avoid loading auth settings in scaler mode
    from notification_common.servers import start_servers

    from app.config import settings
    from app.grpc.grpc_server import GrpcServer
    from app.lib.rabbitmq import rabbitmq_connector
    from app.rest.rest_server import RestServer

    # Importing the singleton registers the channel-pool observable gauges with
    # the meter provider. Nothing else consumes it (its gauges are callback-
    # driven and read pool stats at scrape time), so it must be imported
    # explicitly at service startup, on the service path only (not the scaler).
    import app.lib.observability.channel_pool_metrics  # noqa: F401

    rest_server_class = RestServer if settings.settings.rest_api_enabled else None

    await start_servers.start_servers(rabbitmq_connector, GrpcServer, rest_server_class)


async def run_scaler():
    """Run the autoscaler instead of the main service."""
    from notification_common.scaler import run_autoscaler

    await run_autoscaler()


if __name__ == "__main__":
    mode = os.environ.get("OMNI_EVENTS_MODE", "service").lower()

    if mode == "scaler":
        asyncio.run(run_scaler())
    else:
        asyncio.run(run())
