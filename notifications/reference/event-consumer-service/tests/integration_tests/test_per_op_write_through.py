# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Integration tests for the per-mutation write-through pipeline.

Storage events flow through the consumer's auth-check codepaths, which
call DirectoryContents mutators, which enqueue Ops on the active
CacheWriter. The drainer batches those Ops and flushes them via a
single Redis pipeline per batch. After ``drain_and_stop`` returns, the
recording fake holds the exact sequence of staged commands so each
scenario can assert the expected SADD / SREM / EXPIRE calls landed on
the right keys.

Five scenarios per Bee `t3.toc.oo.q3.q5` AC:

    1. CREATE event   -> SADD on files key, dir indexed in
       `cache:{queue}:dirs`, TTLs set on touched keys.
    2. DELETE event   -> SREM on files key (key TTL was set during
       the CREATE step that primed the entry).
    3. DIR_CREATED    -> SADD on folders key, TTL set, dir indexed.
    4. DIR_DELETED    -> SREM on folders key.
    5. Redis outage during cleanup -> consumer terminates within the
       drain timeout, no exception escapes.

Each scenario uses a uuid-suffixed queue NAME so concurrent xdist
workers cannot collide on the recorder, and the recording fake is
fixture-scoped so its ``commands`` list is fresh per test (the
"teardown that DELs Redis keys" mandate is satisfied automatically:
each test gets a brand-new in-memory recorder, so no key state can
leak across tests).
"""

import asyncio
import uuid

import pytest
import pytest_asyncio
from notification_common.redis import redis_client

from app.lib.permissions.storage_permissions import storage_cache as sc_module
from app.lib.permissions.storage_permissions.cache_scope import CacheScope
from app.lib.permissions.storage_permissions.cache_writer import (
    CacheWriter,
    register_writer,
    reset_active_queue_id,
    set_active_queue_id,
    unregister_writer,
)
from app.lib.permissions.storage_permissions.directory_contents import DirectoryContents
from app.lib.permissions.storage_permissions.storage_cache import StorageCache
from app.lib.permissions.storage_permissions.storage_permissions import (
    can_access_created_event,
    can_access_deleted_event,
    can_access_dir_created_event,
    can_access_dir_deleted_event,
)
from tests.fixtures.cache_scope import SECOND_PRINCIPAL_REF, TEST_PRINCIPAL_REF

# ---------------------------------------------------------------------------
# Recording fake — captures every staged Redis command + execute() awaits
# ---------------------------------------------------------------------------


class _RecordingPipeline:
    """
    Records every staged command, returns self to support method
    chaining, and provides an awaitable ``execute()``.

    For tests that want to simulate Redis going down, an exception
    instance can be set on the parent recorder via ``raise_on_execute``.
    """

    def __init__(self, recorder):
        self._recorder = recorder

    def __getattr__(self, name: str):
        def _record(*args, **kwargs):
            self._recorder.commands.append((name, args, kwargs))
            return self

        return _record

    async def execute(self):
        if self._recorder.raise_on_execute is not None:
            raise self._recorder.raise_on_execute
        self._recorder.commands.append(("execute", (), {}))
        return []


class _RecordingRedisClient:
    """
    Stand-in for ``redis_client.redis_client`` exposing only the
    ``pipeline()`` factory used by ``CacheWriter._flush``. Stores
    every staged command across all pipelines on ``commands``.
    """

    def __init__(self):
        self.commands: list[tuple] = []
        self.raise_on_execute: Exception | None = None

    def pipeline(self):
        return _RecordingPipeline(self)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def recording_redis(mocker):
    fake = _RecordingRedisClient()
    mocker.patch.object(redis_client, "redis_client", fake)
    return fake


@pytest.fixture(autouse=True)
def isolate_storage_cache():
    """Reset the per-context StorageCache so tests don't bleed."""
    yield
    sc_module._cache_dict_context_var.set(None)


