# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

from enum import StrEnum
from time import time

from notification_common.errors.errors import ServerError

from app.config import settings
from app.lib.permissions.storage_permissions import redis_cache
from app.lib.permissions.storage_permissions.cache_writer import active_writer


class ListingOutcome(StrEnum):
    """
    How the storage API answered one directory listing.

    The storage service runs its List permission check on the folder before
    it reads any blobs, so ``LISTED`` and ``EMPTY_LISTING`` are both answers
    given only to a principal that may list the folder. ``EMPTY_LISTING`` is
    the storage API's ``404`` for a prefix that holds no blobs (its detail
    text contains "No elements found"); the folder may be genuinely empty, or
    its listing may not have caught up with a write yet. The other three
    outcomes do not authorize the listing.

    The values are the exported ``listing_outcome`` log attribute.
    """

    LISTED = "listed"
    EMPTY_LISTING = "empty_listing"
    FORBIDDEN = "forbidden"
    UNSUPPORTED_ADDRESS = "unsupported_address"
    UNRECOGNIZED_NOT_FOUND = "unrecognized_not_found"

    @property
    def storage_status(self) -> int:
        """The storage-API HTTP status this outcome was classified from."""
        return _STORAGE_STATUS_BY_OUTCOME[self]


_STORAGE_STATUS_BY_OUTCOME: dict[ListingOutcome, int] = {
    ListingOutcome.LISTED: 200,
    ListingOutcome.EMPTY_LISTING: 404,
    ListingOutcome.FORBIDDEN: 403,
    ListingOutcome.UNSUPPORTED_ADDRESS: 400,
    ListingOutcome.UNRECOGNIZED_NOT_FOUND: 404,
}


