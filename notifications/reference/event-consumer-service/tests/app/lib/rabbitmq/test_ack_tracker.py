# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import AsyncMock

import aio_pika
import pytest

from app.lib.rabbitmq.ack_tracker import AckTracker
from tests.fixtures.dead_channel import (
    builtin_connection_error,
    dead_channel_failures,
    unrelated_runtime_error,
)


def _mock_message(delivery_tag: int) -> AsyncMock:
    msg = AsyncMock()
    msg.delivery_tag = delivery_tag
    return msg


class TestTrack:
    def test_returns_delivery_tag(self):
        tracker = AckTracker()
        msg = _mock_message(42)

        tag = tracker.track(msg)

        assert tag == 42

    def test_increments_in_flight_count(self):
        tracker = AckTracker()

        tracker.track(_mock_message(1))
        tracker.track(_mock_message(2))

        assert tracker.in_flight_count == 2


class TestOriginQueue:
    def test_caches_origin_queue_when_provided(self):
        tracker = AckTracker()

        tracker.track(_mock_message(1), origin_queue="origin-queue")

        assert tracker.origin_queue(1) == "origin-queue"

    def test_returns_none_when_tracked_without_origin_queue(self):
        tracker = AckTracker()
        tracker.track(_mock_message(1))

        assert tracker.origin_queue(1) is None

    def test_returns_none_for_unknown_tag(self):
        tracker = AckTracker()

        assert tracker.origin_queue(999) is None

    def test_does_not_cache_explicit_none_origin_queue(self):
        tracker = AckTracker()

        tracker.track(_mock_message(1), origin_queue=None)

        assert tracker.origin_queue(1) is None

    @pytest.mark.asyncio
    async def test_evicted_after_ack(self):
        tracker = AckTracker()
        tracker.track(_mock_message(1), origin_queue="origin-queue")

        await tracker.ack(1)

        assert tracker.origin_queue(1) is None

    @pytest.mark.asyncio
    async def test_evicted_after_nack(self):
        tracker = AckTracker()
        tracker.track(_mock_message(1), origin_queue="origin-queue")

        await tracker.nack(1)

        assert tracker.origin_queue(1) is None

    @pytest.mark.asyncio
    async def test_cleared_after_nack_all(self):
        tracker = AckTracker()
        tracker.track(_mock_message(1), origin_queue="queue-a")
        tracker.track(_mock_message(2), origin_queue="queue-b")

        await tracker.nack_all()

        assert tracker.origin_queue(1) is None
        assert tracker.origin_queue(2) is None


class TestAck:
    @pytest.mark.asyncio
    async def test_acks_the_message(self):
        tracker = AckTracker()
        msg = _mock_message(1)
        tracker.track(msg)

        await tracker.ack(1)

        msg.ack.assert_awaited_once()

    @pytest.mark.asyncio
    async def test_removes_from_in_flight(self):
        tracker = AckTracker()
        tracker.track(_mock_message(1))

        await tracker.ack(1)

        assert tracker.in_flight_count == 0

    @pytest.mark.asyncio
    async def test_raises_key_error_for_unknown_tag(self):
        tracker = AckTracker()

        with pytest.raises(KeyError):
            await tracker.ack(999)


class TestNack:
    @pytest.mark.asyncio
    async def test_nacks_the_message_with_requeue_by_default(self):
        # Both production callers want the message back on a queue; an ack
        # already covers "drop and forget", so requeue must default on.
        tracker = AckTracker()
        msg = _mock_message(1)
        tracker.track(msg)

        await tracker.nack(1)

        msg.nack.assert_awaited_once_with(requeue=True)

    @pytest.mark.asyncio
    async def test_forwards_requeue_false(self):
        tracker = AckTracker()
        msg = _mock_message(1)
        tracker.track(msg)

        await tracker.nack(1, requeue=False)

        msg.nack.assert_awaited_once_with(requeue=False)

    @pytest.mark.asyncio
    async def test_removes_from_in_flight(self):
        tracker = AckTracker()
        tracker.track(_mock_message(1))

        await tracker.nack(1)

        assert tracker.in_flight_count == 0

    @pytest.mark.asyncio
    async def test_leaves_other_tags_in_flight(self):
        tracker = AckTracker()
        tracker.track(_mock_message(1))
        msg2 = _mock_message(2)
        tracker.track(msg2)

        await tracker.nack(1)

        assert tracker.in_flight_count == 1
        msg2.nack.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_raises_key_error_for_unknown_tag(self):
        tracker = AckTracker()

        with pytest.raises(KeyError):
            await tracker.nack(999)

    @pytest.mark.asyncio
    async def test_raises_key_error_for_already_nacked_tag(self):
        # The already-settled case a client repeat-nack hits. It must raise
        # rather than nack a delivery the broker has already taken back.
        tracker = AckTracker()
        tracker.track(_mock_message(1))
        await tracker.nack(1)

        with pytest.raises(KeyError):
            await tracker.nack(1)

    @pytest.mark.asyncio
    async def test_nacked_message_is_not_nacked_again_by_nack_all(self):
        # The pop happens before the nack, so a client-nacked delivery is
        # gone from the registry by the time stream cleanup drains it.
        tracker = AckTracker()
        msg = _mock_message(1)
        tracker.track(msg)

        await tracker.nack(1)
        await tracker.nack_all()

        msg.nack.assert_awaited_once_with(requeue=True)


