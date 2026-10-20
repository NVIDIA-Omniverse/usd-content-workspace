# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import datetime
import json

import pytest

from notification_common.errors.errors import BadRequestError
from notification_common.rabbitmq.omni_event import OmniEvent
from notification_common.storage.event_types import STORAGE_CREATED


@pytest.fixture
def event_data_no_principal(freezer):
    freezer.move_to("2023-01-01 12:00:00")
    return {
        "event_type": "test-event",
        "occurred_at": datetime.datetime.now(datetime.UTC)
        .isoformat()
        .replace("+00:00", "Z"),
        "resource_id": "test-resource-id",
        "message": {"k": "v"},
    }


@pytest.fixture
def event_data(
    event_data_no_principal,
):
    return {
        **event_data_no_principal,
        "principal_identity": "test-principal",
    }


@pytest.fixture
def omni_event(event_data):
    return OmniEvent(**event_data)


@pytest.fixture
def omni_event_no_principal(event_data_no_principal):
    return OmniEvent(**event_data_no_principal)


class TestInit:
    def test_handles_empty_resource_id(self, event_data: dict[str, str | None]):
        event_data["resource_id"] = None
        event = OmniEvent(**event_data)

        assert event.resource_id == ""

    @pytest.mark.parametrize("expect_principal_identity", [True, False])
    def test_initialization_creates_with_expected_attributes(
        self,
        expect_principal_identity,
        omni_event,
        event_data,
        omni_event_no_principal,
        event_data_no_principal,
    ):
        if expect_principal_identity:
            event_instance = omni_event
            source_data = event_data
        else:
            event_instance = omni_event_no_principal
            source_data = event_data_no_principal

        assert event_instance.event_type == source_data["event_type"]
        assert event_instance.occurred_at == source_data["occurred_at"]
        assert event_instance.resource_id == source_data["resource_id"]
        assert event_instance.message == source_data["message"]

        if expect_principal_identity:
            assert (
                event_instance.principal_identity == source_data["principal_identity"]
            )
        else:
            assert event_instance.principal_identity is None


class TestStr:
    @pytest.mark.parametrize("expect_principal_identity", [True, False])
    def test_str_representation_contains_expected_fields(
        self,
        expect_principal_identity,
        omni_event,
        event_data,
        omni_event_no_principal,
        event_data_no_principal,
    ):
        if expect_principal_identity:
            event_instance = omni_event
            source_data = event_data
        else:
            event_instance = omni_event_no_principal
            source_data = event_data_no_principal

        event_str = str(event_instance)
        event_dict = json.loads(event_str)

        expected_published_at = (
            datetime.datetime.now(datetime.UTC).isoformat().replace("+00:00", "Z")
        )
        expected_dict = {
            **source_data,
            "published_at": expected_published_at,
        }

        assert event_dict == expected_dict


class TestEncodedForPublish:
    def test_returns_utf_8_encoded_event_json_string(self, omni_event):
        expected_bytes = str(omni_event).encode("utf-8")
        assert omni_event.encoded_for_publish() == expected_bytes


class TestValidate:
    @pytest.mark.parametrize("attr_name", ["occurred_at", "message", "event_type"])
    def test_raises_error_for_missing_attr(self, omni_event, attr_name):
        setattr(omni_event, attr_name, "")

        with pytest.raises(BadRequestError, match=f"{attr_name} must be specified"):
            OmniEvent(**vars(omni_event))

    def test_raises_error_for_storage_events_with_missing_resource_id(self, omni_event):
        omni_event.resource_id = None
        omni_event.event_type = STORAGE_CREATED
        expected_msg = "resource_id must be specified for storage events"

        with pytest.raises(BadRequestError, match=expected_msg):
            OmniEvent(**vars(omni_event))

    def test_doesnt_raise_error_for_not_storage_events_with_missing_resource_id(
        self, omni_event
    ):
        omni_event.resource_id = None
        OmniEvent(**vars(omni_event))
