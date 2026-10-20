# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Integration tests for the ``ManageDeadLetteredEvents`` bidirectional RPC,
driven end-to-end through ``grpc_server`` / ``event_consumer_stub`` with the
opt-in ``mock_dlq_browse`` fixture standing in for the broker (no live broker).

The stream is driven manually via the grpc.aio ``StreamStreamCall`` (write /
read / done_writing) rather than the one-shot request-generator pattern from
``test_consume_durable_events.py``: the operator must first observe the browsed
dead-letter events (to learn each session ``delivery_tag``) before writing the
REPLAY / DISCARD / LEAVE action that references that tag. Reading-then-writing
keeps the action ordered after the browse-track, so the servicer's REPLAY
origin-queue resolution and the per-tag ack / nack hooks run deterministically
under ``pytest-random-order``.
"""

import grpc
import pytest
import pytest_asyncio
from google.protobuf import json_format

import aio_pika

from app.lib.permissions import permissions
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ManageDeadLetteredEventsRequest,
)

Action = ManageDeadLetteredEventsRequest.Action
ActionEntry = ManageDeadLetteredEventsRequest.ActionEntry


@pytest_asyncio.fixture
def allow_dlq_permission(mocker):
    """
    Grant the ``inspect-and-replay-all-dlq`` gate for the happy-path tests.

    The autouse ``before_each`` mock only stubs ``permissions._can_or_raise``
    as a plain (non-awaitable) ``MagicMock``, which the real
    ``raise_if_cant_inspect_and_replay_all_dlq`` coroutine cannot ``await``.
    Patching the gate coroutine itself with an ``AsyncMock`` lets the RPC open
    cleanly, and the unauthorized test overrides the same object to deny.
    """
    return mocker.patch.object(
        permissions,
        "raise_if_cant_inspect_and_replay_all_dlq",
        new=mocker.AsyncMock(return_value=True),
    )


async def _open_stream_and_browse(stub, metadata, expected_count):
    """
    Open the management stream, write the opening request, and read exactly
    ``expected_count`` ``dead_lettered_event``-arm responses.

    Returns ``(call, [browse_response_dict, ...])``. The opening request
    carries no actions; subsequent action requests are written by the caller
    once the browse delivery tags are known.
    """
    call = stub.ManageDeadLetteredEvents(metadata=metadata)
    await call.write(ManageDeadLetteredEventsRequest())

    browse = []
    while len(browse) < expected_count:
        response = await call.read()
        response_dict = json_format.MessageToDict(
            response, preserving_proto_field_name=True
        )
        assert "dead_lettered_event" in response_dict
        browse.append(response_dict["dead_lettered_event"])
    return call, browse


async def _action_outcome(call, delivery_tag, action):
    """
    Write a single-entry action request and read back its ``action_outcome``.

    Returns the ``action_outcome`` arm as a dict (proto field names preserved).
    """
    await call.write(
        ManageDeadLetteredEventsRequest(
            actions=[ActionEntry(delivery_tag=delivery_tag, action=action)]
        )
    )
    response = await call.read()
    response_dict = json_format.MessageToDict(
        response, preserving_proto_field_name=True
    )
    assert "action_outcome" in response_dict
    return response_dict["action_outcome"]


async def _action_outcome_from_next(call):
    """
    Read the next response off the stream and return its ``action_outcome`` arm.

    Unlike :func:`_action_outcome`, this writes nothing — the caller has already
    written the action(s). Used when a skipped (stale-tag) action must NOT emit
    a response, so the next response read belongs to a different, live action.
    """
    response = await call.read()
    response_dict = json_format.MessageToDict(
        response, preserving_proto_field_name=True
    )
    assert "action_outcome" in response_dict
    return response_dict["action_outcome"]


class TestBrowse:
    """Opening the stream surfaces decoded dead-letter triage metadata."""

    @pytest.mark.asyncio
    async def test_streams_one_event_per_dead_lettered_message(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        call.cancel()

        assert len(browse) == len(mock_dlq_browse.messages)

    @pytest.mark.asyncio
    async def test_decodes_reason(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        call.cancel()

        reasons = {event["reason"] for event in browse}
        assert reasons == {"REASON_DELIVERY_LIMIT", "REASON_EXPIRED"}

    @pytest.mark.asyncio
    async def test_decodes_origin_queue(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        call.cancel()

        origin_queues = {event["origin_queue"] for event in browse}
        assert origin_queues == {
            "event-queue:queue-1",
            "event-queue:queue-2",
        }

    @pytest.mark.asyncio
    async def test_decodes_delivery_count(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        call.cancel()

        # The fixture default delivery_count is 3 for every browsed message.
        assert all(event["delivery_count"] == "3" for event in browse)

    @pytest.mark.asyncio
    async def test_includes_dead_lettered_at_timestamp(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        call.cancel()

        assert all(event.get("dead_lettered_at") is not None for event in browse)

    @pytest.mark.asyncio
    async def test_decodes_event_type(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        call.cancel()

        event_types = {event["event"]["event_type"] for event in browse}
        assert event_types == {"e1", "e2"}

    @pytest.mark.asyncio
    async def test_decodes_principal_identity(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        call.cancel()

        identities = {event["event"]["principal_identity"] for event in browse}
        assert identities == {"p1", "p2"}

    @pytest.mark.asyncio
    async def test_decodes_resource_id(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        call.cancel()

        resource_ids = {event["resource_id"] for event in browse}
        assert resource_ids == {"r1", "r2"}

    @pytest.mark.asyncio
    async def test_includes_event_payload(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        call.cancel()

        assert all(event["event"]["message"] == {"test": "message"} for event in browse)

    @pytest.mark.asyncio
    async def test_includes_session_delivery_tag(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        call.cancel()

        # Tags are assigned 1-based in browse order; every browsed event carries
        # the session-scoped delivery tag the client sends back to act on it.
        tags = {event["delivery_tag"] for event in browse}
        assert tags == {"1", "2"}


class TestReplay:
    """REPLAY republishes to the origin queue, confirm-gated, then drops it."""

    @pytest.mark.asyncio
    async def test_republishes_to_origin_queue(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        target = browse[0]
        tag = int(target["delivery_tag"])

        await _action_outcome(call, tag, Action.ACTION_REPLAY)
        call.cancel()

        mock_dlq_browse.publish.assert_awaited_once()
        assert (
            mock_dlq_browse.publish.call_args.kwargs["routing_key"]
            == target["origin_queue"]
        )

    @pytest.mark.asyncio
    async def test_republishes_mandatory(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        tag = int(browse[0]["delivery_tag"])

        await _action_outcome(call, tag, Action.ACTION_REPLAY)
        call.cancel()

        assert mock_dlq_browse.publish.call_args.kwargs["mandatory"] is True

    @pytest.mark.asyncio
    async def test_acks_dlq_copy_after_confirm(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        tag = int(browse[0]["delivery_tag"])

        await _action_outcome(call, tag, Action.ACTION_REPLAY)
        call.cancel()

        # The publish-confirm returned normally, so the DLQ copy is acked
        # (dropped) only after the republish — and never nacked.
        raw_message = mock_dlq_browse.message_by_tag[tag]
        raw_message.ack.assert_awaited_once()
        raw_message.nack.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_reports_replayed_outcome(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        tag = int(browse[0]["delivery_tag"])

        outcome = await _action_outcome(call, tag, Action.ACTION_REPLAY)
        call.cancel()

        assert outcome["outcome"] == "OUTCOME_REPLAYED"
        assert outcome["delivery_tag"] == str(tag)


class TestReplayOriginQueueDeleted:
    """REPLAY against a deleted origin queue is no-loss (SR-3.4)."""

    @pytest.mark.asyncio
    async def test_reports_origin_queue_deleted_outcome(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        # Simulate the origin queue being gone: an unroutable mandatory publish
        # raises DeliveryError on the republish boundary.
        mock_dlq_browse.publish.side_effect = aio_pika.exceptions.DeliveryError(
            None, None
        )

        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        tag = int(browse[0]["delivery_tag"])

        outcome = await _action_outcome(call, tag, Action.ACTION_REPLAY)
        call.cancel()

        assert outcome["outcome"] == "OUTCOME_ORIGIN_QUEUE_DELETED"
        assert outcome["delivery_tag"] == str(tag)

    @pytest.mark.asyncio
    async def test_leaves_dlq_copy_in_place_on_deleted_origin(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        mock_dlq_browse.publish.side_effect = aio_pika.exceptions.DeliveryError(
            None, None
        )

        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        tag = int(browse[0]["delivery_tag"])

        await _action_outcome(call, tag, Action.ACTION_REPLAY)
        call.cancel()

        # No loss: the DLQ copy is nacked requeue=True (LEAVE), never acked.
        raw_message = mock_dlq_browse.message_by_tag[tag]
        raw_message.nack.assert_awaited_once_with(requeue=True)
        raw_message.ack.assert_not_awaited()


class TestDiscard:
    """DISCARD acks (drops) the DLQ copy and reports DISCARDED."""

    @pytest.mark.asyncio
    async def test_acks_dlq_copy(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        tag = int(browse[0]["delivery_tag"])

        await _action_outcome(call, tag, Action.ACTION_DISCARD)
        call.cancel()

        raw_message = mock_dlq_browse.message_by_tag[tag]
        raw_message.ack.assert_awaited_once()
        raw_message.nack.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_does_not_republish(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        tag = int(browse[0]["delivery_tag"])

        await _action_outcome(call, tag, Action.ACTION_DISCARD)
        call.cancel()

        mock_dlq_browse.publish.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_reports_discarded_outcome(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        tag = int(browse[0]["delivery_tag"])

        outcome = await _action_outcome(call, tag, Action.ACTION_DISCARD)
        call.cancel()

        assert outcome["outcome"] == "OUTCOME_DISCARDED"
        assert outcome["delivery_tag"] == str(tag)


class TestLeave:
    """LEAVE nacks requeue=True (no loss) and reports LEFT."""

    @pytest.mark.asyncio
    async def test_nacks_requeue_true(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        tag = int(browse[0]["delivery_tag"])

        await _action_outcome(call, tag, Action.ACTION_LEAVE)
        call.cancel()

        raw_message = mock_dlq_browse.message_by_tag[tag]
        raw_message.nack.assert_awaited_once_with(requeue=True)
        raw_message.ack.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_reports_left_outcome(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        tag = int(browse[0]["delivery_tag"])

        outcome = await _action_outcome(call, tag, Action.ACTION_LEAVE)
        call.cancel()

        assert outcome["outcome"] == "OUTCOME_LEFT"
        assert outcome["delivery_tag"] == str(tag)


class TestReplayPublishFailure:
    """A non-DeliveryError publish failure is REPLAY_FAILED + no loss."""

    @pytest.mark.asyncio
    async def test_reports_replay_failed_outcome(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        # A publish failure that is NOT a DeliveryError (e.g. the broker nacked
        # the publish or the channel faulted) must surface as REPLAY_FAILED.
        mock_dlq_browse.publish.side_effect = RuntimeError("broker nacked publish")

        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        tag = int(browse[0]["delivery_tag"])

        outcome = await _action_outcome(call, tag, Action.ACTION_REPLAY)
        call.cancel()

        assert outcome["outcome"] == "OUTCOME_REPLAY_FAILED"
        assert outcome["delivery_tag"] == str(tag)

    @pytest.mark.asyncio
    async def test_surfaces_failure_detail(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        mock_dlq_browse.publish.side_effect = RuntimeError("broker nacked publish")

        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        tag = int(browse[0]["delivery_tag"])

        outcome = await _action_outcome(call, tag, Action.ACTION_REPLAY)
        call.cancel()

        assert outcome["detail"]

    @pytest.mark.asyncio
    async def test_leaves_dlq_copy_in_place_on_failure(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        mock_dlq_browse.publish.side_effect = RuntimeError("broker nacked publish")

        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        tag = int(browse[0]["delivery_tag"])

        await _action_outcome(call, tag, Action.ACTION_REPLAY)
        call.cancel()

        # No loss: the DLQ copy is nacked requeue=True (LEAVE), never acked.
        raw_message = mock_dlq_browse.message_by_tag[tag]
        raw_message.nack.assert_awaited_once_with(requeue=True)
        raw_message.ack.assert_not_awaited()


class TestUnknownTag:
    """A stale / unknown-tag action is skipped without crashing the stream."""

    @pytest.mark.asyncio
    async def test_stale_action_emits_no_outcome_and_stream_survives(
        self, event_consumer_stub, metadata, mock_dlq_browse, allow_dlq_permission
    ):
        call, browse = await _open_stream_and_browse(
            event_consumer_stub, metadata, len(mock_dlq_browse.messages)
        )
        first_tag = int(browse[0]["delivery_tag"])
        second_tag = int(browse[1]["delivery_tag"])

        # Act on the first tag, retiring it from the in-flight registry.
        first_outcome = await _action_outcome(call, first_tag, Action.ACTION_DISCARD)
        assert first_outcome["delivery_tag"] == str(first_tag)

        # A SECOND action on the now-retired first tag is stale: the servicer
        # skips it (no action_outcome) without crashing the stream. Acting on a
        # still-live second tag right after must still produce its outcome — the
        # only response the operator reads next — proving the stale action was
        # silently skipped rather than emitting a spurious outcome or faulting
        # the stream.
        await call.write(
            ManageDeadLetteredEventsRequest(
                actions=[
                    ActionEntry(delivery_tag=first_tag, action=Action.ACTION_DISCARD),
                    ActionEntry(delivery_tag=second_tag, action=Action.ACTION_DISCARD),
                ]
            )
        )
        next_outcome = await _action_outcome_from_next(call)
        call.cancel()

        assert next_outcome["delivery_tag"] == str(second_tag)
        assert next_outcome["outcome"] == "OUTCOME_DISCARDED"


class TestUnauthorized:
    """A caller lacking the DLQ permission is denied the RPC."""

    @pytest.mark.asyncio
    async def test_raises_permission_denied_when_gate_denies(
        self, event_consumer_stub, metadata, mock_dlq_browse, mocker
    ):
        # Override the autouse permission mock so the coarse DLQ gate raises.
        mocker.patch.object(
            permissions,
            "raise_if_cant_inspect_and_replay_all_dlq",
            new=mocker.AsyncMock(
                side_effect=PermissionError(
                    "Not allowed to inspect and replay dead-lettered messages"
                )
            ),
        )

        # The gate denies before the handler reads the request iterator, so no
        # client write is needed to trigger the abort — and sending one races
        # it: a write that lands after the server aborted fails on the CLIENT
        # side with grpc.aio's own status (``InvalidStateError`` or
        # ``AioRpcError(INTERNAL)``), masking the PERMISSION_DENIED the server
        # really sent. Reading the server-initiated stream surfaces the real
        # status without the race. (See test_capacity_guard.py:211-224.)
        with pytest.raises(
            grpc.aio.AioRpcError,
            match=".*inspect and replay dead-lettered messages.*",
        ):
            call = event_consumer_stub.ManageDeadLetteredEvents(metadata=metadata)
            async for _ in call:
                pass

    @pytest.mark.asyncio
    async def test_streams_no_dead_letters_to_denied_caller(
        self, event_consumer_stub, metadata, mock_dlq_browse, mocker
    ):
        mocker.patch.object(
            permissions,
            "raise_if_cant_inspect_and_replay_all_dlq",
            new=mocker.AsyncMock(
                side_effect=PermissionError(
                    "Not allowed to inspect and replay dead-lettered messages"
                )
            ),
        )

        # No client write: the gate aborts before reading the request
        # iterator, and a racing write would mask the real status under
        # oversubscription. Reading the stream surfaces the abort. (See the
        # sibling test above and test_capacity_guard.py:211-224.)
        responses = []
        with pytest.raises(grpc.aio.AioRpcError):
            call = event_consumer_stub.ManageDeadLetteredEvents(metadata=metadata)
            async for response in call:
                responses.append(response)

        assert responses == []
