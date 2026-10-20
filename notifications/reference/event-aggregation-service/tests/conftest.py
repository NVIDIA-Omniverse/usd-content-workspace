# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import datetime
import importlib
import json
import os
import time
from contextlib import asynccontextmanager

import aio_pika
import grpc
import pytest
import pytest_asyncio

# noinspection PyUnresolvedReferences
from google.protobuf.timestamp_pb2 import Timestamp
from grpc.aio import insecure_channel
from notification_common.auth.principal_source import PrincipalSource
from notification_common.test_tools.mock_auth_settings import mock_auth_settings

mock_auth_settings()

# mock_auth_settings() is the authenticated module-level baseline: it selects
# principal_source=jwt (a token is required) plus permissions_enabled=true.
# Because permissions_enabled=true, a permissions endpoint must be supplied
# here, before the first Settings() construction at the
# `from app.config import settings` import below, for validation to pass.
# Tokenless loads instead select principal_source=disabled plus
# permissions_enabled=false (see reset_env_vars); the two are mutually
# exclusive on a single AuthSettings load.
os.environ["OMNI_EVENTS_PERMISSIONS_ENDPOINT"] = "test-permissions-endpoint"

# flake8: noqa: E402
from notification_common.interceptors import interceptors
from notification_common.rabbitmq.omni_event import OmniEvent

from app.config import settings as settings_module
from app.config.settings import settings


def reset_env_vars():
    # This is the tokenless reseed: principal_source=disabled means no
    # token is required. The AuthSettings validator rejects
    # permissions_enabled=true together with principal_source=disabled, so
    # permissions stay NOT enabled here. (The authenticated jwt +
    # permissions baseline lives in the module-level mock_auth_settings()
    # call above.) Aggregation is single-gate: there is no
    # storage-permissions path, so no storage env vars or YAML to seed.
    os.environ["OMNI_EVENTS_PRINCIPAL_SOURCE"] = PrincipalSource.DISABLED.value
    os.environ["OMNI_EVENTS_PERMISSIONS_ENABLED"] = "false"

    env_vars_to_del = [
        "OMNI_EVENTS_GRPC_PORT",
        "OMNI_EVENTS_PERMISSIONS_ENDPOINT",
    ]

    for env_var in env_vars_to_del:
        try:
            del os.environ[env_var]
        except KeyError:
            pass

    importlib.reload(settings_module)


from nvidia.omniverse.notifications.publisher.v1beta import event_publisher_pb2_grpc

from nvidia.omniverse.notifications.publisher.v1beta.event_publisher_pb2 import (
    Event,
    EventResource,
)


@pytest.fixture
def message_of_size():
    """Return a builder for a message whose JSON encoding is exactly ``size`` bytes.

    The message is ``{"k": "<padding>"}``, measured as the message-size limit
    measures it (``json.dumps`` with its defaults).
    """
    overhead = len(json.dumps({"k": ""}))

    def _build(size: int) -> dict:
        return {"k": "x" * (size - overhead)}

    return _build


def make_timestamp():
    ts = Timestamp()
    ts.FromSeconds(int(time.time()))
    return ts


@pytest.fixture
def mocked_omni_event(freezer):
    freezer.move_to("2024-10-23 00:00:00")
    occurred_at = datetime.datetime.now(datetime.UTC).isoformat().replace("+00:00", "Z")

    return OmniEvent(
        event_type="test-event",
        message={"body": "test message"},
        occurred_at=occurred_at,
        resource_id="test-resource-id",
    )


@pytest.fixture
def mocked_event(freezer):
    freezer.move_to("2024-10-23")

    return Event(
        event_type="test-event",
        resource=EventResource(resource_id="test-resource-id"),
        message={"body": "test message"},
        occurred_at=make_timestamp(),
    )


@pytest_asyncio.fixture(autouse=True, scope="function")
async def before_each(mocked_omni_event, mocker):
    mocker.patch.object(aio_pika, "connect_robust", autospec=True)


@pytest.fixture
def grpc_host():
    if settings.app_env == "test":
        return "host.docker.internal"

    return "localhost"


@pytest_asyncio.fixture
async def grpc_server():
    from app.grpc.event_aggregation_servicer import EventAggregationServicer

    server = grpc.aio.server(interceptors=await interceptors.get_interceptors())

    event_publisher_pb2_grpc.add_EventPublishingServiceServicer_to_server(
        EventAggregationServicer(), server
    )

    server.add_insecure_port("localhost:50055")

    await server.start()
    yield server
    await server.stop(None)


@asynccontextmanager
async def channel(grpc_host):
    async with insecure_channel(f"{grpc_host}:50055") as c:
        yield c


@pytest_asyncio.fixture
async def event_aggregation_stub(grpc_server, grpc_host):
    async with channel(grpc_host) as c:
        yield event_publisher_pb2_grpc.EventPublishingServiceStub(c)
