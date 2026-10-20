# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import pytest
import redis.exceptions as redis_exceptions
from redis.asyncio.client import Pipeline
from unittest.mock import call

from app.config import settings as _settings_module
from app.lib.permissions.storage_permissions import redis_cache
from app.lib.permissions.storage_permissions.directory_contents import DirectoryContents
from app.lib.permissions.storage_permissions.storage_cache import StorageCache
from app.lib.queue_names import redact_reconnect_token

from tests.fixtures.cache_scope import CACHE_SCOPE, CACHE_SCOPE_KEY, SECOND_CACHE_SCOPE
from tests.fixtures.dropped_storage_events import (
    DROPPED,
    unauthorized_entry_with_drops,
)

# The hydrate helpers take a cache SCOPE, and the only queues that hydrate
# storage permissions are non-durable ones -- whose name embeds the stream's
# reconnect token. Used by the log-hygiene tests at the bottom of this file,
# which are the only ones that care what the scope's queue half looks like.
from tests.fixtures.reconnect_token import RECONNECT_TOKEN

#: What every key in this module is namespaced by: the broker queue name
#: plus the presenting principal's reference. Spelled as the real thing --
#: a queue-only stand-in would let the key tests pass whether or not the
#: principal component reaches the key at all, which is the property the
#: whole design turns on.
SCOPE = CACHE_SCOPE
SCOPE_KEY = CACHE_SCOPE_KEY
DIR_PATH = "/projects/alpha"

#: A second scope on the SAME queue, one principal over. Its keys must not
#: collide with ``SCOPE_KEY``'s.
OTHER_SCOPE_KEY = SECOND_CACHE_SCOPE.key

UUID_SCOPE_KEY = "8a3b9d2e-4c12-4f9a-9b7e-1d2c3e4f5a6b"


class TestMetaKey:
    def test_returns_exact_key_for_representative_input(self):
        assert (
            redis_cache.meta_key(SCOPE_KEY, DIR_PATH)
            == f"cache:{SCOPE_KEY}:meta:/projects/alpha"
        )

    def test_returns_exact_key_when_dir_path_is_empty(self):
        assert redis_cache.meta_key(SCOPE_KEY, "") == f"cache:{SCOPE_KEY}:meta:"

    def test_returns_exact_key_for_uuid_queue_id_with_hyphens(self):
        assert redis_cache.meta_key(UUID_SCOPE_KEY, DIR_PATH) == (
            f"cache:{UUID_SCOPE_KEY}:meta:/projects/alpha"
        )

    def test_returns_exact_key_when_dir_path_contains_colons(self):
        assert (
            redis_cache.meta_key(SCOPE_KEY, "a:b:c") == f"cache:{SCOPE_KEY}:meta:a:b:c"
        )

    def test_returns_exact_key_when_dir_path_contains_special_characters(self):
        assert (
            redis_cache.meta_key(SCOPE_KEY, "/a b/c#d?e")
            == f"cache:{SCOPE_KEY}:meta:/a b/c#d?e"
        )

    def test_same_inputs_produce_same_key(self):
        assert redis_cache.meta_key(SCOPE_KEY, DIR_PATH) == redis_cache.meta_key(
            SCOPE_KEY, DIR_PATH
        )


class TestFilesKey:
    def test_returns_exact_key_for_representative_input(self):
        assert (
            redis_cache.files_key(SCOPE_KEY, DIR_PATH)
            == f"cache:{SCOPE_KEY}:files:/projects/alpha"
        )

    def test_returns_exact_key_when_dir_path_is_empty(self):
        assert redis_cache.files_key(SCOPE_KEY, "") == f"cache:{SCOPE_KEY}:files:"

    def test_returns_exact_key_for_uuid_queue_id_with_hyphens(self):
        assert redis_cache.files_key(UUID_SCOPE_KEY, DIR_PATH) == (
            f"cache:{UUID_SCOPE_KEY}:files:/projects/alpha"
        )

    def test_returns_exact_key_when_dir_path_contains_colons(self):
        assert (
            redis_cache.files_key(SCOPE_KEY, "a:b:c")
            == f"cache:{SCOPE_KEY}:files:a:b:c"
        )

    def test_returns_exact_key_when_dir_path_contains_special_characters(self):
        assert (
            redis_cache.files_key(SCOPE_KEY, "/a b/c#d?e")
            == f"cache:{SCOPE_KEY}:files:/a b/c#d?e"
        )

    def test_same_inputs_produce_same_key(self):
        assert redis_cache.files_key(SCOPE_KEY, DIR_PATH) == redis_cache.files_key(
            SCOPE_KEY, DIR_PATH
        )


class TestFoldersKey:
    def test_returns_exact_key_for_representative_input(self):
        assert (
            redis_cache.folders_key(SCOPE_KEY, DIR_PATH)
            == f"cache:{SCOPE_KEY}:folders:/projects/alpha"
        )

    def test_returns_exact_key_when_dir_path_is_empty(self):
        assert redis_cache.folders_key(SCOPE_KEY, "") == f"cache:{SCOPE_KEY}:folders:"

    def test_returns_exact_key_for_uuid_queue_id_with_hyphens(self):
        assert redis_cache.folders_key(UUID_SCOPE_KEY, DIR_PATH) == (
            f"cache:{UUID_SCOPE_KEY}:folders:/projects/alpha"
        )

    def test_returns_exact_key_when_dir_path_contains_colons(self):
        assert (
            redis_cache.folders_key(SCOPE_KEY, "a:b:c")
            == f"cache:{SCOPE_KEY}:folders:a:b:c"
        )

    def test_returns_exact_key_when_dir_path_contains_special_characters(self):
        assert (
            redis_cache.folders_key(SCOPE_KEY, "/a b/c#d?e")
            == f"cache:{SCOPE_KEY}:folders:/a b/c#d?e"
        )

    def test_same_inputs_produce_same_key(self):
        assert redis_cache.folders_key(SCOPE_KEY, DIR_PATH) == (
            redis_cache.folders_key(SCOPE_KEY, DIR_PATH)
        )


class TestPrevKey:
    def test_returns_exact_key_for_representative_input(self):
        assert (
            redis_cache.prev_key(SCOPE_KEY, DIR_PATH)
            == f"cache:{SCOPE_KEY}:prev:/projects/alpha"
        )

    def test_returns_exact_key_when_dir_path_is_empty(self):
        assert redis_cache.prev_key(SCOPE_KEY, "") == f"cache:{SCOPE_KEY}:prev:"

    def test_returns_exact_key_for_uuid_queue_id_with_hyphens(self):
        assert redis_cache.prev_key(UUID_SCOPE_KEY, DIR_PATH) == (
            f"cache:{UUID_SCOPE_KEY}:prev:/projects/alpha"
        )

    def test_returns_exact_key_when_dir_path_contains_colons(self):
        assert (
            redis_cache.prev_key(SCOPE_KEY, "a:b:c") == f"cache:{SCOPE_KEY}:prev:a:b:c"
        )

    def test_returns_exact_key_when_dir_path_contains_special_characters(self):
        assert (
            redis_cache.prev_key(SCOPE_KEY, "/a b/c#d?e")
            == f"cache:{SCOPE_KEY}:prev:/a b/c#d?e"
        )

    def test_same_inputs_produce_same_key(self):
        assert redis_cache.prev_key(SCOPE_KEY, DIR_PATH) == redis_cache.prev_key(
            SCOPE_KEY, DIR_PATH
        )


