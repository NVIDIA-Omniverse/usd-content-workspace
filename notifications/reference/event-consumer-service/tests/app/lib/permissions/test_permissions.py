# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
import pytest_asyncio
from notification_common.config.common_settings import common_settings
from notification_common.permissions.models import Resource
from notification_common.storage.event_types import (
    STORAGE_CREATED,
    STORAGE_DELETED,
    STORAGE_DIR_DELETED,
    STORAGE_DIR_CREATED,
)

from app.config.event_permission import EventPermission
from app.config.settings import settings
from app.lib.permissions import permissions
from app.lib.permissions.storage_permissions import storage_permissions
from app.lib.permissions.actions import INSPECT_AND_REPLAY_ALL_DLQ
from app.lib.permissions.permissions import (
    _can_or_raise,
    can_consume_durable_queue_event,
    can_consume_non_durable_queue_event,
    can_do_something_event_specific,
    permissions_client,
)

# Bind the real _can_or_raise and permissions_client.can at import time. The
# autouse before_each fixture (tests/conftest.py) patches permissions._can_or_raise
# and permissions_client.can for every test, so tests that need the genuine
# allowed/denied or permissions_enabled short-circuit path restore the real ones.
_real_can_or_raise = _can_or_raise
_real_client_can = permissions_client.can


@pytest.fixture(autouse=True)
def enable_permissions(mocker):
    # The split gates two independent backends. ``permissions_enabled`` gates the
    # ``permissions_client.can`` path (read off ``common_settings``); the storage
    # branches of the non-durable path gate on ``storage_permissions_enabled``,
    # read off the consumer-local ``app.config.settings.settings`` singleton (the
    # SAME object the storage readers import). Enable BOTH by default so tests can
    # override either one in isolation.
    mocker.patch.object(common_settings, "permissions_enabled", True)
    mocker.patch.object(settings, "storage_permissions_enabled", True)


@pytest_asyncio.fixture(autouse=True)
async def clear_cached_func():
    # noinspection PyUnresolvedReferences
    permissions_client._cached_can.cache_clear()


@pytest_asyncio.fixture
async def can_or_raise_func(mocker):
    return mocker.patch.object(permissions, "_can_or_raise")


@pytest.mark.asyncio
class TestRaiseIfCantConsumeDurable:
    async def test_calls_can_or_raise_func(self, can_or_raise_func):
        await permissions.raise_if_cant_consume_durable()

        can_or_raise_func.assert_called_once_with(
            "consume-durable-queues", "Not allowed to consume from durable queue"
        )


@pytest.mark.asyncio
class TestRaiseIfCantInspectAndReplayAllDlq:
    async def test_calls_can_or_raise_func(self, can_or_raise_func):
        await permissions.raise_if_cant_inspect_and_replay_all_dlq()

        can_or_raise_func.assert_called_once_with(
            "inspect-and-replay-all-dlq",
            "Not allowed to inspect and replay dead-lettered messages",
        )

    async def test_pins_action_constant(self):
        # Pin the name-only action constant so drift in actions.py breaks the
        # test. Combined with the project name "event-consumer-service", this
        # is the cross-repo deployment-config wire contract
        # "event-consumer-service:inspect-and-replay-all-dlq" (t2.bc7.9n.km).
        assert INSPECT_AND_REPLAY_ALL_DLQ == "inspect-and-replay-all-dlq"


@pytest.mark.asyncio
class TestRaiseIfCantInspectAndReplayAllDlqBranches:
    @pytest.fixture
    def mocked_can(self, mocker):
        # Restore the real _can_or_raise (the autouse before_each fixture mocks
        # it) so this exercises the genuine helper path, and mock only the
        # permissions_client.can boundary.
        mocker.patch.object(permissions, "_can_or_raise", _real_can_or_raise)
        return mocker.patch.object(permissions_client, "can")

    async def test_returns_true_and_does_not_raise_when_allowed(self, mocked_can):
        mocked_can.return_value = True

        result = await permissions.raise_if_cant_inspect_and_replay_all_dlq()

        assert result is True
        mocked_can.assert_called_once_with("inspect-and-replay-all-dlq")

    async def test_raises_permission_error_when_denied(self, mocked_can):
        mocked_can.return_value = False

        with pytest.raises(
            PermissionError,
            match="Not allowed to inspect and replay dead-lettered messages",
        ):
            await permissions.raise_if_cant_inspect_and_replay_all_dlq()


