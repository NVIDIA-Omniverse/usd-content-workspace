# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import MagicMock, call

import aio_pika
import grpc
import pytest
from grpc.aio import AioRpcError

from app.grpc import event_consumer_servicer
from app.lib.rabbitmq import NonDurableQueueConsumer
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ConsumeNonDurableEventsRequest,
)
from tests.conftest import assert_non_durable_get_instance_call
from tests.fixtures.mock_filter_group import filter_group as model_filter_group
from tests.integration_tests.conftest import assert_all_messages_streamed, filter_group


@pytest.fixture
def converted_filter_groups():
    return [model_filter_group()]


@pytest.fixture
def mocked_convert_to_models(converted_filter_groups, mocker):
    return mocker.patch.object(
        event_consumer_servicer,
        "convert_to_models",
        return_value=converted_filter_groups,
    )


@pytest.fixture
def consume_request():
    request = ConsumeNonDurableEventsRequest()
    request.filter_groups.append(filter_group(event_type="e1"))

    return request


@pytest.fixture
def request_with_no_event_type():
    request = ConsumeNonDurableEventsRequest()
    request.filter_groups.append(filter_group(event_type=""))

    return request


@pytest.fixture
def request_stream(consume_request):
    def requests():
        yield consume_request  # event_type = "e1"

        consume_request.previous_filter_groups.append(filter_group(event_type="e1"))
        consume_request.filter_groups[0].event_type = "e2"
        yield consume_request

        consume_request.previous_filter_groups[0].event_type = "e2"
        consume_request.filter_groups[0].event_type = "e3"
        yield consume_request

        return

    return requests


@pytest.fixture
def request_stream_bad_first_request(consume_request, request_with_no_event_type):
    def requests():
        yield request_with_no_event_type
        yield consume_request

    return requests


@pytest.fixture
def request_stream_bad_second_request(consume_request, request_with_no_event_type):
    def requests():
        yield consume_request
        yield request_with_no_event_type

    return requests


@pytest.fixture
def consume_request_with_reconnect_token():
    request = ConsumeNonDurableEventsRequest()
    request.filter_groups.append(filter_group(event_type="e_reconnect"))
    request.reconnect_token = "test-reconnect-token"
    return request


@pytest.fixture
def request_stream_with_reconnect_token(consume_request_with_reconnect_token):
    def requests():
        yield consume_request_with_reconnect_token
        return

    return requests


#: The queue id the stubbed ``_create_queue`` hands back on a fresh connect.
#: Distinct from the id the shared stand-in consumer carries, so a test can
#: tell which of the two the stream was actually built from.
_FRESH_QUEUE_ID = "fresh-queue-id"

#: The real ``get_instance``, captured at import time -- before the autouse
#: ``mock_queue_iterator`` fixture replaces it with one returning the shared
#: stand-in consumer.
_REAL_GET_INSTANCE = NonDurableQueueConsumer.get_instance


@pytest.fixture
def real_get_instance(mock_queue_iterator, mocker):
    """
    Put the REAL ``get_instance`` back for one test, with only the broker
    rebind stubbed.

    The shared rig hands the servicer a consumer built directly, which means
    the connect-time update never runs: the verdict the stream's opening
    acknowledgement reports is whatever the fixture set, not what
    ``update_queue`` decided. This fixture is what lets a test drive the
    whole production chain -- ``get_instance`` -> ``update_queue`` ->
    the servicer's opening response -- so the verdict a client reads over
    the wire is the one production computes.

    Depends on ``mock_queue_iterator`` so it applies after that autouse
    fixture and wins, the same ordering trick
    ``mock_redelivering_durable_queue`` uses. ``get_queue`` stays patched by
    it, so the reconnect finds a queue without a live broker.
    """
    mocker.patch.object(NonDurableQueueConsumer, "get_instance", _REAL_GET_INSTANCE)


@pytest.fixture
def connect_time_rebind(real_get_instance, mocker):
    """
    Stub the one part of the real ``update_queue`` that needs a broker, and
    hand the spy back.

    Everything else on the connect-time update path runs for real, so the
    spy answers the question the acknowledgement's ``unchanged`` field
    claims to answer: was a bind or unbind call issued for this update?
    """
    return mocker.patch.object(NonDurableQueueConsumer, "_update_bindings")


@pytest.fixture
def real_fresh_connect(real_get_instance, mocker):
    """
    The real ``get_instance`` again, with the queue declaration stubbed
    instead of the rebind: the fresh-connect branch, which creates the queue
    with a binding per key derived from the set the request named.

    Its verdict is the one the shared rig hand-sets everywhere else, so this
    is what says the value a fresh-connect client reads over the wire is the
    one ``get_instance`` records rather than the one a fixture supplied.
    """
    queue = MagicMock(spec=aio_pika.abc.AbstractQueue)
    queue.name = f"event-queue:{_FRESH_QUEUE_ID}:nd"
    mocker.patch.object(
        RabbitMQBase, "_create_queue", return_value=(_FRESH_QUEUE_ID, queue)
    )

    return queue