class TestDirsKey:
    def test_returns_exact_key_for_representative_input(self):
        assert redis_cache.dirs_key(SCOPE_KEY) == f"cache:{SCOPE_KEY}:dirs"

    def test_returns_exact_key_for_uuid_queue_id_with_hyphens(self):
        assert redis_cache.dirs_key(UUID_SCOPE_KEY) == f"cache:{UUID_SCOPE_KEY}:dirs"

    def test_returns_exact_key_when_queue_id_is_empty(self):
        assert redis_cache.dirs_key("") == "cache::dirs"

    def test_same_inputs_produce_same_key(self):
        assert redis_cache.dirs_key(SCOPE_KEY) == redis_cache.dirs_key(SCOPE_KEY)


class TestDirPathNotNormalized:
    def test_meta_key_preserves_trailing_slash(self):
        assert (
            redis_cache.meta_key(SCOPE_KEY, "/foo/") == f"cache:{SCOPE_KEY}:meta:/foo/"
        )

    def test_files_key_preserves_trailing_slash(self):
        assert (
            redis_cache.files_key(SCOPE_KEY, "/foo/")
            == f"cache:{SCOPE_KEY}:files:/foo/"
        )

    def test_folders_key_preserves_trailing_slash(self):
        assert (
            redis_cache.folders_key(SCOPE_KEY, "/foo/")
            == f"cache:{SCOPE_KEY}:folders:/foo/"
        )

    def test_prev_key_preserves_trailing_slash(self):
        assert (
            redis_cache.prev_key(SCOPE_KEY, "/foo/") == f"cache:{SCOPE_KEY}:prev:/foo/"
        )

    def test_meta_key_preserves_dir_path_without_leading_slash(self):
        assert redis_cache.meta_key(SCOPE_KEY, "foo") == f"cache:{SCOPE_KEY}:meta:foo"

    def test_files_key_preserves_dir_path_without_leading_slash(self):
        assert redis_cache.files_key(SCOPE_KEY, "foo") == f"cache:{SCOPE_KEY}:files:foo"

    def test_folders_key_preserves_dir_path_without_leading_slash(self):
        assert (
            redis_cache.folders_key(SCOPE_KEY, "foo")
            == f"cache:{SCOPE_KEY}:folders:foo"
        )

    def test_prev_key_preserves_dir_path_without_leading_slash(self):
        assert redis_cache.prev_key(SCOPE_KEY, "foo") == f"cache:{SCOPE_KEY}:prev:foo"


class TestKeysAreNamespacedPerScope:
    """
    The isolation the scope key exists for, at the storage layer: two
    principals attached to ONE non-durable queue must never read or
    overwrite each other's cached directory state.
    """

    @pytest.mark.parametrize(
        "key_helper",
        [
            redis_cache.meta_key,
            redis_cache.files_key,
            redis_cache.folders_key,
            redis_cache.prev_key,
        ],
        ids=["meta", "files", "folders", "prev"],
    )
    def test_per_dir_key_differs_between_two_scopes_on_one_queue(self, key_helper):
        assert key_helper(SCOPE_KEY, DIR_PATH) != key_helper(OTHER_SCOPE_KEY, DIR_PATH)

    def test_dirs_index_key_differs_between_two_scopes_on_one_queue(self):
        # The reconnect path reads this index to decide what to hydrate, so
        # a shared one would hand a second principal the first's directory
        # list even if every per-dir key were separate.
        assert redis_cache.dirs_key(SCOPE_KEY) != redis_cache.dirs_key(OTHER_SCOPE_KEY)

    def test_key_carries_the_principal_reference(self):
        assert SECOND_CACHE_SCOPE.principal_ref in redis_cache.dirs_key(OTHER_SCOPE_KEY)


class TestTtlAccessor:
    def test_returns_buffered_ttl_seconds_from_settings(self):
        assert (
            redis_cache.ttl_seconds() == _settings_module.settings.buffered_ttl_seconds
        )

    def test_returns_int(self):
        assert isinstance(redis_cache.ttl_seconds(), int)

    def test_reflects_changes_to_underlying_setting(self, mocker):
        mocker.patch.object(
            type(_settings_module.settings),
            "buffered_ttl_seconds",
            new_callable=mocker.PropertyMock,
        ).return_value = 999
        assert redis_cache.ttl_seconds() == 999


# --- Op class apply_to() tests ---


# Pipeline staging methods (sadd/srem/hset/hdel/delete/expire) are sync on the
# real redis-py async pipeline; only execute() is a coroutine. create_autospec
# against the real class produces the right MagicMock/AsyncMock split per the
# subtask's "AsyncMock(autospec=True) against a redis-py async pipeline" intent.
@pytest.fixture
def pipeline(mocker):
    return mocker.create_autospec(Pipeline, instance=True)


@pytest.fixture
def fixed_ttl(mocker):
    """Pin ttl_seconds() so EXPIRE assertions can use a known value."""
    mocker.patch.object(redis_cache, "ttl_seconds", return_value=42)
    return 42


FILES_KEY = f"cache:{SCOPE_KEY}:files:/projects/alpha"
FOLDERS_KEY = f"cache:{SCOPE_KEY}:folders:/projects/alpha"
PREV_KEY = f"cache:{SCOPE_KEY}:prev:/projects/alpha"
META_KEY = f"cache:{SCOPE_KEY}:meta:/projects/alpha"
DIRS_KEY = f"cache:{SCOPE_KEY}:dirs"


class TestAddFile:
    def test_stages_sadd_then_expire_in_order(self, pipeline, fixed_ttl):
        redis_cache.add_file(SCOPE_KEY, DIR_PATH, "f.txt").apply_to(pipeline)
        assert pipeline.method_calls == [
            call.sadd(FILES_KEY, "f.txt"),
            call.expire(FILES_KEY, fixed_ttl),
        ]

    def test_does_not_call_execute(self, pipeline, fixed_ttl):
        redis_cache.add_file(SCOPE_KEY, DIR_PATH, "f.txt").apply_to(pipeline)
        pipeline.execute.assert_not_called()

    def test_handles_path_with_special_characters(self, pipeline, fixed_ttl):
        redis_cache.add_file(SCOPE_KEY, DIR_PATH, "weird path/with:colons#?").apply_to(
            pipeline
        )
        assert pipeline.method_calls == [
            call.sadd(FILES_KEY, "weird path/with:colons#?"),
            call.expire(FILES_KEY, fixed_ttl),
        ]

    def test_equality_for_identical_fields(self):
        a = redis_cache.add_file(SCOPE_KEY, DIR_PATH, "f.txt")
        b = redis_cache.add_file(SCOPE_KEY, DIR_PATH, "f.txt")
        assert a == b

    def test_inequality_for_different_path(self):
        a = redis_cache.add_file(SCOPE_KEY, DIR_PATH, "f.txt")
        b = redis_cache.add_file(SCOPE_KEY, DIR_PATH, "g.txt")
        assert a != b


