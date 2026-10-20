# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import importlib
import os
import time
from asyncio import Future
from contextlib import asynccontextmanager
from pathlib import Path
from unittest.mock import MagicMock

import aio_pika
import grpc
import pytest
import pytest_asyncio

# noinspection PyUnresolvedReferences
from google.protobuf.timestamp_pb2 import Timestamp
from grpc.aio import insecure_channel
from notification_common.auth.principal_source import PrincipalSource
from notification_common.errors.errors import ChannelNotAvailableError
from notification_common.test_tools.mock_auth_settings import mock_auth_settings

from tests.fixtures.cache_scope import (
    CACHE_SCOPE,
    SECOND_CACHE_SCOPE,
    TEST_PRINCIPAL_REF,
)
from tests.fixtures.mock_filter_group import filter_group

# The Settings validator now enforces two independent per-backend gates:
#   - permissions_enabled=true requires permissions_endpoint specifically
#     (the permission-service gate).
#   - storage_permissions_enabled=true requires storage_permissions_endpoints
#     (the storage gate).
# These no longer share OR-semantics, so a YAML-provided
# storage_permissions_endpoints does NOT satisfy the permissions_enabled gate.
#
# mock_auth_settings() is the authenticated module-level baseline: it selects
# principal_source=jwt (a token is required) plus permissions_enabled=true.
# Because permissions_enabled=true, we must supply permissions_endpoint here,
# before the first Settings() construction at the `from app.config import
# settings` import below. The storage gate stays at its default
# (storage_permissions_enabled=false), so the YAML-provided storage endpoints
# are inert at baseline; individual test files enable the storage gate
# themselves. We still set the YAML path so those storage endpoints are
# available to the test files that opt in.
#
# Tokenless loads instead select principal_source=disabled (see
# reset_env_vars). The two are mutually exclusive on a single AuthSettings
# load: the AuthSettings validator rejects permissions_enabled=true together
# with principal_source=disabled.
_config_path = Path(__file__).parent / "fixtures/test_event_permissions.yml"
os.environ["OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH"] = str(_config_path)
os.environ["OMNI_EVENTS_PERMISSIONS_ENDPOINT"] = "http://test-permissions:8080"

mock_auth_settings()


# Imported only after mock_auth_settings(): importing the module builds the
# auth_settings singleton from the environment, and the library refuses its
# default principal_source (jwtUnverified) without the explicit opt-in.
from notification_common.config.auth_settings import AuthSettings
from notification_common.auth.auth_context import auth_token_var
from notification_common.auth.auth_tokens import jwt_decoder
from notification_common.config.common_settings import common_settings
from notification_common.auth import principal_id
from notification_common.interceptors import interceptors
from notification_common.permissions import permissions_client

from app.lib.rabbitmq import (
    rabbitmq_connector,
    capacity_guard,
    NonDurableQueueConsumer,
)
from app.lib.permissions.storage_permissions.cache_scope import CacheScope
from app.lib.permissions.storage_permissions.storage_cache import StorageCache

# flake8: noqa: E402
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase
from app.lib.observability.consumer_metrics import consumer_metrics
from app.lib.permissions import permissions
from app.config import settings
from notification_common.redis import redis_client

from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2_grpc


def assert_non_durable_get_instance_call(queue_id, filter_groups):
    # noinspection PyUnresolvedReferences
    NonDurableQueueConsumer.get_instance.assert_called_once()
    call_args = NonDurableQueueConsumer.get_instance.call_args[0]
    assert call_args[0] == queue_id
    assert call_args[1] == filter_groups
    assert call_args[2] == filter_groups


def reset_env_vars():
    # Set the YAML path before any auth-settings construction so the
    # storage-permissions endpoints are present for settings validation.
    config_path = Path(__file__).parent / "fixtures/test_event_permissions.yml"
    os.environ["OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH"] = str(config_path)

    # This is the tokenless reseed: principal_source=disabled means no
    # token is required. The AuthSettings validator rejects
    # permissions_enabled=true together with principal_source=disabled, so
    # permissions stay NOT enabled here. (The authenticated jwt +
    # permissions baseline lives in the module-level mock_auth_settings()
    # call above.) With permissions_enabled=false the Settings validator
    # passes regardless of the YAML-provided storage endpoints.
    os.environ["OMNI_EVENTS_PRINCIPAL_SOURCE"] = PrincipalSource.DISABLED.value
    os.environ["OMNI_EVENTS_PERMISSIONS_ENABLED"] = "false"

    evn_vars_to_del = [
        "OMNI_EVENTS_GRPC_PORT",
        "OMNI_EVENTS_EVENT_PERMISSIONS",
        "OMNI_EVENTS_PERMISSIONS_ENDPOINT",
        "OMNI_EVENTS_STORAGE_PERMISSIONS_ENDPOINTS",
        "OMNI_EVENTS_PERMISSIONS_API_TYPE",
        "OMNI_EVENTS_CONSUMER_LIVENESS_POLL_SECONDS",
        "OMNI_EVENTS_REDIS_ENABLED",
        "OMNI_EVENTS_REDIS_URI",
    ]

    for env_var in evn_vars_to_del:
        try:
            del os.environ[env_var]
        except KeyError:
            pass

    importlib.reload(settings)