class TestNackAll:
    @pytest.mark.asyncio
    async def test_nacks_all_in_flight_messages(self):
        tracker = AckTracker()
        msg1 = _mock_message(1)
        msg2 = _mock_message(2)
        tracker.track(msg1)
        tracker.track(msg2)

        await tracker.nack_all()

        msg1.nack.assert_awaited_once_with(requeue=True)
        msg2.nack.assert_awaited_once_with(requeue=True)

    @pytest.mark.asyncio
    async def test_clears_in_flight_after_nack(self):
        tracker = AckTracker()
        tracker.track(_mock_message(1))

        await tracker.nack_all()

        assert tracker.in_flight_count == 0

    @pytest.mark.asyncio
    async def test_continues_on_nack_failure(self):
        tracker = AckTracker()
        msg1 = _mock_message(1)
        msg1.nack.side_effect = Exception("connection lost")
        msg2 = _mock_message(2)
        tracker.track(msg1)
        tracker.track(msg2)

        await tracker.nack_all()

        msg2.nack.assert_awaited_once_with(requeue=True)
        assert tracker.in_flight_count == 0


class TestNackAllUnderConcurrentSettles:
    """
    ``nack_all`` runs in a detached cleanup task and awaits once per
    message, so a client ack or nack can land on the tracker during any of
    those awaits. Draining by popping is what makes that safe; iterating
    the live dict raised ``RuntimeError: dictionary changed size during
    iteration`` and abandoned every message the drain had not yet reached,
    neither acked nor nacked.
    """

    @pytest.mark.asyncio
    async def test_completes_when_a_delivery_is_tracked_mid_drain(self):
        # A message arriving during cleanup GROWS the registry mid-drain.
        # Asserted on the late message rather than on the absence of an
        # exception, so the test still means something if the failure mode
        # ever changes from RuntimeError to a silent early exit.
        tracker = AckTracker()
        late_message = _mock_message(99)
        tracked_late = False

        async def _track_late_delivery(requeue=True):
            nonlocal tracked_late
            if tracked_late:
                return
            tracked_late = True
            tracker.track(late_message)

        for tag in (1, 2, 3):
            message = _mock_message(tag)
            message.nack.side_effect = _track_late_delivery
            tracker.track(message)

        await tracker.nack_all()

        late_message.nack.assert_awaited_once_with(requeue=True)
        assert tracker.in_flight_count == 0

    @pytest.mark.asyncio
    async def test_every_message_is_settled_exactly_once(self):
        # A client ack landing during the drain must not leave a delivery
        # settled twice -- the pop-before-await ordering routes it to the
        # tracker's unknown-tag KeyError instead. The client targets a fixed
        # tag and tolerates the KeyError, so the test does not depend on the
        # order the drain happens to pop in; the exactly-once invariant
        # below holds either way.
        tracker = AckTracker()
        messages = {tag: _mock_message(tag) for tag in (1, 2, 3)}
        client_ack_sent = False

        async def _client_acks_tag_two(requeue=True):
            nonlocal client_ack_sent
            if client_ack_sent:
                return
            client_ack_sent = True
            try:
                await tracker.ack(2)
            except KeyError:
                pass

        for message in messages.values():
            message.nack.side_effect = _client_acks_tag_two
            tracker.track(message)

        await tracker.nack_all()

        assert tracker.in_flight_count == 0
        settle_counts = {
            tag: message.ack.await_count + message.nack.await_count
            for tag, message in messages.items()
        }
        assert settle_counts == {1: 1, 2: 1, 3: 1}

    @pytest.mark.asyncio
    async def test_a_drained_tag_is_unknown_to_a_later_client_settle(self):
        # The other direction: once the drain has popped a tag, a client ack
        # for it takes the KeyError path (which the servicer warns about and
        # skips) rather than acking a delivery the broker already took back.
        tracker = AckTracker()
        tracker.track(_mock_message(1))

        await tracker.nack_all()

        with pytest.raises(KeyError):
            await tracker.ack(1)