@pytest.fixture
def request_stream_fresh_connect():
    """
    A client opening a brand new queue: filter groups, no reconnect token.

    ``get_instance`` binds that set while creating the queue, so -- as with
    a connect-time update -- the opening response is the only
    acknowledgement the set ever gets.
    """

    def requests():
        request = ConsumeNonDurableEventsRequest()
        request.filter_groups.append(filter_group(event_type="e1"))
        yield request

    return requests


@pytest.fixture
def request_stream_reconnect_with_update():
    """
    Build a client stream whose opening request reattaches with a token AND
    names a connect-time filter update: both the set it wants and the set it
    believes the queue already holds.

    ``get_instance`` applies that update before the stream yields anything,
    so it never reaches the servicer's mid-stream loop -- the opening
    response is the only acknowledgement the client ever gets for it.
    """

    def _requests(new_event_type):
        def requests():
            request = ConsumeNonDurableEventsRequest(
                reconnect_token="test-reconnect-token"
            )
            request.previous_filter_groups.append(filter_group(event_type="e1"))
            request.filter_groups.append(filter_group(event_type=new_event_type))
            yield request

        return requests()

    return _requests


@pytest.fixture
def request_stream_token_only():
    """
    A client reattaching with its token alone, naming no filter set.

    The service keeps no filter state and cannot read a queue's bindings
    back from the broker, so it has no set to echo to this client.
    """

    def requests():
        yield ConsumeNonDurableEventsRequest(reconnect_token="test-reconnect-token")

    return requests


async def _drain(stub, requests, metadata):
    """Collect every response the server sends over the real wire."""
    return [
        response
        async for response in stub.ConsumeNonDurableEvents(requests, metadata=metadata)
    ]


def _acknowledgements(responses):
    """Every response on the stream that acknowledges a filter update."""
    return [r for r in responses if r.HasField("filter_update_ack")]


@pytest.mark.asyncio
async def test_validates_filter_groups_on_first_request(
    event_consumer_stub, request_stream_bad_first_request, metadata
):
    with pytest.raises(AioRpcError, match=str(grpc.StatusCode.INVALID_ARGUMENT)):
        async for _ in event_consumer_stub.ConsumeNonDurableEvents(
            request_stream_bad_first_request(), metadata=metadata
        ):
            pass


@pytest.mark.asyncio
async def test_validates_filter_groups_on_subsequent_requests(
    event_consumer_stub, request_stream_bad_second_request, metadata
):
    with pytest.raises(AioRpcError, match=str(grpc.StatusCode.INVALID_ARGUMENT)):
        async for _ in event_consumer_stub.ConsumeNonDurableEvents(
            request_stream_bad_second_request(), metadata=metadata
        ):
            pass


@pytest.mark.asyncio
async def test_gets_instance_using_filter_groups(
    request_stream,
    event_consumer_stub,
    metadata,
    mocked_convert_to_models,
    converted_filter_groups,
    mocker,
):
    mocker.patch.object(NonDurableQueueConsumer, "update_queue")
    async for _ in event_consumer_stub.ConsumeNonDurableEvents(
        request_stream(), metadata=metadata
    ):
        pass

    assert_non_durable_get_instance_call("", converted_filter_groups)


@pytest.mark.asyncio
async def test_updates_queue_bindings_based_on_subsequent_requests(
    request_stream,
    event_consumer_stub,
    metadata,
    mocked_non_durable_consumer,
    mocker,
):
    # One conversion per request the servicer acts on: the first, converted
    # by ``_get_non_durable_consumer``, and one per update, converted in the
    # merged loop where the update is applied. The listener no longer
    # converts a request of its own -- it forwards the raw request and lets
    # the loop convert it once -- so this list is three entries, not five.
    mocker.patch.object(
        event_consumer_servicer.EventConsumerServicer,
        "_filter_grps_from_request",
        side_effect=[
            ("original", None),
            ("new", "prev-1"),
            ("really-new", "prev-2"),
        ],
    )

    spy = mocker.patch.object(mocked_non_durable_consumer, "update_queue")

    async for _ in event_consumer_stub.ConsumeNonDurableEvents(
        request_stream(), metadata=metadata
    ):
        pass

    assert spy.call_args_list == [call("new", "prev-1"), call("really-new", "prev-2")]