class TestRemoveFile:
    def test_stages_only_srem(self, pipeline):
        redis_cache.remove_file(SCOPE_KEY, DIR_PATH, "f.txt").apply_to(pipeline)
        assert pipeline.method_calls == [call.srem(FILES_KEY, "f.txt")]

    def test_does_not_stage_expire(self, pipeline):
        redis_cache.remove_file(SCOPE_KEY, DIR_PATH, "f.txt").apply_to(pipeline)
        pipeline.expire.assert_not_called()

    def test_does_not_call_execute(self, pipeline):
        redis_cache.remove_file(SCOPE_KEY, DIR_PATH, "f.txt").apply_to(pipeline)
        pipeline.execute.assert_not_called()

    def test_equality_for_identical_fields(self):
        a = redis_cache.remove_file(SCOPE_KEY, DIR_PATH, "f.txt")
        b = redis_cache.remove_file(SCOPE_KEY, DIR_PATH, "f.txt")
        assert a == b


class TestAddFolder:
    def test_stages_sadd_then_expire_in_order(self, pipeline, fixed_ttl):
        redis_cache.add_folder(SCOPE_KEY, DIR_PATH, "sub").apply_to(pipeline)
        assert pipeline.method_calls == [
            call.sadd(FOLDERS_KEY, "sub"),
            call.expire(FOLDERS_KEY, fixed_ttl),
        ]

    def test_does_not_call_execute(self, pipeline, fixed_ttl):
        redis_cache.add_folder(SCOPE_KEY, DIR_PATH, "sub").apply_to(pipeline)
        pipeline.execute.assert_not_called()

    def test_handles_path_with_special_characters(self, pipeline, fixed_ttl):
        redis_cache.add_folder(SCOPE_KEY, DIR_PATH, "/a b/c#d").apply_to(pipeline)
        assert pipeline.method_calls == [
            call.sadd(FOLDERS_KEY, "/a b/c#d"),
            call.expire(FOLDERS_KEY, fixed_ttl),
        ]

    def test_equality_for_identical_fields(self):
        a = redis_cache.add_folder(SCOPE_KEY, DIR_PATH, "sub")
        b = redis_cache.add_folder(SCOPE_KEY, DIR_PATH, "sub")
        assert a == b


class TestRemoveFolder:
    def test_stages_only_srem(self, pipeline):
        redis_cache.remove_folder(SCOPE_KEY, DIR_PATH, "sub").apply_to(pipeline)
        assert pipeline.method_calls == [call.srem(FOLDERS_KEY, "sub")]

    def test_does_not_stage_expire(self, pipeline):
        redis_cache.remove_folder(SCOPE_KEY, DIR_PATH, "sub").apply_to(pipeline)
        pipeline.expire.assert_not_called()

    def test_does_not_call_execute(self, pipeline):
        redis_cache.remove_folder(SCOPE_KEY, DIR_PATH, "sub").apply_to(pipeline)
        pipeline.execute.assert_not_called()

    def test_equality_for_identical_fields(self):
        a = redis_cache.remove_folder(SCOPE_KEY, DIR_PATH, "sub")
        b = redis_cache.remove_folder(SCOPE_KEY, DIR_PATH, "sub")
        assert a == b


class TestSetMeta:
    def test_stages_hset_then_expire_in_order(self, pipeline, fixed_ttl):
        redis_cache.set_meta(SCOPE_KEY, DIR_PATH, True, 1234.5).apply_to(pipeline)
        assert pipeline.method_calls == [
            call.hset(META_KEY, mapping={"authorized": 1, "expires_at": 1234.5}),
            call.expire(META_KEY, fixed_ttl),
        ]

    def test_stores_authorized_false_as_int_zero(self, pipeline, fixed_ttl):
        redis_cache.set_meta(SCOPE_KEY, DIR_PATH, False, 9.0).apply_to(pipeline)
        assert pipeline.method_calls == [
            call.hset(META_KEY, mapping={"authorized": 0, "expires_at": 9.0}),
            call.expire(META_KEY, fixed_ttl),
        ]

    def test_does_not_call_execute(self, pipeline, fixed_ttl):
        redis_cache.set_meta(SCOPE_KEY, DIR_PATH, True, 1234.5).apply_to(pipeline)
        pipeline.execute.assert_not_called()

    def test_equality_for_identical_fields(self):
        a = redis_cache.set_meta(SCOPE_KEY, DIR_PATH, True, 1.0)
        b = redis_cache.set_meta(SCOPE_KEY, DIR_PATH, True, 1.0)
        assert a == b


class TestSetPrev:
    def test_stages_per_entry_hset_then_single_expire(self, pipeline, fixed_ttl):
        entries = {"a.txt": 100.0, "b.txt": 200.0}
        redis_cache.set_prev(SCOPE_KEY, DIR_PATH, entries).apply_to(pipeline)
        assert pipeline.method_calls == [
            call.hset(PREV_KEY, "a.txt", 100.0),
            call.hset(PREV_KEY, "b.txt", 200.0),
            call.expire(PREV_KEY, fixed_ttl),
        ]

    def test_empty_mapping_stages_zero_pipeline_calls(self, pipeline, fixed_ttl):
        redis_cache.set_prev(SCOPE_KEY, DIR_PATH, {}).apply_to(pipeline)
        assert pipeline.method_calls == []

    def test_empty_mapping_does_not_stage_expire(self, pipeline, fixed_ttl):
        redis_cache.set_prev(SCOPE_KEY, DIR_PATH, {}).apply_to(pipeline)
        pipeline.expire.assert_not_called()

    def test_long_member_list_pipelines_in_one_call(self, pipeline, fixed_ttl):
        entries = {f"path-{i}": float(i) for i in range(500)}
        redis_cache.set_prev(SCOPE_KEY, DIR_PATH, entries).apply_to(pipeline)
        expected = [call.hset(PREV_KEY, f"path-{i}", float(i)) for i in range(500)]
        expected.append(call.expire(PREV_KEY, fixed_ttl))
        assert pipeline.method_calls == expected

    def test_handles_paths_with_special_characters(self, pipeline, fixed_ttl):
        redis_cache.set_prev(
            SCOPE_KEY, DIR_PATH, {"a:b#c?": 1.0, "/x y/z": 2.0}
        ).apply_to(pipeline)
        assert pipeline.method_calls == [
            call.hset(PREV_KEY, "a:b#c?", 1.0),
            call.hset(PREV_KEY, "/x y/z", 2.0),
            call.expire(PREV_KEY, fixed_ttl),
        ]

    def test_does_not_call_execute(self, pipeline, fixed_ttl):
        redis_cache.set_prev(SCOPE_KEY, DIR_PATH, {"a": 1.0}).apply_to(pipeline)
        pipeline.execute.assert_not_called()

    def test_equality_for_identical_fields(self):
        a = redis_cache.set_prev(SCOPE_KEY, DIR_PATH, {"a": 1.0})
        b = redis_cache.set_prev(SCOPE_KEY, DIR_PATH, {"a": 1.0})
        assert a == b


