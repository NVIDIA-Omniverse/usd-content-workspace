# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import logging
import signal
from typing import Type

from notification_common.config import logging_config, observability_config
from notification_common.config.common_settings import common_settings
from notification_common.http_client.http_client import http_client
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector
from notification_common.servers.server_base import ServerBase

logger = logging.getLogger(__name__)


async def start_servers(
    rabbitmq_connector: RabbitMQConnector,
    grpc_server: Type[ServerBase],
    rest_server: Type[ServerBase] | None = None,
) -> None:
    """
    Configures and starts up the servers.
    Also starts the rabbitmq connector, and the aio-http session.
    """
    try:
        await _config_and_connect(rabbitmq_connector)
        shutdown_event = await _add_shutdown_handler(rabbitmq_connector)

        servers = [grpc_server, rest_server]
        tasks = [s.start_server(shutdown_event) for s in servers if s]

        await asyncio.gather(*tasks)

    finally:
        await _close_clients(rabbitmq_connector)


async def _add_shutdown_handler(rabbitmq: RabbitMQConnector):
    shutdown_event = asyncio.Event()

    loop = asyncio.get_running_loop()

    # Guard so that the graceful-shutdown coroutine is scheduled at most once,
    # even if both SIGINT and SIGTERM (or a repeated signal) fire. Signal
    # callbacks are synchronous and cannot await, so they schedule the coroutine
    # on the running loop rather than running the drain sequence inline.
    scheduled = asyncio.Event()

    # Retain a strong reference to the scheduled task. loop.create_task only
    # keeps a weak reference, so without this the drain coroutine could be
    # garbage-collected mid-drain. This set is captured by the signal callback
    # closure, which the loop holds alive, so it outlives the drain sequence.
    shutdown_tasks: set[asyncio.Task] = set()

    def _schedule_graceful_shutdown():
        if scheduled.is_set():
            return
        scheduled.set()
        task = loop.create_task(_graceful_shutdown(rabbitmq, shutdown_event))
        shutdown_tasks.add(task)
        task.add_done_callback(shutdown_tasks.discard)

    for signal_value in (signal.SIGINT, signal.SIGTERM):
        # noinspection PyTypeChecker
        loop.add_signal_handler(signal_value, _schedule_graceful_shutdown)

    return shutdown_event


async def _close_clients(rabbitmq_connector: RabbitMQConnector):
    # doing this import here soe that we can change settings
    # for the redis client even after importing start_servers.py
    from notification_common.redis.redis_client import close_redis_client

    await rabbitmq_connector.close()
    await http_client.close()
    await close_redis_client()


async def _settle_before_drain() -> None:
    """
    Keep serving normally for the configured endpoint-settle delay.

    This runs before the drain begins, so readiness stays SERVING and new
    streams are still admitted for the whole delay. It gives the routers and
    endpoint controllers in front of the process time to converge on this pod's
    removal while it can still serve the requests they send. The delay is read
    from the settings singleton at call time so that a consumer can adjust it
    after importing this module. A delay of zero — the default — skips the
    phase entirely, including its log lines.

    The phase swallows its own exceptions: the settle is an optional courtesy
    to the routing layer, whereas the drain that follows it is what keeps
    in-flight streams from being force-dropped. So a settle that fails is
    logged and returns normally, and the caller goes on to begin_draining() /
    drain_non_durable() as if the settle had simply been skipped. Cancellation
    is deliberately not caught — asyncio.CancelledError is a BaseException, so
    it still propagates out to the caller's finally, which stops the servers.
    """
    try:
        delay = common_settings.shutdown_settle_delay_seconds
        if delay <= 0:
            return

        logger.info(
            "Endpoint-settle delay: serving normally for %.1fs before draining",
            delay,
        )
        await asyncio.sleep(delay)
        logger.info("Endpoint-settle delay elapsed; beginning drain")
    except Exception:
        logger.exception(
            "Endpoint-settle delay failed during graceful shutdown; "
            "proceeding to drain anyway."
        )


async def _graceful_shutdown(
    rabbitmq: RabbitMQConnector, shutdown_event: asyncio.Event
):
    """
    Drain-before-close shutdown sequence for SIGTERM/SIGINT.

    Runs, in order: (1) hold for the optional endpoint-settle delay, during
    which the process keeps serving exactly as before — readiness stays SERVING
    and the capacity guard still admits new streams — so that routers and
    endpoint controllers have time to stop sending traffic to this pod;
    (2) flip the draining signal so the readiness servicer reports NOT_SERVING
    and the capacity guard refuses new streams; (3) wait up to the configured
    window for in-flight non-durable streams to drain; (4) mark the connector
    closing so any still-looping consume loops return; (5) set shutdown_event to
    stop the gRPC/REST servers. The actual pool teardown (connector.close() ->
    close_all()) happens afterwards in start_servers' finally block, i.e.
    strictly after the drain window, never before — so live streams are not
    force-dropped on scale-down.

    The settle only postpones the drain; begin_draining() is still the single
    point at which readiness and admission both flip, and it still does so
    together.

    The settle and the drain (steps 1-3) run inside a try whose finally always
    runs the server-stop steps (4-5). This is general defensive
    exception-safety: if the drain raises for any reason, the servers still stop
    and shutdown still completes, rather than hanging until the kubelet SIGKILLs
    at the grace deadline. The except below therefore covers the drain only —
    the settle (step 1) guards itself and always returns normally, so a failed
    settle still falls through into the drain rather than skipping it. What the
    settle does not swallow is cancellation, which propagates past the except to
    the finally and stops the servers.
    """
    try:
        await _settle_before_drain()
        rabbitmq.begin_draining()
        await rabbitmq.drain_non_durable()
    except Exception:
        logger.exception(
            "Non-durable drain failed during graceful shutdown; "
            "proceeding to stop servers anyway."
        )
    finally:
        rabbitmq.prepare_to_shutdown()
        shutdown_event.set()


async def _config_and_connect(rabbitmq_connector: RabbitMQConnector):
    logging_config.configure_logging()
    observability_config.configure_observability()

    await rabbitmq_connector.connect()
    await http_client.open()
