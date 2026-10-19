# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
The broker edge of a stream whose REAL consume loop finds the channel dead
on one of its own broker operations (issue b.7yv), shared by the gRPC
servicer tests and the REST stream tests.

The sequence is the one the issue measured. Two deliveries reach the client
and stay outstanding; the next is one the authorization check denies, whose
skip-ack raises ``ChannelInvalidStateError`` because the broker has just
killed the channel; and a fourth is buffered behind it. The outstanding and
buffered deliveries are what a healthy-channel cleanup would nack, so the
assertions that it does not are not vacuous.

Everything above the broker edge is real on every surface: the servicer
RPC or ``event_stream_generator``, ``DurableQueueConsumer.consume`` /
``consume_with_acks`` or ``NonDurableQueueConsumer.consume``, the consume
loop and its cleanup. Only the pool channel, the queue
(:class:`~tests.fixtures.consumer_liveness.PumpedQueue`) and the deliveries
are doubled, plus the authorization check's verdict.
"""

import asyncio
import json
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock

import aio_pika
import pytest

from app.config import settings
from app.lib.permissions import permissions
from app.lib.rabbitmq import DurableQueueConsumer
from app.lib.rabbitmq import rabbitmq_base
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase, rabbitmq_connector
from tests.fixtures.consumer_liveness import (
    CONSUMER_LOSS_TIMEOUT_S,
    IDLE_POLL_NEVER_FIRES_SECONDS,
    PumpedQueue,
)

#: What the client is told when its stream ends because the broker took the
#: consumer away -- the actionable text every dead-channel path converges on.
RECONNECT_GUIDANCE = (
    "Internal Server Error: The broker cancelled this stream's consumer. "
    "Reconnect to resume consuming."
)

#: The resource the authorization check denies. Every other delivery names a
#: different one and is authorized.
DENIED_RESOURCE_ID = "resource-the-check-denies"

#: The durable queue id the durable surfaces consume from.
DURABLE_QUEUE_ID = "queue-whose-channel-dies"


def _event_body(resource_id):
    """A body every surface turns into a client event: gRPC and SSE alike."""
    return json.dumps(
        {
            "event_type": "asset.updated",
            "resource_id": resource_id,
            "principal_identity": "user-1",
            "occurred_at": "2026-01-01T00:00:00Z",
            "published_at": "2026-01-01T00:00:01Z",
            "message": {"key": "value"},
        }
    ).encode("utf-8")


async def _authorize_all_but_the_denied_resource(message):
    return message.get("resource_id") != DENIED_RESOURCE_ID


class OwnOperationDeadChannelRig:
    """
    The broker edge, the deliveries and the cleanup capture for one run.

    :param mocker: The test's ``mocker``.
    :param queue_name: The broker name the pumped queue reports.
    """

    def __init__(self, mocker, queue_name):
        self.queue = PumpedQueue(queue_name)
        self.failure = aio_pika.exceptions.ChannelInvalidStateError("channel is closed")
        self.outstanding = [self._delivery(tag, "resource-1") for tag in (1, 2)]
        self.denied = self._delivery(3, DENIED_RESOURCE_ID)
        self.denied.ack.side_effect = self.failure
        self.buffered = self._delivery(4, "resource-1")
        self._cleanup_tasks = []

        channel = MagicMock()
        channel.set_qos = AsyncMock()
        mocker.patch.object(
            rabbitmq_connector,
            "get_channel_direct",
            new_callable=AsyncMock,
            return_value=channel,
        )
        self.return_channel = mocker.patch.object(
            rabbitmq_connector, "return_channel", new_callable=AsyncMock
        )
        mocker.patch.object(
            DurableQueueConsumer,
            "get_queue",
            new_callable=AsyncMock,
            return_value=self.queue,
        )
        for check in (
            "can_consume_durable_queue_event",
            "can_consume_non_durable_queue_event",
        ):
            mocker.patch.object(
                permissions,
                check,
                side_effect=_authorize_all_but_the_denied_resource,
            )
        # The idle liveness poll must not fire: the point is that the loop's
        # own skip-ack finds the channel dead, not the poll.
        mocker.patch.object(
            RabbitMQBase,
            "_liveness_poll_timeout",
            return_value=IDLE_POLL_NEVER_FIRES_SECONDS,
        )
        # The delayed-ack loop must not ack an outstanding delivery mid-run,
        # or a healthy-channel cleanup would have nothing left to nack.
        mocker.patch.object(settings.settings, "server_ack_delay_seconds", 999.0)

        real_create_task = asyncio.create_task

        def _capture(coro, **kwargs):
            task = real_create_task(coro, **kwargs)
            if coro.__name__ == "_cleanup":
                self._cleanup_tasks.append(task)
            return task

        mocker.patch.object(rabbitmq_base.asyncio, "create_task", side_effect=_capture)

    @staticmethod
    def _delivery(tag, resource_id):
        message = AsyncMock(spec=aio_pika.abc.AbstractIncomingMessage)
        message.delivery_tag = tag
        message.body = _event_body(resource_id)
        return message

    async def run(self, client):
        """
        Drive ``client`` -- a coroutine that runs the stream to its end and
        returns what the client saw -- through the sequence.

        :returns: What ``client`` returned, with what the stream's cleanup did.
        """
        client_task = asyncio.ensure_future(client)
        await _within_deadline(
            self.queue.consuming.wait(), "the consume loop did not start consuming"
        )
        for message in (*self.outstanding, self.denied, self.buffered):
            await self.queue.deliver(message)
        seen = await _within_deadline(client_task, "the client stream did not end")
        # Exactly one cleanup ran, so the cleanup assertions are about a
        # cleanup that happened rather than one that was never scheduled.
        if len(self._cleanup_tasks) != 1:
            pytest.fail(
                f"expected the consume loop to schedule one cleanup, "
                f"got {len(self._cleanup_tasks)}"
            )
        await _within_deadline(
            asyncio.gather(*self._cleanup_tasks),
            "the consume loop's cleanup did not finish",
        )

        return SimpleNamespace(
            seen=seen,
            cancel=self.queue.cancel,
            outstanding_nacks=[m.nack.await_count for m in self.outstanding],
            buffered_nacks=self.buffered.nack.await_count,
            return_channel=self.return_channel,
        )


async def _within_deadline(awaitable, what):
    """Await ``awaitable``, failing by name rather than hanging the suite."""
    try:
        return await asyncio.wait_for(awaitable, timeout=CONSUMER_LOSS_TIMEOUT_S)
    except asyncio.TimeoutError:
        pytest.fail(f"{what} within {CONSUMER_LOSS_TIMEOUT_S}s")