class TestDelPrev:
    def test_stages_only_hdel(self, pipeline):
        redis_cache.del_prev(SCOPE_KEY, DIR_PATH, "a.txt").apply_to(pipeline)
        assert pipeline.method_calls == [call.hdel(PREV_KEY, "a.txt")]

    def test_does_not_stage_expire(self, pipeline):
        redis_cache.del_prev(SCOPE_KEY, DIR_PATH, "a.txt").apply_to(pipeline)
        pipeline.expire.assert_not_called()

    def test_does_not_call_execute(self, pipeline):
        redis_cache.del_prev(SCOPE_KEY, DIR_PATH, "a.txt").apply_to(pipeline)
        pipeline.execute.assert_not_called()

    def test_equality_for_identical_fields(self):
        a = redis_cache.del_prev(SCOPE_KEY, DIR_PATH, "a.txt")
        b = redis_cache.del_prev(SCOPE_KEY, DIR_PATH, "a.txt")
        assert a == b


class TestEnsureDirIndex:
    def test_stages_sadd_then_expire_in_order(self, pipeline, fixed_ttl):
        redis_cache.ensure_dir_index(SCOPE_KEY, DIR_PATH).apply_to(pipeline)
        assert pipeline.method_calls == [
            call.sadd(DIRS_KEY, DIR_PATH),
            call.expire(DIRS_KEY, fixed_ttl),
        ]

    def test_does_not_call_execute(self, pipeline, fixed_ttl):
        redis_cache.ensure_dir_index(SCOPE_KEY, DIR_PATH).apply_to(pipeline)
        pipeline.execute.assert_not_called()

    def test_handles_dir_path_with_special_characters(self, pipeline, fixed_ttl):
        redis_cache.ensure_dir_index(SCOPE_KEY, "/a b/c:d#?").apply_to(pipeline)
        assert pipeline.method_calls == [
            call.sadd(DIRS_KEY, "/a b/c:d#?"),
            call.expire(DIRS_KEY, fixed_ttl),
        ]

    def test_equality_for_identical_fields(self):
        a = redis_cache.ensure_dir_index(SCOPE_KEY, DIR_PATH)
        b = redis_cache.ensure_dir_index(SCOPE_KEY, DIR_PATH)
        assert a == b


class TestReplaceDir:
    def test_stages_full_sequence_with_populated_collections(self, pipeline, fixed_ttl):
        op = redis_cache.replace_dir(
            scope_key=SCOPE_KEY,
            dir_path=DIR_PATH,
            files=frozenset({"f1"}),
            folders=frozenset({"d1"}),
            authorized=True,
            expires_at=99.0,
            prev={"p1": 1.0},
        )
        op.apply_to(pipeline)
        assert pipeline.method_calls == [
            call.delete(FILES_KEY),
            call.delete(FOLDERS_KEY),
            call.delete(PREV_KEY),
            call.sadd(FILES_KEY, "f1"),
            call.expire(FILES_KEY, fixed_ttl),
            call.sadd(FOLDERS_KEY, "d1"),
            call.expire(FOLDERS_KEY, fixed_ttl),
            call.hset(META_KEY, mapping={"authorized": 1, "expires_at": 99.0}),
            call.expire(META_KEY, fixed_ttl),
            call.hset(PREV_KEY, "p1", 1.0),
            call.expire(PREV_KEY, fixed_ttl),
            call.sadd(DIRS_KEY, DIR_PATH),
            call.expire(DIRS_KEY, fixed_ttl),
        ]

    def test_stages_full_sequence_with_empty_collections(self, pipeline, fixed_ttl):
        op = redis_cache.replace_dir(
            scope_key=SCOPE_KEY,
            dir_path=DIR_PATH,
            authorized=False,
            expires_at=0.0,
        )
        op.apply_to(pipeline)
        assert pipeline.method_calls == [
            call.delete(FILES_KEY),
            call.delete(FOLDERS_KEY),
            call.delete(PREV_KEY),
            call.expire(FILES_KEY, fixed_ttl),
            call.expire(FOLDERS_KEY, fixed_ttl),
            call.hset(META_KEY, mapping={"authorized": 0, "expires_at": 0.0}),
            call.expire(META_KEY, fixed_ttl),
            call.sadd(DIRS_KEY, DIR_PATH),
            call.expire(DIRS_KEY, fixed_ttl),
        ]

    def test_delete_precedes_sadd_on_files_key(self, pipeline, fixed_ttl):
        op = redis_cache.replace_dir(
            scope_key=SCOPE_KEY,
            dir_path=DIR_PATH,
            files=frozenset({"f1"}),
        )
        op.apply_to(pipeline)
        files_calls = [
            i
            for i, c in enumerate(pipeline.method_calls)
            if c.args and c.args[0] == FILES_KEY
        ]
        first_delete = next(
            i for i in files_calls if pipeline.method_calls[i][0] == "delete"
        )
        first_sadd = next(
            i for i in files_calls if pipeline.method_calls[i][0] == "sadd"
        )
        assert first_delete < first_sadd

    def test_delete_precedes_sadd_on_folders_key(self, pipeline, fixed_ttl):
        op = redis_cache.replace_dir(
            scope_key=SCOPE_KEY,
            dir_path=DIR_PATH,
            folders=frozenset({"d1"}),
        )
        op.apply_to(pipeline)
        folders_calls = [
            i
            for i, c in enumerate(pipeline.method_calls)
            if c.args and c.args[0] == FOLDERS_KEY
        ]
        first_delete = next(
            i for i in folders_calls if pipeline.method_calls[i][0] == "delete"
        )
        first_sadd = next(
            i for i in folders_calls if pipeline.method_calls[i][0] == "sadd"
        )
        assert first_delete < first_sadd

    def test_delete_precedes_hset_on_prev_key(self, pipeline, fixed_ttl):
        op = redis_cache.replace_dir(
            scope_key=SCOPE_KEY,
            dir_path=DIR_PATH,
            prev={"p1": 1.0},
        )
        op.apply_to(pipeline)
        prev_calls = [
            i
            for i, c in enumerate(pipeline.method_calls)
            if c.args and c.args[0] == PREV_KEY
        ]
        first_delete = next(
            i for i in prev_calls if pipeline.method_calls[i][0] == "delete"
        )
        first_hset = next(
            i for i in prev_calls if pipeline.method_calls[i][0] == "hset"
        )
        assert first_delete < first_hset

    def test_does_not_call_execute(self, pipeline, fixed_ttl):
        redis_cache.replace_dir(SCOPE_KEY, DIR_PATH).apply_to(pipeline)
        pipeline.execute.assert_not_called()

    def test_long_member_lists_pipelined_in_one_sadd_per_set(self, pipeline, fixed_ttl):
        files = frozenset(f"f{i}" for i in range(500))
        folders = frozenset(f"d{i}" for i in range(500))
        op = redis_cache.replace_dir(
            scope_key=SCOPE_KEY,
            dir_path=DIR_PATH,
            files=files,
            folders=folders,
        )
        op.apply_to(pipeline)
        sadd_files = [
            c
            for c in pipeline.method_calls
            if c[0] == "sadd" and c.args[0] == FILES_KEY
        ]
        sadd_folders = [
            c
            for c in pipeline.method_calls
            if c[0] == "sadd" and c.args[0] == FOLDERS_KEY
        ]
        assert len(sadd_files) == 1
        assert len(sadd_folders) == 1
        assert set(sadd_files[0].args[1:]) == files
        assert set(sadd_folders[0].args[1:]) == folders

    def test_equality_for_identical_fields(self):
        a = redis_cache.replace_dir(
            SCOPE_KEY,
            DIR_PATH,
            files=frozenset({"f1"}),
            folders=frozenset({"d1"}),
            authorized=True,
            expires_at=1.0,
            prev={"p": 2.0},
        )
        b = redis_cache.replace_dir(
            SCOPE_KEY,
            DIR_PATH,
            files=frozenset({"f1"}),
            folders=frozenset({"d1"}),
            authorized=True,
            expires_at=1.0,
            prev={"p": 2.0},
        )
        assert a == b


