# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

from notification_common.errors.errors import ClientFacingServerError

from app.config.settings import settings
from app.lib.models.filter_group import FilterGroup
from app.lib.permissions.storage_permissions import storage_permissions_cache as cache
from app.lib.permissions.storage_permissions.cache_scope import CacheScope
from app.lib.permissions.storage_permissions.directory_contents import ListingOutcome
from app.lib.permissions.storage_permissions.storage_permissions_cache import (
    fetch_from_storage_api,
)

logger = logging.getLogger(__name__)


async def initialize(scope: CacheScope, filter_groups: list[FilterGroup]):
    """
    Need to do some initialization right away when a
    client connects to consume storage events.
    We need to cache everything in the directory
    they are observing, so we can check their access
    when there are storage delete events.

    :param scope: The queue and presenting principal the cache being
        initialized is namespaced by
    :param filter_groups: The filters for the queue being used
    """
    # Skip storage API calls if the storage permission gate is disabled
    if not settings.storage_permissions_enabled:
        return

    storage_event_filter_groups = [
        filter_group
        for filter_group in filter_groups
        if filter_group.is_storage_event_type()
    ]

    resource_filters = [
        _filter
        for filter_group in storage_event_filter_groups
        for _filter in filter_group.filters
    ]

    await cache.init_cache_for_user(scope, resource_filters)


async def can_access_created_event(rabbitmq_message: dict) -> bool:
    """
    If the user may list the dir at the path for the event (a 200
    listing, even one with no visible entries, or the storage API's
    empty-listing 404), then they can see created events in it.
    """
    # Skip storage API calls if the storage permission gate is disabled
    if not settings.storage_permissions_enabled:
        return True

    path = _resource_path(rabbitmq_message)
    dir_contents = await cache.get_parent_resource(path)

    if not dir_contents.authorized:
        dir_contents.dropped_events += 1
        logger.debug(
            "Storage created event skipped: user not authorized for parent dir "
            "of %s (listing outcome %s)",
            path,
            dir_contents.outcome,
        )
        return False

    # add the file path to the cached DirectoryContents object so
    # we will see it there if there's a future delete event for
    # that path
    dir_contents.add_file_path(path)

    return True


async def can_access_deleted_event(rabbitmq_message: dict) -> bool:
    """
    If the user has access to the dir or file that is in
    this event, then they can see the event.
    """
    # Skip storage API calls if the storage permission gate is disabled
    if not settings.storage_permissions_enabled:
        return True

    path = _resource_path(rabbitmq_message)
    dir_contents = await cache.get_parent_resource(path)

    if not dir_contents.authorized:
        dir_contents.dropped_events += 1
        logger.debug(
            "Storage deleted event skipped: user not authorized for parent dir "
            "of %s (listing outcome %s)",
            path,
            dir_contents.outcome,
        )
        return False

    if path in dir_contents.resource_paths:
        dir_contents.remove_file_path(path)
        return True

    if dir_contents.consume_previously_visible_path(path):
        return True

    logger.debug(
        "Storage deleted event skipped: path %s not in user's authorized files",
        path,
    )
    return False


async def can_access_dir_created_event(rabbitmq_message: dict) -> bool:
    """
    If the user may list the parent dir and may list the created dir at
    the path for the event (a 200 listing, even one with no visible
    entries, or the storage API's empty-listing 404), then they can see
    that it was created.
    """
    # Skip storage API calls if the storage permission gate is disabled
    if not settings.storage_permissions_enabled:
        return True

    path = _resource_path(rabbitmq_message)
    parent_dir_contents = await cache.get_parent_resource(path)

    # if they can't access the parent directory, then give up
    if not parent_dir_contents.authorized:
        parent_dir_contents.dropped_events += 1
        logger.debug(
            "Storage dir created event skipped: "
            "user not authorized for parent dir of %s (listing outcome %s)",
            path,
            parent_dir_contents.outcome,
        )
        return False

    # if we already know they have already seen the created dir
    # (from their initial load of the data, or a prev create_dir event),
    # then we don't need to show them this event.
    if path in parent_dir_contents.folders:
        logger.debug(
            "Storage dir created event skipped: user already knows about dir %s", path
        )
        return False

    # see if they can access the directory from the event, that's being created
    if not await _can_access_dir(path):
        logger.debug(
            "Storage dir created event skipped: user may not list dir %s", path
        )
        return False

    # add the folder path to the cached DirectoryContents object so
    # we will see it there if there's a future directory delete event
    parent_dir_contents.add_folder_path(path)

    return True