@pytest.fixture
def storage_permissions_enabled(mocker):
    """
    Enable the storage permission gate the readers actually consult.

    After the settings+readers re-gating, the storage readers
    (``can_access_*_event``) short-circuit on
    ``settings.storage_permissions_enabled`` rather than on
    ``common_settings.permissions_enabled``. ``storage_permissions.py``
    binds the settings instance directly at import time
    (``from app.config.settings import settings``), so it keeps reading
    whatever object that name pointed at when the module was imported —
    NOT the live ``app.config.settings.settings`` module attribute.
    ``reset_env_vars()`` in the config tests calls
    ``importlib.reload(settings)``, which rebinds the module attribute to
    a fresh instance while ``storage_permissions``'s captured reference
    stays on the pre-reload object. Patching the module attribute would
    therefore miss the object the reader consults once a config test has
    run earlier in the worker's order. Patch the bound reference the
    reader actually holds — matching the unit-test fixture in
    ``test_storage_permissions.py`` — so the gate is observed regardless
    of any reload.
    """
    from app.lib.permissions.storage_permissions import storage_permissions

    mocker.patch.object(
        storage_permissions.settings, "storage_permissions_enabled", True
    )


@pytest.fixture
def redis_enabled(mocker):
    """
    Enable redis for the integration code paths.

    Production modules now read settings via the module-import form
    (``from app.config import settings`` then ``settings.settings.X``),
    so patching the live ``app.config.settings.settings`` instance is
    transparently observed by every consumer regardless of any
    ``importlib.reload`` performed by other test fixtures.
    """
    import app.config.settings as _config_module

    mocker.patch.object(_config_module.settings, "redis_enabled", True)


@pytest.fixture
def queue_name():
    """A unique broker queue name per test — prevents concurrent collisions."""
    return f"event-queue:test-queue-{uuid.uuid4()}:nd"


@pytest.fixture
def scope(queue_name):
    """
    What the stream's cache is namespaced by: that queue, as seen by the
    principal presenting the request. Every Redis key below is built from
    ``scope.key``.
    """
    return CacheScope(queue_name, TEST_PRINCIPAL_REF)


@pytest.fixture
def second_principal_scope(queue_name):
    """The SAME queue, as seen by a different authenticated principal."""
    return CacheScope(queue_name, SECOND_PRINCIPAL_REF)


def _ttl_for_key(commands: list[tuple], key: str) -> int | None:
    """
    Return the (last) TTL value passed to ``expire(key, ttl)`` in the
    recorded command stream, or ``None`` if no expire was staged for
    that key. The recorder captures ``("expire", (key, ttl), {})``.
    """
    ttls = [
        args[1]
        for (name, args, _kw) in commands
        if name == "expire" and args and args[0] == key
    ]
    return ttls[-1] if ttls else None


def _assert_ttl_set_positive(commands: list[tuple], key: str) -> None:
    ttl = _ttl_for_key(commands, key)
    assert ttl is not None, f"expected expire(...) staged for {key!r}; saw {commands}"
    assert ttl > 0, f"expected positive TTL for {key!r}; saw {ttl}"


def _staged(commands: list[tuple], op_name: str, key: str | None = None) -> list[tuple]:
    return [
        (n, args, kw)
        for (n, args, kw) in commands
        if n == op_name and (key is None or (args and args[0] == key))
    ]


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


async def _setup_writer(scope: CacheScope):
    """
    Build and start a CacheWriter, register it, and bind the active
    cache-scope key. Returns ``(writer, token)``; caller is responsible
    for drain_and_stop + unregister + reset_active_queue_id.
    """
    writer = CacheWriter(scope)
    writer.start()
    register_writer(writer)
    token = set_active_queue_id(scope.key)
    return writer, token


async def _teardown_writer(writer: CacheWriter, token, drain_timeout=1.0):
    try:
        await writer.drain_and_stop(timeout=drain_timeout)
    finally:
        unregister_writer(writer)
        reset_active_queue_id(token)


