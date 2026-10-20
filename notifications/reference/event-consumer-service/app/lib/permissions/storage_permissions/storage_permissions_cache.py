# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import json
import logging
import time as time_module
import urllib.parse
from time import time

from tenacity import (
    before_sleep_log,
    retry,
    retry_if_exception,
    stop_after_attempt,
    wait_random_exponential,
)

from notification_common.auth.auth_context import auth_token_var
from notification_common.errors.errors import ServerError, AuthenticationError
from notification_common.http_client.http_client import http_client

from app.config import settings
from app.lib.models.resource_filter import ResourceFilter
from app.lib.observability.cache_hydration_metrics import cache_hydration_metrics
from app.lib.permissions.storage_permissions import redis_cache
from app.lib.permissions.storage_permissions.cache_scope import CacheScope
from app.lib.permissions.storage_permissions.cache_writer import active_writer
from app.lib.permissions.storage_permissions.storage_cache import StorageCache
from app.lib.permissions.storage_permissions.directory_contents import (
    DirectoryContents,
    ListingOutcome,
)

logger = logging.getLogger(__name__)

# storage-service returns HTTP 400 with this exact detail (its
# Error::UnsupportedAddress, in backend/router/error.rs) when a resource
# address's host matches no configured storage backend — i.e. the address
# routes nowhere and no recheck will change that. We treat it as an
# unauthorized directory (ListingOutcome.UNSUPPORTED_ADDRESS) rather than a
# fatal error. This is a deliberately narrow match on the message text: it is
# the only discriminator storage exposes today (its 400 body carries
# type="about:blank" / title="Bad Request" with no machine-readable error
# code). Every other 400 still raises, so genuine malformed-request bugs
# surface.
_UNSUPPORTED_ADDRESS_DETAIL = "resource address is not supported"

# storage-core answers a listing of a prefix that holds no blobs with a 404
# whose detail contains this text ("No elements found for folder address" on
# Azure, "No elements found" on S3), and the storage API's list cache replays
# that message when it serves a cached 404. Both producers run only after the
# storage service's List permission check on the folder has passed, and a
# denied caller gets a 403, never a 404. So this 404 is an authorized, empty
# listing (ListingOutcome.EMPTY_LISTING): the folder may be genuinely empty,
# or its listing may lag a write that just happened. Every other 404 (an
# empty body, a gateway, an unmatched route, a missing container) stays
# unauthorized as ListingOutcome.UNRECOGNIZED_NOT_FOUND, so a reworded
# message fails closed with a WARNING rather than failing open.
_EMPTY_LISTING_DETAIL = "no elements found"

# Module-level tuple so the `except` below stays a name, not an inline
# `except (A, B):` that Black 25 strips parens from on py3.14 (see b.8qg).
_JSON_PARSE_EXC = (ValueError, TypeError, AttributeError)


