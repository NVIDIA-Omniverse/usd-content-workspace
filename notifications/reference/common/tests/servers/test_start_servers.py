# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import signal
from typing import Callable
from unittest.mock import AsyncMock, call, MagicMock

import pytest

from notification_common.config import logging_config, observability_config
from notification_common.config.common_settings import common_settings
from notification_common.http_client.http_client import http_client
from notification_common.redis import redis_client
from notification_common.servers import start_servers
from notification_common.servers.server_base import ServerBase
from notification_common.test_tools.any_instance_of import AnyInstanceOf


class TestServer(ServerBase):
    @classmethod
    async def start_server(cls, shutdown_event: asyncio.Event):
        pass


@pytest.fixture(autouse=True)
def mock_test_server(mocker):
    mocker.patch.object(TestServer, "start_server")


@pytest.mark.asyncio
class TestStartServers:
    @staticmethod
    async def start_servers(rabbitmq):
        await start_servers.start_servers(rabbitmq, TestServer)

    @pytest.mark.parametrize(
        "func_name", ["_close_clients", "_config_and_connect", "_add_shutdown_handler"]
    )
    async def test_calls_functions(self, func_name, mocker):
        spy = mocker.patch.object(start_servers, func_name)
        rabbitmq_mock = AsyncMock()
        await self.start_servers(rabbitmq_mock)
        spy.assert_called_once_with(rabbitmq_mock)

    async def test_gathers_tasks(self, mocker):
        spy = mocker.spy(asyncio, "gather")
        await start_servers.start_servers(AsyncMock(), TestServer, TestServer)

        spy.assert_called_once()
        assert len(spy.call_args_list[0][0]) == 2

    async def test_starts_tasks_with_shutdown_event_and_deps(self, mocker):
        event = asyncio.Event()
        mocker.patch.object(start_servers, "_add_shutdown_handler", return_value=event)

        await start_servers.start_servers(AsyncMock(), TestServer, TestServer)

        # noinspection PyUnresolvedReferences
        TestServer.start_server.assert_has_calls([call(event), call(event)])

    async def test_missing_rest_server_does_not_fail(self, mocker):
        spy = mocker.spy(asyncio, "gather")
        await start_servers.start_servers(AsyncMock(), TestServer, None)

        spy.assert_called_once()


@pytest.mark.asyncio
class TestCloseClients:
    async def test_closes_rabbitmq_connector(self):
        mocked_rabbitmq_connector = AsyncMock()
        await start_servers._close_clients(mocked_rabbitmq_connector)
        mocked_rabbitmq_connector.close.assert_called_once_with()

    async def test_closes_http_client(self, mocker):
        spy = mocker.patch.object(http_client, "close")
        await start_servers._close_clients(AsyncMock())
        spy.assert_called_once_with()

    async def test_closes_redis_client(self, mocker):
        spy = mocker.patch.object(redis_client, "close_redis_client")
        await start_servers._close_clients(AsyncMock())
        spy.assert_called_once_with()


@pytest.mark.asyncio
class TestConfigAndConnect:
    async def test_configures_logging(self, mocker):
        spy = mocker.patch.object(logging_config, "configure_logging")
        await start_servers._config_and_connect(AsyncMock())
        spy.assert_called_once_with()

    async def test_configures_observability(self, mocker):
        spy = mocker.patch.object(observability_config, "configure_observability")
        await start_servers._config_and_connect(AsyncMock())
        spy.assert_called_once_with()

    async def test_connects_rabbitmq(self, mocker):
        rabbitmq = AsyncMock()
        spy = mocker.patch.object(rabbitmq, "connect")
        await start_servers._config_and_connect(rabbitmq)
        spy.assert_called_once_with()

    async def test_opens_http_client(self, mocker):
        spy = mocker.patch.object(http_client, "open")
        await start_servers._config_and_connect(AsyncMock())
        spy.assert_called_once_with()


@pytest.mark.asyncio
class TestAddShutdownHandler:
    async def test_adds_signal_handler(self, mocker):
        loop = asyncio.get_running_loop()
        mocker.patch.object(asyncio, "get_running_loop", return_value=loop)

        spy = mocker.spy(loop, "add_signal_handler")
        await start_servers._add_shutdown_handler(AsyncMock())

        spy.assert_has_calls(
            [
                call(signal.SIGINT, AnyInstanceOf(Callable)),
                call(signal.SIGTERM, AnyInstanceOf(Callable)),
            ]
        )

    async def test_returns_an_asyncio_event(self):
        results = await start_servers._add_shutdown_handler(AsyncMock())
        assert isinstance(results, asyncio.Event)


def _rabbitmq_mock(drain_side_effect=None) -> MagicMock:
    """A RabbitMQConnector mock whose async drain is an AsyncMock."""
    rabbitmq = MagicMock()
    rabbitmq.drain_non_durable = AsyncMock(side_effect=drain_side_effect)
    return rabbitmq


