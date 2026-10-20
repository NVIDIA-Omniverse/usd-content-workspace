# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

from contextvars import ContextVar

from notification_common.errors.errors import ServerError

from app.lib.permissions.storage_permissions.cache_scope import CacheScope
from app.lib.permissions.storage_permissions.directory_contents import DirectoryContents

_cache_dict_context_var: ContextVar[StorageCache | None] = ContextVar(
    "storage_permissions_cache", default=None
)


class StorageCache(dict):
    """
    This class contains a dict that we store storage permissions in.
    It also has machinery for setting/getting from a ContestVar.
    """

    def __init__(self, scope: CacheScope) -> None:
        super().__init__()
        self._scope = scope

    @property
    def scope(self) -> CacheScope:
        """
        The (queue, presenting principal) pair this cache instance is bound
        to. The instance itself is per-stream, but what it persists to and
        hydrates from Redis is per-scope.
        """
        return self._scope

    @classmethod
    async def get_instance(
        cls, scope: CacheScope | None = None
    ) -> StorageCache[str, DirectoryContents]:
        """
        The cache dict is either:
            1) Already defined in the ContextVar, so return it.
            2) Non-existent, so we need to create an empty dict and store in
               the ContextVar.
        """
        instance = _cache_dict_context_var.get()

        if instance is not None:
            return instance

        if scope is None:
            raise ServerError("No cache scope provided")

        instance = cls(scope)

        _cache_dict_context_var.set(instance)

        return instance

    @classmethod
    def get_current_instance(cls) -> StorageCache | None:
        """
        Return the current instance from the ContextVar
        without any side effects. Returns None if no
        instance has been set in the current context.
        """
        return _cache_dict_context_var.get()

    @classmethod
    async def clear_instance(cls, instance: StorageCache | None = None) -> None:
        """
        Clear the instance.

        :param instance: If provided, use this instance
            directly instead of looking it up from the
            ContextVar. This is needed when running in a
            background task whose context may not have the
            ContextVar set.
        """
        if instance is None:
            instance = _cache_dict_context_var.get()

        if instance is None:
            return

        instance.clear()

        _cache_dict_context_var.set(None)