def make_timestamp():
    ts = Timestamp()
    ts.FromSeconds(int(time.time()))
    return ts


@pytest.fixture
def cache_scope():
    """
    The (queue, presenting principal) pair the storage-permissions cache is
    namespaced by, for the suite's stand-in non-durable queue as seen by the
    principal the autouse ``mock_principal_id`` fixture resolves.
    """
    return CACHE_SCOPE


@pytest.fixture
def second_cache_scope():
    """
    The same queue as seen by a DIFFERENT authenticated principal -- the one
    that attaches with someone else's reconnect token. Shares
    ``cache_scope``'s ``queue_ref`` and never its ``key``.
    """
    return SECOND_CACHE_SCOPE


@pytest_asyncio.fixture
async def cache_instance(mocker, cache_scope):
    instance = StorageCache(cache_scope)
    mocker.patch.object(StorageCache, "get_instance", return_value=instance)
    return instance


@pytest.fixture
def mock_create_task(mocker):
    return mocker.patch("asyncio.create_task")


@pytest.fixture
def mock_principal():
    return "test_principal"


@pytest.fixture(autouse=True)
def mock_principal_id(mocker, mock_principal):
    mocker.patch.object(principal_id, "principal_id", return_value=mock_principal)


@pytest.fixture
def authentication_enabled(mocker):
    """
    Turn the authentication gate on or off for one test.

    Patched on the class rather than on the ``auth_settings`` singleton the
    code under test reads: that singleton is a pydantic model, and mocker's
    teardown cannot delete an attribute it set on the instance. A class
    patch still reaches the singleton's method lookup.

    Returns a callable taking the desired state and returning the patch, so
    a test that also wants to assert on it can.
    """

    def _set(enabled):
        return mocker.patch.object(
            AuthSettings, "authentication_enabled", return_value=enabled
        )

    return _set


@pytest.fixture
def mock_queue_id():
    return "test-queue-id"


@pytest.fixture
def mock_queue_name(mock_queue_id):
    """
    The broker name a durable queue_id resolves to.

    Derived from the queue id alone -- no principal component -- so the
    same id names the same broker queue for every caller. Spelled out as a
    literal rather than by calling ``_queue_name`` so it pins the shape
    independently of the code under test.
    """
    return f"event-queue:{mock_queue_id}"


@pytest_asyncio.fixture
async def mock_chan(mocker):
    return mocker.patch.object(aio_pika.robust_channel, "RobustChannel", autospec=True)


@pytest_asyncio.fixture
async def mock_conn(mock_chan, mocker):
    conn = mocker.patch.object(
        aio_pika.robust_connection, "RobustConnection", autospec=True
    )
    conn.channel.return_value = Future()
    conn.channel.return_value.set_result(mock_chan)
    return conn


@pytest.fixture
def mocked_non_durable_consumer(mock_queue_name):
    mocked_queue = MagicMock(spec=aio_pika.abc.AbstractQueue)
    mocked_channel = MagicMock()

    # Set a default name which can be overridden if needed per test
    mocked_queue.name = mock_queue_name

    consumer = NonDurableQueueConsumer(mocked_queue, "queue_id", mocked_channel)

    # This rig hands the servicer a consumer built directly, standing in for
    # the one ``get_instance`` would have returned -- and ``get_instance`` is
    # what binds the stream's cache scope. Without it, any test that drives a
    # mid-stream filter update hits ``update_queue``'s scope guard instead of
    # the behavior it means to exercise. Derived exactly as ``get_instance``
    # derives it: this queue's name, plus the principal the autouse
    # ``mock_principal_id`` fixture resolves, redacted.
    consumer._cache_scope = CacheScope(mocked_queue.name, TEST_PRINCIPAL_REF)

    # Mirrored from ``get_instance`` for the same reason as the scope above:
    # the tests this rig serves open their streams with a request that names
    # filter groups and no reconnect token, which is the fresh connect
    # ``get_instance`` records True for. Left at the constructor's None, the
    # servicer would send no opening acknowledgement on any of them, and a
    # test counting acknowledgements would silently be counting one fewer
    # than production sends. A test driving a token-only reattach names the
    # verdict for THAT shape on this consumer instead -- see
    # ``test_a_token_only_reconnect_gets_no_acknowledgement``.
    consumer.connect_time_update_changed = True

    return consumer


