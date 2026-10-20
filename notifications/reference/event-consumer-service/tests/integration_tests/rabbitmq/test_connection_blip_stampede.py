# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REAL-broker measurement: a connection blip under many concurrent streams.

Companion to ``test_consumer_liveness.py``. That module proves ONE stream
terminates correctly for each way a consumer can die. This module measures what
happens when the loss is CONNECTION-WIDE and MANY streams see it at once --
the stampede question the b.9gj fix added two protections for:

1. ``RabbitMQBase._liveness_poll_timeout`` jitters each idle poll deadline, so
   streams that were left idle by the same connection-level event do not all
   wake on the same instant.
2. ``RabbitMQBase._connection_is_down`` short-circuits the CLASSIFYING broker
   round trip. Classification is what costs the broker something: it opens a
   temporary channel through ``connector.get_temporary_channel`` and
   ``_assert_queue_exists`` retries that up to
   ``RabbitMQBase._QUEUE_EXISTS_RETRIES`` times, a second apart. Against a
   connection that is itself down, that traffic can only fail -- and every
   stream on the pod shares that one connection, so an unguarded classification
   is precisely a stampede at the broker's worst moment.

Neither protection is observable from a single-stream test, and neither is
provable from a unit double: the whole claim is about what a REAL
``RobustConnection`` reports mid-blip. So this module opens ``STREAM_COUNT``
real consume streams over one real connection, drops that connection, and
records three numbers the PM asked for: how many streams there were, how far
apart their detections landed, and how many temporary channels the
classification path opened during the outage.

The fault
---------
The connection is dropped by closing the AMQP transport underneath aio-pika --
``connection.transport.connection.close(exc)`` -- rather than by
``connection.close()``. The distinction is the whole point: aio-pika's
``close()`` sets ``_close_called`` and the RobustConnection then deliberately
does NOT reconnect, which is a clean local shutdown, not a blip. Closing the
underlay leaves ``_close_called`` False, so ``RobustConnection`` sees an
unexpected drop and starts its reconnect cycle -- exactly the production shape.
It is the same style of fault injection ``test_channel_fatal_blast_radius.py``
uses one level down (there, forcing a channel-level error to kill a channel).

Measured on the shipped stack (aio-pika 9.6.2 / RabbitMQ 4.x), the blip leaves
the connection reading ``is_closed=False connected=False`` -- which is exactly
why ``_connection_is_down`` reads the ``connected`` event and not ``is_closed``,
and this module is the live proof of that premise rather than a docstring
claim about it.

``BLIP_RECONNECT_INTERVAL_S`` deliberately holds the connection down for far
longer than one poll window. That is not padding for its own sake: the
zero-temporary-channels claim is only meaningful for detections that land while
the connection is genuinely down, so the outage has to comfortably outlast the
detection it is being measured against. A too-short outage would turn a real
regression into a coin flip.

Scope notes
-----------
- The autouse fakes from the parent conftests are bypassed exactly as in
  ``test_consumer_liveness.py``: the consume generator and ``connect_robust``
  are pre-patch references captured at import time.
- This module opens its OWN connection rather than using
  ``class_live_broker_connection``, because it needs a non-default
  ``reconnect_interval`` (above) and because it destroys the connection it is
  given -- a shared fixture connection would be a poor thing to hand back.
- Fixtures are CLASS-scoped with matching ``loop_scope`` so one live blip
  serves every assertion. See ``test_consumer_liveness.py`` for the measured
  caveat about xdist's ``--dist load`` re-running class fixtures per test.
- Runs in CI's ``integration:`` job against a live broker with
  ``OMNI_EVENTS_REQUIRE_LIVE_BROKER=1`` so a missing broker HARD-fails; skips
  cleanly in a dev sandbox with no broker. It needs no management API.