# Parametrized TTL refresh check across write Ops: every write Op must
# refresh EXPIRE with the value returned by ttl_seconds(), and that value
# must track settings.buffered_ttl_seconds.
@pytest.mark.parametrize(
    "op_factory, expire_keys",
    [
        (
            lambda: redis_cache.add_file(SCOPE_KEY, DIR_PATH, "f"),
            [FILES_KEY],
        ),
        (
            lambda: redis_cache.add_folder(SCOPE_KEY, DIR_PATH, "d"),
            [FOLDERS_KEY],
        ),
        (
            lambda: redis_cache.set_meta(SCOPE_KEY, DIR_PATH, True, 1.0),
            [META_KEY],
        ),
        (
            lambda: redis_cache.set_prev(SCOPE_KEY, DIR_PATH, {"p": 1.0}),
            [PREV_KEY],
        ),
        (
            lambda: redis_cache.ensure_dir_index(SCOPE_KEY, DIR_PATH),
            [DIRS_KEY],
        ),
        (
            lambda: redis_cache.replace_dir(
                SCOPE_KEY,
                DIR_PATH,
                files=frozenset({"f"}),
                folders=frozenset({"d"}),
                prev={"p": 1.0},
            ),
            [FILES_KEY, FOLDERS_KEY, META_KEY, PREV_KEY, DIRS_KEY],
        ),
    ],
)
class TestExpireRefreshesEveryTouchedKey:
    def test_expire_called_with_fixed_ttl(self, op_factory, expire_keys, pipeline):
        ttl = 7
        with pytest.MonkeyPatch.context() as m:
            m.setattr(redis_cache, "ttl_seconds", lambda: ttl)
            op_factory().apply_to(pipeline)
        expire_calls = [c.args for c in pipeline.method_calls if c[0] == "expire"]
        for key in expire_keys:
            assert (key, ttl) in expire_calls

    def test_change_to_buffered_ttl_setting_flows_through(
        self, op_factory, expire_keys, pipeline, mocker
    ):
        mocker.patch.object(
            type(_settings_module.settings),
            "buffered_ttl_seconds",
            new_callable=mocker.PropertyMock,
            return_value=314,
        )
        op_factory().apply_to(pipeline)
        expire_calls = [c.args for c in pipeline.method_calls if c[0] == "expire"]
        for key in expire_keys:
            assert (key, 314) in expire_calls


# --- hydrate_dir_from_redis + DirectoryContents.from_redis tests ---


@pytest.fixture
def mock_pipeline(mocker):
    """
    Async pipeline mock matching the redis-py shape: staging methods are
    sync MagicMocks, ``execute`` is an AsyncMock returning a 4-tuple
    matching the helper's expected unpack order.
    """
    pipeline = mocker.AsyncMock(spec=Pipeline)
    return pipeline


@pytest.fixture
def mock_redis_client(mocker, mock_pipeline):
    """
    Patch ``redis_client.redis_client`` (the binding the helper reads via
    `redis_client.redis_client.pipeline()`) with a client whose
    `pipeline()` returns the autospec'd pipeline mock.
    """
    client = mocker.MagicMock()
    client.pipeline.return_value = mock_pipeline
    mocker.patch.object(redis_cache.redis_client, "redis_client", client)
    return client


@pytest.fixture(autouse=True)
def redis_enabled(mocker):
    """Default the helper into the enabled-and-clienful happy path."""
    mocker.patch.object(_settings_module.settings, "redis_enabled", True)


@pytest.fixture
def freeze_now(mocker):
    """Pin ``time()`` inside redis_cache so expires_at comparisons are deterministic."""
    mocker.patch.object(redis_cache, "time", return_value=1_000_000.0)
    return 1_000_000.0


def _populated_pipeline_result(*, expires_at: float, authorized: bool = True):
    """
    Build the 4-tuple shape the helper unpacks from ``pipeline.execute()``:
    (meta_hgetall_dict, files_smembers_set, folders_smembers_set, prev_hgetall_dict).
    All keys/values are bytes — matching real Redis responses with
    decode_responses=False.
    """
    meta = {
        b"authorized": b"1" if authorized else b"0",
        b"expires_at": str(expires_at).encode("utf-8"),
    }
    files = {b"f1.txt", b"f2.txt"}
    folders = {b"sub-a", b"sub-b"}
    prev = {b"gone.txt": b"500.5", b"also-gone.txt": b"600.0"}
    return [meta, files, folders, prev]


