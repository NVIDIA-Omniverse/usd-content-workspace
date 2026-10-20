# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import aio_pika
import grpc
import pytest
from grpc.aio import AioRpcError

from app.lib.limits import (
    MAX_BATCH_EVENTS,
    MAX_EVENT_MESSAGE_BYTES,
    MAX_EVENT_TYPE_LENGTH,
)
from app.lib.rabbitmq import publisher

from nvidia.omniverse.notifications.publisher.v1beta.event_publisher_pb2 import (
    BatchPublishEventsRequest,
    BatchPublishEventsResponse,
    PublishingResult,
)

pytestmark = pytest.mark.asyncio


@pytest.fixture
def mocked_request(mocked_event) -> BatchPublishEventsRequest:
    request = BatchPublishEventsRequest()
    request.events.append(mocked_event)
    request.events.append(mocked_event)
    return request


async def test_publishes_events(
    event_aggregation_stub, metadata, mock_principal, mocked_request, mocker
):
    mocker.patch.object(publisher, "batch_publish")
    await event_aggregation_stub.BatchPublishEvents(mocked_request, metadata=metadata)

    assert publisher.batch_publish.call_count == 1

    for call_args in publisher.batch_publish.call_args_list:
        published_omni_event = call_args[0][0][0]
        assert published_omni_event.event_type == "test-event"
        assert published_omni_event.resource_id == "test-resource-id"
        assert published_omni_event.message == {"body": "test message"}
        assert published_omni_event.occurred_at == "2024-10-23T00:00:00Z"
        assert published_omni_event.principal_identity == mock_principal


async def test_returns_response(
    event_aggregation_stub, mocked_request, mocker, metadata
):
    mocker.patch.object(aio_pika.abc, "AbstractRobustConnection", autospec=True)
    response = await event_aggregation_stub.BatchPublishEvents(
        mocked_request, metadata=metadata
    )
    assert isinstance(response, BatchPublishEventsResponse)
    results = response.results
    assert len(results) == 2
    for result in results:
        assert isinstance(result, PublishingResult)


async def test_returns_invalid_argument_status_code_when_missing_events(
    event_aggregation_stub, metadata
):
    request = BatchPublishEventsRequest()

    with pytest.raises(AioRpcError) as exc_info:
        await event_aggregation_stub.BatchPublishEvents(request, metadata=metadata)

    assert exc_info.value.code().value[0] == grpc.StatusCode.INVALID_ARGUMENT.value[0]
    err_msg = str(exc_info.value.details())
    assert "Publish request must contain at least one event" in err_msg


@pytest.fixture
def batch_of(mocked_event):
    """Return a factory for a batch request.

    ``batch_of(count)`` holds ``count`` copies of ``mocked_event``;
    ``batch_of(events=[...])`` holds exactly the given events.
    """

    def _build(count: int = 0, events=None) -> BatchPublishEventsRequest:
        request = BatchPublishEventsRequest()
        request.events.extend(events if events is not None else [mocked_event] * count)
        return request

    return _build