def _seed_authorized_dir(
    scope: CacheScope,
    dir_path: str,
    files: list[str] | None = None,
    folders: list[str] | None = None,
) -> StorageCache:
    """
    Pre-populate the StorageCache with an authorized DirectoryContents
    for ``dir_path`` so auth-check codepaths take the cache-hit branch
    (no storage API call required).
    """
    storage_cache = StorageCache(scope)
    sc_module._cache_dict_context_var.set(storage_cache)
    storage_cache[f"storage-permissions:{dir_path}"] = DirectoryContents(
        file_paths=files or [],
        folder_paths=folders or [],
        dir_path=dir_path,
    )
    return storage_cache


# ---------------------------------------------------------------------------
# Scenario tests
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
class TestPerOpWriteThrough:
    async def test_create_event_writes_through_to_files_key_and_dirs_index(
        self,
        recording_redis,
        redis_enabled,
        storage_permissions_enabled,
        scope,
        mocker,
    ):
        """
        Scenario 1: CREATE event for `/foo/file1` results in SADD on
        ``cache:{queue}:files:/foo/`` containing `/foo/file1`, the
        parent dir indexed in ``cache:{queue}:dirs``, and TTLs on every
        touched key.

        Drives the storage-API path so the bulk-replace replace_dir Op
        fires (which is what populates ``cache:{queue}:dirs``).
        """
        # Force a cache-miss so _fetch_dir_contents takes the api-fetch
        # path and enqueues the replace_dir Op (responsible for the
        # dirs-index SADD).
        sc_module._cache_dict_context_var.set(StorageCache(scope))
        mocker.patch(
            "app.lib.permissions.storage_permissions"
            ".storage_permissions_cache.fetch_from_storage_api",
            return_value=DirectoryContents(
                file_paths=[],
                folder_paths=[],
                dir_path="/foo/",
            ),
        )

        writer, token = await _setup_writer(scope)
        try:
            event = {"message": {"resource_address": "/foo/file1"}}
            assert await can_access_created_event(event) is True
        finally:
            await _teardown_writer(writer, token)

        cmds = recording_redis.commands
        files_key = f"cache:{scope.key}:files:/foo/"
        dirs_key = f"cache:{scope.key}:dirs"

        # add_file Op staged a SADD on the files key with the new path.
        sadds_on_files = _staged(cmds, "sadd", files_key)
        assert any(
            "/foo/file1" in args[1:] for (_n, args, _kw) in sadds_on_files
        ), f"expected SADD {files_key} /foo/file1; saw {sadds_on_files}"

        # replace_dir Op staged a SADD on the dirs index with the parent.
        sadds_on_dirs = _staged(cmds, "sadd", dirs_key)
        assert any(
            "/foo/" in args[1:] for (_n, args, _kw) in sadds_on_dirs
        ), f"expected SADD {dirs_key} /foo/; saw {sadds_on_dirs}"

        # TTLs > 0 on every touched key.
        _assert_ttl_set_positive(cmds, files_key)
        _assert_ttl_set_positive(cmds, dirs_key)
        _assert_ttl_set_positive(cmds, f"cache:{scope.key}:meta:/foo/")

    async def test_delete_event_removes_path_from_files_key(
        self,
        recording_redis,
        redis_enabled,
        storage_permissions_enabled,
        scope,
    ):
        """
        Scenario 2: DELETE event for a previously-cached file results
        in SREM on ``cache:{queue}:files:/foo/`` for that path.

        The TTL on the files key was set during the CREATE step that
        primed the entry — covered by scenario 1. Here we assert only
        the SREM (the contract of `remove_file` Op).
        """
        # Seed an authorized dir with the to-be-deleted path already
        # in resource_paths so can_access_deleted_event takes the
        # remove_file_path branch.
        _seed_authorized_dir(scope, "/foo/", files=["/foo/file1"])

        writer, token = await _setup_writer(scope)
        try:
            event = {"message": {"resource_address": "/foo/file1"}}
            assert await can_access_deleted_event(event) is True
        finally:
            await _teardown_writer(writer, token)

        cmds = recording_redis.commands
        files_key = f"cache:{scope.key}:files:/foo/"

        srem_calls = _staged(cmds, "srem", files_key)
        assert any(
            "/foo/file1" in args[1:] for (_n, args, _kw) in srem_calls
        ), f"expected SREM {files_key} /foo/file1; saw {srem_calls}"

    async def test_dir_created_event_writes_through_to_folders_key(
        self,
        recording_redis,
        redis_enabled,
        storage_permissions_enabled,
        scope,
        mocker,
    ):
        """
        Scenario 3: DIR_CREATED event for `/foo/bar/` results in SADD
        on ``cache:{queue}:folders:/foo/`` for `/foo/bar/`, and TTL
        set on the folders key.
        """
        # Seed an authorized parent dir without the new folder yet.
        _seed_authorized_dir(scope, "/foo/", files=[], folders=[])

        # can_access_dir_created_event also calls _can_access_dir which
        # hits the storage API for the new dir; mock it to authorized.
        mocker.patch(
            "app.lib.permissions.storage_permissions"
            ".storage_permissions._can_access_dir",
            return_value=True,
        )

        writer, token = await _setup_writer(scope)
        try:
            event = {"message": {"resource_address": "/foo/bar/"}}
            assert await can_access_dir_created_event(event) is True
        finally:
            await _teardown_writer(writer, token)

        cmds = recording_redis.commands
        folders_key = f"cache:{scope.key}:folders:/foo/"

        sadds = _staged(cmds, "sadd", folders_key)
        assert any(
            "/foo/bar/" in args[1:] for (_n, args, _kw) in sadds
        ), f"expected SADD {folders_key} /foo/bar/; saw {sadds}"

        # add_folder Op stages an EXPIRE alongside the SADD.
        _assert_ttl_set_positive(cmds, folders_key)

    async def test_dir_deleted_event_removes_path_from_folders_key(
        self,
        recording_redis,
        redis_enabled,
        storage_permissions_enabled,
        scope,
        mocker,
    ):
        """
        Scenario 4: DIR_DELETED event for a previously-cached folder
        results in SREM on ``cache:{queue}:folders:/foo/`` for the
        folder path.
        """
        # Seed an authorized parent dir with the folder already known.
        _seed_authorized_dir(scope, "/foo/", files=[], folders=["/foo/bar/"])

        # can_access_dir_deleted_event consults `_dir_was_not_deleted`
        # to decide whether the dir was synthetically deleted. We want
        # the real-deletion branch, which is `False` here.
        mocker.patch(
            "app.lib.permissions.storage_permissions"
            ".storage_permissions._dir_was_not_deleted",
            return_value=False,
        )

        writer, token = await _setup_writer(scope)
        try:
            event = {"message": {"resource_address": "/foo/bar/"}}
            assert await can_access_dir_deleted_event(event) is True
        finally:
            await _teardown_writer(writer, token)

        cmds = recording_redis.commands
        folders_key = f"cache:{scope.key}:folders:/foo/"

        srem_calls = _staged(cmds, "srem", folders_key)
        assert any(
            "/foo/bar/" in args[1:] for (_n, args, _kw) in srem_calls
        ), f"expected SREM {folders_key} /foo/bar/; saw {srem_calls}"

    async def test_redis_outage_during_cleanup_does_not_escape(
        self,
        recording_redis,
        redis_enabled,
        storage_permissions_enabled,
        scope,
    ):
        """
        Scenario 5: Redis is unreachable during cleanup. The consumer
        path (here: drain_and_stop on the writer) must terminate within
        the drain timeout and no exception escapes — the CacheWriter's
        documented contract is that the drainer never raises out of its
        loop and ``drain_and_stop`` is bounded.
        """
        _seed_authorized_dir(scope, "/foo/", files=[])

        # Configure the recording fake to raise on every pipeline
        # execute(), simulating a hard Redis outage.
        recording_redis.raise_on_execute = ConnectionError("simulated redis outage")

        writer, token = await _setup_writer(scope)
        try:
            # Drive at least one mutation so the drainer has work to
            # flush — guaranteeing it hits the failing execute().
            event = {"message": {"resource_address": "/foo/file1"}}
            assert await can_access_created_event(event) is True

            # The flush will hit the simulated outage; drain_and_stop
            # must still return cleanly within the timeout.
            try:
                await asyncio.wait_for(writer.drain_and_stop(timeout=1.0), timeout=2.0)
            except Exception as exc:
                pytest.fail(
                    f"drain_and_stop raised under simulated Redis outage: {exc!r}"
                )
        finally:
            unregister_writer(writer)
            reset_active_queue_id(token)


