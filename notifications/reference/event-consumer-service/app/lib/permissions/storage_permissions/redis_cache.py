# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Redis key-naming helpers and write-through Op classes for the auth cache.

Every key here is namespaced by a ``scope_key`` -- the queue name plus the
presenting principal's reference, from
:class:`~app.lib.permissions.storage_permissions.cache_scope.CacheScope` --
so two principals attached to one non-durable queue never read or overwrite
each other's cached directory state. The key is never logged: the two hydrate
functions take the scope itself and report ``scope.queue_ref``.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from time import time
from typing import Any, TYPE_CHECKING

import redis.exceptions as redis_exceptions

from notification_common.redis import redis_client

from app.config import settings
from app.lib.permissions.storage_permissions.cache_scope import CacheScope

if TYPE_CHECKING:
    from app.lib.permissions.storage_permissions.directory_contents import (
        DirectoryContents,
    )
    from app.lib.permissions.storage_permissions.storage_cache import StorageCache

logger = logging.getLogger(__name__)


def meta_key(scope_key: str, dir_path: str) -> str:
    return f"cache:{scope_key}:meta:{dir_path}"


def files_key(scope_key: str, dir_path: str) -> str:
    return f"cache:{scope_key}:files:{dir_path}"


def folders_key(scope_key: str, dir_path: str) -> str:
    return f"cache:{scope_key}:folders:{dir_path}"


def prev_key(scope_key: str, dir_path: str) -> str:
    return f"cache:{scope_key}:prev:{dir_path}"


def dirs_key(scope_key: str) -> str:
    return f"cache:{scope_key}:dirs"


def ttl_seconds() -> int:
    return settings.settings.buffered_ttl_seconds


@dataclass(frozen=True)
class add_file:
    scope_key: str
    dir_path: str
    path: str

    def apply_to(self, pipeline: Any) -> None:
        key = files_key(self.scope_key, self.dir_path)
        pipeline.sadd(key, self.path)
        pipeline.expire(key, ttl_seconds())


@dataclass(frozen=True)
class remove_file:
    scope_key: str
    dir_path: str
    path: str

    def apply_to(self, pipeline: Any) -> None:
        pipeline.srem(files_key(self.scope_key, self.dir_path), self.path)


@dataclass(frozen=True)
class add_folder:
    scope_key: str
    dir_path: str
    path: str

    def apply_to(self, pipeline: Any) -> None:
        key = folders_key(self.scope_key, self.dir_path)
        pipeline.sadd(key, self.path)
        pipeline.expire(key, ttl_seconds())


@dataclass(frozen=True)
class remove_folder:
    scope_key: str
    dir_path: str
    path: str

    def apply_to(self, pipeline: Any) -> None:
        pipeline.srem(folders_key(self.scope_key, self.dir_path), self.path)


@dataclass(frozen=True)
class set_meta:
    scope_key: str
    dir_path: str
    authorized: bool
    expires_at: float

    def apply_to(self, pipeline: Any) -> None:
        key = meta_key(self.scope_key, self.dir_path)
        pipeline.hset(
            key,
            mapping={
                "authorized": int(self.authorized),
                "expires_at": self.expires_at,
            },
        )
        pipeline.expire(key, ttl_seconds())


@dataclass(frozen=True)
class set_prev:
    scope_key: str
    dir_path: str
    entries: dict[str, float]

    def apply_to(self, pipeline: Any) -> None:
        if not self.entries:
            return
        key = prev_key(self.scope_key, self.dir_path)
        for path, expires_at in self.entries.items():
            pipeline.hset(key, path, expires_at)
        pipeline.expire(key, ttl_seconds())


@dataclass(frozen=True)
class del_prev:
    scope_key: str
    dir_path: str
    path: str

    def apply_to(self, pipeline: Any) -> None:
        pipeline.hdel(prev_key(self.scope_key, self.dir_path), self.path)


@dataclass(frozen=True)
class replace_dir:
    scope_key: str
    dir_path: str
    files: frozenset[str] = field(default_factory=frozenset)
    folders: frozenset[str] = field(default_factory=frozenset)
    authorized: bool = True
    expires_at: float = 0.0
    prev: dict[str, float] = field(default_factory=dict)

    def apply_to(self, pipeline: Any) -> None:
        ttl = ttl_seconds()
        f_key = files_key(self.scope_key, self.dir_path)
        d_key = folders_key(self.scope_key, self.dir_path)
        p_key = prev_key(self.scope_key, self.dir_path)
        m_key = meta_key(self.scope_key, self.dir_path)
        idx_key = dirs_key(self.scope_key)

        # DEL must precede SADD/HSET on the SAME keys: a partial flush
        # must never leave a mix of old and new contents.
        pipeline.delete(f_key)
        pipeline.delete(d_key)
        pipeline.delete(p_key)

        if self.files:
            pipeline.sadd(f_key, *self.files)
        pipeline.expire(f_key, ttl)

        if self.folders:
            pipeline.sadd(d_key, *self.folders)
        pipeline.expire(d_key, ttl)

        pipeline.hset(
            m_key,
            mapping={
                "authorized": int(self.authorized),
                "expires_at": self.expires_at,
            },
        )
        pipeline.expire(m_key, ttl)

        if self.prev:
            for path, expires_at in self.prev.items():
                pipeline.hset(p_key, path, expires_at)
            pipeline.expire(p_key, ttl)

        pipeline.sadd(idx_key, self.dir_path)
        pipeline.expire(idx_key, ttl)