class DirectoryContents:
    """
    The contents of a directory from the storage API

    Empty (as in "[]" rather than None) resource_paths AND folders,
    means that the user may list the directory, but there are no
    contents in the dir that they have access to. A 200 listing with
    no visible entries and the storage API's empty-listing 404 both
    produce this state.

    On the other hand, if the two lists are set to None,
    then that means the user may not list the directory
    (a 403, an unsupported-address 400, or any other 404).
    """

    def __init__(
        self,
        file_paths: list[str] = None,
        folder_paths: list[str] = None,
        dir_path: str | None = None,
        outcome: ListingOutcome | None = None,
    ):
        super().__init__()

        # both lists having value None means that
        # the user is unauthorized for the dir
        self.authorized = file_paths is not None or folder_paths is not None

        # lists can be empty or None,
        # where None means unauthorized and empty means that the user
        # is authorized for dir but no content in it
        self.resource_paths = set(file_paths) if file_paths is not None else None
        self.folders = set(folder_paths) if folder_paths is not None else None

        # Which storage-API answer produced this entry. Set by
        # fetch_from_storage_api on every return; None for entries built
        # any other way (Redis hydration, tests). In memory only: the Redis
        # meta carries just `authorized` and `expires_at`.
        self.outcome = outcome

        # How many storage events the readers in storage_permissions dropped
        # against this entry because it is unauthorized and that have not
        # been reported yet. storage_permissions_cache reports the count, and
        # resets it to 0, when the entry leaves the stream's cache: when
        # another entry is installed over it (the replaced entry is read at
        # install time), when a filter update removes it, and at stream end.
        # The reset means a count is never reported twice. In memory only.
        self.dropped_events = 0

        # Authorized results (a 200 listing or the storage API's
        # empty-listing 404) are cached for the full permissions TTL.
        # Unauthorized results (a 403, an unsupported-address 400, or any
        # other 404) are not cached by default — _fetch_dir_contents
        # applies a debounce TTL on re-checks so the first event after
        # connect triggers an immediate re-check.
        if self.authorized:
            self.expires_at = settings.settings.permissions_ttl_seconds + time()
        else:
            self.expires_at = 0

        # Paths that the user previously had access to but that
        # are no longer returned by the storage API after a cache
        # refresh. Keyed by path with the value being an expiry
        # timestamp. Used to authorize in-flight delete events
        # for files that were already deleted when the cache
        # was refreshed.
        self.previously_visible_paths: dict[str, float] = {}

        # When None, mutator methods skip per-op Redis enqueue (e.g. test
        # constructions and any code path that doesn't have a directory
        # identity to write through). Production constructions in
        # `_fetch_dir_contents` pass the resource_address.
        self.dir_path = dir_path

    @classmethod
    def from_redis(
        cls,
        authorized: bool,
        files: set[str] | None,
        folders: set[str] | None,
        expires_at: float,
        prev: dict[str, float],
        dir_path: str,
    ) -> DirectoryContents:
        """
        Build a DirectoryContents from Redis-hydrated state without
        re-deriving expires_at — Redis already stores the persisted
        TTL boundary set by the most recent storage-API refresh.
        """
        instance = cls.__new__(cls)
        instance.authorized = authorized
        instance.resource_paths = files if authorized else None
        instance.folders = folders if authorized else None
        instance.expires_at = expires_at
        instance.previously_visible_paths = dict(prev)
        instance.dir_path = dir_path
        # Redis does not persist the listing outcome or the drop count.
        instance.outcome = None
        instance.dropped_events = 0
        return instance

    def add_file_path(self, file_path: str) -> None:
        if not self.authorized:
            raise ServerError("Attempting to add a file to an unauthorized directory")

        self.resource_paths.add(file_path)
        self._enqueue_op(
            lambda writer: redis_cache.add_file(
                scope_key=writer.scope_key,
                dir_path=self.dir_path,
                path=file_path,
            )
        )

    def add_folder_path(self, folder_path: str) -> None:
        if not self.authorized:
            raise ServerError("Attempting to add a dir to an unauthorized directory")

        self.folders.add(folder_path)
        self._enqueue_op(
            lambda writer: redis_cache.add_folder(
                scope_key=writer.scope_key,
                dir_path=self.dir_path,
                path=folder_path,
            )
        )

    def remove_folder_path(self, folder_path: str) -> None:
        if not self.authorized:
            raise ServerError(
                "Attempting to remove a folder from an unauthorized directory"
            )

        try:
            self.folders.remove(folder_path)
        except KeyError:
            """
            Possible there could be concurrency issue where another
            thread already deleted the key.
            """
            pass

        # SREM is idempotent; emit even on the KeyError-swallow path so
        # Redis converges if a different pod's view still has the entry.
        self._enqueue_op(
            lambda writer: redis_cache.remove_folder(
                scope_key=writer.scope_key,
                dir_path=self.dir_path,
                path=folder_path,
            )
        )

    def remove_file_path(self, file_path: str) -> None:
        """
        Remove ``file_path`` from ``resource_paths`` and emit a single
        ``remove_file`` Op. Mirrors :meth:`remove_folder_path`'s shape
        and unconditional Op emission so SREM converges even when the
        path was already gone from the in-memory set.
        """
        if not self.authorized:
            raise ServerError(
                "Attempting to remove a file from an unauthorized directory"
            )

        self.resource_paths.discard(file_path)
        self._enqueue_op(
            lambda writer: redis_cache.remove_file(
                scope_key=writer.scope_key,
                dir_path=self.dir_path,
                path=file_path,
            )
        )

    def absorb_disappeared_paths(self, old: DirectoryContents) -> None:
        """
        When refreshing the cache, paths that were visible in
        the old cache but are no longer in the new listing need
        to be preserved temporarily. This allows in-flight
        delete events for those paths to still be authorized.

        Also carries forward any unexpired previously visible
        paths from the old entry.
        """
        now = time()
        ttl = settings.settings.previously_visible_paths_ttl_seconds

        added: dict[str, float] = {}

        if old.previously_visible_paths:
            for path, expires_at in old.previously_visible_paths.items():
                if expires_at > now and path not in self.resource_paths:
                    self.previously_visible_paths[path] = expires_at
                    added[path] = expires_at

        if old.resource_paths:
            disappeared = old.resource_paths - self.resource_paths
            for path in disappeared:
                expires_at = now + ttl
                self.previously_visible_paths[path] = expires_at
                added[path] = expires_at

        # Single set_prev Op carrying every entry added during this call.
        # Empty mutation -> no Op (Op.apply_to is a no-op for {} but we
        # also avoid the enqueue to keep counters/depth honest).
        if added:
            self._enqueue_op(
                lambda writer: redis_cache.set_prev(
                    scope_key=writer.scope_key,
                    dir_path=self.dir_path,
                    entries=added,
                )
            )

    def consume_previously_visible_path(self, path: str) -> bool:
        """
        Check if the path exists in previously_visible_paths
        and hasn't expired. If so, remove it (it has been
        consumed by a delete event) and return True.
        """
        expires_at = self.previously_visible_paths.get(path)
        if expires_at is None:
            return False

        if expires_at < time():
            del self.previously_visible_paths[path]
            self._enqueue_op(
                lambda writer: redis_cache.del_prev(
                    scope_key=writer.scope_key,
                    dir_path=self.dir_path,
                    path=path,
                )
            )
            return False

        del self.previously_visible_paths[path]
        self._enqueue_op(
            lambda writer: redis_cache.del_prev(
                scope_key=writer.scope_key,
                dir_path=self.dir_path,
                path=path,
            )
        )
        return True

    def expired(self) -> bool:
        return self.expires_at < time()

    def _enqueue_op(self, build_op) -> None:
        """
        Build and enqueue a redis_cache Op via the active CacheWriter.

        Silently skipped when ``dir_path is None`` (no directory identity
        configured for this instance, e.g. tests) or when there is no
        active writer for the bound context (Redis disabled, no consumer
        registered, or ContextVar unset).
        """
        if self.dir_path is None:
            return
        writer = active_writer()
        if writer is None:
            return
        writer.enqueue(build_op(writer))