# ---------------------------------------------------------------------------
# Reconnect bulk-hydration end-to-end (Bee toc / Epic .o7)
#
# The headline bug-fix flow: Pod A's per-op write-through populates Redis;
# Pod B reconnects with empty filters and must inherit Pod A's
# resource_paths via hydrate_all_dirs_from_redis so an in-flight DELETE
# event for a path that was created on Pod A is still authorized on B.
#
# Approach: replace the command-recorder with a stateful in-memory fake
# that ALSO records commands (preserves write-through assertions) and
# additionally serves the read paths used by `hydrate_*_from_redis`.
# The fake covers exactly the surface the production code paths touch:
#   pipeline staging: sadd, srem, hset, hdel, delete, expire (no-op on
#   TTL since wall-clock isn't simulated).
#   pipeline reads: hgetall, smembers (via execute() result list).
#   direct reads: smembers (used by hydrate_all_dirs_from_redis).
# ---------------------------------------------------------------------------


class _StatefulPipeline:
    """
    Pipeline that mirrors `_RecordingPipeline`'s recording behavior but
    also accumulates writes/reads against the parent client's in-memory
    store. ``execute()`` returns a list whose order matches the staged
    read commands — this is what `hydrate_dir_from_redis` unpacks.
    """

    def __init__(self, client):
        self._client = client
        self._read_results: list = []

    # --- write staging ---
    def sadd(self, key, *members):
        self._client.commands.append(("sadd", (key, *members), {}))
        self._client.sets.setdefault(key, set()).update(
            m if isinstance(m, bytes) else m.encode("utf-8") for m in members
        )
        return self

    def srem(self, key, *members):
        self._client.commands.append(("srem", (key, *members), {}))
        bucket = self._client.sets.get(key)
        if bucket is not None:
            for m in members:
                bucket.discard(m if isinstance(m, bytes) else m.encode("utf-8"))
        return self

    def hset(self, key, *args, **kwargs):
        self._client.commands.append(("hset", (key, *args), kwargs))
        bucket = self._client.hashes.setdefault(key, {})
        if "mapping" in kwargs:
            for f, v in kwargs["mapping"].items():
                fb = f if isinstance(f, bytes) else f.encode("utf-8")
                bucket[fb] = self._encode(v)
        elif len(args) >= 2:
            f, v = args[0], args[1]
            fb = f if isinstance(f, bytes) else f.encode("utf-8")
            bucket[fb] = self._encode(v)
        return self

    def hdel(self, key, *fields):
        self._client.commands.append(("hdel", (key, *fields), {}))
        bucket = self._client.hashes.get(key)
        if bucket is not None:
            for f in fields:
                bucket.pop(f if isinstance(f, bytes) else f.encode("utf-8"), None)
        return self

    def delete(self, key):
        self._client.commands.append(("delete", (key,), {}))
        self._client.sets.pop(key, None)
        self._client.hashes.pop(key, None)
        return self

    def expire(self, key, ttl):
        # TTL semantics aren't simulated; record only.
        self._client.commands.append(("expire", (key, ttl), {}))
        return self

    # --- read staging (results queued in pipeline-order for execute) ---
    def hgetall(self, key):
        self._client.commands.append(("hgetall", (key,), {}))
        self._read_results.append(dict(self._client.hashes.get(key, {})))
        return self

    def smembers(self, key):
        self._client.commands.append(("smembers", (key,), {}))
        self._read_results.append(set(self._client.sets.get(key, set())))
        return self

    @staticmethod
    def _encode(v):
        if isinstance(v, bytes):
            return v
        return str(v).encode("utf-8")

    async def execute(self):
        if self._client.raise_on_execute is not None:
            raise self._client.raise_on_execute
        self._client.commands.append(("execute", (), {}))
        # Reads done after writes return the post-write state by the
        # time execute resolves; this matches Redis's pipeline semantics
        # closely enough for these scenarios.
        return list(self._read_results)


