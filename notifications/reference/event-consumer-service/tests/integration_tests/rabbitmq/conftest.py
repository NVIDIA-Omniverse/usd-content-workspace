# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Module-scoped fixtures for the REAL-broker dead-letter capture tests.

These tests exercise RabbitMQ's quorum dead-lettering against a live
RabbitMQ 4.x broker (the conformance/CI broker is ``rabbitmq:4.1.3``).
They deliberately bypass the fakes installed by the autouse fixtures in
``tests/conftest.py`` (``before_each`` patches ``aio_pika.connect_robust``)
and ``tests/integration_tests/conftest.py`` (``mock_queue_iterator`` patches
``RabbitMQBase.get_queue`` / ``_consume_with_*`` / ``DurableQueueConsumer``):

- We capture a reference to the genuine ``aio_pika.connect_robust`` at import
  time of this conftest, before any test-setup patching runs, and open our own
  connection with it. The autouse patches therefore never intercept this
  module's broker traffic.
- We never call the patched ``RabbitMQBase`` / ``DurableQueueConsumer``
  consume helpers; all queue declaration, publishing, and consumption here
  go straight through ``aio_pika``.
"""

import asyncio
import os

import aio_pika
import pytest
import pytest_asyncio

# aio_pika re-exports only part of aiormq's exception hierarchy, and the 405
# RESOURCE_LOCKED class is not among the names it forwards. aiormq is
# aio_pika's own transport dependency, so importing it here is safe and it is
# what keeps the mutex below from having to catch a broad ``ChannelClosed``
# (which would swallow a genuine broker error as "lock busy").
from aiormq.exceptions import ChannelLockedResource
from notification_common.config.common_settings import common_settings

# Captured at conftest import time, before the autouse ``before_each``
# fixture patches ``aio_pika.connect_robust`` for each test. Using this
# reference guarantees we reach the real broker.
_REAL_CONNECT_ROBUST = aio_pika.connect_robust

# The environment variable the ``integration:`` CI job (`.gitlab-ci.yml`) sets
# to ``"1"`` when it starts a ``rabbitmq:4.1.3`` service beside the tests. When
# it is set, a broker MUST be reachable, so a connect failure is a HARD error
# rather than a skip -- this is the guard that stops the live-broker arms from
# silently skipping to an illusory green (issue b.y7y). When it is unset (local
# dev without a broker), a connect failure skips cleanly as before.
_REQUIRE_LIVE_BROKER_ENV = "OMNI_EVENTS_REQUIRE_LIVE_BROKER"

# Name of the exclusive queue used as a cross-worker mutex; see
# ``shared_dead_letter_queue_lock``.
_SHARED_DLQ_LOCK_QUEUE = "test-shared-dead-letter-queue-lock"
_SHARED_DLQ_LOCK_TIMEOUT_S = 120.0
_SHARED_DLQ_LOCK_RETRY_S = 0.5


def _live_broker_required() -> bool:
    """True when the environment demands a reachable broker (CI integration job).

    Truthy values are ``1``/``true``/``yes``/``on`` (case-insensitive); anything
    else -- including unset -- leaves the fixture free to skip when no broker is
    present.
    """
    return os.environ.get(_REQUIRE_LIVE_BROKER_ENV, "").strip().lower() in (
        "1",
        "true",
        "yes",
        "on",
    )


async def _connect_or_skip():
    """
    Open a real AMQP connection, or skip/fail per the environment's promise.

    A short connection timeout keeps the caller from hanging when no broker is
    present. Behaviour on a connect failure is governed by
    ``OMNI_EVENTS_REQUIRE_LIVE_BROKER``:

    - When it is set/truthy -- as the ``integration:`` CI job in
      ``.gitlab-ci.yml`` sets it (``"1"``), running these tests against a
      ``rabbitmq:4.1.3`` service -- a connect failure is a HARD failure. A
      broker was promised; its absence is an error, not a reason to skip. This
      is what keeps the live-broker arms from silently skipping to an illusory
      green.
    - When it is unset (a dev box without the compose stack), the test is
      skipped cleanly rather than hung or failed.
    """
    try:
        return await _REAL_CONNECT_ROBUST(common_settings.rabbitmq_uri, timeout=5)
    except Exception as exc:  # noqa: BLE001 - any connect failure -> skip or fail
        message = (
            "RabbitMQ 4.x broker not reachable at "
            f"{common_settings.rabbitmq_uri!r} ({exc!r})"
        )
        if _live_broker_required():
            pytest.fail(
                f"{message}; {_REQUIRE_LIVE_BROKER_ENV} is set, so a live broker "
                "is required and its absence is a hard failure (not a skip)"
            )
        pytest.skip(f"{message}; skipping live-broker test")


@pytest_asyncio.fixture
async def live_broker_connection():
    """A real AMQP connection for one test, closed on teardown."""
    connection = await _connect_or_skip()
    try:
        yield connection
    finally:
        await connection.close()


@pytest_asyncio.fixture
async def shared_dead_letter_queue_lock():
    """
    Exclusive access to the service's SHARED dead-letter queue, across every
    xdist worker, for the life of one test.

    An AMQP queue is a destructive-read work queue, so two tests polling the
    same DLQ at once do not both observe it -- they compete for its messages,
    and the loser's evidence is gone. That is what broke this package under
    ``pytest -n auto``. The per-test DLX/DLQ pairs in
    ``test_dead_letter_capture.py`` remove the sharing for almost everything;
    this fixture serialises the one test that still has to use the real,
    configured ``events.dlx``/``events.dlq`` names.

    The mutex is held BY THE BROKER, not by a file or an xdist scheduling
    mark, because the contending parties are separate OS processes that share
    nothing but the broker:

    - An exclusive queue may exist on exactly one connection at a time. A
      second connection declaring the same name gets AMQP 405
      RESOURCE_LOCKED, which is the "lock is taken" signal.
    - The lock is therefore held for the life of THIS fixture's dedicated
      connection (exclusive queues are torn down with the connection, not the
      channel), and closing that connection on teardown releases it.
    - A worker that crashes or is killed drops its connection, so the lock
      cannot be orphaned -- there is no stale-lock file to clean up.

    Acquisition is bounded: a lock that never frees fails the test loudly
    rather than hanging the suite.
    """
    connection = await _connect_or_skip()
    loop = asyncio.get_running_loop()
    deadline = loop.time() + _SHARED_DLQ_LOCK_TIMEOUT_S
    try:
        while True:
            # A fresh channel per attempt: the 405 closes the channel it was
            # raised on, so a retry cannot reuse it.
            channel = await connection.channel()
            try:
                await channel.declare_queue(
                    _SHARED_DLQ_LOCK_QUEUE, exclusive=True, robust=False
                )
                break
            except ChannelLockedResource:
                if loop.time() >= deadline:
                    pytest.fail(
                        "could not acquire the shared dead-letter-queue lock "
                        f"({_SHARED_DLQ_LOCK_QUEUE!r}) within "
                        f"{_SHARED_DLQ_LOCK_TIMEOUT_S}s; another worker has "
                        "held it for the whole window"
                    )
                await asyncio.sleep(_SHARED_DLQ_LOCK_RETRY_S)
        yield
    finally:
        await connection.close()


@pytest_asyncio.fixture(scope="class", loop_scope="class")
async def class_live_broker_connection():
    """
    A real AMQP connection shared by every test in one class.

    For modules whose live episode is a single expensive drive that all the
    class's assertions read fields off: the connection has to outlive the
    individual test, so it is scoped -- and loop-scoped -- to the class that
    records against it.
    """
    connection = await _connect_or_skip()
    try:
        yield connection
    finally:
        await connection.close()


@pytest_asyncio.fixture(scope="module", loop_scope="module")
async def module_live_broker_connection():
    """
    A real AMQP connection shared by every test in one module.

    The same idea as ``class_live_broker_connection``, one scope wider, for
    a module whose single expensive live drive is read by assertions split
    across MORE THAN ONE class. At class scope such a module pays for the
    drive once per class; at module scope it pays once.
    """
    connection = await _connect_or_skip()
    try:
        yield connection
    finally:
        await connection.close()