@dataclass(frozen=True)
class ensure_dir_index:
    scope_key: str
    dir_path: str

    def apply_to(self, pipeline: Any) -> None:
        key = dirs_key(self.scope_key)
        pipeline.sadd(key, self.dir_path)
        pipeline.expire(key, ttl_seconds())


def _coerce_float(value: Any) -> float | None:
    """Coerce a Redis-returned bytes/str/None value to float, or None on failure."""
    if value is None:
        return None
    try:
        return float(value)
    except ValueError:
        return None
    except TypeError:
        return None


def _coerce_authorized(value: Any) -> bool:
    """Coerce a Redis-returned bytes/str/None value to bool. Defaults to False."""
    if value is None:
        return False
    try:
        return bool(int(value))
    except ValueError:
        return False
    except TypeError:
        return False


async def hydrate_dir_from_redis(
    scope: CacheScope, dir_path: str
) -> DirectoryContents | None:
    """
    Read a directory's persisted state from Redis in a single 4-op pipeline
    and reconstruct a `DirectoryContents`.

    Takes the whole scope rather than its key because it also logs: the key
    addresses Redis, `scope.queue_ref` is the part that may be written down.

    Returns None on any of: Redis disabled, no client, missing/empty meta,
    expired meta, malformed `expires_at`, or any Redis connection error.
    Never raises.

    Lazy-imports `DirectoryContents` to keep this module's dependency
    direction one-way (mutators import this module, not the other way).
    """
    if not settings.settings.redis_enabled:
        return None
    client = redis_client.redis_client
    if client is None:
        return None

    scope_key = scope.key

    try:
        pipeline = client.pipeline()
        pipeline.hgetall(meta_key(scope_key, dir_path))
        pipeline.smembers(files_key(scope_key, dir_path))
        pipeline.smembers(folders_key(scope_key, dir_path))
        pipeline.hgetall(prev_key(scope_key, dir_path))
        results = await pipeline.execute()
    except (
        redis_exceptions.ConnectionError,
        redis_exceptions.TimeoutError,
        TimeoutError,
        OSError,
    ):
        # No dir_path: it is the client's folder path, and record extras are
        # exported as log attributes when telemetry is on.
        logger.warning("redis hydrate failed", extra={"queue_ref": scope.queue_ref})
        return None

    # A partial / malformed pipeline response (fewer than four results)
    # is treated as a Redis miss rather than crashing the auth path.
    if not results or len(results) < 4:
        return None
    meta_raw, files_raw, folders_raw, prev_raw = results[:4]

    if not meta_raw:
        return None

    expires_at = _coerce_float(meta_raw.get(b"expires_at"))
    if expires_at is None or expires_at <= time():
        return None

    authorized = _coerce_authorized(meta_raw.get(b"authorized"))

    files = {member.decode("utf-8") for member in files_raw}
    folders = {member.decode("utf-8") for member in folders_raw}

    prev: dict[str, float] = {}
    for path_bytes, expires_bytes in prev_raw.items():
        prev_expires = _coerce_float(expires_bytes)
        if prev_expires is not None:
            prev[path_bytes.decode("utf-8")] = prev_expires

    from app.lib.permissions.storage_permissions.directory_contents import (
        DirectoryContents,
    )

    return DirectoryContents.from_redis(
        authorized=authorized,
        files=files,
        folders=folders,
        expires_at=expires_at,
        prev=prev,
        dir_path=dir_path,
    )


async def hydrate_all_dirs_from_redis(
    scope: CacheScope, cache_dict: StorageCache
) -> int:
    """
    Bulk-hydrate every directory this scope has touched from Redis into
    `cache_dict`. Reads `cache:{scope.key}:dirs` once to get the list of
    directories, then issues one `hydrate_dir_from_redis` per dir.

    Scoped by queue AND presenting principal, so a reconnect inherits the
    state its own principal accumulated and never another's.

    Returns the count of directories successfully hydrated. Per-dir
    failures (None returns from `hydrate_dir_from_redis`) are silently
    skipped. Reads the dirs index once; per-dir hydration is N round
    trips. Acceptable for the reconnect path; a future MGET-style batch
    is out of scope.

    Returns 0 on any of: Redis disabled, no client, empty dirs index,
    or any Redis connection error reading the index. Never raises —
    reconnect must not fail because Redis is unreachable.
    """
    if not settings.settings.redis_enabled:
        return 0
    client = redis_client.redis_client
    if client is None:
        return 0

    try:
        dir_paths_raw = await client.smembers(dirs_key(scope.key))
    except (
        redis_exceptions.ConnectionError,
        redis_exceptions.TimeoutError,
        TimeoutError,
        OSError,
    ):
        logger.warning(
            "redis hydrate_all failed reading dirs index",
            extra={"queue_ref": scope.queue_ref},
        )
        return 0

    if not dir_paths_raw:
        return 0

    # Lazy import to keep redis_cache.py's module-level dependency
    # direction one-way (storage_permissions_cache imports redis_cache,
    # not the other way around).
    from app.lib.permissions.storage_permissions.storage_permissions_cache import (
        _cache_key,
        _install_entry,
    )

    hydrated = 0
    for member in dir_paths_raw:
        dir_path = member.decode("utf-8") if isinstance(member, bytes) else member
        dir_contents = await hydrate_dir_from_redis(scope, dir_path)
        if dir_contents is None:
            continue
        # On a reconnect the cache starts empty and nothing is replaced. A
        # filter update that names no storage filters also lands here, on a
        # cache that may hold entries, so the install reports a replaced
        # entry's drops as it leaves.
        _install_entry(cache_dict, _cache_key(dir_path), dir_contents)
        hydrated += 1

    return hydrated