class TestBatchPublishEvents:
    @pytest.fixture(autouse=True)
    def mock_batch_publish(self, mocker):
        mocker.patch.object(publisher, "batch_publish")

    class TestWhenTheBatchExceedsTheEventLimit:
        """A batch over ``MAX_BATCH_EVENTS`` is refused, as on REST."""

        @pytest.fixture
        def over_limit_request(self, batch_of) -> BatchPublishEventsRequest:
            return batch_of(MAX_BATCH_EVENTS + 1)

        async def test_returns_invalid_argument(
            self, event_aggregation_stub, metadata, over_limit_request
        ):
            with pytest.raises(AioRpcError) as exc_info:
                await event_aggregation_stub.BatchPublishEvents(
                    over_limit_request, metadata=metadata
                )

            assert exc_info.value.code() == grpc.StatusCode.INVALID_ARGUMENT

        async def test_names_the_limit(
            self, event_aggregation_stub, metadata, over_limit_request
        ):
            with pytest.raises(AioRpcError) as exc_info:
                await event_aggregation_stub.BatchPublishEvents(
                    over_limit_request, metadata=metadata
                )

            assert f"must contain at most {MAX_BATCH_EVENTS} events" in (
                exc_info.value.details()
            )

        async def test_does_not_publish(
            self, event_aggregation_stub, metadata, over_limit_request
        ):
            with pytest.raises(AioRpcError):
                await event_aggregation_stub.BatchPublishEvents(
                    over_limit_request, metadata=metadata
                )

            publisher.batch_publish.assert_not_called()

    class TestWhenTheBatchIsExactlyTheEventLimit:
        """The limit is inclusive: a batch of exactly the limit is published."""

        async def test_publishes(self, event_aggregation_stub, metadata, batch_of):
            await event_aggregation_stub.BatchPublishEvents(
                batch_of(MAX_BATCH_EVENTS), metadata=metadata
            )

            publisher.batch_publish.assert_called_once()

    class TestWhenAnEventMessageExceedsTheLimit:
        """One oversize message refuses the whole batch, as REST validation does."""

        @pytest.fixture
        def oversize_message_request(
            self, batch_of, mocked_event, event_with, message_of_size
        ) -> BatchPublishEventsRequest:
            oversize = event_with(message=message_of_size(MAX_EVENT_MESSAGE_BYTES + 1))
            return batch_of(events=[mocked_event, oversize, mocked_event])

        async def test_returns_invalid_argument(
            self, event_aggregation_stub, metadata, oversize_message_request
        ):
            with pytest.raises(AioRpcError) as exc_info:
                await event_aggregation_stub.BatchPublishEvents(
                    oversize_message_request, metadata=metadata
                )

            assert exc_info.value.code() == grpc.StatusCode.INVALID_ARGUMENT

        async def test_names_the_offending_event(
            self, event_aggregation_stub, metadata, oversize_message_request
        ):
            with pytest.raises(AioRpcError) as exc_info:
                await event_aggregation_stub.BatchPublishEvents(
                    oversize_message_request, metadata=metadata
                )

            assert exc_info.value.details().startswith("events[1].message must be")

        async def test_does_not_publish(
            self, event_aggregation_stub, metadata, oversize_message_request
        ):
            with pytest.raises(AioRpcError):
                await event_aggregation_stub.BatchPublishEvents(
                    oversize_message_request, metadata=metadata
                )

            publisher.batch_publish.assert_not_called()

    class TestWhenAnEventHasNoOccurredAt:
        """The missing-field error names the event, as the limit errors do."""

        async def test_names_the_offending_event(
            self, event_aggregation_stub, metadata, batch_of, mocked_event, event_with
        ):
            undated = event_with()
            undated.ClearField("occurred_at")
            request = batch_of(events=[mocked_event, undated])

            with pytest.raises(AioRpcError) as exc_info:
                await event_aggregation_stub.BatchPublishEvents(
                    request, metadata=metadata
                )

            assert exc_info.value.details() == (
                "events[1].occurred_at must be specified"
            )

    class TestWhenAnEventFieldExceedsItsLength:
        """A string-length bound on any event refuses the whole batch."""

        @pytest.fixture
        def over_length_request(
            self, batch_of, mocked_event, event_with
        ) -> BatchPublishEventsRequest:
            long_type = event_with(event_type="x" * (MAX_EVENT_TYPE_LENGTH + 1))
            return batch_of(events=[mocked_event, mocked_event, long_type])

        async def test_names_the_offending_event(
            self, event_aggregation_stub, metadata, over_length_request
        ):
            with pytest.raises(AioRpcError) as exc_info:
                await event_aggregation_stub.BatchPublishEvents(
                    over_length_request, metadata=metadata
                )

            assert exc_info.value.details().startswith("events[2].event_type must be")

        async def test_does_not_publish(
            self, event_aggregation_stub, metadata, over_length_request
        ):
            with pytest.raises(AioRpcError):
                await event_aggregation_stub.BatchPublishEvents(
                    over_length_request, metadata=metadata
                )

            publisher.batch_publish.assert_not_called()


async def test_uses_undetermined_when_auth_disabled(
    event_aggregation_stub, metadata, mocked_request, mocker
):
    """Test that 'undetermined' is used as principal when auth is disabled."""
    from notification_common.auth import principal_id

    # Override the autouse fixture to return None (auth disabled)
    mocker.patch.object(principal_id, "principal_id", return_value=None)
    mocker.patch.object(publisher, "batch_publish")

    await event_aggregation_stub.BatchPublishEvents(mocked_request, metadata=metadata)

    assert publisher.batch_publish.call_count == 1
    published_omni_event = publisher.batch_publish.call_args[0][0][0]
    assert published_omni_event.principal_identity == "undetermined"
