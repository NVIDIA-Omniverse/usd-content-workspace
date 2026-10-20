# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import call

import pytest
from app.lib.permissions import permissions
from app.lib.permissions.permissions import PUBLISH_ACTION
from notification_common.config.common_settings import common_settings
from notification_common.permissions.models import Resource


@pytest.fixture(autouse=True)
def enable_permissions(mocker):
    mocker.patch.object(common_settings, "permissions_enabled", True)


@pytest.fixture
def mocked_permissions(mocker):
    return mocker.patch.object(permissions.permissions_client, "can", return_value=True)


@pytest.mark.asyncio
class TestRaiseIfCantPublish:
    async def test_does_nothing_if_permissions_disabled(
        self, mocked_permissions, mocker
    ):
        mocker.patch.object(common_settings, "permissions_enabled", False)

        await permissions.raise_if_cant_publish({"event-type", "other-event-type"})
        mocked_permissions.assert_not_called()

    class TestWhenPermissionsEndpointIsSet:
        @pytest.fixture(autouse=True)
        def setup_endpoint(self):
            common_settings.permissions_endpoint = "test-endpoint"
            yield
            common_settings.permissions_endpoint = None

        async def test_calls_permissions_client_with_all_event_types(
            self, mocked_permissions
        ):
            await permissions.raise_if_cant_publish({"event-type", "other-event-type"})

            mocked_permissions.assert_has_calls(
                [
                    call(
                        PUBLISH_ACTION,
                        resource=Resource(type="EventType", id="event-type"),
                    ),
                    call(
                        PUBLISH_ACTION,
                        resource=Resource(type="EventType", id="other-event-type"),
                    ),
                ],
                any_order=True,
            )

        async def test_doesnt_raise_if_permissions_pass(self, mocked_permissions):
            await permissions.raise_if_cant_publish({"event-type"})

        async def test_raises_if_permissions_fail(self, mocked_permissions):
            mocked_permissions.return_value = False
            expected_msg = "Not allowed to publish events of type event-type"

            with pytest.raises(PermissionError, match=expected_msg):
                await permissions.raise_if_cant_publish({"event-type"})