class _StatefulRedisClient:
    """
    Recording redis client with an in-memory store, sufficient for the
    pipeline ops used by the auth-cache write-through and for the
    direct ``smembers`` call used by ``hydrate_all_dirs_from_redis``.
    """

    def __init__(self):
        self.commands: list[tuple] = []
        self.raise_on_execute: Exception | None = None
        self.sets: dict[str, set[bytes]] = {}
        self.hashes: dict[str, dict[bytes, bytes]] = {}

    def pipeline(self):
        return _StatefulPipeline(self)

    async def smembers(self, key):
        return set(self.sets.get(key, set()))


@pytest.fixture
def stateful_redis(mocker):
    fake = _StatefulRedisClient()
    mocker.patch.object(redis_client, "redis_client", fake)
    return fake


def _purge_keys_for_queue(client: _StatefulRedisClient, scope: CacheScope) -> None:
    """
    Best-effort teardown — drop any in-memory state keyed under the
    scope's namespace. Matches the brief's mandate: "teardown removes
    all queue keys".
    """
    prefix = f"cache:{scope.key}:"
    for key in [k for k in client.sets if k.startswith(prefix)]:
        client.sets.pop(key, None)
    for key in [k for k in client.hashes if k.startswith(prefix)]:
        client.hashes.pop(key, None)


