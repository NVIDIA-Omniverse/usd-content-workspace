# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from notification_common.errors.errors import ServerError

# noinspection PyProtectedMember
from app.lib.permissions.storage_permissions.storage_cache import (
    StorageCache,
    _cache_dict_context_var,
)


@pytest.fixture(autouse=True)
def use_redis_mock(mock_redis):
    pass


@pytest.mark.asyncio
class TestGetInstance:
    @pytest.fixture
    def cache_instance(self, cache_scope):
        instance = StorageCache(cache_scope)
        instance["key"] = "value"
        return instance

    class TestWhenAlreadyInContextVar:
        async def test_returns_instance_from_context_var(self, cache_instance):
            _cache_dict_context_var.set(cache_instance)
            assert await StorageCache.get_instance() == cache_instance

    class TestWhenNotAlreadyInContextVar:
        async def test_raises_server_error_if_scope_not_provided(self):
            with pytest.raises(ServerError, match="No cache scope provided"):
                await StorageCache.get_instance()

        async def test_returns_a_new_instance(self, cache_scope):
            assert await StorageCache.get_instance(cache_scope) == {}

        async def test_sets_new_instance_in_context_var(self, cache_scope):
            await StorageCache.get_instance(cache_scope)
            assert _cache_dict_context_var.get() == {}

        async def test_binds_the_new_instance_to_the_scope(self, cache_scope):
            instance = await StorageCache.get_instance(cache_scope)
            assert instance.scope is cache_scope


@pytest.mark.asyncio
class TestClearInstance:
    @pytest.fixture
    def cache_in_context(self, cache_scope):
        instance = StorageCache(cache_scope)
        _cache_dict_context_var.set(instance)
        yield instance
        _cache_dict_context_var.set(None)

    class TestWhenContextVarIsNone:
        async def test_context_var_remains_none(self):
            _cache_dict_context_var.set(None)
            await StorageCache.clear_instance()
            assert _cache_dict_context_var.get() is None

    class TestWhenContextVarIsSet:
        async def test_clears_all_keys_from_cache_dict(self, cache_in_context):
            # noinspection PyTypeChecker
            cache_in_context.update(
                {
                    "storage-permissions:a": "a",
                    "storage-permissions:b": "b",
                }
            )
            await StorageCache.clear_instance()

            assert len(cache_in_context.keys()) == 0

        async def test_sets_context_var_to_none(self, cache_in_context):
            await StorageCache.clear_instance()
            assert _cache_dict_context_var.get() is None

    class TestWhenInstancePassedDirectly:
        async def test_clears_passed_instance(self, cache_scope):
            instance = StorageCache(cache_scope)
            instance["key"] = "value"

            await StorageCache.clear_instance(instance)

            assert len(instance) == 0


class TestGetCurrentInstance:
    def test_returns_instance_when_set(self, cache_scope):
        instance = StorageCache(cache_scope)
        _cache_dict_context_var.set(instance)

        assert StorageCache.get_current_instance() is instance

        _cache_dict_context_var.set(None)

    def test_returns_none_when_not_set(self):
        _cache_dict_context_var.set(None)
        assert StorageCache.get_current_instance() is None


class TestScope:
    """
    The cache reports the whole scope, not a queue name. Its consumers need
    both halves: ``scope.key`` addresses Redis, ``scope.queue_ref`` is the
    only part of it they may log.
    """

    def test_reports_the_scope_it_was_constructed_with(self, cache_scope):
        assert StorageCache(cache_scope).scope is cache_scope

    def test_two_principals_on_one_queue_get_distinct_scope_keys(
        self, cache_scope, second_cache_scope
    ):
        first = StorageCache(cache_scope)
        second = StorageCache(second_cache_scope)

        assert first.scope.key != second.scope.key