@pytest.mark.asyncio
async def test_consumes_all_events(
    event_consumer_stub, request_stream, metadata, mocker, mock_record_success
):
    mocker.patch.object(NonDurableQueueConsumer, "_update_bindings")
    responses = []

    async for grpc_response in event_consumer_stub.ConsumeNonDurableEvents(
        request_stream(), metadata=metadata
    ):
        responses.append(grpc_response)

    # Three acknowledgements: the opening set, which rides the opening
    # response, and one for each of the two mid-stream changes
    # ``request_stream`` sends.
    assert_all_messages_streamed(
        responses, non_durable=True, expected_acknowledgements=3
    )
    assert mock_record_success.call_count == 3


@pytest.mark.asyncio
async def test_acknowledges_the_opening_set_and_every_update(
    event_consumer_stub, request_stream, metadata, mocker, mock_record_success
):
    # Over the real wire, with the real ``update_queue``: three
    # acknowledgements for the three sets this client names -- the one it
    # opened with, which ``get_instance`` applied before the stream yielded,
    # and one for each of the two mid-stream changes.
    mocker.patch.object(NonDurableQueueConsumer, "_update_bindings")

    responses = await _drain(event_consumer_stub, request_stream(), metadata)

    assert len(_acknowledgements(responses)) == 3


@pytest.mark.asyncio
async def test_acknowledgements_echo_the_sets_in_the_order_they_were_sent(
    event_consumer_stub, request_stream, metadata, mocker, mock_record_success
):
    # The ordering the replay recipe rests on: a client takes the last
    # acknowledgement it received as the queue's current set and replays
    # only the updates that followed it.
    mocker.patch.object(NonDurableQueueConsumer, "_update_bindings")

    responses = await _drain(event_consumer_stub, request_stream(), metadata)

    assert [
        [g.event_type for g in r.filter_update_ack.applied_filter_groups]
        for r in _acknowledgements(responses)
    ] == [["e1"], ["e2"], ["e3"]]


@pytest.mark.asyncio
async def test_an_applied_update_is_not_acknowledged_as_unchanged(
    event_consumer_stub, request_stream, metadata, mocker, mock_record_success
):
    # Every set this client sends differs from the one before it, so the
    # real ``update_queue`` rebinds each time and nothing is a no-op.
    mocker.patch.object(NonDurableQueueConsumer, "_update_bindings")

    responses = await _drain(event_consumer_stub, request_stream(), metadata)

    assert [r.filter_update_ack.unchanged for r in _acknowledgements(responses)] == [
        False,
        False,
        False,
    ]


@pytest.mark.asyncio
async def test_a_fresh_connect_streams_under_the_created_queues_token(
    event_consumer_stub,
    request_stream_fresh_connect,
    metadata,
    real_fresh_connect,
    mock_record_success,
):
    # Anti-vacuity for the two rows below, and the only thing that can tell
    # them apart from a run against the shared stand-in consumer: the token
    # is the id the REAL ``get_instance`` got back from ``_create_queue``,
    # not the stand-in's. Were the rig still installed, its verdict would
    # produce an acknowledgement indistinguishable from the real one.
    responses = await _drain(
        event_consumer_stub, request_stream_fresh_connect(), metadata
    )

    assert responses[0].reconnect_token == _FRESH_QUEUE_ID


@pytest.mark.asyncio
async def test_a_fresh_connect_is_acknowledged_on_the_opening_response(
    event_consumer_stub,
    request_stream_fresh_connect,
    metadata,
    real_fresh_connect,
    mock_record_success,
):
    # Through the real ``get_instance``: a brand new queue's opening set is
    # acknowledged like any other, which is what lets a client use one
    # code path -- read the acknowledgement, take it as the queue's current
    # set -- whether it opened fresh or resumed.
    responses = await _drain(
        event_consumer_stub, request_stream_fresh_connect(), metadata
    )

    assert responses[0].HasField("filter_update_ack")