async def can_access_dir_deleted_event(rabbitmq_message: dict) -> bool:
    """
    If the user has access to the dir, then they have access to the event.
    It's possible that this is a synthetic event and this user
    still has files in the dir.

    If the user still has any files in the (possibly) deleted dir
    at the path for the event, then they have access to the dir,
    but it shouldn't be deleted.
    """
    # Skip storage API calls if the storage permission gate is disabled
    if not settings.storage_permissions_enabled:
        return True

    deleted_dir_path = _resource_path(rabbitmq_message)
    parent_dir_contents = await cache.get_parent_resource(deleted_dir_path)

    # if they can't access the parent directory, then give up
    if not parent_dir_contents.authorized:
        parent_dir_contents.dropped_events += 1
        logger.debug(
            "Storage dir deleted event skipped: "
            "user not authorized for parent dir of %s (listing outcome %s)",
            deleted_dir_path,
            parent_dir_contents.outcome,
        )
        return False

    # if the deleted directory isn't there in their current data,
    # then they haven't seen it so they don't need the delete_dir event
    if deleted_dir_path not in parent_dir_contents.folders:
        logger.debug(
            "Storage dir deleted event skipped: "
            "user hasn't seen dir %s so doesn't need the delete event",
            deleted_dir_path,
        )
        return False

    # they can access the event but need to see if they can still
    # see files in the deleted directory,
    # which would mean this is a synthetic event
    # and the directory wasn't really deleted for this user
    if await _dir_was_not_deleted(deleted_dir_path):
        logger.debug(
            "Storage dir deleted event skipped: "
            "user still has files in dir %s (synthetic delete)",
            deleted_dir_path,
        )
        return False

    # remove the folder path from the cached DirectoryContents object
    # so that if it gets readded we won't skip that event
    parent_dir_contents.remove_folder_path(deleted_dir_path)

    return True


async def _can_access_dir(resource_address: str) -> bool:
    """
    Doesn't check the cache.
    Goes directly to the storage API to see if they may list this folder:
    True for a 200 listing (even one with no visible entries) and for the
    storage API's empty-listing 404.
    """
    dir_path = _ensure_trailing_slash(resource_address)
    dir_contents = await fetch_from_storage_api(dir_path, limit_to_one_result=True)
    return dir_contents.authorized


async def _dir_was_not_deleted(resource_address: str) -> bool:
    """
    Doesn't check the cache.
    Goes directly to the storage API and returns True only for a 200
    listing, which means the folder still holds blobs and a dir-deleted
    event for it is synthetic. The storage API's empty-listing 404 is
    authorized but means the folder is gone, so it returns False and the
    delete is delivered. Distinct from _can_access_dir for that reason.
    """
    dir_path = _ensure_trailing_slash(resource_address)
    dir_contents = await fetch_from_storage_api(dir_path, limit_to_one_result=True)
    return dir_contents.outcome is ListingOutcome.LISTED


def _ensure_trailing_slash(path):
    return path if path.endswith("/") else path + "/"


def _resource_path(rabbitmq_message: dict) -> str:
    try:
        return rabbitmq_message["message"]["resource_address"]
    except KeyError:
        raise ClientFacingServerError(
            "Cannot process a storage event message that "
            "doesn't include a resource address"
        )