async def init_cache_for_user(
    scope: CacheScope, filters_used_in_request: list[ResourceFilter]
) -> None:
    """
    For storage events, filters will always have resource_id
    values that represent a folder.
    We need to cache the contents of the folder specified.

    For storage events, the FilterType always is STARTS_WITH_(LAZY or GREEDY),
    so we want to know about events "in" the folder specified by the resource_id.

    We may be re-initializing, so free up memory from any filters that
    were in the cache already but are no longer needed.

    :param scope: The queue and presenting principal this cache is
        namespaced by; `scope.key` is what the Redis keys are built from
    :param filters_used_in_request: List of ResourceFilters objects used in request
    """
    # The first thing to do is to initialize the cache dict, possibly from Redis
    cache_dict = await StorageCache.get_instance(scope)

    if not filters_used_in_request:
        # Reconnect: bulk-hydrate every dir this scope had touched from Redis
        # so a Pod B that didn't see the original consume still inherits Pod
        # A's accumulated resource_paths and previously_visible_paths. On
        # Redis empty/disabled/unreachable the helper returns 0 and the
        # cache stays empty — _fetch_dir_contents will lazily populate.
        # Another principal's state on the same queue lives under a
        # different scope key and is not reachable from here.
        hydrated = await redis_cache.hydrate_all_dirs_from_redis(scope, cache_dict)
        # This branch IS the reconnect path, so the scope's queue name is
        # always a non-durable broker name and always embeds the reconnect
        # token the client just presented. Log the reference, never the
        # name and never the scope key.
        logger.info(
            "reconnect hydrated %d directories from Redis for queue %s",
            hydrated,
            scope.queue_ref,
        )
        return

    filters_used_in_request = set(filters_used_in_request)

    # for each filter, make sure we have results from the storage API cached
    for _filter in set(filters_used_in_request):
        await _fetch_dir_contents(_filter.resource_id)

    # if there are old filters in the cache no longer being used,
    # free up memory by deleting those keys from the cache dict
    keys_still_in_use = {_cache_key(rf.resource_id) for rf in filters_used_in_request}
    keys_to_delete = {key for key in cache_dict if key not in keys_still_in_use}

    # Contractual: this branch does NOT enqueue Redis ops in this Epic
    # — Redis entries TTL out naturally; future readers should not add
    # a Redis cleanup here.
    for key in keys_to_delete:
        # pop with a default rather than del: another task may already
        # have removed the key, which is not an error here.
        removed = cache_dict.pop(key, None)
        if removed is not None:
            # The entry leaves the stream's cache here, so any events
            # dropped against it are reported now or never.
            report_dropped_events(cache_dict.scope, removed)


def _should_retry_storage(exc: BaseException) -> bool:
    # Don't retry on auth errors — token won't get better with more attempts.
    return isinstance(exc, Exception) and not isinstance(exc, AuthenticationError)


def _response_detail(body_text: str | None) -> str:
    """Return a storage-API error body's detail, lower-cased; never raise.

    The detail is the JSON body's ``detail`` field, or the raw body text when
    the body is not JSON. A missing or non-string ``detail`` yields "".
    """
    try:
        detail = json.loads(body_text).get("detail", "")
    except _JSON_PARSE_EXC:
        detail = body_text or ""
    if not isinstance(detail, str):
        return ""
    return detail.strip().lower()


def _is_unsupported_address_response(body_text: str) -> bool:
    """Return True if a storage-API 400 body is the UnsupportedAddress error.

    The match is on the message text because that is the only stable
    discriminator storage's 400 exposes today; see _UNSUPPORTED_ADDRESS_DETAIL.
    """
    return _UNSUPPORTED_ADDRESS_DETAIL in _response_detail(body_text)


def _is_empty_listing_response(body_text: str) -> bool:
    """Return True if a storage-API 404 body is the empty-listing answer.

    The match is on the message text; see _EMPTY_LISTING_DETAIL.
    """
    return _EMPTY_LISTING_DETAIL in _response_detail(body_text)


async def _classify_not_found(response, resource_address: str) -> DirectoryContents:
    """
    Classify a storage-API 404 listing by its body. Never raises: an
    exception here would fail the event's authorization check, which nacks
    the message and tears the stream down.
    """
    try:
        body_text = await response.text()
    except Exception as exc:
        # An unreadable body cannot be shown to be the empty-listing answer,
        # so it is handled as an unrecognized 404 (unauthorized), not raised.
        logger.debug(
            "storage listing 404 body unreadable (%s) for %s",
            type(exc).__name__,
            resource_address,
        )
        body_text = ""

    if _is_empty_listing_response(body_text):
        return DirectoryContents(
            file_paths=[],
            folder_paths=[],
            dir_path=resource_address,
            outcome=ListingOutcome.EMPTY_LISTING,
        )

    return DirectoryContents(
        dir_path=resource_address, outcome=ListingOutcome.UNRECOGNIZED_NOT_FOUND
    )