@pytest.mark.asyncio
class TestRaiseIfCantCreateDurableQueue:
    async def test_calls_can_or_raise_func(self, can_or_raise_func):
        await permissions.raise_if_cant_create_durable_queue()

        can_or_raise_func.assert_called_once_with(
            "create-durable-queues", "Not allowed to create durable queues"
        )


@pytest.mark.asyncio
class TestRaiseIfCantDeleteDurableQueue:
    async def test_calls_can_or_raise_func(self, can_or_raise_func):
        await permissions.raise_if_cant_delete_durable_queue()

        can_or_raise_func.assert_called_once_with(
            "delete-durable-queues", "Not allowed to create durable queues"
        )


@pytest.mark.asyncio
class TestCanConsumeDurableQueueEvent:
    @pytest.fixture
    def event_specific_permissions_processor(self, mocker):
        return mocker.patch.object(
            permissions,
            "can_do_something_event_specific",
            return_value="event-specific-results",
        )

    @pytest.fixture
    def client(self, mocker):
        return mocker.patch.object(
            permissions_client, "can", return_value="can_results"
        )

    async def test_returns_true_when_authorization_is_disabled(
        self, mocker, enable_permissions
    ):
        mocker.patch.object(common_settings, "permissions_enabled", False)

        spy = mocker.patch.object(permissions_client, "can", return_value=False)
        result = await can_consume_durable_queue_event({"event_type": STORAGE_CREATED})
        assert result is True
        spy.assert_not_called()

    async def test_storage_branch_unaffected_by_storage_permissions_gate(
        self, mocker, enable_permissions
    ):
        # The durable path goes through permissions_client.can for storage events
        # too, so it is governed by permissions_enabled ONLY. Disabling the
        # storage-API gate must NOT short-circuit the durable storage branch.
        mocker.patch.object(settings, "storage_permissions_enabled", False)

        spy = mocker.patch.object(permissions_client, "can", return_value="can_results")
        result = await can_consume_durable_queue_event({"event_type": STORAGE_CREATED})

        assert result == "can_results"
        spy.assert_called_once_with("consume-all-storage-create-events")

    class TestForNonStorageEvents:
        async def test_calls_can_do_something_event_specific(
            self, event_specific_permissions_processor
        ):
            message = {"event_type": "event-type"}
            res = await can_consume_durable_queue_event(message)
            event_specific_permissions_processor.assert_called_once_with(message)

            assert res == "event-specific-results"

    class TestForStorageEvents:
        @pytest.mark.parametrize(
            "event_type, action",
            [
                (STORAGE_CREATED, "consume-all-storage-create-events"),
                (STORAGE_DELETED, "consume-all-storage-delete-events"),
            ],
        )
        async def test_calls_permissions_client(self, client, event_type, action):
            res = await can_consume_durable_queue_event({"event_type": event_type})
            assert res == "can_results"
            client.assert_called_once_with(action)

        @pytest.mark.parametrize("event_type", [STORAGE_CREATED, STORAGE_DELETED])
        async def test_doesnt_call_event_type_permissions(
            self, event_specific_permissions_processor, event_type
        ):
            await can_consume_durable_queue_event({"event_type": event_type})
            event_specific_permissions_processor.assert_not_called()


