# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import AsyncMock

import grpc
import pytest

from app.grpc import grpc_helpers
from app.grpc.grpc_helpers import (
    describe_non_durable_request,
    validate_consume_durable_request,
    convert_to_event_proto,
    convert_to_models,
)
from app.lib.models.filter_group import FilterGroup
from app.lib.models.filter_type import FilterType
from app.lib.models.resource_filter import ResourceFilter
from app.lib.queue_names import redact_reconnect_token
from notification_common.errors.error_details import ErrorDetails
from notification_common.errors.errors import UnprocessableError
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    Event,
    ConsumeDurableEventsRequest,
    ConsumeDurableEventsWithAcksRequest,
    ConsumeNonDurableEventsRequest,
)
from tests.fixtures.queue_id_bounds import (
    MALFORMED_QUEUE_IDS,
    MALFORMED_QUEUE_ID_IDS,
)
from tests.fixtures.reconnect_token import RECONNECT_TOKEN
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    FilterGroup as ProtoFilterGroup,
)
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ResourceFilter as ProtoResourceFilter,
)


def _proto_filter_group(event_type):
    return ProtoFilterGroup(event_type=event_type)


class TestValidateConsumeDurableRequest:
    """
    The consume surfaces reject a bad queue_id the same way the create
    surface does.

    A malformed id is deferred to the shared ``validate_queue_id``, so
    consume, create, update and delete all fail on the same rule with the
    same status. Only the MISSING case keeps its own abort, because that
    message names the field a client forgot to send and predates the shared
    check.
    """

    @pytest.mark.asyncio
    async def test_valid_request_passes(self):
        request = ConsumeDurableEventsRequest(queue_id="valid-queue-id")
        mock_context = AsyncMock()

        await validate_consume_durable_request(request, mock_context)
        mock_context.abort.assert_not_called()

    @pytest.mark.asyncio
    async def test_invalid_request_aborts_with_invalid_argument(self):
        request = ConsumeDurableEventsRequest(queue_id="")  # Empty queue_id
        mock_context = AsyncMock()

        await validate_consume_durable_request(request, mock_context)
        mock_context.abort.assert_called_once_with(
            grpc.StatusCode.INVALID_ARGUMENT,
            "Queue ID is missing from consume durable request",
        )

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "queue_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    async def test_a_malformed_queue_id_is_rejected(self, queue_id):
        request = ConsumeDurableEventsRequest(queue_id=queue_id)

        with pytest.raises(UnprocessableError):
            await validate_consume_durable_request(request, AsyncMock())

    @pytest.mark.asyncio
    async def test_a_malformed_queue_id_maps_to_invalid_argument(self):
        # The status the client actually receives: the exception
        # interceptor aborts with ``ErrorDetails(error).grpc_code``, so an
        # id rejected here lands on the same code as the missing-id abort
        # above rather than on a second, different error class.
        request = ConsumeDurableEventsRequest(queue_id="bad:id")

        with pytest.raises(UnprocessableError) as exc_info:
            await validate_consume_durable_request(request, AsyncMock())

        assert (
            ErrorDetails(exc_info.value).grpc_code == grpc.StatusCode.INVALID_ARGUMENT
        )

    @pytest.mark.asyncio
    async def test_a_missing_queue_id_does_not_fall_through_to_the_pattern(
        self, mocker
    ):
        # ``abort`` raises in a real servicer context but not under this
        # test double, so the missing case has to RETURN rather than reach
        # the pattern check -- otherwise a client that forgot the field
        # would be told its id failed a regex it never supplied a value
        # for. Patched where the helper module bound the name, so a rename
        # of the import fails here rather than leaving the assertion
        # silently vacuous.
        validate_queue_id = mocker.patch.object(grpc_helpers, "validate_queue_id")
        request = ConsumeDurableEventsRequest(queue_id="")

        await validate_consume_durable_request(request, AsyncMock())

        validate_queue_id.assert_not_called()

    @pytest.mark.asyncio
    async def test_a_missing_queue_id_reports_the_missing_field(self):
        # The other half: returning early must not turn the missing case
        # into a silent success. The client is still told which field it
        # forgot.
        context = AsyncMock()

        await validate_consume_durable_request(
            ConsumeDurableEventsRequest(queue_id=""), context
        )

        context.abort.assert_awaited_once_with(
            grpc.StatusCode.INVALID_ARGUMENT,
            "Queue ID is missing from consume durable request",
        )

    @pytest.mark.asyncio
    async def test_applies_the_same_check_to_the_with_acks_request(self):
        # Both consume surfaces share this helper, and the with-acks one is
        # the surface that would otherwise reach the broker holding a
        # client-settled tracker.
        request = ConsumeDurableEventsWithAcksRequest(queue_id="bad:id")

        with pytest.raises(UnprocessableError):
            await validate_consume_durable_request(request, AsyncMock())