@pytest.mark.asyncio
class TestHydrateFromRedis:
    async def test_returns_dir_contents_when_pipeline_returns_full_state(
        self, mock_pipeline, mock_redis_client, freeze_now
    ):
        mock_pipeline.execute.return_value = _populated_pipeline_result(
            expires_at=freeze_now + 100
        )

        result = await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert isinstance(result, DirectoryContents)
        assert result.authorized is True
        assert result.resource_paths == {"f1.txt", "f2.txt"}
        assert result.folders == {"sub-a", "sub-b"}
        assert result.expires_at == freeze_now + 100
        assert result.previously_visible_paths == {
            "gone.txt": 500.5,
            "also-gone.txt": 600.0,
        }
        assert result.dir_path == DIR_PATH

    async def test_returns_none_when_meta_hash_is_absent(
        self, mock_pipeline, mock_redis_client, freeze_now
    ):
        mock_pipeline.execute.return_value = [{}, set(), set(), {}]

        result = await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert result is None

    async def test_returns_none_when_meta_expires_at_is_in_the_past(
        self, mock_pipeline, mock_redis_client, freeze_now
    ):
        mock_pipeline.execute.return_value = _populated_pipeline_result(
            expires_at=freeze_now - 1.0
        )

        result = await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert result is None

    async def test_returns_none_when_meta_expires_at_is_malformed(
        self, mock_pipeline, mock_redis_client, freeze_now
    ):
        meta = {b"authorized": b"1", b"expires_at": b"not-a-number"}
        mock_pipeline.execute.return_value = [meta, set(), set(), {}]

        result = await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert result is None

    async def test_returns_none_when_meta_expires_at_is_missing(
        self, mock_pipeline, mock_redis_client, freeze_now
    ):
        meta = {b"authorized": b"1"}
        mock_pipeline.execute.return_value = [meta, set(), set(), {}]

        result = await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert result is None

    async def test_returns_none_when_redis_disabled(self, mocker, mock_redis_client):
        mocker.patch.object(_settings_module.settings, "redis_enabled", False)

        result = await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert result is None
        mock_redis_client.pipeline.assert_not_called()

    async def test_returns_none_when_redis_client_is_none(self, mocker):
        mocker.patch.object(redis_cache.redis_client, "redis_client", None)

        result = await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert result is None

    async def test_returns_unauthorized_dir_contents_when_meta_authorized_is_false(
        self, mock_pipeline, mock_redis_client, freeze_now
    ):
        meta = {
            b"authorized": b"0",
            b"expires_at": str(freeze_now + 100).encode("utf-8"),
        }
        mock_pipeline.execute.return_value = [meta, set(), set(), {}]

        result = await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert isinstance(result, DirectoryContents)
        assert result.authorized is False
        assert result.resource_paths is None
        assert result.folders is None

    async def test_returns_none_on_redis_connection_error(
        self, mock_pipeline, mock_redis_client, freeze_now, caplog
    ):
        mock_pipeline.execute.side_effect = redis_exceptions.ConnectionError("boom")

        with caplog.at_level(logging.WARNING, logger=redis_cache.logger.name):
            result = await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert result is None
        assert any(
            "redis hydrate failed" in record.message for record in caplog.records
        )

    async def test_returns_none_on_redis_timeout_error(
        self, mock_pipeline, mock_redis_client, freeze_now
    ):
        mock_pipeline.execute.side_effect = redis_exceptions.TimeoutError("slow")

        result = await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert result is None

    async def test_returns_none_on_os_error(
        self, mock_pipeline, mock_redis_client, freeze_now
    ):
        mock_pipeline.execute.side_effect = OSError("network down")

        result = await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert result is None

    async def test_issues_pipeline_with_four_expected_ops_in_order(
        self, mock_pipeline, mock_redis_client, freeze_now
    ):
        mock_pipeline.execute.return_value = _populated_pipeline_result(
            expires_at=freeze_now + 100
        )

        await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert mock_pipeline.method_calls == [
            call.hgetall(redis_cache.meta_key(SCOPE_KEY, DIR_PATH)),
            call.smembers(redis_cache.files_key(SCOPE_KEY, DIR_PATH)),
            call.smembers(redis_cache.folders_key(SCOPE_KEY, DIR_PATH)),
            call.hgetall(redis_cache.prev_key(SCOPE_KEY, DIR_PATH)),
            call.execute(),
        ]

    async def test_issues_exactly_one_pipeline_per_call(
        self, mock_pipeline, mock_redis_client, freeze_now
    ):
        mock_pipeline.execute.return_value = _populated_pipeline_result(
            expires_at=freeze_now + 100
        )

        await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        mock_redis_client.pipeline.assert_called_once_with()

    async def test_silently_drops_malformed_prev_entries(
        self, mock_pipeline, mock_redis_client, freeze_now
    ):
        meta = {
            b"authorized": b"1",
            b"expires_at": str(freeze_now + 100).encode("utf-8"),
        }
        prev = {
            b"good.txt": b"123.5",
            b"bad.txt": b"not-a-number",
        }
        mock_pipeline.execute.return_value = [meta, set(), set(), prev]

        result = await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert result is not None
        assert result.previously_visible_paths == {"good.txt": 123.5}

    async def test_decodes_files_and_folders_as_utf8_strings(
        self, mock_pipeline, mock_redis_client, freeze_now
    ):
        meta = {
            b"authorized": b"1",
            b"expires_at": str(freeze_now + 100).encode("utf-8"),
        }
        files = {"resumé.txt".encode("utf-8")}
        folders = {"sub-folder".encode("utf-8")}
        mock_pipeline.execute.return_value = [meta, files, folders, {}]

        result = await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)

        assert result.resource_paths == {"resumé.txt"}
        assert result.folders == {"sub-folder"}


class TestDirectoryContentsFromRedis:
    def test_constructs_authorized_instance_without_expires_at_override(self):
        result = DirectoryContents.from_redis(
            authorized=True,
            files={"a.txt"},
            folders={"sub"},
            expires_at=42.0,
            prev={"p": 1.0},
            dir_path=DIR_PATH,
        )

        assert result.authorized is True
        assert result.resource_paths == {"a.txt"}
        assert result.folders == {"sub"}
        assert result.expires_at == 42.0
        assert result.previously_visible_paths == {"p": 1.0}
        assert result.dir_path == DIR_PATH

    def test_unauthorized_instance_has_none_resource_paths_and_folders(self):
        result = DirectoryContents.from_redis(
            authorized=False,
            files=set(),
            folders=set(),
            expires_at=42.0,
            prev={},
            dir_path=DIR_PATH,
        )

        assert result.authorized is False
        assert result.resource_paths is None
        assert result.folders is None

    def test_copies_prev_dict_so_caller_mutations_dont_leak_in(self):
        prev = {"p": 1.0}
        result = DirectoryContents.from_redis(
            authorized=True,
            files=set(),
            folders=set(),
            expires_at=42.0,
            prev=prev,
            dir_path=DIR_PATH,
        )

        prev["new"] = 9.0

        assert result.previously_visible_paths == {"p": 1.0}

    def test_does_not_call_init(self, mocker):
        # __init__ overwrites expires_at based on authorized flag; from_redis
        # must skip that path so the persisted Redis expires_at survives.
        spy = mocker.spy(DirectoryContents, "__init__")

        DirectoryContents.from_redis(
            authorized=True,
            files=set(),
            folders=set(),
            expires_at=42.0,
            prev={},
            dir_path=DIR_PATH,
        )

        spy.assert_not_called()


