# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REAL-broker integration tests for consumer-liveness termination (issue b.9gj).

The unit suite in ``tests/app/lib/rabbitmq/test_rabbitmq_base.py`` proves the
liveness poll against hand-written channel doubles: the "broker cancel" is a
``dict.pop`` on a fake registry and the "closed channel" is a boolean we set.
That is the right shape for the branch coverage, but it cannot answer the
question the fix actually rests on -- does a REAL RabbitMQ leave the traces
the predicate reads? This module runs the production consume loop against a
LIVE RabbitMQ 4.x broker so every link is real:

1. A uuid-named queue is declared and consumed through the real
   ``RabbitMQBase._consume_with_client_acks`` (captured at import time, before
   the autouse ``mock_queue_iterator`` swaps it for a canned generator).
2. A message published to the bound routing key really arrives through that
   generator -- the non-vacuity control. "The stream terminated" proves
   nothing about a stream that never started.
3. The consumer is then killed the two ways production sees it killed:

   - ``TestQueueDeletedUnderALiveStream``: the queue is deleted from a
     SEPARATE channel. The broker sends a per-consumer ``Basic.Cancel``,
     which aiormq handles by silently dropping the tag; the channel stays
     OPEN, so no channel-close callback ever fires. This is the case that
     kept the thumbnail-service stream "healthy" for ~47 hours.
   - ``TestChannelFatalFaultUnderALiveStream``: a channel-fatal fault
     (``queue_delete(if_unused=True)`` against a still-consumed queue -- the
     documented ``PRECONDITION_FAILED``, and the same technique
     ``test_channel_fatal_blast_radius.py`` uses) kills the channel the
     consumer lives on, standing in for the production ack-timeout kill
     ``precondition_failed: delivery acknowledgement on channel N timed
     out``.
   - ``TestBrokerCancelOverASurvivingQueue``: the third and hardest case --
     a broker-initiated ``Basic.Cancel`` with the queue ALIVE and the channel
     OPEN at classification time. See that class for how it is induced.
   - ``TestOutstandingDeliveryAfterALiveChannelCancel``: what cleanup does to
     the in-flight delivery on that live-channel path, measured on the real
     queue rather than on a mock's ``nack`` call count.

4. Each episode records how the generator ended.

The load-bearing assertions are that the generator ENDS AT ALL (pre-fix it
blocked forever on ``messages_queue.get()``, which is the whole defect) and
that it ends with the error the client can act on: ``QueueNotFoundError``
means reprovision, ``ConsumerCancelledError`` means reconnect.

The channel-fatal arm additionally pins the claim the fix's design rests on:
production declares queues with ``robust=False``, so aio-pika's channel
recovery does NOT restore the consumer -- the channel reopens with an empty
registry and our tag stays absent. If a future aio-pika ever did restore it,
the liveness predicate would go blind and this arm is what catches that.

Scope notes
-----------

- The autouse fakes from the parent conftests are bypassed the same way
  ``test_dead_letter_argument_drift.py`` bypasses them: the connection comes
  from ``class_live_broker_connection`` (which holds a pre-patch reference to
  the real ``aio_pika.connect_robust``), and the consume generator is the
  pre-patch reference captured at import.
- ``RabbitMQBase.connector._connection`` is pointed at the live connection for
  the duration of a class, because the liveness check's CLASSIFYING step
  (``_assert_queue_exists``) opens a temporary channel off the connector.
- Every fixture here is CLASS-scoped, with a matching ``loop_scope``, so one
  live episode serves all of its class's assertions: each test reads a field
  off the record rather than re-running the rig -- declare, publish, wait for
  delivery, induce the fault, wait out the poll -- for every assertion. Class
  scope also puts the drive ahead of the per-test autouse ``before_each``,
  which nulls ``connector._connection``; the episode is complete before that
  ever runs.

  CAVEAT, measured not assumed: ``pytest.ini`` sets ``addopts = -n auto``, and
  xdist's default ``--dist load`` hands out tests individually, so a class's
  tests land on different workers and each worker builds its own copy of the
  class fixture. Under that default this module still builds one rig per test
  (19 of them), exactly as function scope did -- the scoping is not wrong, it
  is simply overridden by the scheduler. It pays off under ``-n0`` (5 rigs
  instead of 19, ~30s) and would pay off suite-wide under ``--dist loadscope``,
  which groups a class's tests onto one worker. Changing that default is a
  repo-wide scheduling decision, not this module's to make.
