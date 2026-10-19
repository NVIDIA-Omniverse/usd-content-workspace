# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

from notification_common.config.common_settings import common_settings
from notification_common.permissions import permissions_client
from notification_common.permissions.models import Resource
from notification_common.storage.event_types import (
    STORAGE_CREATED,
    STORAGE_DELETED,
    STORAGE_DIR_DELETED,
    STORAGE_DIR_CREATED,
    is_storage_event,
)

from app.config.settings import event_permissions_config
from app.lib.permissions.actions import (
    CONSUME_DURABLE_QUEUES,
    CREATE_DURABLE_QUEUES,
    DELETE_DURABLE_QUEUES,
    CONSUME_STORAGE_CREATE_EVENTS,
    CONSUME_STORAGE_DELETE_EVENTS,
    INSPECT_AND_REPLAY_ALL_DLQ,
)
from app.lib.permissions.storage_permissions.storage_permissions import (
    can_access_created_event,
    can_access_deleted_event,
    can_access_dir_created_event,
    can_access_dir_deleted_event,
)

logger = logging.getLogger(__name__)


async def raise_if_cant_consume_durable():
    return await _can_or_raise(
        CONSUME_DURABLE_QUEUES, "Not allowed to consume from durable queue"
    )


async def raise_if_cant_inspect_and_replay_all_dlq():
    return await _can_or_raise(
        INSPECT_AND_REPLAY_ALL_DLQ,
        "Not allowed to inspect and replay dead-lettered messages",
    )


async def raise_if_cant_create_durable_queue():
    return await _can_or_raise(
        CREATE_DURABLE_QUEUES, "Not allowed to create durable queues"
    )


async def raise_if_cant_delete_durable_queue() -> bool:
    return await _can_or_raise(
        DELETE_DURABLE_QUEUES, "Not allowed to create durable queues"
    )


async def can_consume_durable_queue_event(rabbitmq_message: dict) -> bool:
    """Permissions for events from durable queues"""
    if not common_settings.permissions_enabled:
        return True

    event_type = rabbitmq_message.get("event_type")

    if event_type == STORAGE_CREATED:
        return await permissions_client.can(CONSUME_STORAGE_CREATE_EVENTS)

    if event_type == STORAGE_DELETED:
        return await permissions_client.can(CONSUME_STORAGE_DELETE_EVENTS)

    return await can_do_something_event_specific(rabbitmq_message)


async def can_consume_non_durable_queue_event(rabbitmq_message: dict) -> bool:
    """
    Permissions for events from non-durable queues.

    Each branch is gated by the backend it ultimately calls. The storage
    branches dispatch to the storage-API readers and are governed by
    ``storage_permissions_enabled``; when that gate is disabled they
    short-circuit (the readers return True). The non-storage tail flows
    through ``can_do_something_event_specific`` -> ``permissions_client.can``,
    which short-circuits internally on ``permissions_enabled``.
    """
    event_type = rabbitmq_message.get("event_type")

    if event_type == STORAGE_CREATED:
        return await can_access_created_event(rabbitmq_message)

    if event_type == STORAGE_DELETED:
        return await can_access_deleted_event(rabbitmq_message)

    if event_type == STORAGE_DIR_CREATED:
        return await can_access_dir_created_event(rabbitmq_message)

    if event_type == STORAGE_DIR_DELETED:
        return await can_access_dir_deleted_event(rabbitmq_message)

    return await can_do_something_event_specific(rabbitmq_message)


async def can_do_something_event_specific(rabbitmq_message: dict) -> bool:
    """Per event type permissions defined through configuration"""
    # not storage event type
    event_type = rabbitmq_message["event_type"]

    if is_storage_event(event_type):
        logger.debug(
            "Event skipped: storage event %s not allowed through this path",
            event_type,
        )
        return False

    permissions_config = event_permissions_config(event_type)

    if not permissions_config:
        return True

    resource = Resource(type="EventResourceId", id=rabbitmq_message["resource_id"])

    result = await permissions_client.can(permissions_config.action, resource=resource)
    if not result:
        logger.debug(
            "Event skipped: user not authorized for action=%s on resource_id=%s",
            permissions_config.action,
            rabbitmq_message["resource_id"],
        )
    return result


async def _can_or_raise(
    action: str,
    message: str,
) -> bool:
    if await permissions_client.can(action):
        return True

    raise PermissionError(message)