class TestChannelDeadVerdict:
    """
    The tracker carries the session's channel-dead verdict.

    It is tracker state rather than a local of whoever noticed because the
    two sides that need it never meet: a client settle discovers the dead
    channel on the settle side of the servicer's merge, while the consume
    loop's cleanup -- which must act on that discovery -- runs in the
    consume generator, which the merge tears down by cancelling its pending
    read and so never sees the settle's error. The tracker is the one object
    both hold.
    """

    def test_a_fresh_tracker_reports_a_live_channel(self):
        # The default has to be False, not merely falsy-by-absence: every
        # ordinary stream cleanup reads this and must take the nack path.
        assert AckTracker().channel_dead is False

    def test_marking_records_the_verdict(self):
        tracker = AckTracker()

        tracker.mark_channel_dead()

        assert tracker.channel_dead is True

    def test_marking_twice_is_idempotent(self):
        # Two settles can fail the same way in one session (a batch of nacks
        # against a channel the broker just killed), so the second mark must
        # be a no-op rather than an error or a toggle.
        tracker = AckTracker()

        tracker.mark_channel_dead()
        tracker.mark_channel_dead()

        assert tracker.channel_dead is True

    def test_the_verdict_is_read_only_from_outside(self):
        # The flag is monotonic by design -- nothing may un-mark a channel
        # the broker has closed -- so the only writer is mark_channel_dead().
        tracker = AckTracker()

        with pytest.raises(AttributeError):
            tracker.channel_dead = False

    def test_marking_is_synchronous(self):
        # Its writer -- ack()/nack() on a dead-channel failure -- calls this
        # from an ``except`` clause, immediately before re-raising, without
        # awaiting it. A coroutine here would be silently never awaited and
        # the verdict would never land.
        assert AckTracker().mark_channel_dead() is None

    def test_tracking_is_unaffected_by_the_verdict(self):
        # Marking does not tear the registry down; it only changes what
        # nack_all does with it. A delivery already in flight is still
        # addressable (the servicer's warning paths depend on that).
        tracker = AckTracker()
        tracker.track(_mock_message(1))

        tracker.mark_channel_dead()

        assert tracker.in_flight_count == 1

    @pytest.mark.asyncio
    async def test_ack_still_attempts_the_settle_on_a_dead_channel(self):
        # ack() is deliberately NOT short-circuited on an already-marked
        # tracker: a settle discovers the death BY attempting it, and the
        # error it raises is what the servicer classifies for the client, so
        # an ack that quietly no-oped would end the stream with no error.
        tracker = AckTracker()
        message = _mock_message(1)
        message.ack.side_effect = aio_pika.exceptions.ChannelInvalidStateError(
            "channel is closed"
        )
        tracker.track(message)
        tracker.mark_channel_dead()

        with pytest.raises(
            aio_pika.exceptions.ChannelInvalidStateError, match="channel is closed"
        ):
            await tracker.ack(1)

    @pytest.mark.asyncio
    async def test_nack_still_attempts_the_settle_on_a_dead_channel(self):
        tracker = AckTracker()
        message = _mock_message(1)
        message.nack.side_effect = aio_pika.exceptions.ChannelInvalidStateError(
            "channel is closed"
        )
        tracker.track(message)
        tracker.mark_channel_dead()

        with pytest.raises(
            aio_pika.exceptions.ChannelInvalidStateError, match="channel is closed"
        ):
            await tracker.nack(1)


# Every class in DEAD_CHANNEL_EXC, plus the subclass aiormq actually raises
# for a broker-closed channel and for a broker-closed connection.
_DEAD_CHANNEL_FAILURES = dead_channel_failures