@retry(
    stop=stop_after_attempt(4),
    wait=wait_random_exponential(multiplier=1, max=60),
    retry=retry_if_exception(_should_retry_storage),
    before_sleep=before_sleep_log(logger, logging.WARNING),
    reraise=True,
)
async def fetch_from_storage_api(
    resource_address, limit_to_one_result=False
) -> DirectoryContents:
    url_path = urllib.parse.quote(resource_address, safe="")
    base_url = f"{settings.storage_endpoint()}v1beta/filefolder/list/{url_path}"
    headers = {"Authorization": f"Bearer {auth_token_var.get()}"}

    all_file_paths: list[str] = []
    all_folder_paths: list[str] = []
    continuation_handle: str | None = None

    while True:
        url = _build_storage_api_url(base_url, limit_to_one_result, continuation_handle)

        try:
            response = await http_client.session.get(url, headers=headers)
        except Exception as e:
            raise ServerError(str(e)) from e

        status = _status_code(response)

        if status == 401:
            raise AuthenticationError(
                "Authentication failed for storage API. "
                "Ensure JWT is valid and not expired.",
                await response.text(),
            )

        if status == 403:
            return DirectoryContents(
                dir_path=resource_address, outcome=ListingOutcome.FORBIDDEN
            )

        if status == 404:
            return await _classify_not_found(response, resource_address)

        if status >= 300:
            body_text = await response.text()
            # An unsupported-address 400 means the address routes to no
            # configured backend — treat it as an unauthorized directory
            # rather than a fatal error. See _UNSUPPORTED_ADDRESS_DETAIL.
            if status == 400 and _is_unsupported_address_response(body_text):
                return DirectoryContents(
                    dir_path=resource_address,
                    outcome=ListingOutcome.UNSUPPORTED_ADDRESS,
                )
            raise ServerError(
                f"Failed to get data from storage API: {status} {body_text}"
            )

        data = await response.json()

        all_file_paths.extend(data["sub_resource_addresses"])
        all_folder_paths.extend(data["subfolder_addresses"])

        # Check if we need to fetch more pages
        continuation_handle = data.get("next_continuation_handle")
        if limit_to_one_result or not continuation_handle:
            break

    logger.debug(
        "Storage API response for %s: files=%s, folders=%s",
        resource_address,
        all_file_paths,
        all_folder_paths,
    )

    return DirectoryContents(
        file_paths=all_file_paths,
        folder_paths=all_folder_paths,
        dir_path=resource_address,
        outcome=ListingOutcome.LISTED,
    )


def _build_storage_api_url(
    base_url: str, limit_to_one_result: bool, continuation_handle: str | None
) -> str:
    """
    Builds the storage API URL with the appropriate query parameters.
    """
    params = []
    if limit_to_one_result:
        params.append("max_page_size=1")
    if continuation_handle:
        encoded_handle = urllib.parse.quote(continuation_handle, safe="")
        params.append(f"continuation_handle={encoded_handle}")

    if params:
        return f"{base_url}?{'&'.join(params)}"
    return base_url


async def get_parent_resource(resource_address: str) -> DirectoryContents:
    """
    We need to check on the permissions for the folder that this
    resource is in.
    So we need to get the DirectoryContents object for the parent
    of the object at the resource_address.
    """
    folder_path = _parent_folder_path(resource_address)
    return await _fetch_dir_contents(folder_path)


def _cache_key(path: str) -> str:
    return f"storage-permissions:{path}"