- Runs in CI's ``integration:`` job against a ``rabbitmq:4.1.3`` service with
  ``OMNI_EVENTS_REQUIRE_LIVE_BROKER=1`` so a missing broker HARD-fails; skips
  cleanly in a dev sandbox with no broker.
"""

import asyncio
import logging
import uuid
from types import SimpleNamespace

import aio_pika
import pytest
import pytest_asyncio
from notification_common.errors.errors import QueueNotFoundError

from app.config import settings
from app.lib.rabbitmq.ack_tracker import AckTracker
from app.lib.rabbitmq.rabbitmq_base import ConsumerCancelledError, RabbitMQBase

# Captured at import time, before the autouse ``mock_queue_iterator`` fixture
# in ``tests/integration_tests/conftest.py`` replaces the attribute with a
# canned generator. Calling this reference runs the REAL consume loop, which
# is the entire point of a live-broker arm.
_REAL_CONSUME_WITH_CLIENT_ACKS = RabbitMQBase._consume_with_client_acks

# The interval the loop runs its liveness check on. Held AT the settings floor
# (``consumer_liveness_poll_seconds`` is ``ge=1.0``) rather than below it, so
# the latency measured here is one a real deployment could actually configure.
TEST_POLL_SECONDS = 1.0

# The ceiling on "the stream ended". Generous enough that broker or CI latency
# cannot flake it, but bounded so the pre-fix behaviour -- a stream that stays
# open forever -- fails as a NAMED timeout instead of hanging CI.
TERMINATION_TIMEOUT_S = 20.0

# The tighter budget the PROMPTNESS assertion uses. Detection costs at most one
# poll plus the classifying round trip, so several polls is a wide margin that
# still fails a fix which only terminated "eventually" (say, on the broker's
# 30-minute default).
PROMPT_DETECTION_BUDGET_S = 8.0

# The non-vacuity control: how long to wait for the first message to arrive
# through the real generator before any fault is introduced.
BASELINE_TIMEOUT_S = 20.0

# How long to let aio-pika's RobustChannel reopen after the channel-fatal
# fault, so the "reopened with an empty consumer registry" claim is measured
# on a channel that genuinely came back rather than one still down.
REOPEN_GRACE_S = 15.0
REOPEN_POLL_S = 0.1

# How long to let the loop's cleanup task hand the outstanding delivery back
# before reading the queue's depth. The nack itself is one round trip; the
# budget is wide because the task is scheduled, not awaited, by the loop.
REQUEUE_GRACE_S = 15.0
REQUEUE_POLL_S = 0.2

BASELINE_BODY = b'{"event_type": "liveness-probe", "message": {"test": "m"}}'


class LivenessOutcome(SimpleNamespace):
    """How one consume generator ended, and how long it took to get there."""


class LivenessEpisode(SimpleNamespace):
    """The measured record of one live consumer-loss episode."""


async def _await_reopen(channel) -> bool:
    """Poll until the channel reopens, returning whether it did."""
    loop = asyncio.get_running_loop()
    deadline = loop.time() + REOPEN_GRACE_S
    while loop.time() < deadline:
        if not channel.is_closed:
            return True
        await asyncio.sleep(REOPEN_POLL_S)
    return not channel.is_closed


async def _consumer_tags_on(channel) -> set:
    """The consumer tags the live channel's aiormq underlay still carries."""
    try:
        underlay = await channel.get_underlay_channel()
    except aio_pika.exceptions.ChannelInvalidStateError:
        return set()
    return set(underlay.consumers)