# A settle delay long enough to be unmistakable in an assertion, and never
# actually slept because the sleep itself is always mocked.
SETTLE_DELAY_SECONDS = 12.5


@pytest.fixture
def settle_sleep(mocker) -> AsyncMock:
    """Replace the sleep the settle phase awaits, so no real time passes."""
    return mocker.patch.object(asyncio, "sleep", AsyncMock())


@pytest.fixture
def settle_delay(mocker) -> float:
    """Configure a positive endpoint-settle delay for the duration of a test."""
    mocker.patch.object(
        common_settings, "shutdown_settle_delay_seconds", SETTLE_DELAY_SECONDS
    )
    return SETTLE_DELAY_SECONDS


@pytest.fixture
def no_settle_delay(mocker) -> float:
    """Configure the default, disabled endpoint-settle delay."""
    mocker.patch.object(common_settings, "shutdown_settle_delay_seconds", 0.0)
    return 0.0


@pytest.fixture
def settle_logger(mocker) -> MagicMock:
    """Spy on the module logger, which was bound before getLogger was mocked."""
    return mocker.patch.object(start_servers, "logger")


@pytest.fixture
def failing_settle_sleep(mocker) -> AsyncMock:
    """Make the sleep that the settle phase awaits blow up."""
    return mocker.patch.object(
        asyncio, "sleep", AsyncMock(side_effect=RuntimeError("settle boom"))
    )


def _ordered_calls_parent(**children) -> MagicMock:
    """
    A parent mock that records every child's calls on one ordered list.

    attach_mock funnels the sub-calls onto the parent, so that the relative
    order of calls made on different collaborators is assertable.
    """
    parent = MagicMock()
    for name, child in children.items():
        parent.attach_mock(child, name)
    return parent


@pytest.mark.asyncio
class TestSettleBeforeDrain:
    """
    Coverage of the optional endpoint-settle phase that precedes the drain.

    The process keeps serving normally for the configured delay so that the
    routers in front of it can converge on the pod's removal. A delay of zero
    skips the phase outright, and a failure inside it is swallowed so that the
    drain still runs -- but cancellation still propagates.
    """

    async def test_does_not_sleep_when_delay_is_zero(
        self, no_settle_delay, settle_sleep
    ):
        await start_servers._settle_before_drain()

        settle_sleep.assert_not_awaited()

    async def test_logs_nothing_when_delay_is_zero(
        self, no_settle_delay, settle_sleep, settle_logger
    ):
        await start_servers._settle_before_drain()

        settle_logger.info.assert_not_called()

    async def test_sleeps_for_the_configured_delay(self, settle_delay, settle_sleep):
        await start_servers._settle_before_drain()

        settle_sleep.assert_awaited_once_with(settle_delay)

    async def test_logs_both_ends_of_the_settle_window(
        self, settle_delay, settle_sleep, settle_logger
    ):
        # The phase announces its entry and its exit, so that an operator can
        # see the delay in the logs; the wording itself is not the contract.
        # Without this, test_logs_nothing_when_delay_is_zero would also pass
        # against a settle phase that logged nothing at all.
        await start_servers._settle_before_drain()

        assert settle_logger.info.call_count == 2

    async def test_settle_failure_does_not_propagate(
        self, settle_delay, failing_settle_sleep
    ):
        # Must not raise: the settle is a courtesy to the routing layer, and
        # the drain that follows it matters more than the settle does.
        await start_servers._settle_before_drain()

    async def test_settle_failure_is_logged(
        self, settle_delay, settle_logger, failing_settle_sleep
    ):
        await start_servers._settle_before_drain()

        settle_logger.exception.assert_called_once()

    async def test_cancellation_propagates(self, settle_delay, mocker):
        mocker.patch.object(
            asyncio, "sleep", AsyncMock(side_effect=asyncio.CancelledError)
        )

        with pytest.raises(asyncio.CancelledError):
            await start_servers._settle_before_drain()