async def _fetch_dir_contents(
    resource_address: str,
) -> DirectoryContents:
    """
    Fetches the contents of the directory at `resource_address`,
    either from the cache or from the storage API.
    """
    cache_key = _cache_key(resource_address)
    cache_dict = await StorageCache.get_instance()
    old_dir_contents = cache_dict.get(cache_key)

    if old_dir_contents and not old_dir_contents.expired():
        return old_dir_contents

    # Wall-clock starts here so it covers Redis hydration AND, on miss,
    # the storage-API call plus in-memory install.
    hydrate_start = time_module.monotonic()
    scope = cache_dict.scope

    redis_dir_contents = await redis_cache.hydrate_dir_from_redis(
        scope, resource_address
    )
    if redis_dir_contents is not None:
        if (
            old_dir_contents
            and old_dir_contents.authorized
            and redis_dir_contents.authorized
        ):
            redis_dir_contents.absorb_disappeared_paths(old_dir_contents)

        _install_entry(cache_dict, cache_key, redis_dir_contents)

        elapsed_ms = (time_module.monotonic() - hydrate_start) * 1000.0
        cache_hydration_metrics.record_hydrate("redis", elapsed_ms)
        logger.debug(
            "hydrated dir from redis",
            extra={
                "queue_ref": scope.queue_ref,
                "dir_path": resource_address,
                "elapsed_ms": elapsed_ms,
            },
        )
        return redis_dir_contents

    new_dir_contents = await fetch_from_storage_api(resource_address)

    if old_dir_contents and old_dir_contents.authorized and new_dir_contents.authorized:
        new_dir_contents.absorb_disappeared_paths(old_dir_contents)

    # An authorized result (a 200 listing or the storage API's empty-listing
    # 404) keeps the full permissions TTL it was built with. For an
    # unauthorized result (a 403, an unsupported-address 400, or any other
    # 404): on the initial check (during initialize()), old_dir_contents is
    # None and the result stays immediately-expired so the first incoming
    # event triggers a fresh Storage API call. On re-checks, the debounce
    # prevents calling the Storage API more than once per
    # unauthorized_recheck_seconds.
    if not new_dir_contents.authorized and old_dir_contents is not None:
        new_dir_contents.expires_at = (
            time() + settings.settings.unauthorized_recheck_seconds
        )

    _install_entry(cache_dict, cache_key, new_dir_contents, from_storage_api_fetch=True)

    # Mirror the freshly fetched DirectoryContents to Redis as a single
    # bulk-replace Op (DEL + SADD + HSET in one pipeline). The cache-hit
    # early return above intentionally does NOT enqueue — we only
    # write-through when we actually fetched from the storage API.
    writer = active_writer()
    if writer is not None:
        writer.enqueue(
            redis_cache.replace_dir(
                scope_key=writer.scope_key,
                dir_path=resource_address,
                files=frozenset(new_dir_contents.resource_paths or ()),
                folders=frozenset(new_dir_contents.folders or ()),
                authorized=new_dir_contents.authorized,
                expires_at=new_dir_contents.expires_at,
                prev=dict(new_dir_contents.previously_visible_paths),
            )
        )

    elapsed_ms = (time_module.monotonic() - hydrate_start) * 1000.0
    cache_hydration_metrics.record_hydrate("storage_api", elapsed_ms)
    logger.debug(
        "hydrated dir from storage api",
        extra={
            "queue_ref": scope.queue_ref,
            "dir_path": resource_address,
            "elapsed_ms": elapsed_ms,
        },
    )

    return new_dir_contents


def _install_entry(
    cache_dict: StorageCache,
    cache_key: str,
    dir_contents: DirectoryContents,
    from_storage_api_fetch: bool = False,
) -> None:
    """
    Install a fetched or hydrated entry under ``cache_key`` and report the
    unreported drops of the entry it replaces, exactly once.

    The replaced entry is read here, at install time, not taken from an
    earlier read: a fetch awaits Redis and the storage API, and while it
    waits another task on the same stream (a gRPC filter update racing the
    consume loop) can install its own entry under the same key and have
    events dropped against it. That entry is the one leaving the cache.
    There is no await between the read, the install, and the report.

    For an unauthorized storage-API fetch, the fetch-time line carries the
    replaced entry's count. Otherwise report_dropped_events does, and emits
    nothing when there is no count to report.
    """
    replaced = cache_dict.get(cache_key)
    cache_dict[cache_key] = dir_contents

    if from_storage_api_fetch and not dir_contents.authorized:
        _log_unauthorized_listing(cache_dict.scope, dir_contents, replaced)
    elif replaced is not None:
        report_dropped_events(cache_dict.scope, replaced)