# --- hydrate_all_dirs_from_redis tests ---


@pytest.mark.asyncio
class TestHydrateAllDirsFromRedis:
    @pytest.fixture
    def cache_dict(self):
        # A real StorageCache, not a plain dict: installing over an existing
        # key reads the cache's scope to report the replaced entry's drops.
        # Constructing one directly leaves the ContextVar untouched.
        return StorageCache(SCOPE)

    @pytest.fixture
    def hydrated(self):
        """A representative populated DirectoryContents for hydration returns."""

        def _make(dir_path: str) -> DirectoryContents:
            return DirectoryContents.from_redis(
                authorized=True,
                files={"f1"},
                folders=set(),
                expires_at=1_000_500.0,
                prev={},
                dir_path=dir_path,
            )

        return _make

    @pytest.fixture
    def redis_client_with_smembers(self, mocker):
        """
        Patch ``redis_cache.redis_client.redis_client`` with an
        AsyncMock client whose ``smembers`` is awaitable. The drainer
        uses ``await client.smembers(...)`` directly (not pipelined).
        """
        client = mocker.AsyncMock()
        mocker.patch.object(redis_cache.redis_client, "redis_client", client)
        return client

    async def test_happy_path_three_dirs_all_hydrate(
        self, mocker, cache_dict, redis_client_with_smembers, hydrated
    ):
        redis_client_with_smembers.smembers.return_value = {
            b"/dir/a",
            b"/dir/b",
            b"/dir/c",
        }
        per_dir = mocker.patch.object(
            redis_cache,
            "hydrate_dir_from_redis",
            mocker.AsyncMock(side_effect=lambda qid, dp: hydrated(dp)),
        )

        count = await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        assert count == 3
        assert per_dir.await_count == 3
        called_dirs = sorted(c.args[1] for c in per_dir.await_args_list)
        assert called_dirs == ["/dir/a", "/dir/b", "/dir/c"]

    async def test_happy_path_populates_cache_dict_with_per_dir_keys(
        self, mocker, cache_dict, redis_client_with_smembers, hydrated
    ):
        redis_client_with_smembers.smembers.return_value = {b"/dir/a", b"/dir/b"}
        mocker.patch.object(
            redis_cache,
            "hydrate_dir_from_redis",
            mocker.AsyncMock(side_effect=lambda qid, dp: hydrated(dp)),
        )

        await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        assert set(cache_dict.keys()) == {
            "storage-permissions:/dir/a",
            "storage-permissions:/dir/b",
        }
        assert cache_dict["storage-permissions:/dir/a"].dir_path == "/dir/a"
        assert cache_dict["storage-permissions:/dir/b"].dir_path == "/dir/b"

    async def test_calls_smembers_on_dirs_index_key(
        self, mocker, cache_dict, redis_client_with_smembers
    ):
        redis_client_with_smembers.smembers.return_value = set()
        mocker.patch.object(redis_cache, "hydrate_dir_from_redis", mocker.AsyncMock())

        await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        redis_client_with_smembers.smembers.assert_awaited_once_with(
            redis_cache.dirs_key(SCOPE_KEY)
        )

    async def test_empty_index_returns_zero_and_no_per_dir_calls(
        self, mocker, cache_dict, redis_client_with_smembers
    ):
        redis_client_with_smembers.smembers.return_value = set()
        per_dir = mocker.patch.object(
            redis_cache, "hydrate_dir_from_redis", mocker.AsyncMock()
        )

        count = await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        assert count == 0
        per_dir.assert_not_awaited()
        assert cache_dict == {}

    async def test_redis_disabled_returns_zero_and_no_redis_calls(
        self, mocker, cache_dict, redis_client_with_smembers
    ):
        mocker.patch.object(_settings_module.settings, "redis_enabled", False)
        per_dir = mocker.patch.object(
            redis_cache, "hydrate_dir_from_redis", mocker.AsyncMock()
        )

        count = await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        assert count == 0
        redis_client_with_smembers.smembers.assert_not_called()
        per_dir.assert_not_awaited()
        assert cache_dict == {}

    async def test_no_redis_client_returns_zero(self, mocker, cache_dict):
        mocker.patch.object(redis_cache.redis_client, "redis_client", None)
        per_dir = mocker.patch.object(
            redis_cache, "hydrate_dir_from_redis", mocker.AsyncMock()
        )

        count = await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        assert count == 0
        per_dir.assert_not_awaited()
        assert cache_dict == {}

    async def test_smembers_connection_error_returns_zero_no_exception_escapes(
        self, mocker, cache_dict, redis_client_with_smembers, caplog
    ):
        redis_client_with_smembers.smembers.side_effect = (
            redis_exceptions.ConnectionError("boom")
        )
        per_dir = mocker.patch.object(
            redis_cache, "hydrate_dir_from_redis", mocker.AsyncMock()
        )

        try:
            with caplog.at_level(logging.WARNING, logger=redis_cache.logger.name):
                count = await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)
        except Exception as exc:  # pragma: no cover - assertion below
            pytest.fail(f"hydrate_all_dirs_from_redis raised: {exc!r}")

        assert count == 0
        per_dir.assert_not_awaited()
        assert cache_dict == {}
        assert any(
            "redis hydrate_all failed" in record.message for record in caplog.records
        )

    async def test_smembers_timeout_error_returns_zero(
        self, mocker, cache_dict, redis_client_with_smembers
    ):
        redis_client_with_smembers.smembers.side_effect = redis_exceptions.TimeoutError(
            "slow"
        )
        per_dir = mocker.patch.object(
            redis_cache, "hydrate_dir_from_redis", mocker.AsyncMock()
        )

        count = await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        assert count == 0
        per_dir.assert_not_awaited()

    async def test_smembers_os_error_returns_zero(
        self, mocker, cache_dict, redis_client_with_smembers
    ):
        redis_client_with_smembers.smembers.side_effect = OSError("network down")
        per_dir = mocker.patch.object(
            redis_cache, "hydrate_dir_from_redis", mocker.AsyncMock()
        )

        count = await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        assert count == 0
        per_dir.assert_not_awaited()

    async def test_partial_hydration_skips_none_returning_dirs(
        self, mocker, cache_dict, redis_client_with_smembers, hydrated
    ):
        redis_client_with_smembers.smembers.return_value = {
            b"/dir/a",
            b"/dir/b",
            b"/dir/c",
        }

        async def fake_hydrate(qid, dp):
            if dp == "/dir/b":
                return None
            return hydrated(dp)

        mocker.patch.object(
            redis_cache,
            "hydrate_dir_from_redis",
            mocker.AsyncMock(side_effect=fake_hydrate),
        )

        count = await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        assert count == 2
        assert set(cache_dict.keys()) == {
            "storage-permissions:/dir/a",
            "storage-permissions:/dir/c",
        }
        assert "storage-permissions:/dir/b" not in cache_dict

    async def test_str_members_decoded_path_handled(
        self, mocker, cache_dict, redis_client_with_smembers, hydrated
    ):
        # Defensive: redis-py with decode_responses=True returns str members;
        # the helper handles both bytes and str without raising.
        redis_client_with_smembers.smembers.return_value = {"/dir/a"}
        mocker.patch.object(
            redis_cache,
            "hydrate_dir_from_redis",
            mocker.AsyncMock(side_effect=lambda qid, dp: hydrated(dp)),
        )

        count = await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        assert count == 1
        assert "storage-permissions:/dir/a" in cache_dict

    async def test_continues_on_per_dir_hydration_returning_none(
        self, mocker, cache_dict, redis_client_with_smembers, hydrated
    ):
        # Per the ticket: hydrate_dir_from_redis already absorbs Redis
        # connection errors internally and returns None, so the bulk
        # function's per-dir resilience is observed via the None-return
        # case rather than via raised exceptions. Names that contract
        # explicitly so the per-dir continuation is regression-protected
        # if hydrate_dir_from_redis's internal try/except ever shrinks.
        redis_client_with_smembers.smembers.return_value = {
            b"/dir/a",
            b"/dir/b",
            b"/dir/c",
        }

        async def fake_hydrate(qid, dp):
            if dp in {"/dir/a", "/dir/c"}:
                return None
            return hydrated(dp)

        mocker.patch.object(
            redis_cache,
            "hydrate_dir_from_redis",
            mocker.AsyncMock(side_effect=fake_hydrate),
        )

        count = await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        assert count == 1
        assert set(cache_dict.keys()) == {"storage-permissions:/dir/b"}

    # A filter update that names no storage filters reaches this helper on a
    # cache that may already hold entries. Hydrating a directory replaces its
    # entry, and the replaced entry's drops are reported as it leaves.

    @pytest.fixture
    def hydrates_dir_a(self, mocker, redis_client_with_smembers, hydrated):
        """Make the dirs index name ``/dir/a`` and Redis hydrate it."""
        redis_client_with_smembers.smembers.return_value = {b"/dir/a"}
        mocker.patch.object(
            redis_cache,
            "hydrate_dir_from_redis",
            mocker.AsyncMock(side_effect=lambda qid, dp: hydrated(dp)),
        )

    async def test_reports_the_drops_of_a_replaced_entry(
        self, cache_dict, hydrates_dir_a, drop_reports
    ):
        cache_dict["storage-permissions:/dir/a"] = unauthorized_entry_with_drops(
            "/dir/a"
        )

        await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        [record] = drop_reports()
        assert record.dropped_events == DROPPED

    async def test_reports_nothing_for_a_replaced_entry_without_drops(
        self, cache_dict, hydrates_dir_a, drop_reports
    ):
        cache_dict["storage-permissions:/dir/a"] = unauthorized_entry_with_drops(
            "/dir/a", dropped=0
        )

        await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        assert drop_reports() == []

    async def test_installs_the_hydrated_entry_over_the_replaced_one(
        self, cache_dict, hydrates_dir_a
    ):
        cache_dict["storage-permissions:/dir/a"] = unauthorized_entry_with_drops(
            "/dir/a"
        )

        await redis_cache.hydrate_all_dirs_from_redis(SCOPE, cache_dict)

        assert cache_dict["storage-permissions:/dir/a"].authorized is True


