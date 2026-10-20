# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
One factory for each exception class an AMQP operation raises when the
channel it was issued on is dead, and the near misses that must NOT count.

``app.lib.rabbitmq.ack_tracker.DEAD_CHANNEL_EXC`` names the classes. The
settle path (``AckTracker``), the servicer's ``_dead_channel_mapped`` and
the consume and browse loops' own broker operations all classify against
it, and their test modules cannot import from each other. Declaring the
cases once keeps every one of them parametrized over the same set: a
module that forgot a class would still pass while pinning less than the
others.

Factories rather than instances, so no two tests share one exception
object -- several assertions check that the error a caller sees IS the
one that was injected, or is chained to it.
"""

import aio_pika
import pytest


def channel_invalid_state():
    """The channel wrapper has no usable underlay."""
    return aio_pika.exceptions.ChannelInvalidStateError("channel is closed")


def channel_closed():
    """The broker closed the channel."""
    return aio_pika.exceptions.ChannelClosed(406, "PRECONDITION_FAILED")


def ack_timeout_close():
    """
    The ``ChannelClosed`` subclass aiormq raises for the broker's 406 close:
    the flagship ack-timeout kill. Covered on its own so a catch narrowed to
    an exact-type match on ``ChannelClosed`` fails here.
    """
    return aio_pika.exceptions.ChannelPreconditionFailed(
        "PRECONDITION_FAILED - delivery acknowledgement on channel 1 timed out"
    )


def connection_lost():
    """
    The connection carrying the channel closed while the operation was in
    flight: aiormq's base class for that, as raised bare.
    """
    return aio_pika.exceptions.AMQPConnectionError("connection lost")


def connection_forced():
    """
    The ``AMQPConnectionError`` subclass for a broker-closed connection.
    Covered on its own for the same reason as ``ack_timeout_close``.
    """
    return aio_pika.exceptions.ConnectionClosed(320, "CONNECTION_FORCED")


#: Every way a dead channel is reported, keyed by test id.
DEAD_CHANNEL_FAILURES = {
    "channel_invalid_state": channel_invalid_state,
    "channel_closed": channel_closed,
    "precondition_failed": ack_timeout_close,
    "connection_lost": connection_lost,
    "connection_forced": connection_forced,
}

#: Parametrizes a test's ``make_failure`` argument over every dead-channel
#: class.
dead_channel_failures = pytest.mark.parametrize(
    "make_failure",
    list(DEAD_CHANNEL_FAILURES.values()),
    ids=list(DEAD_CHANNEL_FAILURES),
)


def unrelated_runtime_error():
    """
    ``ChannelInvalidStateError`` subclasses ``RuntimeError``, so this pins
    that a catch names the dead-channel class rather than its base: an
    ordinary ``RuntimeError`` says nothing about the channel.
    """
    return RuntimeError("an unrelated runtime failure")


def builtin_connection_error():
    """
    ``AMQPConnectionError`` subclasses the builtin ``ConnectionError``, so
    this pins that a catch names aiormq's family rather than its base: only
    aiormq's own classes say the AMQP connection closed.
    """
    return ConnectionError("an unrelated connection failure")


#: Failures that say nothing about the channel, keyed by test id. The
#: control arms run these through the same paths and expect the healthy
#: channel's handling.
UNRELATED_FAILURES = {
    "runtime_error": unrelated_runtime_error,
    "builtin_connection_error": builtin_connection_error,
}

#: Parametrizes a test's ``make_failure`` argument over the unrelated
#: failures.
unrelated_failures = pytest.mark.parametrize(
    "make_failure",
    list(UNRELATED_FAILURES.values()),
    ids=list(UNRELATED_FAILURES),
)