"""

import asyncio
import logging
import uuid
from types import SimpleNamespace

import aio_pika
import pytest
import pytest_asyncio
from notification_common.config.common_settings import common_settings

from app.config import settings
from app.lib.rabbitmq.ack_tracker import AckTracker
from app.lib.rabbitmq.rabbitmq_base import ConsumerCancelledError, RabbitMQBase
from tests.integration_tests.rabbitmq.conftest import (
    _live_broker_required,
    _REQUIRE_LIVE_BROKER_ENV,
)

# Captured at import time, before the autouse fixtures in the parent conftests
# replace them. Calling these references reaches the REAL broker and runs the
# REAL consume loop, which is the entire point of a live-broker arm.
_REAL_CONNECT_ROBUST = aio_pika.connect_robust
_REAL_CONSUME_WITH_CLIENT_ACKS = RabbitMQBase._consume_with_client_acks

# How many concurrent consume streams share the one connection. Sized to what a
# sandbox/CI rig comfortably supports while still being a crowd: a stampede that
# only shows up at four streams is not a stampede, and at 600 (the AC-1 burst
# figure) the measurement would be dominated by rig setup cost rather than by
# the behaviour under test.
STREAM_COUNT = 32

# Streams packed per channel, at the shipped non-durable ``client_limit``. The
# packing is not itself under test here -- the fault is connection-wide, so it
# takes every channel with it either way -- but consuming the way production
# consumes keeps the rig honest about how many channels a pod really holds.
STREAMS_PER_CHANNEL = 8
CHANNEL_COUNT = STREAM_COUNT // STREAMS_PER_CHANNEL

# The poll interval the streams run at. Above the settings floor, unlike
# ``test_consumer_liveness.py``'s 1.0, because this module is the one that
# measures the JITTER and the band is a FRACTION of the poll: the absolute
# spread scales with this number, so a longer window separates the jitter from
# scheduling noise by a wider margin. The band itself is live at every setting
# including the floor -- it stretches upward only, so nothing collapses onto
# the floor -- which is why 1.0 would also work, just with half the absolute
# spread to measure. 2.0 is still a setting a real deployment could configure
# (the shipped default is 15.0).
TEST_POLL_SECONDS = 2.0

# How long the connection stays down after the fault. aio-pika's default is 5s,
# which is close enough to the detection latency that a loaded CI box could let
# a straggler classify against an already-recovered connection and make the
# zero-temporary-channels assertion flaky for a reason unrelated to the fix.
# Twenty seconds puts the outage an order of magnitude clear of one poll window.
BLIP_RECONNECT_INTERVAL_S = 20.0

# The ceiling on "every stream ended". Generous enough that broker or CI latency
# cannot flake it, but bounded so the pre-fix behaviour -- streams that stay
# open forever -- fails as a NAMED timeout instead of hanging CI.
TERMINATION_TIMEOUT_S = 45.0

# The promptness budget, measured from the fault rather than from when each
# driver task started. Detection costs at most one jittered poll
# (``TEST_POLL_SECONDS`` x 1.15) plus scheduling; several times that is a wide
# margin that still fails a fix which only terminated "eventually".
PROMPT_DETECTION_BUDGET_S = 8.0

# The spread the jittered deadline is expected to produce, bounded from BOTH
# sides.
#
# Upper: every stream went idle on the same instant, so each one's next deadline
# is at most one jittered poll window away and the last detection can trail the
# first by at most that window plus scheduling slack.
#
# Lower: this is the assertion that the jitter is actually DOING something. With
# ``_LIVENESS_POLL_JITTER`` at 15% applied upward only, the deadlines are drawn
# from a 0.3s-wide band at this poll interval and 32 draws fill most of it
# (measured 0.276-0.288s across three episodes on RabbitMQ 4.0.5).
# Strip the jitter out and every stream fires exactly ``TEST_POLL_SECONDS``
# after a park time they all share, so the spread collapses to scheduling noise
# -- single-digit milliseconds. The 0.15s floor sits an order of magnitude above
# that noise while staying well under the expected spread, so it separates the
# two without being a coin flip in either direction.
DETECTION_SPREAD_BUDGET_S = 6.0
MIN_DETECTION_SPREAD_S = 0.15

# The non-vacuity control: how long to wait for each stream's first message to
# arrive through the real generator before the connection is touched.
BASELINE_TIMEOUT_S = 30.0

# How long to wait for the RobustConnection to report itself down after the
# fault. Measured at ~0.02s; the budget is wide because a loaded box schedules
# aiormq's close callbacks on its own time.
DOWN_OBSERVATION_TIMEOUT_S = 15.0
DOWN_POLL_S = 0.02

# The connect timeout for this module's own connection. Deliberately looser than
# the conftest's 5s: a cold first connection through a container gateway has
# been measured at ~2s, and this rig then does 32 declares on top.
CONNECT_TIMEOUT_S = 30.0

# Backstop reaping for the rig's queues, so a run killed before teardown (a
# SIGPIPE from a piped pytest invocation will do it) does not strand
# STREAM_COUNT queues on the broker. Long enough that it can never fire during
# an episode, short enough that stragglers do not accumulate.
QUEUE_EXPIRES_MS = 10 * 60 * 1000

BASELINE_BODY = b'{"event_type": "blip-probe", "message": {"test": "m"}}'


class BlipOutcome(SimpleNamespace):
    """How one consume generator ended, and when."""


class BlipEpisode(SimpleNamespace):
    """The measured record of one live connection-blip episode."""


class TemporaryChannelCounter:
    """
    A counting pass-through for ``connector.get_temporary_channel``.

    That method is the ONLY door the classifying round trip goes through
    (``RabbitMQBase._assert_queue_exists`` opens one temporary channel per
    attempt), so counting calls to it counts exactly the broker traffic the
    connection-down short-circuit exists to suppress. It delegates to the real
    method rather than faking it, so a stream that DID classify really would
    open a channel and really would be counted -- the assertion is not resting
    on a stub that quietly does nothing.
    """

    def __init__(self, wrapped):
        self._wrapped = wrapped
        self.calls = 0

    def __call__(self, *args, **kwargs):
        self.calls += 1
        return self._wrapped(*args, **kwargs)


async def _connect_or_skip():
    """
    Open this module's own AMQP connection, or skip/fail per the environment.

    Mirrors the conftest helper's promise -- a connect failure is a HARD failure
    when ``OMNI_EVENTS_REQUIRE_LIVE_BROKER`` is set and a clean skip otherwise --
    but supplies the ``reconnect_interval`` this module's fault depends on.
    """
    try:
        return await _REAL_CONNECT_ROBUST(
            common_settings.rabbitmq_uri,
            timeout=CONNECT_TIMEOUT_S,
            reconnect_interval=BLIP_RECONNECT_INTERVAL_S,
        )
    except Exception as exc:  # noqa: BLE001 - any connect failure -> skip or fail
        message = (
            "RabbitMQ broker not reachable at "
            f"{common_settings.rabbitmq_uri!r} ({exc!r})"
        )
        if _live_broker_required():
            pytest.fail(
                f"{message}; {_REQUIRE_LIVE_BROKER_ENV} is set, so a live broker "
                "is required and its absence is a hard failure (not a skip)"
            )
        pytest.skip(f"{message}; skipping live-broker test")


async def _await_connection_down() -> float | None:
    """
    Poll until the connector reports its connection down, returning when.

    Returns ``None`` if it never did, which the premise assertion reports as a
    fault that failed to land -- every other measurement here would be vacuous
    on a connection that never actually dropped.
    """
    loop = asyncio.get_running_loop()
    started = loop.time()
    deadline = started + DOWN_OBSERVATION_TIMEOUT_S
    while loop.time() < deadline:
        if RabbitMQBase._connection_is_down():
            return loop.time()
        await asyncio.sleep(DOWN_POLL_S)
    return None


async def _drive_to_termination(index, generator) -> BlipOutcome:
    """
    Step one generator once more and record how, and when, it ended.

    The end time is absolute (the running loop's clock) rather than an elapsed
    span, because every latency here is measured from the FAULT -- a shared
    instant these drivers are all parked before -- not from when each driver
    happened to start.
    """
    loop = asyncio.get_running_loop()
    try:
        record = await asyncio.wait_for(anext(generator), timeout=TERMINATION_TIMEOUT_S)
    except asyncio.TimeoutError:
        return BlipOutcome(
            index=index, error=None, record=None, timed_out=True, ended_at=loop.time()
        )
    except StopAsyncIteration:
        return BlipOutcome(
            index=index, error=None, record=None, timed_out=False, ended_at=loop.time()
        )
    # Any other exception IS the observation under test, so it is recorded
    # rather than allowed to error the fixture.
    except Exception as exc:  # noqa: BLE001
        return BlipOutcome(
            index=index, error=exc, record=None, timed_out=False, ended_at=loop.time()
        )
    return BlipOutcome(
        index=index, error=None, record=record, timed_out=False, ended_at=loop.time()
    )


async def _best_effort_delete_queues(queue_names) -> None:
    """
    Delete the test-created queues over a FRESH connection.

    Deliberately NOT over the episode's connection: that one was destroyed
    seconds earlier and is still inside its ``BLIP_RECONNECT_INTERVAL_S``
    backoff when teardown runs, so every channel open on it fails and the queues
    survive the run. (They did, during this module's development -- 32 stranded
    queues per run, silently, because the failures are best-effort by design.)

    A fresh channel per deletion because a failed delete closes the channel and
    would take every later deletion down with it. The queues additionally carry
    ``x-expires`` (see the fixture), so even a run killed before teardown ever
    ran self-cleans.
    """
    log = logging.getLogger(__name__)
    try:
        connection = await _REAL_CONNECT_ROBUST(
            common_settings.rabbitmq_uri, timeout=CONNECT_TIMEOUT_S
        )
    except Exception:  # noqa: BLE001 - cleanup is best-effort
        log.warning(
            "Could not reconnect to delete %d test queues; they carry "
            "x-expires and the broker will reap them.",
            len(queue_names),
            exc_info=True,
        )
        return
    try:
        for queue_name in queue_names:
            channel = await connection.channel()
            try:
                await channel.queue_delete(queue_name)
            except Exception:  # noqa: BLE001 - cleanup is best-effort
                log.debug(
                    "Best-effort cleanup could not delete queue %r",
                    queue_name,
                    exc_info=True,
                )
            finally:
                try:
                    await channel.close()
                except Exception:  # noqa: BLE001 - cleanup is best-effort
                    pass
    finally:
        await connection.close()


@pytest_asyncio.fixture(scope="class", loop_scope="class")
async def blip_episode(class_mocker):
    """
    Open ``STREAM_COUNT`` real streams on one connection, drop it, measure.

    Steps, all against the live broker:

    1. Shorten ``consumer_liveness_poll_seconds`` to the settings floor, point
       ``RabbitMQBase.connector`` at this module's connection (the liveness
       check's classifying step reaches the broker through the connector), and
       wrap ``connector.get_temporary_channel`` in a counter.
    2. Open ``CHANNEL_COUNT`` channels and, on each, ``STREAMS_PER_CHANNEL``
       uuid-named queues consumed through the REAL
       ``_consume_with_client_acks``. Queues are declared ``robust=False``,
       matching production, so aio-pika's recovery does not restore the
       consumers behind the fix's back.
    3. Baseline: publish one message per queue and require every stream to
       deliver it. "The streams terminated" proves nothing about streams that
       never started.
    4. Park a driver task on every generator, then drop the connection by
       closing the transport underneath aio-pika.
    5. Record when the connection was observed down, when each stream ended and
       with what, and how many temporary channels the classification path
       opened between the fault and the last termination.

    Yields the record; teardown closes the generators, deletes every queue and
    closes the connection.
    """
    connection = await _connect_or_skip()

    class_mocker.patch.object(
        settings.settings, "consumer_liveness_poll_seconds", TEST_POLL_SECONDS
    )
    class_mocker.patch.object(RabbitMQBase.connector, "_connection", connection)
    class_mocker.patch.object(RabbitMQBase.connector, "closing", False)
    temp_channels = TemporaryChannelCounter(
        RabbitMQBase.connector.get_temporary_channel
    )
    class_mocker.patch.object(
        RabbitMQBase.connector, "get_temporary_channel", temp_channels
    )

    suffix = uuid.uuid4()
    channels = []
    generators = []
    queue_names = []
    publish_channel = None
    try:
        for channel_index in range(CHANNEL_COUNT):
            channel = await connection.channel()
            channels.append(channel)
            for slot in range(STREAMS_PER_CHANNEL):
                queue_name = f"blip-q-{suffix}-{channel_index}-{slot}"
                # NOT auto_delete: the queue has to OUTLIVE its consumer, so
                # that a stream whose consumer the blip took away classifies as
                # ConsumerCancelledError rather than QueueNotFoundError.
                # x-expires is the backstop for a run that dies before teardown
                # -- it is also what production declares non-durable queues
                # with, so it costs no realism.
                queue = await channel.declare_queue(
                    queue_name,
                    durable=False,
                    auto_delete=False,
                    arguments={"x-expires": QUEUE_EXPIRES_MS},
                    robust=False,
                )
                queue_names.append(queue_name)

                async def _authz(_message):
                    return True

                generators.append(
                    _REAL_CONSUME_WITH_CLIENT_ACKS(queue, _authz, AckTracker())
                )

        publish_channel = await connection.channel()
        for queue_name in queue_names:
            await publish_channel.default_exchange.publish(
                aio_pika.Message(body=BASELINE_BODY), routing_key=queue_name
            )

        baseline_delivered = 0
        for index, generator in enumerate(generators):
            try:
                record = await asyncio.wait_for(
                    anext(generator), timeout=BASELINE_TIMEOUT_S
                )
            except asyncio.TimeoutError:
                pytest.fail(
                    f"stream {index} of {STREAM_COUNT} never delivered its "
                    f"baseline message within {BASELINE_TIMEOUT_S}s; the live "
                    "rig is broken, so nothing this module measures would mean "
                    "anything"
                )
            if record.message["event_type"] == "blip-probe":
                baseline_delivered += 1

        # Every driver has to be parked on the generator BEFORE the fault, so
        # each one's next poll deadline is already ticking when the connection
        # goes. Gathering on per-driver "started" events does that without a
        # bare sleep to "let things happen".
        started_events = [asyncio.Event() for _ in generators]

        async def _driver(index, generator, started):
            started.set()
            return await _drive_to_termination(index, generator)

        tasks = [
            asyncio.create_task(_driver(index, generator, started_events[index]))
            for index, generator in enumerate(generators)
        ]
        await asyncio.gather(*(event.wait() for event in started_events))

        temp_channels_before_fault = temp_channels.calls
        loop = asyncio.get_running_loop()
        fault_at = loop.time()
        # NOT connection.close(): that sets aio-pika's ``_close_called`` and
        # suppresses the reconnect, making it a clean shutdown rather than a
        # blip. Closing the transport underneath leaves the RobustConnection
        # believing it lost the broker, which is the production shape.
        await connection.transport.connection.close(
            ConnectionError("simulated connection blip")
        )
        down_at = await _await_connection_down()

        outcomes = await asyncio.gather(*tasks)
        temp_channels_during_outage = temp_channels.calls - temp_channels_before_fault

        latencies = [outcome.ended_at - fault_at for outcome in outcomes]
        episode = BlipEpisode(
            baseline_delivered=baseline_delivered,
            channel_count=len(channels),
            outcomes=outcomes,
            latencies=latencies,
            down_latency=None if down_at is None else down_at - fault_at,
            temp_channels_before_fault=temp_channels_before_fault,
            temp_channels_during_outage=temp_channels_during_outage,
        )
        # Emitted, not just asserted on: the numbers are the deliverable here,
        # and a green run should still hand them to whoever asked for them
        # (``pytest -o log_cli=true --log-cli-level=INFO``). The assertions
        # below restate them on failure.
        logging.getLogger(__name__).info("%s", _measurement(episode))
        yield episode
    finally:
        for generator in generators:
            try:
                await generator.aclose()
            except Exception:  # noqa: BLE001 - cleanup is best-effort
                pass
        for channel in channels + [publish_channel]:
            if channel is None:
                continue
            try:
                await channel.close()
            except Exception:  # noqa: BLE001 - cleanup is best-effort
                pass
        # Release the blipped connection BEFORE the deletes: it is mid-backoff
        # and unusable, and leaving it reconnecting would have the cleanup
        # connection racing a revival nobody wants.
        await connection.close()
        await _best_effort_delete_queues(queue_names)


def _measurement(episode) -> str:
    """The measured numbers, restated on every failure so the run is the record."""
    return (
        f"[measured] streams={STREAM_COUNT} on {episode.channel_count} channels; "
        f"first detection {min(episode.latencies):.3f}s after the fault, last "
        f"{max(episode.latencies):.3f}s, spread "
        f"{max(episode.latencies) - min(episode.latencies):.3f}s; temporary "
        f"channels opened during the outage: {episode.temp_channels_during_outage}"
    )


class TestBlipPremise:
    """The rig really was many live streams on one connection that really dropped."""

    @pytest.mark.asyncio(loop_scope="class")
    async def test_every_stream_delivered_before_the_blip(self, blip_episode):
        # Non-vacuity for the whole module: a stream that never delivered would
        # "terminate promptly" for reasons having nothing to do with the fix.
        assert blip_episode.baseline_delivered == STREAM_COUNT

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_streams_were_packed_onto_shared_channels(self, blip_episode):
        # Production packs streams onto shared channels; a rig that quietly
        # spread 32 streams over 32 channels would be measuring a different
        # shape of pod than the one that hits this in production.
        assert blip_episode.channel_count == CHANNEL_COUNT

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_connection_was_observed_down(self, blip_episode):
        # The fault has to actually land. This is also the live proof of the
        # premise ``_connection_is_down`` rests on -- that a dropped
        # RobustConnection is detectable at all -- since the predicate reads
        # the ``connected`` event precisely because ``is_closed`` stays False
        # through a blip.
        assert blip_episode.down_latency is not None, (
            "the connector never reported its connection down after the "
            f"transport was closed; {_measurement(blip_episode)}"
        )


class TestEveryStreamTerminates:
    """A connection-wide loss ends every stream with the reconnect signal."""

    @pytest.mark.asyncio(loop_scope="class")
    async def test_no_stream_stayed_open(self, blip_episode):
        # THE regression assertion, at scale. Pre-fix each loop blocked forever
        # on ``messages_queue.get()``, so a connection blip left every stream on
        # the pod reporting healthy against a consumer the broker had dropped.
        stuck = [
            outcome.index for outcome in blip_episode.outcomes if outcome.timed_out
        ]
        assert stuck == [], (
            f"{len(stuck)} of {STREAM_COUNT} consume generators were still open "
            f"{TERMINATION_TIMEOUT_S}s after the connection dropped (indices "
            f"{stuck}); this is the zombie stream issue b.9gj reports. "
            f"{_measurement(blip_episode)}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_every_stream_ends_with_consumer_cancelled(self, blip_episode):
        # The queues all still exist, so the client's remedy is RECONNECT.
        # Reporting NOT_FOUND here would send every client off to reprovision a
        # queue that never went anywhere.
        wrong = [
            (outcome.index, repr(outcome.error))
            for outcome in blip_episode.outcomes
            if not isinstance(outcome.error, ConsumerCancelledError)
        ]
        assert wrong == [], (
            f"{len(wrong)} of {STREAM_COUNT} streams ended with something other "
            f"than ConsumerCancelledError: {wrong[:5]}. {_measurement(blip_episode)}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_every_stream_reports_its_channel_as_closed(self, blip_episode):
        # The connection-down short-circuit raises with ``channel_closed=True``
        # deliberately: the broker requeues every unacked delivery on a
        # connection it has lost, so cleanup must NOT then nack them (see
        # ``_should_nack_outstanding``). Getting this wrong would put 32
        # doomed nacks onto a recovering broker.
        wrong = [
            outcome.index
            for outcome in blip_episode.outcomes
            if not getattr(outcome.error, "channel_closed", None)
        ]
        assert wrong == [], (
            f"{len(wrong)} of {STREAM_COUNT} streams reported channel_closed "
            f"False after a connection-wide loss (indices {wrong[:5]}); their "
            "cleanup would nack deliveries the broker already requeued. "
            f"{_measurement(blip_episode)}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_every_stream_terminates_within_one_poll_window(self, blip_episode):
        assert max(blip_episode.latencies) < PROMPT_DETECTION_BUDGET_S, (
            "the slowest stream took "
            f"{max(blip_episode.latencies):.3f}s to notice a connection-wide "
            f"loss, over the {PROMPT_DETECTION_BUDGET_S}s budget. "
            f"{_measurement(blip_episode)}"
        )


class TestNoClassificationStampede:
    """
    The classification path opens NO temporary channels while the broker is gone.

    This is the measurement the fix's second protection exists for. Without the
    ``_connection_is_down`` short-circuit each of the ``STREAM_COUNT`` streams
    would enter ``_assert_queue_exists``, which opens a temporary channel and
    retries it ``RabbitMQBase._QUEUE_EXISTS_RETRIES`` times a second apart --
    up to ``STREAM_COUNT x _QUEUE_EXISTS_RETRIES`` doomed channel opens aimed at
    a connection that is down, from every pod at once.
    """

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_classification_opened_zero_temporary_channels(
        self, blip_episode
    ):
        worst_case = STREAM_COUNT * RabbitMQBase._QUEUE_EXISTS_RETRIES
        assert blip_episode.temp_channels_during_outage == 0, (
            "the connection-down short-circuit did not hold: the classification "
            f"path opened {blip_episode.temp_channels_during_outage} temporary "
            f"channels against a down connection (unguarded worst case is "
            f"{worst_case}). {_measurement(blip_episode)}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_detections_land_inside_one_jittered_poll_window(
        self, blip_episode
    ):
        # The envelope: the last stream to notice trails the first by no more
        # than one poll window, so nothing is straggling into the recovered
        # connection where it would classify after all.
        spread = max(blip_episode.latencies) - min(blip_episode.latencies)
        assert spread < DETECTION_SPREAD_BUDGET_S, (
            f"detections spread over {spread:.3f}s, more than the "
            f"{DETECTION_SPREAD_BUDGET_S}s a single jittered "
            f"{TEST_POLL_SECONDS}s poll window allows. {_measurement(blip_episode)}"
        )

    @pytest.mark.asyncio(loop_scope="class")
    async def test_the_jittered_deadline_spreads_the_detections_out(self, blip_episode):
        # The first protection, measured rather than assumed. All 32 streams
        # were left idle by the same connection-level event; on an unjittered
        # deadline they would all wake within scheduling noise of each other and
        # arrive at the broker as one burst. A spread that collapses to
        # milliseconds is exactly what removing the jitter looks like.
        spread = max(blip_episode.latencies) - min(blip_episode.latencies)
        assert spread > MIN_DETECTION_SPREAD_S, (
            f"the {STREAM_COUNT} detections landed within {spread:.3f}s of each "
            f"other, under the {MIN_DETECTION_SPREAD_S}s that a jittered "
            f"{TEST_POLL_SECONDS}s poll window should produce -- the streams "
            f"woke as a burst. {_measurement(blip_episode)}"
        )