_SETTLE_VERBS = pytest.mark.parametrize("verb", ["ack", "nack"])


def _tracker_whose_settle_fails(verb, failure, tag=1):
    """A tracker holding one delivery whose ``verb`` raises ``failure``."""
    tracker = AckTracker()
    message = _mock_message(tag)
    getattr(message, verb).side_effect = failure
    tracker.track(message)
    return tracker


async def _settle(tracker, verb, tag=1):
    if verb == "ack":
        await tracker.ack(tag)
    else:
        await tracker.nack(tag)


class TestADeadChannelSettleMarksTheTracker:
    """
    ``ack`` and ``nack`` mark the tracker themselves when the broker call
    fails because the delivery's channel is dead (issue b.xr7).

    The settle side is the first to learn of the death whenever the client
    is settling at the moment of the kill. Its error reaches the servicer's
    ``merge``, which tears the consume generator down by cancelling that
    generator's pending read, and so schedules the generator's cleanup
    before the error reaches anything above the merge. The cleanup reads
    the verdict off this tracker, so the mark must be in place before the
    error leaves the tracker.
    """

    @pytest.mark.asyncio
    @_SETTLE_VERBS
    @_DEAD_CHANNEL_FAILURES
    async def test_the_tracker_is_marked(self, verb, make_failure):
        failure = make_failure()
        tracker = _tracker_whose_settle_fails(verb, failure)

        with pytest.raises(type(failure)):
            await _settle(tracker, verb)

        assert tracker.channel_dead is True

    @pytest.mark.asyncio
    @_SETTLE_VERBS
    @_DEAD_CHANNEL_FAILURES
    async def test_the_mark_is_in_place_when_the_callers_except_clause_runs(
        self, verb, make_failure
    ):
        # Ordering, not just the end state: the caller's except clause is the
        # first code the error reaches outside the tracker. A mark that was
        # deferred -- scheduled as a task, or left to whoever classifies the
        # error further up -- would pass the end-state assertion above while
        # still arriving after the consume generator's cleanup has read it.
        tracker = _tracker_whose_settle_fails(verb, make_failure())
        seen_by_the_except_clause = []

        try:
            await _settle(tracker, verb)
        except Exception:
            seen_by_the_except_clause.append(tracker.channel_dead)

        assert seen_by_the_except_clause == [True]

    @pytest.mark.asyncio
    @_SETTLE_VERBS
    @_DEAD_CHANNEL_FAILURES
    async def test_the_same_exception_object_is_re_raised(self, verb, make_failure):
        # Not wrapped, not replaced: ``merge`` passes a source's error on as
        # the same object (its contract C2), and the servicer's
        # ``_dead_channel_mapped`` classifies it with a plain ``except``.
        failure = make_failure()
        tracker = _tracker_whose_settle_fails(verb, failure)

        with pytest.raises(type(failure)) as exc_info:
            await _settle(tracker, verb)

        assert exc_info.value is failure

    @pytest.mark.asyncio
    @_SETTLE_VERBS
    @_DEAD_CHANNEL_FAILURES
    async def test_the_tag_leaves_the_tracker(self, verb, make_failure):
        # The pop still precedes the broker call, so a failed settle cannot
        # be settled a second time by a later client retry or by cleanup.
        failure = make_failure()
        tracker = _tracker_whose_settle_fails(verb, failure)

        with pytest.raises(type(failure)):
            await _settle(tracker, verb)

        assert tracker.in_flight_count == 0