def _log_unauthorized_listing(
    scope: CacheScope,
    new_dir_contents: DirectoryContents,
    replaced_dir_contents: DirectoryContents | None,
) -> None:
    """
    Record, without DEBUG logging, that a storage-API listing left a
    directory unauthorized for this stream, so its storage events are being
    dropped until the next recheck.

    ``dropped_events`` is the count of unreported events the readers
    dropped against the unauthorized entry this fetch replaced, or 0 when
    there was none. This line is that entry's one report, so
    report_dropped_events is not also called for it (see _install_entry).
    Drops against an entry that leaves the cache any other way are reported
    by report_dropped_events.
    No folder path, token, or queue name: record extras are exported as log
    attributes when telemetry is on, and client folder paths stay out of
    INFO-and-above telemetry. The folder path is in the DEBUG lines.
    """
    outcome = new_dir_contents.outcome
    dropped_events = (
        0
        if replaced_dir_contents is None
        else _take_dropped_events(replaced_dir_contents)
    )

    logger.log(
        _unauthorized_log_level(outcome),
        "storage listing did not authorize a directory; "
        "its storage events are dropped until the recheck",
        extra=_unauthorized_log_extra(scope, outcome, dropped_events),
    )


def report_dropped_events(scope: CacheScope, dir_contents: DirectoryContents) -> None:
    """
    Report the storage events dropped against an unauthorized entry that is
    leaving this stream's cache, unless a fetch-time line already carried
    them (see _log_unauthorized_listing).

    Called wherever an entry leaves the cache other than by an unauthorized
    storage-API refetch: an authorized refetch, Redis hydration, a filter
    update that drops the directory, and stream end. Emits nothing for an
    authorized entry or when no events were dropped. The count is taken from
    the entry, so a second call for the same entry reports nothing and each
    drop is reported once.

    Same level rule and extras as the fetch-time line, and for the same
    reason no folder path, token, or queue name. An entry hydrated from
    Redis has no outcome and is reported at WARNING.
    """
    dropped_events = _take_dropped_events(dir_contents)
    if dropped_events == 0:
        return

    outcome = dir_contents.outcome
    logger.log(
        _unauthorized_log_level(outcome),
        "storage events were dropped for a directory the storage listing "
        "did not authorize",
        extra=_unauthorized_log_extra(scope, outcome, dropped_events),
    )


def report_dropped_events_at_stream_end(cache_dict: StorageCache) -> None:
    """
    Report the drops against every unauthorized entry still in a stream's
    cache when the stream ends. The entries are left in place; their counts
    are taken, so a later report for the same entry emits nothing.
    """
    for dir_contents in list(cache_dict.values()):
        report_dropped_events(cache_dict.scope, dir_contents)


def _take_dropped_events(dir_contents: DirectoryContents) -> int:
    """
    Return the count of events dropped against an unauthorized entry and
    reset it to 0, so the same drops are never reported twice. An authorized
    entry has no drops to report and returns 0.
    """
    if dir_contents.authorized:
        return 0
    dropped_events = dir_contents.dropped_events
    dir_contents.dropped_events = 0
    return dropped_events


def _unauthorized_log_level(outcome: ListingOutcome | None) -> int:
    """
    A 403 is an ordinary denial, so it is INFO. An unsupported-address 400
    or an unrecognized 404 points at an address no backend serves, a
    misrouted storage endpoint, or a reworded empty-listing message, so it
    warns. So does an unknown outcome (an entry hydrated from Redis).
    """
    return logging.INFO if outcome is ListingOutcome.FORBIDDEN else logging.WARNING


def _unauthorized_log_extra(
    scope: CacheScope, outcome: ListingOutcome | None, dropped_events: int
) -> dict:
    return {
        "queue_ref": scope.queue_ref,
        "storage_status": None if outcome is None else outcome.storage_status,
        "listing_outcome": None if outcome is None else outcome.value,
        "dropped_events": dropped_events,
    }


def _parent_folder_path(file_path: str) -> str:
    """
    Given a path like a/b/c, a/b/c/, or a/b/file.usd
    this will return the parent folder path:
    a/b/
    """
    return file_path.rstrip("/").rsplit("/", 1)[0] + "/"


def _status_code(response):
    """
    Extracted to facilitate mocking during tests
    """
    return response.status