@pytest.mark.asyncio
class TestCanConsumeNonDurableQueueEvent:
    @pytest.mark.parametrize("event_type", [STORAGE_CREATED, STORAGE_DELETED])
    async def test_storage_branch_short_circuits_when_storage_gate_disabled(
        self, mocker, enable_permissions, event_type
    ):
        # The non-durable storage branches dispatch to the storage-API readers,
        # which are governed by storage_permissions_enabled (NOT
        # permissions_enabled). With permissions_enabled=True but the storage
        # gate off, the reader short-circuits to True without touching the
        # storage cache (the only downstream backend call).
        mocker.patch.object(settings, "storage_permissions_enabled", False)

        cache_spy = mocker.patch.object(
            storage_permissions.cache, "get_parent_resource"
        )
        message = {"event_type": event_type, "message": {"resource_address": "/a/b"}}
        result = await can_consume_non_durable_queue_event(message)

        assert result is True
        cache_spy.assert_not_called()

    async def test_non_storage_branch_governed_by_permissions_enabled(
        self, mocker, enable_permissions
    ):
        # The non-storage tail flows through can_do_something_event_specific ->
        # permissions_client.can, which short-circuits to True on
        # permissions_enabled BEFORE reaching the backend (_cached_can). With a
        # permission configured for the event, an enabled permissions_enabled
        # would hit _cached_can; disabling it must short-circuit. The storage
        # gate being enabled must not keep this path alive.
        mocker.patch.object(common_settings, "permissions_enabled", False)
        mocker.patch.object(settings, "storage_permissions_enabled", True)
        mocker.patch.object(
            permissions,
            "event_permissions_config",
            return_value=EventPermission(event_type="e-t", action="test-action"),
        )
        # Restore the real client.can (conftest mocks it) so the genuine
        # permissions_enabled short-circuit is exercised; spy on the backend.
        mocker.patch.object(permissions_client, "can", _real_client_can)
        backend = mocker.patch.object(permissions_client, "_cached_can")

        result = await can_consume_non_durable_queue_event(
            {"event_type": "non-storage-event", "resource_id": "r"}
        )

        assert result is True
        backend.assert_not_called()

    @pytest.mark.parametrize(
        "event_type, func",
        [
            (STORAGE_CREATED, "can_access_created_event"),
            (STORAGE_DELETED, "can_access_deleted_event"),
            (STORAGE_DIR_CREATED, "can_access_dir_created_event"),
            (STORAGE_DIR_DELETED, "can_access_dir_deleted_event"),
            ("non-storage-event", "can_do_something_event_specific"),
        ],
    )
    async def test_calls_correct_func_for_each_event_type(
        self, event_type, func, mocker, enable_permissions
    ):
        # Both gates enabled (autouse enable_permissions) so dispatch reaches the
        # readers rather than short-circuiting.
        mocker.patch.object(settings, "storage_permissions_enabled", True)
        spy = mocker.patch.object(permissions, func, return_value="result")
        message = {"event_type": event_type}
        assert await can_consume_non_durable_queue_event(message) == "result"
        spy.assert_called_once_with(message)


@pytest.mark.asyncio
class TestCanDoSomethingEventSpecific:
    @pytest.fixture
    def mocked_can(self, mocker):
        return mocker.patch.object(permissions_client, "can")

    @pytest.mark.parametrize(
        "event_type",
        [STORAGE_CREATED, STORAGE_DELETED, STORAGE_DIR_CREATED, STORAGE_DIR_DELETED],
    )
    async def test_skips_storage_events_by_returning_false(self, event_type, mocker):
        mocker.patch.object(permissions, "is_storage_event", return_value=True)
        assert (
            await can_do_something_event_specific({"event_type": event_type}) is False
        )

    async def test_skips_by_returning_true_when_not_configured_for_event_type(self):
        assert (
            await can_do_something_event_specific({"event_type": "unconfigured-type"})
            is True
        )

    class TestWhenPermissionsAreConfiguredForEventType:
        @pytest.fixture(autouse=True)
        def mock_config(self, mocker):
            event_permission = EventPermission(event_type="e-t", action="test-action")
            mocker.patch.object(
                permissions, "event_permissions_config", return_value=event_permission
            )

        async def test_checks_permissions_api(self, mocked_can):
            expected_resource = Resource(type="EventResourceId", id="test-resource-id")
            await can_do_something_event_specific(
                {"event_type": "e-t", "resource_id": expected_resource.id}
            )

            mocked_can.assert_called_once_with(
                "test-action", resource=expected_resource
            )

        @pytest.mark.parametrize("api_result", [True, False])
        async def test_returns_results_of_permissions_api_call(
            self, mocked_can, api_result
        ):
            mocked_can.return_value = api_result
            result = await can_do_something_event_specific(
                {"event_type": "e-t", "resource_id": "test-resource-id"}
            )

            assert result == api_result


@pytest.mark.asyncio
class TestCanOrRaise:
    async def test_raises_error_if_not_allowed(self, mocker):
        mocker.patch.object(permissions, "_can_or_raise", side_effect=PermissionError())

        with pytest.raises(PermissionError):
            await permissions.raise_if_cant_consume_durable()
