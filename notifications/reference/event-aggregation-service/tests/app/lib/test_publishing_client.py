# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import copy

import pytest
from app.lib.rabbitmq import publishing_client, publisher
from notification_common.rabbitmq.publisher import Publisher


@pytest.fixture
def client():
    return publisher


@pytest.fixture(autouse=True)
def mocked_permissions(mocker):
    mocker.patch.object(Publisher, "batch_publish")
    mocker.patch.object(Publisher, "publish")
    return mocker.patch.object(publishing_client, "raise_if_cant_publish")


@pytest.mark.asyncio
class TestBatchPublish:
    async def test_checks_permissions(self, mocked_permissions, mocked_event, client):
        other_event = copy.deepcopy(mocked_event)
        other_event.event_type = "other-event-type"

        await client.batch_publish([mocked_event, other_event])

        mocked_permissions.assert_called_once_with(
            {mocked_event.event_type, "other-event-type"}
        )

    async def test_calls_base_class_method(self, mocked_event, client, mocker):
        spy = mocker.spy(Publisher, "batch_publish")
        await client.batch_publish([mocked_event])
        spy.assert_called_once_with([mocked_event])

    async def test_doesnt_publish_if_permissions_fail(
        self, mocked_event, mocked_permissions, client, mocker
    ):
        mocked_permissions.side_effect = PermissionError()
        spy = mocker.spy(Publisher, "batch_publish")

        with pytest.raises(PermissionError):
            await client.batch_publish([mocked_event])

        spy.assert_not_called()


@pytest.mark.asyncio
class TestPublishWithAuth:
    async def test_checks_permissions(self, mocked_permissions, mocked_event, client):
        await client.publish(mocked_event)
        mocked_permissions.assert_called_once_with({mocked_event.event_type})

    async def test_calls_base_class_method(self, mocked_event, client, mocker):
        spy = mocker.spy(Publisher, "publish")
        await client.publish(mocked_event)
        spy.assert_called_once_with(mocked_event)

    async def test_doesnt_publish_if_permissions_fail(
        self, mocked_event, mocked_permissions, client, mocker
    ):
        mocked_permissions.side_effect = PermissionError()
        spy = mocker.spy(Publisher, "publish")

        with pytest.raises(PermissionError):
            await client.publish(mocked_event)

        spy.assert_not_called()