@pytest.mark.asyncio
class TestReconnectHydration:
    """
    End-to-end coverage of the reconnect bug-fix flow.

    Flow:
        1. Pod A connects, CREATE event for ``/foo/file1`` writes
           through to Redis (`cache:Q:files:/foo/` SADD, dirs index
           SADD via `replace_dir`).
        2. Storage backend "deletes" `file1` between sessions.
        3. Pod A disconnects (in-memory cache dropped).
        4. Pod B reconnects under the same cache SCOPE but with empty
           `filter_groups` (the reconnect signal).
        5. `init_cache_for_user` calls `hydrate_all_dirs_from_redis`
           which reads `cache:Q:dirs` SMEMBERS, then per-dir
           `hydrate_dir_from_redis` rebuilds DirectoryContents from
           Redis state — so Pod B's in-memory cache sees `file1` even
           though storage no longer does.
        6. DELETE event for `/foo/file1` arrives at Pod B; the auth
           check finds `file1` in `resource_paths` and authorizes.
    """

    async def test_pod_b_authorizes_delete_after_reconnect_hydration(
        self,
        stateful_redis,
        redis_enabled,
        storage_permissions_enabled,
        scope,
        mocker,
    ):
        # ----- Pod A: storage API returns /foo/ as empty (will SADD on CREATE).
        sc_module._cache_dict_context_var.set(StorageCache(scope))
        mocker.patch(
            "app.lib.permissions.storage_permissions"
            ".storage_permissions_cache.fetch_from_storage_api",
            return_value=DirectoryContents(
                file_paths=[],
                folder_paths=[],
                dir_path="/foo/",
            ),
        )

        writer_a, token_a = await _setup_writer(scope)
        try:
            event_create = {"message": {"resource_address": "/foo/file1"}}
            assert await can_access_created_event(event_create) is True
        finally:
            await _teardown_writer(writer_a, token_a)

        # Sanity: Pod A's write-through actually populated Redis state.
        files_key = f"cache:{scope.key}:files:/foo/"
        dirs_key = f"cache:{scope.key}:dirs"
        assert b"/foo/file1" in stateful_redis.sets.get(files_key, set())
        assert b"/foo/" in stateful_redis.sets.get(dirs_key, set())

        # ----- Storage backend "deletes" file1 between sessions; storage
        # API mock would return without it now. We assert below that the
        # DELETE event still authorizes — Redis is the source of truth.
        mocker.patch(
            "app.lib.permissions.storage_permissions"
            ".storage_permissions_cache.fetch_from_storage_api",
            return_value=DirectoryContents(
                file_paths=[],  # storage no longer reports file1
                folder_paths=[],
                dir_path="/foo/",
            ),
        )

        # ----- Pod A disconnects: drop the in-memory cache. The
        # autouse `isolate_storage_cache` fixture would do this between
        # tests, but here we simulate within one test.
        sc_module._cache_dict_context_var.set(None)

        # ----- Pod B reconnects: fresh in-memory cache, same scope,
        # empty filters → reconnect path.
        from app.lib.permissions.storage_permissions.storage_permissions_cache import (
            init_cache_for_user,
        )

        writer_b, token_b = await _setup_writer(scope)
        try:
            await init_cache_for_user(scope, [])

            cache_b = await StorageCache.get_instance(scope)
            hydrated = cache_b.get("storage-permissions:/foo/")
            assert (
                hydrated is not None
            ), "Pod B's reconnect should have hydrated /foo/ from Redis"
            assert "/foo/file1" in hydrated.resource_paths

            # DELETE for /foo/file1 must now authorize on Pod B.
            event_delete = {"message": {"resource_address": "/foo/file1"}}
            assert await can_access_deleted_event(event_delete) is True
        finally:
            await _teardown_writer(writer_b, token_b)
            _purge_keys_for_queue(stateful_redis, scope)

    async def test_redis_is_source_of_truth_even_when_storage_api_drifts(
        self,
        stateful_redis,
        redis_enabled,
        storage_permissions_enabled,
        scope,
        mocker,
    ):
        """
        Variant of scenario 1: storage API has ALREADY drifted by the
        time hydration runs. Pod B's reconnect must authorize the
        DELETE event using the Redis-persisted resource_paths, not by
        round-tripping to storage.
        """
        # Pod A: storage initially empty; CREATE adds file1 to Redis.
        sc_module._cache_dict_context_var.set(StorageCache(scope))
        mocker.patch(
            "app.lib.permissions.storage_permissions"
            ".storage_permissions_cache.fetch_from_storage_api",
            return_value=DirectoryContents(
                file_paths=[],
                folder_paths=[],
                dir_path="/foo/",
            ),
        )
        writer_a, token_a = await _setup_writer(scope)
        try:
            assert (
                await can_access_created_event(
                    {"message": {"resource_address": "/foo/file1"}}
                )
                is True
            )
        finally:
            await _teardown_writer(writer_a, token_a)

        # Pre-disconnect drift: have the storage API mock track a state
        # in which /foo/ DOES NOT include file1. If Pod B were to call
        # the storage API after reconnect, it would see no file1 and
        # the DELETE could be unauthorized. The test guards against that
        # by ensuring the reconnect hydrates from Redis (not storage).
        api_mock = mocker.patch(
            "app.lib.permissions.storage_permissions"
            ".storage_permissions_cache.fetch_from_storage_api",
            return_value=DirectoryContents(
                file_paths=[],
                folder_paths=[],
                dir_path="/foo/",
            ),
        )

        sc_module._cache_dict_context_var.set(None)

        from app.lib.permissions.storage_permissions.storage_permissions_cache import (
            init_cache_for_user,
        )

        writer_b, token_b = await _setup_writer(scope)
        try:
            await init_cache_for_user(scope, [])

            # Storage API must NOT have been called during the reconnect
            # hydrate flow — Redis is authoritative.
            assert (
                api_mock.call_count == 0
            ), "reconnect should hydrate from Redis without storage API"

            # And the DELETE for file1 still authorizes because Redis
            # carried it forward via cache:Q:files:/foo/.
            assert (
                await can_access_deleted_event(
                    {"message": {"resource_address": "/foo/file1"}}
                )
                is True
            )
        finally:
            await _teardown_writer(writer_b, token_b)
            _purge_keys_for_queue(stateful_redis, scope)