@pytest.mark.asyncio
class TestHydrateLogHygiene:
    """
    The hydrate warnings name the queue, and the queue they run for is a
    non-durable one whose broker name embeds the stream's reconnect token.

    The Redis keys keep the exact name -- the cache would point at nothing
    otherwise -- so the redaction has to happen at the log line and nowhere
    else, which is what these pin. The directory is left out of the warning
    altogether: it is the client's folder path, and record extras are
    exported as log attributes when telemetry is on.
    """

    @pytest.fixture
    def hydrate_dir_warning(self, mock_pipeline, mock_redis_client, freeze_now, caplog):
        """The record ``hydrate_dir_from_redis`` emits when Redis is down."""

        async def _run():
            mock_pipeline.execute.side_effect = redis_exceptions.ConnectionError("boom")
            with caplog.at_level(logging.WARNING, logger=redis_cache.logger.name):
                await redis_cache.hydrate_dir_from_redis(SCOPE, DIR_PATH)
            return next(
                r for r in caplog.records if "redis hydrate failed" in r.message
            )

        return _run

    @pytest.fixture
    def hydrate_all_warning(self, mocker, caplog):
        """The record ``hydrate_all_dirs_from_redis`` emits when Redis is down."""

        async def _run():
            client = mocker.AsyncMock()
            client.smembers.side_effect = redis_exceptions.ConnectionError("boom")
            mocker.patch.object(redis_cache.redis_client, "redis_client", client)

            with caplog.at_level(logging.WARNING, logger=redis_cache.logger.name):
                await redis_cache.hydrate_all_dirs_from_redis(SCOPE, {})
            return next(
                r for r in caplog.records if "redis hydrate_all failed" in r.message
            )

        return _run

    async def test_the_dir_warning_carries_the_redacted_reference(
        self, hydrate_dir_warning
    ):
        record = await hydrate_dir_warning()

        assert record.queue_ref == redact_reconnect_token(RECONNECT_TOKEN)

    async def test_the_dir_warning_does_not_carry_the_token(self, hydrate_dir_warning):
        record = await hydrate_dir_warning()

        assert RECONNECT_TOKEN not in str(record.__dict__)

    async def test_the_dir_warning_does_not_name_the_directory(
        self, hydrate_dir_warning
    ):
        record = await hydrate_dir_warning()

        assert DIR_PATH not in str(record.__dict__)

    async def test_the_hydrate_all_warning_carries_the_redacted_reference(
        self, hydrate_all_warning
    ):
        record = await hydrate_all_warning()

        assert record.queue_ref == redact_reconnect_token(RECONNECT_TOKEN)

    async def test_the_hydrate_all_warning_does_not_carry_the_token(
        self, hydrate_all_warning
    ):
        record = await hydrate_all_warning()

        assert RECONNECT_TOKEN not in str(record.__dict__)


class TestHydrateAllDirsFromRedisImportSurface:
    def test_imports_as_top_level(self):
        # The ticket calls out top-level importability so consumers
        # (storage_permissions_cache) can `from ... import
        # hydrate_all_dirs_from_redis` without circular-import grief.
        from app.lib.permissions.storage_permissions.redis_cache import (
            hydrate_all_dirs_from_redis,
        )

        assert callable(hydrate_all_dirs_from_redis)