@pytest.mark.asyncio
class TestGracefulShutdown:
    """
    Direct coverage of the real SIGTERM/SIGINT drain-before-close sequence.

    `_graceful_shutdown` holds for the optional endpoint-settle delay, flips
    the draining signal, waits for in-flight non-durable streams to drain, then
    -- in a finally that always runs -- marks the connector closing and sets
    the shutdown event so the servers stop. The pool teardown happens strictly
    afterwards in start_servers' finally block, never before the drain window.
    """

    async def test_settles_and_drains_before_closing_in_order(self, mocker):
        settle = mocker.patch.object(start_servers, "_settle_before_drain", AsyncMock())
        rabbitmq = _rabbitmq_mock()
        event = MagicMock()
        parent = _ordered_calls_parent(settle=settle, rabbitmq=rabbitmq, event=event)

        await start_servers._graceful_shutdown(rabbitmq, event)

        assert parent.mock_calls == [
            call.settle(),
            call.rabbitmq.begin_draining(),
            call.rabbitmq.drain_non_durable(),
            call.rabbitmq.prepare_to_shutdown(),
            call.event.set(),
        ]

    async def test_drains_even_when_the_settle_fails(
        self, settle_delay, failing_settle_sleep
    ):
        # The settle guards itself, so a failure inside it must fall through
        # into the drain rather than skipping it.
        rabbitmq = _rabbitmq_mock()
        event = MagicMock()
        parent = _ordered_calls_parent(rabbitmq=rabbitmq, event=event)

        await start_servers._graceful_shutdown(rabbitmq, event)

        assert parent.mock_calls == [
            call.rabbitmq.begin_draining(),
            call.rabbitmq.drain_non_durable(),
            call.rabbitmq.prepare_to_shutdown(),
            call.event.set(),
        ]

    async def test_drain_failure_does_not_propagate(self):
        rabbitmq = _rabbitmq_mock(drain_side_effect=RuntimeError("drain boom"))
        event = MagicMock()

        # Must not raise: the finally branch is defensive exception-safety.
        await start_servers._graceful_shutdown(rabbitmq, event)

    async def test_stops_servers_even_when_drain_raises(self):
        rabbitmq = _rabbitmq_mock(drain_side_effect=RuntimeError("drain boom"))
        event = MagicMock()

        await start_servers._graceful_shutdown(rabbitmq, event)

        # Despite the drain blowing up, the servers are still told to stop.
        rabbitmq.prepare_to_shutdown.assert_called_once()
        event.set.assert_called_once()

    async def test_stops_servers_when_the_settle_is_cancelled(self, mocker):
        # Cancellation is a BaseException, so it escapes the except clause that
        # covers the drain. The finally must still run, or a cancelled shutdown
        # would leave the servers up until the kubelet SIGKILLs the process.
        mocker.patch.object(
            start_servers,
            "_settle_before_drain",
            AsyncMock(side_effect=asyncio.CancelledError),
        )
        rabbitmq = _rabbitmq_mock()
        event = MagicMock()

        with pytest.raises(asyncio.CancelledError):
            await start_servers._graceful_shutdown(rabbitmq, event)

        rabbitmq.prepare_to_shutdown.assert_called_once()
        event.set.assert_called_once()


@pytest.mark.asyncio
class TestScheduleGracefulShutdown:
    """
    Coverage of the signal-callback scheduling wrapper installed by
    `_add_shutdown_handler`: the `scheduled` once-only guard and the
    strong-reference `shutdown_tasks` set.
    """

    @staticmethod
    def _capture_scheduler(mocker, loop):
        """
        Install `_add_shutdown_handler` against ``loop`` (fed via a patched
        `asyncio.get_running_loop`) and return the `_schedule_graceful_shutdown`
        callback it registers with the signal handlers.
        """
        mocker.patch.object(asyncio, "get_running_loop", return_value=loop)

        captured = {}

        def _capture(sig, callback):
            captured["callback"] = callback

        mocker.patch.object(loop, "add_signal_handler", side_effect=_capture)
        return captured

    async def test_schedules_graceful_shutdown_at_most_once(self, mocker):
        # A fully mocked loop: this test never needs a real task to run, and
        # replacing the *running* loop's create_task with a plain mock would
        # break pytest-asyncio's own teardown (it schedules via create_task).
        loop = MagicMock()
        captured = self._capture_scheduler(mocker, loop)

        await start_servers._add_shutdown_handler(_rabbitmq_mock())
        schedule = captured["callback"]

        # Two signals (SIGINT then SIGTERM, or a repeat) must schedule the
        # drain coroutine only once -- the `scheduled` Event guard.
        schedule()
        schedule()

        assert loop.create_task.call_count == 1

        # create_task was mocked, so its coroutine argument was never awaited;
        # close it to avoid a "coroutine was never awaited" warning.
        for c in loop.create_task.call_args_list:
            c.args[0].close()

    async def test_scheduled_task_runs_to_completion(self, mocker):
        # The real running loop, so the scheduled task actually runs.
        loop = asyncio.get_running_loop()
        captured = self._capture_scheduler(mocker, loop)

        rabbitmq = _rabbitmq_mock()
        shutdown_event = await start_servers._add_shutdown_handler(rabbitmq)
        schedule = captured["callback"]

        schedule()

        # Drive the scheduled task to completion via the shutdown event,
        # holding no strong reference to it ourselves.
        await asyncio.wait_for(shutdown_event.wait(), timeout=1)

        rabbitmq.begin_draining.assert_called_once()
        rabbitmq.drain_non_durable.assert_awaited_once()
        rabbitmq.prepare_to_shutdown.assert_called_once()
        assert shutdown_event.is_set()