async def _drive_to_termination(generator) -> LivenessOutcome:
    """
    Step the generator once more and record how it ended.

    Records rather than asserts, so the expensive live episode runs once per
    class and each test reads a field (the record-then-assert pattern of
    ``test_channel_fatal_blast_radius.py``). Three outcomes are distinguished:
    a terminal ERROR (the fix working), a clean StopAsyncIteration (the stream
    ended but told the client nothing), and a TIMEOUT (the pre-fix zombie).
    """
    loop = asyncio.get_running_loop()
    started = loop.time()
    try:
        record = await asyncio.wait_for(anext(generator), timeout=TERMINATION_TIMEOUT_S)
    except asyncio.TimeoutError:
        return LivenessOutcome(
            error=None, record=None, timed_out=True, elapsed=loop.time() - started
        )
    except StopAsyncIteration:
        return LivenessOutcome(
            error=None, record=None, timed_out=False, elapsed=loop.time() - started
        )
    # Any other exception IS the observation under test, so it is recorded
    # rather than allowed to error the fixture.
    except Exception as exc:  # noqa: BLE001
        return LivenessOutcome(
            error=exc, record=None, timed_out=False, elapsed=loop.time() - started
        )
    return LivenessOutcome(
        error=None, record=record, timed_out=False, elapsed=loop.time() - started
    )


async def _best_effort_delete_queue(connection, queue_name) -> None:
    """
    Delete a test-created queue on its own channel.

    A fresh channel per deletion because a failed delete (a 404 on a queue the
    test already removed) closes the channel and would take any later cleanup
    step down with it.
    """
    channel = await connection.channel()
    try:
        await channel.queue_delete(queue_name)
    except Exception:  # noqa: BLE001 - cleanup is best-effort
        logging.getLogger(__name__).debug(
            "Best-effort cleanup could not delete queue %r", queue_name, exc_info=True
        )
    finally:
        try:
            await channel.close()
        except Exception:  # noqa: BLE001 - cleanup is best-effort
            pass


async def _await_message_count(connection, queue_name, expected) -> int:
    """
    Poll the real queue until it holds ``expected`` messages, or time out.

    A passive declare is the only way to read this without consuming, and it
    is polled rather than read once because the requeue it observes happens in
    the loop's cleanup task -- scheduled with ``asyncio.create_task``, so it
    completes on its own schedule. Returns the last count seen either way, so
    a failing assertion reports the real number.
    """
    loop = asyncio.get_running_loop()
    deadline = loop.time() + REQUEUE_GRACE_S
    channel = await connection.channel()
    count = -1
    try:
        while True:
            declared = await channel.declare_queue(
                queue_name, passive=True, robust=False
            )
            count = declared.declaration_result.message_count
            if count == expected or loop.time() >= deadline:
                return count
            await asyncio.sleep(REQUEUE_POLL_S)
    finally:
        try:
            await channel.close()
        except Exception:  # noqa: BLE001 - cleanup is best-effort
            pass