@pytest.mark.asyncio
async def test_a_fresh_connect_is_not_acknowledged_as_unchanged(
    event_consumer_stub,
    request_stream_fresh_connect,
    metadata,
    real_fresh_connect,
    mock_record_success,
):
    # ``unchanged`` says a rebind was unnecessary. Reported for a queue that
    # was just created, it would invite the client to skip the bindings it
    # had only now asked for.
    responses = await _drain(
        event_consumer_stub, request_stream_fresh_connect(), metadata
    )

    assert _acknowledgements(responses)[0].filter_update_ack.unchanged is False


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "new_event_type",
    ["e1", "e2"],
    ids=["equal_set", "changed_set"],
)
async def test_a_connect_time_update_is_acknowledged_on_the_opening_response(
    event_consumer_stub,
    request_stream_reconnect_with_update,
    metadata,
    new_event_type,
    connect_time_rebind,
    mock_record_success,
):
    # Guards the verdict row below: an opening response carrying no
    # acknowledgement at all reads ``unchanged`` as proto3's False, which
    # would make the changed-set arm pass while proving nothing.
    responses = await _drain(
        event_consumer_stub,
        request_stream_reconnect_with_update(new_event_type),
        metadata,
    )

    assert responses[0].HasField("filter_update_ack")


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "new_event_type, expected_unchanged",
    [("e1", True), ("e2", False)],
    ids=["equal_set", "changed_set"],
)
async def test_the_opening_acknowledgement_reports_the_connect_time_verdict(
    event_consumer_stub,
    request_stream_reconnect_with_update,
    metadata,
    new_event_type,
    expected_unchanged,
    connect_time_rebind,
    mock_record_success,
):
    # The connect-time update end to end: the REAL ``get_instance`` runs the
    # REAL ``update_queue``, and its verdict travels to the client on the
    # stream's first response. This is the value a client reads to decide
    # whether an update it lost the connection over still needs replaying,
    # and ``unchanged=True`` is produced over the wire nowhere else -- every
    # other stream in this file changes its set on every request.
    responses = await _drain(
        event_consumer_stub,
        request_stream_reconnect_with_update(new_event_type),
        metadata,
    )

    assert responses[0].filter_update_ack.unchanged is expected_unchanged


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "new_event_type",
    ["e1", "e2"],
    ids=["equal_set", "changed_set"],
)
async def test_the_opening_acknowledgement_echoes_the_requested_set(
    event_consumer_stub,
    request_stream_reconnect_with_update,
    metadata,
    new_event_type,
    connect_time_rebind,
    mock_record_success,
):
    # The echo is the set the client asked for, on both arms -- including
    # the no-op, where the queue was already there. A client that re-sent
    # its current filters has to be able to match the acknowledgement to the
    # request it sent.
    responses = await _drain(
        event_consumer_stub,
        request_stream_reconnect_with_update(new_event_type),
        metadata,
    )

    assert [
        g.event_type for g in responses[0].filter_update_ack.applied_filter_groups
    ] == [new_event_type]


@pytest.mark.asyncio
async def test_a_connect_time_update_that_changed_the_set_rebinds_the_queue(
    event_consumer_stub,
    request_stream_reconnect_with_update,
    metadata,
    connect_time_rebind,
    mock_record_success,
):
    # The anti-vacuity control for the verdicts above: ``unchanged=False``
    # is only honest if the rebind really was issued, and ``unchanged=True``
    # only if it was not. This arm pins the first half; the row below pins
    # the second.
    await _drain(
        event_consumer_stub, request_stream_reconnect_with_update("e2"), metadata
    )

    assert connect_time_rebind.await_count == 1


@pytest.mark.asyncio
async def test_a_connect_time_update_naming_an_equal_set_rebinds_nothing(
    event_consumer_stub,
    request_stream_reconnect_with_update,
    metadata,
    connect_time_rebind,
    mock_record_success,
):
    await _drain(
        event_consumer_stub, request_stream_reconnect_with_update("e1"), metadata
    )

    connect_time_rebind.assert_not_awaited()


@pytest.mark.asyncio
async def test_every_response_advertises_the_capability(
    event_consumer_stub, request_stream, metadata, mocker, mock_record_success
):
    # proto3 has no absent bool, so the flag has to ride the event
    # responses and the acknowledgements too -- a client reading it off any
    # message but the first would otherwise conclude the capability was
    # gone.
    mocker.patch.object(NonDurableQueueConsumer, "_update_bindings")

    responses = await _drain(event_consumer_stub, request_stream(), metadata)

    assert all(r.supports_filter_update_acks for r in responses)


@pytest.mark.asyncio
async def test_a_token_only_reconnect_gets_no_acknowledgement(
    event_consumer_stub,
    request_stream_token_only,
    metadata,
    mocked_non_durable_consumer,
    mock_record_success,
):
    # An echo here would be a claim the service is not entitled to make: it
    # was told no set, and AMQP offers no way to read the queue's bindings
    # back.
    #
    # The shared rig stands in for a fresh connect, so this test names the
    # verdict ``get_instance`` records for the shape it actually drives: a
    # reattach carrying a token and no filter set at all.
    mocked_non_durable_consumer.connect_time_update_changed = None

    responses = await _drain(event_consumer_stub, request_stream_token_only(), metadata)

    assert _acknowledgements(responses) == []


@pytest.mark.asyncio
@pytest.mark.parametrize("filter_groups", [[], [filter_group()]])
async def test_can_use_reconnect_token_to_get_previous_queue(
    request_stream_with_reconnect_token,
    event_consumer_stub,
    metadata,
    filter_groups,
    mocked_convert_to_models,
    mocked_non_durable_consumer,
    converted_filter_groups,
):
    async for _ in event_consumer_stub.ConsumeNonDurableEvents(
        request_stream_with_reconnect_token(), metadata=metadata
    ):
        pass

    assert_non_durable_get_instance_call(
        "test-reconnect-token", converted_filter_groups
    )
