# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import aio_pika
import grpc
import pytest
from grpc.aio import AioRpcError
from notification_common.rabbitmq.omni_event import OmniEvent

from app.lib.limits import (
    MAX_EVENT_MESSAGE_BYTES,
    MAX_EVENT_TYPE_LENGTH,
    MAX_RESOURCE_ID_LENGTH,
)
from app.lib.rabbitmq import publisher

from nvidia.omniverse.notifications.publisher.v1beta.event_publisher_pb2 import (
    PublishEventRequest,
    PublishEventResponse,
    Event,
)
from tests.conftest import make_timestamp

pytestmark = pytest.mark.asyncio


@pytest.fixture
def mocked_publish_event_request(mocked_event) -> PublishEventRequest:
    return PublishEventRequest(event=mocked_event)


async def test_publishes_event(
    event_aggregation_stub,
    metadata,
    mocked_publish_event_request,
    mock_principal,
    mocked_omni_event,
    mocked_event,
    successful_result,
    mocker,
):
    mocker.patch.object(publisher, "publish", return_value=successful_result)

    await event_aggregation_stub.PublishEvent(
        mocked_publish_event_request, metadata=metadata
    )

    published_omni_event: OmniEvent = publisher.publish.call_args[0][0]

    assert published_omni_event.event_type == "test-event"
    assert published_omni_event.resource_id == "test-resource-id"
    assert published_omni_event.message == {"body": "test message"}
    assert published_omni_event.occurred_at == "2024-10-23T00:00:00Z"
    assert published_omni_event.principal_identity == mock_principal


async def test_returns_response(
    event_aggregation_stub, mocked_publish_event_request, mocker, metadata
):
    mocker.patch.object(aio_pika.abc, "AbstractRobustConnection", autospec=True)
    response = await event_aggregation_stub.PublishEvent(
        mocked_publish_event_request, metadata=metadata
    )
    assert isinstance(response, PublishEventResponse)


@pytest.mark.parametrize(
    "field_to_omit,expected_error",
    [
        ("occurred_at", "event.occurred_at must be specified"),
        ("message", "message must be specified"),
        ("event_type", "event_type must be specified"),
    ],
)
async def test_returns_invalid_argument_status_code_when_missing_event(
    event_aggregation_stub, field_to_omit, expected_error, metadata
):
    event_data = {
        "event_type": "test-event",
        "message": {"body": "test message"},
        "occurred_at": make_timestamp(),
    }

    # Remove the field we want to test validation for
    if field_to_omit in event_data:
        del event_data[field_to_omit]

    request = PublishEventRequest(event=Event(**event_data))

    with pytest.raises(AioRpcError) as exc_info:
        await event_aggregation_stub.PublishEvent(request, metadata=metadata)

    assert exc_info.value.code().value[0] == grpc.StatusCode.INVALID_ARGUMENT.value[0]
    assert expected_error in str(exc_info.value.details())


async def test_uses_undetermined_when_auth_disabled(
    event_aggregation_stub,
    metadata,
    mocked_publish_event_request,
    successful_result,
    mocker,
):
    """Test that 'undetermined' is used as principal when auth is disabled."""
    from notification_common.auth import principal_id

    # Override the autouse fixture to return None (auth disabled)
    mocker.patch.object(principal_id, "principal_id", return_value=None)
    mocker.patch.object(publisher, "publish", return_value=successful_result)

    await event_aggregation_stub.PublishEvent(
        mocked_publish_event_request, metadata=metadata
    )

    published_omni_event: OmniEvent = publisher.publish.call_args[0][0]
    assert published_omni_event.principal_identity == "undetermined"


class TestPublishEvent:
    @pytest.fixture(autouse=True)
    def mock_publish(self, mocker, successful_result):
        mocker.patch.object(publisher, "publish", return_value=successful_result)

    class TestWhenTheMessageExceedsTheLimit:
        """A message over ``MAX_EVENT_MESSAGE_BYTES`` is refused, as on REST."""

        @pytest.fixture
        def oversize_request(self, event_with, message_of_size) -> PublishEventRequest:
            return PublishEventRequest(
                event=event_with(message=message_of_size(MAX_EVENT_MESSAGE_BYTES + 1))
            )

        async def test_returns_invalid_argument(
            self, event_aggregation_stub, metadata, oversize_request
        ):
            with pytest.raises(AioRpcError) as exc_info:
                await event_aggregation_stub.PublishEvent(
                    oversize_request, metadata=metadata
                )

            assert exc_info.value.code() == grpc.StatusCode.INVALID_ARGUMENT

        async def test_names_the_field_and_the_limit(
            self, event_aggregation_stub, metadata, oversize_request
        ):
            with pytest.raises(AioRpcError) as exc_info:
                await event_aggregation_stub.PublishEvent(
                    oversize_request, metadata=metadata
                )

            assert exc_info.value.details().startswith(
                f"event.message must be at most {MAX_EVENT_MESSAGE_BYTES} bytes"
            )

        async def test_does_not_publish(
            self, event_aggregation_stub, metadata, oversize_request
        ):
            with pytest.raises(AioRpcError):
                await event_aggregation_stub.PublishEvent(
                    oversize_request, metadata=metadata
                )

            publisher.publish.assert_not_called()

    class TestWhenTheMessageIsExactlyTheLimit:
        """The limit is inclusive: a message of exactly the limit is published."""

        async def test_publishes(
            self, event_aggregation_stub, metadata, event_with, message_of_size
        ):
            request = PublishEventRequest(
                event=event_with(message=message_of_size(MAX_EVENT_MESSAGE_BYTES))
            )

            await event_aggregation_stub.PublishEvent(request, metadata=metadata)

            publisher.publish.assert_called_once()

    class TestWhenAFieldExceedsItsLength:
        """The REST models' string-length bounds apply on gRPC too."""

        @pytest.fixture(
            params=[
                ("event_type", "event.event_type", MAX_EVENT_TYPE_LENGTH),
                (
                    "resource_id",
                    "event.resource.resource_id",
                    MAX_RESOURCE_ID_LENGTH,
                ),
            ],
            ids=["event_type", "resource_id"],
        )
        def bounded_field(self, request):
            """Return ``(field, location in errors, length limit)``."""
            return request.param

        @pytest.fixture
        def over_length_request(self, event_with, bounded_field) -> PublishEventRequest:
            field, _, limit = bounded_field
            return PublishEventRequest(event=event_with(**{field: "x" * (limit + 1)}))

        async def test_names_the_field(
            self, event_aggregation_stub, metadata, over_length_request, bounded_field
        ):
            _, location, _ = bounded_field

            with pytest.raises(AioRpcError) as exc_info:
                await event_aggregation_stub.PublishEvent(
                    over_length_request, metadata=metadata
                )

            assert exc_info.value.details().startswith(f"{location} must be at most")

        async def test_does_not_publish(
            self, event_aggregation_stub, metadata, over_length_request
        ):
            with pytest.raises(AioRpcError):
                await event_aggregation_stub.PublishEvent(
                    over_length_request, metadata=metadata
                )

            publisher.publish.assert_not_called()

        async def test_publishes_at_the_limit(
            self, event_aggregation_stub, metadata, event_with, bounded_field
        ):
            field, _, limit = bounded_field
            request = PublishEventRequest(event=event_with(**{field: "x" * limit}))

            await event_aggregation_stub.PublishEvent(request, metadata=metadata)

            publisher.publish.assert_called_once()