@pytest.mark.asyncio
class TestSecondPrincipalDoesNotInheritTheCache:
    """
    Issue b.ize, end to end: a non-durable queue's id IS its reconnect
    token, so a second authenticated principal that obtains one can attach
    to the same queue. It must build its own authorization state rather
    than inheriting the first principal's cached directory listings.

    Same shape as ``TestReconnectHydration`` above, with one difference:
    the attaching principal is a different one, so it hydrates from its
    OWN scope's Redis namespace -- which is empty -- and the delete the
    first principal's state would have authorized is refused.
    """

    @pytest_asyncio.fixture
    async def pod_a_state_in_redis(
        self,
        stateful_redis,
        redis_enabled,
        storage_permissions_enabled,
        scope,
        mocker,
    ):
        """
        Drive the first principal's CREATE write-through, then drop its
        in-memory cache so only the Redis state remains.
        """
        sc_module._cache_dict_context_var.set(StorageCache(scope))

        def _empty_listing(*_args, **_kwargs):
            # A FRESH object per call, as a real API round trip returns.
            # A single shared ``return_value`` would hand the second
            # principal the very instance the first principal mutated,
            # and the isolation below would fail on the test rig rather
            # than on the code under test.
            return DirectoryContents(file_paths=[], folder_paths=[], dir_path="/foo/")

        mocker.patch(
            "app.lib.permissions.storage_permissions"
            ".storage_permissions_cache.fetch_from_storage_api",
            side_effect=_empty_listing,
        )

        writer_a, token_a = await _setup_writer(scope)
        try:
            assert (
                await can_access_created_event(
                    {"message": {"resource_address": "/foo/file1"}}
                )
                is True
            )
        finally:
            await _teardown_writer(writer_a, token_a)

        # Guard: the rest of this suite is only meaningful if the first
        # principal really did persist something to inherit.
        assert b"/foo/file1" in stateful_redis.sets.get(
            f"cache:{scope.key}:files:/foo/", set()
        )

        sc_module._cache_dict_context_var.set(None)
        yield stateful_redis
        _purge_keys_for_queue(stateful_redis, scope)

    @pytest_asyncio.fixture
    async def second_principal_reconnect(
        self, pod_a_state_in_redis, second_principal_scope
    ):
        """
        The second principal attaches with the same reconnect token: same
        queue, its own scope, empty filters (the reconnect signal).
        """
        from app.lib.permissions.storage_permissions.storage_permissions_cache import (
            init_cache_for_user,
        )

        writer_b, token_b = await _setup_writer(second_principal_scope)
        await init_cache_for_user(second_principal_scope, [])
        yield await StorageCache.get_instance(second_principal_scope)
        await _teardown_writer(writer_b, token_b)
        _purge_keys_for_queue(pod_a_state_in_redis, second_principal_scope)

    async def test_hydrates_nothing_from_the_first_principals_namespace(
        self, second_principal_reconnect
    ):
        assert second_principal_reconnect.get("storage-permissions:/foo/") is None

    async def test_a_delete_is_not_authorized_from_the_first_principals_state(
        self, second_principal_reconnect
    ):
        # The inherited-capability the issue reports: with one shared
        # namespace this read the first principal's resource_paths and
        # returned True, on a file the storage API no longer reports.
        assert (
            await can_access_deleted_event(
                {"message": {"resource_address": "/foo/file1"}}
            )
            is False
        )

    async def test_the_first_principals_redis_state_is_left_intact(
        self, second_principal_reconnect, pod_a_state_in_redis, scope
    ):
        # Separate namespaces both ways: the attach must not clobber the
        # live stream's persisted state either.
        assert b"/foo/file1" in pod_a_state_in_redis.sets.get(
            f"cache:{scope.key}:files:/foo/", set()
        )