class TestDescribeNonDurableRequest:
    """
    The non-durable request cannot be logged as itself.

    Its ``reconnect_token`` is not a credential: presenting it requires a
    valid bearer token and every delivered event is authorized per principal
    by the permissions policy. It is the queue's identifier, so an
    *authorized* principal that obtains it could attach to the same stream
    and take a share of its events; it is kept out of telemetry as defense
    in depth. Protobuf's text format, what an interpolated message renders
    as, prints every populated field.
    This helper is the summary that replaces it, so what it does NOT contain
    is as load-bearing as what it does.
    """

    @staticmethod
    def _request(**kwargs):
        return ConsumeNonDurableEventsRequest(**kwargs)

    def test_a_fresh_connect_reports_it_is_not_a_reconnect(self):
        assert "is_reconnect=False" in describe_non_durable_request(self._request())

    def test_a_fresh_connect_names_no_queue(self):
        # There is no queue to refer to yet: the stream's first response is
        # where its reference is reported.
        assert "queue_ref=" not in describe_non_durable_request(self._request())

    def test_a_reconnect_reports_it_is_a_reconnect(self):
        request = self._request(reconnect_token=RECONNECT_TOKEN)

        assert "is_reconnect=True" in describe_non_durable_request(request)

    def test_a_reconnect_names_the_redacted_queue_reference(self):
        request = self._request(reconnect_token=RECONNECT_TOKEN)

        assert (
            f"queue_ref={redact_reconnect_token(RECONNECT_TOKEN)}"
            in describe_non_durable_request(request)
        )

    def test_a_reconnect_does_not_name_the_token(self):
        request = self._request(reconnect_token=RECONNECT_TOKEN)

        assert RECONNECT_TOKEN not in describe_non_durable_request(request)

    def test_counts_the_filter_groups(self):
        request = self._request(
            filter_groups=[_proto_filter_group("a"), _proto_filter_group("b")]
        )

        assert "filter_groups=2" in describe_non_durable_request(request)

    def test_counts_the_previous_filter_groups(self):
        request = self._request(previous_filter_groups=[_proto_filter_group("a")])

        assert "previous_filter_groups=1" in describe_non_durable_request(request)

    def test_reports_zero_filter_groups_on_an_empty_request(self):
        assert "filter_groups=0" in describe_non_durable_request(self._request())

    def test_is_a_single_line(self):
        # It is interpolated into one log record; a newline would split the
        # summary across records in the aggregator.
        request = self._request(
            reconnect_token=RECONNECT_TOKEN, filter_groups=[_proto_filter_group("a")]
        )

        assert "\n" not in describe_non_durable_request(request)

    def test_renders_the_whole_summary_for_a_reconnect(self):
        request = self._request(
            reconnect_token=RECONNECT_TOKEN,
            filter_groups=[_proto_filter_group("a"), _proto_filter_group("b")],
            previous_filter_groups=[_proto_filter_group("a")],
        )

        assert describe_non_durable_request(request) == (
            "is_reconnect=True "
            f"queue_ref={redact_reconnect_token(RECONNECT_TOKEN)} "
            "filter_groups=2 previous_filter_groups=1"
        )


class TestConvertToModels:
    @pytest.fixture
    def converted_filter_groups(self):
        filter_type = ProtoResourceFilter.FilterType.FILTER_TYPE_STARTS_WITH_GREEDY

        return self.convert(
            ProtoResourceFilter(filter_type=filter_type, resource_id="test-resource-id")
        )

    @staticmethod
    def convert(proto_resource_filter=None):
        proto_filter_group = ProtoFilterGroup(event_type="test-type")

        if proto_resource_filter:
            proto_filter_group.filters.append(proto_resource_filter)

        # noinspection PyTypeChecker
        return convert_to_models([proto_filter_group])

    def test_converts_proto_filter_group_to_model_filter_group(
        self, converted_filter_groups
    ):
        assert len(converted_filter_groups) == 1
        assert isinstance(converted_filter_groups[0], FilterGroup)

    def test_converted_filter_groups_have_correct_event_type(
        self, converted_filter_groups
    ):
        assert converted_filter_groups[0].event_type == "test-type"

    def test_converted_filter_groups_have_correct_resource_filters(
        self, converted_filter_groups
    ):
        expected_resource_filter = ResourceFilter(
            filter_type=FilterType.STARTS_WITH_GREEDY,
            resource_id="test-resource-id",
        )
        assert converted_filter_groups[0].filters == (expected_resource_filter,)

    def test_can_convert_proto_group_with_no_resource_filters(self):
        converted_filter_groups = self.convert()
        assert converted_filter_groups[0].filters == ()


class TestConvertToEventProto:
    def test_successfully_converts_event_dict_to_event_proto(self):
        event_dict = {
            "event_type": "test-event",
            "principal_identity": "test-user",
            "occurred_at": "2023-01-01T00:00:00Z",
            "published_at": "2023-01-01T00:01:00Z",
            "message": {"key1": "value1", "key2": "value2"},
        }

        result = convert_to_event_proto(event_dict)

        assert isinstance(result, Event)
        assert result.event_type == "test-event"
        assert result.principal_identity == "test-user"
        assert result.message["key1"] == "value1"
        assert result.message["key2"] == "value2"

    def test_successfully_converts_minimal_event_dict_to_event_proto(self):
        event_dict = {
            "event_type": "minimal-event",
            "principal_identity": "test-user",
        }

        result = convert_to_event_proto(event_dict)

        assert result.event_type == "minimal-event"
        assert result.principal_identity == "test-user"

        assert len(result.message) == 0