@pytest_asyncio.fixture(scope="class", loop_scope="class")
async def live_consume_stream(class_live_broker_connection, class_mocker):
    """
    A REAL consume generator, live on the broker and proven to be delivering.

    Steps, all against the live broker:

    1. Shorten ``consumer_liveness_poll_seconds`` to the settings floor and
       point ``RabbitMQBase.connector`` at the live connection, so the
       liveness check's classifying ``_assert_queue_exists`` reaches the real
       broker on a real temporary channel.
    2. Declare a uuid-named topic exchange and a uuid-named queue bound to a
       uuid routing key. The queue is NOT ``auto_delete``: the channel-fatal
       arm needs it to OUTLIVE its consumer, so that a lost consumer over a
       surviving queue classifies as ``ConsumerCancelledError`` rather than
       ``QueueNotFoundError``. Both declares use ``robust=False``, matching
       production.
    3. Start the REAL ``_consume_with_client_acks`` on it, publish one
       message, and require it to come out of the generator -- the non-vacuity
       control recorded as ``baseline_record``.

    Yields the rig; teardown closes the generator and deletes every broker
    object it created.
    """
    connection = class_live_broker_connection

    class_mocker.patch.object(
        settings.settings, "consumer_liveness_poll_seconds", TEST_POLL_SECONDS
    )
    class_mocker.patch.object(RabbitMQBase.connector, "_connection", connection)
    class_mocker.patch.object(RabbitMQBase.connector, "closing", False)

    suffix = uuid.uuid4()
    exchange_name = f"liveness-x-{suffix}"
    queue_name = f"liveness-q-{suffix}"
    routing_key = f"liveness.rk.{suffix}"

    setup_channel = None
    consume_channel = None
    generator = None
    try:
        setup_channel = await connection.channel()
        exchange = await setup_channel.declare_exchange(
            exchange_name, type="topic", durable=False, auto_delete=False
        )

        consume_channel = await connection.channel()
        queue = await consume_channel.declare_queue(
            queue_name, durable=False, auto_delete=False, robust=False
        )
        await queue.bind(exchange_name, routing_key)

        async def _authz(_message):
            return True

        generator = _REAL_CONSUME_WITH_CLIENT_ACKS(queue, _authz, AckTracker())

        await exchange.publish(
            aio_pika.Message(body=BASELINE_BODY), routing_key=routing_key
        )
        try:
            baseline_record = await asyncio.wait_for(
                anext(generator), timeout=BASELINE_TIMEOUT_S
            )
        except asyncio.TimeoutError:
            pytest.fail(
                "the real consume generator never delivered the baseline "
                f"message within {BASELINE_TIMEOUT_S}s; the live rig is broken, "
                "so nothing this module measures would mean anything"
            )

        yield SimpleNamespace(
            connection=connection,
            exchange=exchange,
            channel=consume_channel,
            queue=queue,
            queue_name=queue_name,
            routing_key=routing_key,
            generator=generator,
            baseline_record=baseline_record,
        )
    finally:
        if generator is not None:
            await generator.aclose()
        await _best_effort_delete_queue(connection, queue_name)
        for channel in (consume_channel, setup_channel):
            if channel is None:
                continue
            try:
                if channel is setup_channel:
                    await channel.exchange_delete(exchange_name)
                await channel.close()
            except Exception:  # noqa: BLE001 - cleanup is best-effort
                pass


@pytest_asyncio.fixture(scope="class", loop_scope="class")
async def queue_deleted_episode(live_consume_stream):
    """
    Delete the queue from a SEPARATE channel under the live stream.

    The separate channel matters: the delete must reach the consumer as the
    broker's own per-consumer ``Basic.Cancel`` -- which leaves the consuming
    channel OPEN and fires no close callback -- rather than as a channel error
    the stream could have noticed some other way.
    """
    stream = live_consume_stream
    delete_channel = await stream.connection.channel()
    try:
        await delete_channel.queue_delete(stream.queue_name)
        outcome = await _drive_to_termination(stream.generator)
        yield LivenessEpisode(
            stream=stream,
            outcome=outcome,
            channel_still_open=not stream.channel.is_closed,
        )
    finally:
        try:
            await delete_channel.close()
        except Exception:  # noqa: BLE001 - cleanup is best-effort
            pass


@pytest_asyncio.fixture(scope="class", loop_scope="class")
async def channel_fatal_episode(live_consume_stream):
    """
    Kill the consuming CHANNEL under the live stream, then measure.

    ``queue_delete(if_unused=True)`` issued on the consumer's own channel
    against its still-consumed queue draws the documented
    ``PRECONDITION_FAILED`` and is channel-fatal -- the same forcing technique
    ``test_channel_fatal_blast_radius.py`` uses. It stands in for the
    production ack-timeout kill, which is the same class of channel death.

    Records whether the RobustChannel reopened and which consumer tags it came
    back carrying, so the ``robust=False`` premise (recovery does NOT restore
    our consumer) is measured rather than assumed.
    """
    stream = live_consume_stream

    fault_error = None
    try:
        await stream.channel.queue_delete(stream.queue_name, if_unused=True)
    # The channel-fatal rejection IS the setup step here, so it is recorded
    # rather than allowed to error the fixture.
    except Exception as exc:  # noqa: BLE001
        fault_error = exc

    closed_immediately = stream.channel.is_closed
    outcome = await _drive_to_termination(stream.generator)
    reopened = await _await_reopen(stream.channel)
    tags_after_reopen = await _consumer_tags_on(stream.channel)

    yield LivenessEpisode(
        stream=stream,
        outcome=outcome,
        fault_error=fault_error,
        closed_immediately=closed_immediately,
        reopened=reopened,
        tags_after_reopen=tags_after_reopen,
    )