@pytest.fixture(autouse=True)
def before_each(mock_conn, mock_chan, mocker):
    # The connector is a module-level singleton, so its connection and
    # channel pools persist across tests. connect() reuses them when set
    # (rabbitmq_connector.connect), which means a stale mocked channel
    # from an earlier test — including any declare_queue.side_effect a
    # unit test left on it — can leak into a later test's grpc_server
    # fixture setup. Reset per-test so each test rebuilds from its own
    # fresh mocks.
    RabbitMQBase.connector._connection = None
    RabbitMQBase.connector._channel_pool = None
    # connect() guards the durable pool separately from the non-durable one,
    # so it must be nulled too — otherwise a fresh connection is paired with
    # a durable pool still bound to the previous test's mock.
    RabbitMQBase.connector._durable_channel_pool = None
    RabbitMQBase.connector.closing = False

    auth_token_var.set("test-token")
    mocker.patch.object(aio_pika, "connect_robust", return_value=mock_conn)

    mocker.patch.object(permissions_client, "can", return_value=True)
    mocker.patch.object(
        jwt_decoder, "get_user_id", return_value=("test-principal-id", 300)
    )

    mocker.patch.object(permissions, "_can_or_raise")
    mocker.patch.object(
        permissions, "can_consume_durable_queue_event", return_value=True
    )
    mocker.patch.object(
        permissions, "can_consume_non_durable_queue_event", return_value=True
    )

    # Neutralize admission for the whole suite. CapacityGuard exposes a
    # single entry point, raise_if_at_capacity(durable_pool=...), so this
    # one patch covers BOTH admission paths: the non-durable-keyed check
    # (default) and the durable-pool-keyed check the durable gRPC/REST
    # entry points make. It has to stay patched because the connector's
    # pools are nulled out above, and a real guard call with no pool
    # fails closed. Tests that need a path to reject configure this same
    # mock -- see make_capacity_side_effect below for split scenarios.
    mocker.patch.object(capacity_guard, "raise_if_at_capacity")

    yield


def make_capacity_side_effect(non_durable_rejects=False, durable_rejects=False):
    """
    Build a ``side_effect`` for the patched
    ``capacity_guard.raise_if_at_capacity`` that rejects each admission
    path independently.

    Both admission paths run through the one patched method, keyed on the
    ``durable_pool`` argument: the non-durable and durable checks are now
    gated on different pools, so a split scenario ("the non-durable pool
    is saturated but the durable pool still admits", or the reverse)
    cannot be expressed by a flat ``side_effect = ChannelNotAvailableError()``
    -- that rejects both surfaces at once. This dispatches on the argument
    instead, so a test can saturate one surface and leave the other open.

    :param non_durable_rejects: raise ChannelNotAvailableError for calls
        that check the non-durable pool (``durable_pool`` falsy).
    :param durable_rejects: raise ChannelNotAvailableError for calls that
        check the durable pool (``durable_pool=True``).
    """

    def _side_effect(*args, **kwargs):
        # Production callers pass durable_pool as a keyword, but accept a
        # positional too so the helper matches the real signature.
        durable_pool = kwargs.get("durable_pool", args[0] if args else False)

        if durable_pool and durable_rejects:
            raise ChannelNotAvailableError()

        if not durable_pool and non_durable_rejects:
            raise ChannelNotAvailableError()

    return _side_effect


@pytest.fixture
def capacity_guard_rejections(before_each):
    """
    Configure which capacity-guard admission paths reject, per test.

    Returns a callable taking the same keyword arguments as
    ``make_capacity_side_effect`` and returning the patched
    ``raise_if_at_capacity`` mock (so callers can assert on its calls).
    """

    def _configure(non_durable_rejects=False, durable_rejects=False):
        capacity_guard.raise_if_at_capacity.side_effect = make_capacity_side_effect(
            non_durable_rejects=non_durable_rejects,
            durable_rejects=durable_rejects,
        )
        return capacity_guard.raise_if_at_capacity

    return _configure


@pytest.fixture
def grpc_host():
    if common_settings.app_env == "test":
        return "host.docker.internal"

    return "localhost"


@pytest_asyncio.fixture
async def grpc_server():
    """
    Done this way to prevent warning
    of aio-pika code running with no event loop.
    Uses port 0 so the OS assigns a free port,
    avoiding collisions under parallel test execution.
    """
    await rabbitmq_connector.connect()

    from app.grpc.event_consumer_servicer import EventConsumerServicer

    server = grpc.aio.server(interceptors=await interceptors.get_interceptors())

    event_consumer_pb2_grpc.add_EventConsumerServiceServicer_to_server(
        EventConsumerServicer(), server
    )

    port = server.add_insecure_port("localhost:0")

    await server.start()
    yield server, port
    await server.stop(None)


@pytest.fixture
def mock_redis(mocker):
    client = mocker.AsyncMock()
    mocker.patch.object(redis_client, "redis_client", client)
    return client


@pytest_asyncio.fixture
async def event_consumer_stub(grpc_server, grpc_host):
    server, port = grpc_server
    async with insecure_channel(f"{grpc_host}:{port}") as c:
        yield event_consumer_pb2_grpc.EventConsumerServiceStub(c)


@pytest.fixture
def mock_record_success(mocker):
    return mocker.patch.object(consumer_metrics, "record_event_processing_success")


@pytest.fixture
def mock_record_failure(mocker):
    return mocker.patch.object(consumer_metrics, "record_event_processing_failure")