class TestASettleThatDoesNotFindTheChannelDeadLeavesTheTrackerUnmarked:
    """
    The control arms for :class:`TestADeadChannelSettleMarksTheTracker`.

    A wrong mark is not harmless: the consume loop's cleanup would then skip
    the cancel and the nacks on a channel that is still alive, leaving the
    consumer attached and the deliveries unacked against it.
    """

    @pytest.mark.asyncio
    @_SETTLE_VERBS
    async def test_a_successful_settle(self, verb):
        tracker = AckTracker()
        tracker.track(_mock_message(1))

        await _settle(tracker, verb)

        assert tracker.channel_dead is False

    @pytest.mark.asyncio
    @_SETTLE_VERBS
    async def test_an_unknown_tag(self, verb):
        tracker = AckTracker()

        with pytest.raises(KeyError):
            await _settle(tracker, verb, tag=999)

        assert tracker.channel_dead is False

    @pytest.mark.asyncio
    @_SETTLE_VERBS
    async def test_an_unknown_tag_never_reaches_the_broker(self, verb):
        # The pop raises before the broker call, so a stale tag from a client
        # that is behind settles nothing -- in particular not some other
        # delivery the tracker still holds.
        tracker = AckTracker()
        held = _mock_message(1)
        tracker.track(held)

        with pytest.raises(KeyError):
            await _settle(tracker, verb, tag=999)

        assert held.ack.await_count + held.nack.await_count == 0

    @pytest.mark.asyncio
    @_SETTLE_VERBS
    @pytest.mark.parametrize(
        "make_failure",
        [
            lambda: ValueError("a genuine internal bug"),
            # Each base class of a dead-channel class: the catch must name
            # the dead-channel classes, not what they inherit from.
            unrelated_runtime_error,
            builtin_connection_error,
        ],
        ids=["value_error", "runtime_error", "builtin_connection_error"],
    )
    async def test_an_unrelated_failure(self, verb, make_failure):
        failure = make_failure()
        tracker = _tracker_whose_settle_fails(verb, failure)

        with pytest.raises(type(failure)):
            await _settle(tracker, verb)

        assert tracker.channel_dead is False


class TestNackAllOnADeadChannel:
    """
    ``nack_all`` abandons its deliveries when the channel is known dead.

    The broker requeued every unacked delivery when it closed the channel,
    and the delivery tags died with that incarnation. Nacking them is a
    guaranteed failure that logs one stack trace per outstanding delivery,
    on exactly the incident path.
    """

    @pytest.mark.asyncio
    async def test_no_message_is_nacked(self):
        tracker = AckTracker()
        messages = [_mock_message(tag) for tag in (1, 2, 3)]
        for message in messages:
            tracker.track(message)
        tracker.mark_channel_dead()

        await tracker.nack_all()

        for message in messages:
            message.nack.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_no_message_is_acked_either(self):
        # Abandoning is not a silent ack: the tracker touches the broker not
        # at all, and it is the broker's own channel-close requeue that
        # preserves the messages.
        tracker = AckTracker()
        message = _mock_message(1)
        tracker.track(message)
        tracker.mark_channel_dead()

        await tracker.nack_all()

        message.ack.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_the_in_flight_registry_is_cleared(self):
        # The registry must still be emptied, or the abandoned deliveries
        # keep the raw aio-pika messages (and their bodies) alive for as
        # long as anything holds the tracker.
        tracker = AckTracker()
        tracker.track(_mock_message(1))
        tracker.mark_channel_dead()

        await tracker.nack_all()

        assert tracker.in_flight_count == 0

    @pytest.mark.asyncio
    async def test_the_origin_queue_cache_is_cleared(self):
        # The parallel DLQ-browse registry is evicted in lockstep on every
        # other settle path; the abandon path must not be the one that leaks
        # it.
        tracker = AckTracker()
        tracker.track(_mock_message(1), origin_queue="origin-queue")
        tracker.mark_channel_dead()

        await tracker.nack_all()

        assert tracker.origin_queue(1) is None

    @pytest.mark.asyncio
    async def test_a_live_channel_still_nacks(self):
        # The control arm. Without it the assertions above would pass for a
        # nack_all that had simply stopped nacking altogether.
        tracker = AckTracker()
        message = _mock_message(1)
        tracker.track(message)

        await tracker.nack_all()

        message.nack.assert_awaited_once_with(requeue=True)

    @pytest.mark.asyncio
    async def test_a_mark_that_lands_between_drains_stops_the_next_one(self):
        # The flag is read at call time, not captured at construction: a
        # tracker that was live for one drain and is marked afterwards
        # abandons on the next. This is the ordering the cleanup task relies
        # on, since it reads the verdict inside the task rather than when
        # the task is scheduled.
        tracker = AckTracker()
        first = _mock_message(1)
        tracker.track(first)

        await tracker.nack_all()

        second = _mock_message(2)
        tracker.track(second)
        tracker.mark_channel_dead()
        await tracker.nack_all()

        first.nack.assert_awaited_once_with(requeue=True)
        second.nack.assert_not_awaited()