class TestLiveStreamPremise:
    """The rig really is a live stream -- the module's non-vacuity control."""

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_real_generator_delivers_before_any_fault(
        self, live_consume_stream
    ):
        # "The stream terminated" is meaningless for a stream that never
        # started, so every episode below builds on this having passed.
        assert live_consume_stream.baseline_record.message["event_type"] == (
            "liveness-probe"
        )


class TestQueueDeletedUnderALiveStream:
    """A queue deleted mid-stream ends the stream with NOT_FOUND."""

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_stream_does_not_stay_open(self, queue_deleted_episode):
        # THE regression assertion. Pre-fix the loop blocked forever on
        # ``messages_queue.get()``: the broker's Basic.Cancel was never turned
        # into a wake-up, so the client's stream reported healthy against a
        # queue that no longer existed -- for ~47 hours in the 2026-08-24
        # dev-cluster incident.
        assert queue_deleted_episode.outcome.timed_out is False, (
            "the consume generator was still open "
            f"{TERMINATION_TIMEOUT_S}s after its queue was deleted; this is "
            "the zombie stream issue b.9gj reports"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_stream_ends_with_queue_not_found(self, queue_deleted_episode):
        # The client's remedy is to REPROVISION, which it can only know from
        # a NOT_FOUND. A generator that merely stopped would leave it
        # reconnecting to a queue that is never coming back.
        assert isinstance(queue_deleted_episode.outcome.error, QueueNotFoundError), (
            "expected QueueNotFoundError after the queue was deleted, got "
            f"{queue_deleted_episode.outcome.error!r}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_stream_ends_promptly(self, queue_deleted_episode):
        assert queue_deleted_episode.outcome.elapsed < PROMPT_DETECTION_BUDGET_S

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_consuming_channel_was_never_closed(self, queue_deleted_episode):
        # Why a channel-close callback could never have caught this: a
        # per-consumer Basic.Cancel leaves the channel perfectly healthy.
        # Only the missing tag distinguishes it, which is exactly what the
        # liveness predicate reads.
        assert queue_deleted_episode.channel_still_open is True


class TestChannelFatalFaultUnderALiveStream:
    """A channel-fatal fault ends the stream with a reconnect signal."""

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_fault_really_killed_the_channel(self, channel_fatal_episode):
        # Non-vacuity for this arm: if the forcing technique stopped being
        # channel-fatal, every assertion below would be measuring nothing.
        assert channel_fatal_episode.closed_immediately is True

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_stream_does_not_stay_open(self, channel_fatal_episode):
        assert channel_fatal_episode.outcome.timed_out is False, (
            "the consume generator was still open "
            f"{TERMINATION_TIMEOUT_S}s after its channel was killed; this is "
            "the zombie stream issue b.9gj reports"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_stream_ends_with_consumer_cancelled(self, channel_fatal_episode):
        # The queue outlived the channel, so the client's remedy is to
        # RECONNECT, not to reprovision -- the classification the liveness
        # check pays a broker round trip to get right.
        assert isinstance(
            channel_fatal_episode.outcome.error, ConsumerCancelledError
        ), (
            "expected ConsumerCancelledError after a channel-fatal fault over "
            f"a surviving queue, got {channel_fatal_episode.outcome.error!r}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_stream_ends_promptly(self, channel_fatal_episode):
        assert channel_fatal_episode.outcome.elapsed < PROMPT_DETECTION_BUDGET_S

    @pytest.mark.asyncio(loop_scope="class")
    async def test_channel_recovery_does_not_restore_the_consumer(
        self, channel_fatal_episode
    ):
        """
        The ``robust=False`` premise the liveness predicate depends on.

        aio-pika's RobustChannel reopens after a channel-fatal error, and if
        it restored the consumer the tag would reappear and the predicate
        would go blind -- a stream that looks alive on a consumer the broker
        no longer feeds. Production declares with ``robust=False`` precisely
        so the channel comes back with an EMPTY consumer registry.
        """
        assert channel_fatal_episode.reopened is True
        assert channel_fatal_episode.tags_after_reopen == set()


@pytest_asyncio.fixture(scope="class", loop_scope="class")
async def broker_cancel_over_a_surviving_queue_episode(live_consume_stream):
    """
    A BROKER-initiated Basic.Cancel with the queue alive at classify time.

    This is the branch a single-node rig cannot reach head-on. RabbitMQ sends
    a per-consumer ``Basic.Cancel`` in exactly three situations: the queue is
    deleted, a quorum queue loses its majority, or a mirrored queue fails over
    with ``x-cancel-on-ha-failover``. The last is gone in 4.x, the middle one
    needs a multi-node cluster to lose a node, and the rig has AMQP plus the
    management HTTP API only -- no ``rabbitmqctl``, no second node, and no
    management endpoint that cancels a consumer.

    So the cancel is induced by the one path that exists, a queue delete, and
    the queue is then RE-DECLARED under the same name before the loop is
    stepped again. The cancel is genuinely the broker's; by the time the poll
    fires, the classifying round trip finds a queue that is really there, and
    the branch under test -- consumer gone, channel open, queue alive -- is
    reached with every link real.

    The re-declare is not racing the poll: an async generator runs only while
    someone is awaiting it, and the fixture parked at ``yield`` after the
    baseline message. Nothing polls between the delete and the re-declare.
    """
    stream = live_consume_stream
    admin_channel = await stream.connection.channel()
    try:
        await admin_channel.queue_delete(stream.queue_name)
        await admin_channel.declare_queue(
            stream.queue_name, durable=False, auto_delete=False, robust=False
        )

        outcome = await _drive_to_termination(stream.generator)
        yield LivenessEpisode(
            stream=stream,
            outcome=outcome,
            channel_still_open=not stream.channel.is_closed,
            tags_after_cancel=await _consumer_tags_on(stream.channel),
        )
    finally:
        try:
            await admin_channel.close()
        except Exception:  # noqa: BLE001 - cleanup is best-effort
            pass


@pytest_asyncio.fixture(scope="class", loop_scope="class")
async def live_channel_cancel_episode(live_consume_stream):
    """
    The same live-channel-cancel branch, measured on the real queue's depth.

    Where the episode above proves the CLASSIFICATION with a broker-initiated
    cancel, this one proves what CLEANUP does with the delivery still
    outstanding -- which the episode above cannot, because the delete that
    induced its cancel destroyed that delivery along with the queue.

    Here the queue is left completely alone and the cancel is a real AMQP
    ``basic.cancel`` round trip to the broker (``queue.cancel(tag)``, issued
    from outside the consume loop). It is client-initiated rather than
    broker-initiated, and that is the single respect in which it is a stand-in
    -- everything the code under test can observe is identical, because a
    cancel is a cancel once aiormq has dropped the tag: real registry, real
    open channel, real surviving queue, real delivery still unacked.

    Records the queue's depth after cleanup, which is the observable that
    matters: a per-consumer cancel on an open channel requeues NOTHING, so the
    delivery only comes back if the loop hands it back.
    """
    stream = live_consume_stream

    # The loop keeps its consumer tag to itself, so it is read back off the
    # live registry. The consuming channel carries exactly one consumer -- the
    # loop's -- so the unpacking is also a check on that premise.
    (consumer_tag,) = await _consumer_tags_on(stream.channel)
    await stream.queue.cancel(consumer_tag)

    outcome = await _drive_to_termination(stream.generator)
    requeued_count = await _await_message_count(
        stream.connection, stream.queue_name, expected=1
    )

    yield LivenessEpisode(
        stream=stream,
        outcome=outcome,
        channel_still_open=not stream.channel.is_closed,
        requeued_count=requeued_count,
    )


class TestBrokerCancelOverASurvivingQueue:
    """A broker cancel with the queue alive ends the stream with RECONNECT."""

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_broker_really_dropped_our_consumer(
        self, broker_cancel_over_a_surviving_queue_episode
    ):
        # Non-vacuity: the whole episode rests on the broker's Basic.Cancel
        # having reached aiormq and popped the tag. Nothing else in this class
        # would mean anything if the consumer were still registered.
        assert broker_cancel_over_a_surviving_queue_episode.tags_after_cancel == set()

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_channel_survived_the_cancel(
        self, broker_cancel_over_a_surviving_queue_episode
    ):
        # The other half of the premise, and the reason this branch exists
        # separately from the channel-fatal one: the channel is fine, so its
        # delivery tags are still live and cleanup still owes the broker its
        # nacks.
        assert broker_cancel_over_a_surviving_queue_episode.channel_still_open is True

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_stream_does_not_stay_open(
        self, broker_cancel_over_a_surviving_queue_episode
    ):
        assert (
            broker_cancel_over_a_surviving_queue_episode.outcome.timed_out is False
        ), (
            "the consume generator was still open "
            f"{TERMINATION_TIMEOUT_S}s after the broker cancelled its "
            "consumer; this is the zombie stream issue b.9gj reports"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_stream_ends_with_consumer_cancelled(
        self, broker_cancel_over_a_surviving_queue_episode
    ):
        # The queue is there, so the remedy is RECONNECT. Reporting NOT_FOUND
        # here would send the client off to reprovision a queue that never
        # went anywhere.
        assert isinstance(
            broker_cancel_over_a_surviving_queue_episode.outcome.error,
            ConsumerCancelledError,
        ), (
            "expected ConsumerCancelledError after a broker cancel over a "
            "surviving queue, got "
            f"{broker_cancel_over_a_surviving_queue_episode.outcome.error!r}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_error_records_the_channel_as_open(
        self, broker_cancel_over_a_surviving_queue_episode
    ):
        # The detection-time fact that routes cleanup. Read off a REAL channel
        # here rather than off a boolean a fake was told to report.
        assert (
            broker_cancel_over_a_surviving_queue_episode.outcome.error.channel_closed
            is False
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_stream_ends_promptly(
        self, broker_cancel_over_a_surviving_queue_episode
    ):
        assert (
            broker_cancel_over_a_surviving_queue_episode.outcome.elapsed
            < PROMPT_DETECTION_BUDGET_S
        )


class TestOutstandingDeliveryAfterALiveChannelCancel:
    """Cleanup hands the outstanding delivery back when the channel lives."""

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_channel_survived_the_cancel(self, live_channel_cancel_episode):
        # Non-vacuity: on a CLOSED channel the broker requeues by itself, so
        # the requeue below would prove nothing about our cleanup.
        assert live_channel_cancel_episode.channel_still_open is True

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_stream_ends_with_consumer_cancelled(
        self, live_channel_cancel_episode
    ):
        assert isinstance(
            live_channel_cancel_episode.outcome.error, ConsumerCancelledError
        ), (
            "expected ConsumerCancelledError after a cancel over a surviving "
            f"queue, got {live_channel_cancel_episode.outcome.error!r}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_outstanding_delivery_goes_back_on_the_queue(
        self, live_channel_cancel_episode
    ):
        # THE assertion this arm exists for, and the one the unit suite can
        # only approximate with a mock's call count. A per-consumer cancel
        # requeues nothing on the broker side, so the message is back on the
        # real queue only because cleanup nacked it. Skipping that nack -- as
        # an earlier revision did on every consumer-lost path -- would leave
        # it stranded unacked, burning the channel's prefetch budget for the
        # rest of that channel's life in the pool.
        assert live_channel_cancel_episode.requeued_count == 1
