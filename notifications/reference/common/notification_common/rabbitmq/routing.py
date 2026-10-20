# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import hashlib
import re
from functools import lru_cache

from notification_common.config.common_settings import common_settings
from notification_common.errors.errors import BadRequestError


def routing_key(event_type: str, resource_id: str) -> str:
    """
    Create a publisher routing key from an event type and resource ID.
    The routing key must not exceed 255 bytes.

    We hash all parts of a resource_id to make it smaller because
    RabbitMQ has a 255-byte restriction on routing/binding key length.

    All single-slashes will be turned into "." in the returned string,
    while leaving any double-slashes "//" or a beginning
    slash from either arg.

    Removes any trailing "/".

    "." is a reserved character but doesn't need to be escaped because
    hashing takes care of that.

    To support deep resource paths, the resource_id portion is capped at
    ``common_settings.routing_max_prefix_depth`` total hashed segments.
    The structure for a path with N segments and cap M is:

      * N <= M: every segment hashed individually.
      * N >  M: first (M-2) segments hashed individually as the "head",
        the segments at positions (M-2)..(N-2) joined with "/" and hashed
        as a single "middle" segment, and the final segment hashed on
        its own as the "last" segment. Total: M segments.

    The last-segment-separate slot exists so that a LAZY filter's ``.*``
    suffix (built by ``lazy_filter_routing_key`` + ``.*``) can line up
    with the publisher's last-segment hash regardless of path depth.
    Without it, LAZY filters either silently mis-match or over-match
    once tail-folding kicks in.

    The event_type side of the budget is also bounded:
    ``common_settings.routing_max_event_type_parts`` caps how many
    dot-separated parts an event_type may have. Over-long event_types
    fail with a clear, dedicated error rather than the generic
    "routing_key too long" message. Together these two limits mean
    that valid event_types produce routing keys that fit the 255-byte
    RabbitMQ cap regardless of resource_id depth. The 255-byte check
    below remains as a defense-in-depth backstop for misconfiguration.


    Example:
        An event type may be something like:

            "omni.storage.created"

        And two resource ids may be something like:

            "s3://a/b/c/astronaut.usd"
            "s3://folder_1/folder_2/"

        The resultant routing keys would be:

            "omni.storage.created.s3://.a.b.c.astronaut.usd"
            "omni.storage.created.s3://folder_one.folder_2"

        And then after hashing, they would look like:

            "3aa53.ddece.e2fa5.06eabbb4.92eb5ffe.4a8a08f0.95291e58"
            "3aa53.ddece.e2fa5.64686ff3.1b1d4e28"


    Args:
        event_type: The type of event (e.g. 'FILE_CREATED')
        resource_id: The resource identifier (e.g. 'abc/123')

    Returns:
        A properly formatted routing key.

    Raises:
        ValueError: If event_type or resource_id is empty
                    or contains the escape string, or if the
                    routing_key is greater than 255 bytes.
    """
    return _build_key(event_type, resource_id, lazy_filter=False)


def lazy_filter_routing_key(event_type: str, resource_id: str) -> str:
    """
    Build the routing-key prefix for a LAZY (``.*``) filter binding.

    The caller is expected to append ``.*`` to the returned string to
    form the actual RabbitMQ binding key.

    Differs from ``routing_key`` (the publisher path) only in how the
    resource_id is encoded for deep paths: there is no separate "last
    segment" hash. Instead, every segment from position (M-2) onward is
    folded into a single trailing hash, where M is
    ``common_settings.routing_max_prefix_depth``. That single trailing
    hash lines up with the publisher's "middle" hash, and the ``.*`` the
    caller appends lines up with the publisher's "last" hash — so a
    LAZY filter at any depth correctly matches publisher events exactly
    one level below the filter path.

    Args:
        event_type: The type of event (e.g. 'FILE_CREATED')
        resource_id: The filter resource identifier (e.g. 'abc/123')

    Returns:
        The encoded prefix (without ``.*``) for a LAZY binding key.

    Raises:
        BadRequestError: Same conditions as ``routing_key``, plus the
            255-byte cap is evaluated against the prefix + a notional
            ``.*`` suffix so a too-long binding cannot be assembled.
    """
    return _build_key(event_type, resource_id, lazy_filter=True)


def _build_key(event_type: str, resource_id: str, lazy_filter: bool) -> str:
    if not event_type:
        raise BadRequestError("event_type must not be empty")

    if not resource_id:
        resource_id = ""

    if "/" in event_type:
        raise BadRequestError("event_type must not contain '/'")

    # Cap event_type parts up front so over-long event_types fail with a
    # clear, dedicated error instead of being detected as a generic
    # "routing_key too long" further down. Combined with the resource_id
    # tail-fold, this means valid event_types never produce a 255-byte
    # failure at runtime regardless of resource_id depth.
    event_type_parts = event_type.lower().strip(".").split(".")
    max_event_type_parts = common_settings.routing_max_event_type_parts
    if len(event_type_parts) > max_event_type_parts:
        raise BadRequestError(
            f"event_type has {len(event_type_parts)} parts, "
            f"maximum is {max_event_type_parts}"
        )

    resource_id = _encode_resource_id(resource_id, lazy_filter=lazy_filter)
    event_type = _encode_event_type(event_type)

    # finally, concat the event_type with the resource_id
    _routing_key = f"{event_type}.{resource_id}" if resource_id else event_type

    # Budget the 255-byte cap against the full binding (prefix + ".*")
    # for the LAZY case, so the caller can safely append ".*" without
    # blowing the cap.
    effective_length = len(_routing_key.encode("utf-8"))
    if lazy_filter:
        effective_length += len(".*")

    if effective_length > 255:
        raise BadRequestError(
            f"The routing_key must not exceed 255 bytes. Routing key: {_routing_key}"
        )

    return _routing_key


def max_safe_greedy_depth() -> int:
    """
    Maximum resource_id path depth at which a STARTS_WITH_GREEDY (``.#``)
    binding is guaranteed to match all deeper events.

    A GREEDY filter past this depth degenerates to (effectively) an EQ
    binding: the publisher folds any segments past the head boundary
    into a single "middle" hash, and the filter's hash at the same
    position is built from a different join of segments, so they fail
    to match. The trailing ``.#`` cannot recover because the publisher
    never emits more than ``routing_max_prefix_depth`` segments.

    Equals ``routing_max_prefix_depth - 2`` (head_end). LAZY filters and
    EQ filters are unaffected and work at any depth.
    """
    return common_settings.routing_max_prefix_depth - 2


def starts_with_greedy_path_depth(resource_id: str) -> int:
    """
    Number of path segments in ``resource_id`` for the purpose of
    STARTS_WITH_GREEDY filter validation.

    Uses the same splitting rules as the routing-key encoder
    (double-slashes and leading slashes are not separators, trailing
    slashes are dropped) so the count lines up with what the encoder
    sees.
    """
    if not resource_id:
        return 0
    return len(_split_ignore_double_slash(resource_id.rstrip("/")))


@lru_cache(maxsize=2048)
def hash_string(input_string: str, length=8) -> str:
    """
    RabbitMQ has a 255-character limit for routing keys.
    We hash the parts of the resource id path to shorten the
    resource_id values.

    Uses an MD5 hash with default length of 8.
    """
    encoded_string = input_string.encode("utf-8")
    hash_object = hashlib.md5(encoded_string)
    hex_digest = hash_object.hexdigest()

    return hex_digest[:length]


def _encode_event_type(event_type):
    """
    Event types need to be hashed to shorten them.
    Event types may have dots in them, in which case
    hash each dot separated section separately.
    """
    # always use lower case event types (case-insensitive)
    # and remove any starting or trailing "."
    event_type = event_type.lower().strip(".")

    # break into parts based on "." separator,
    # and hash each part of the event type to
    # make sure it's short
    hash_len = common_settings.routing_event_type_hash_length
    parts = [hash_string(part, length=hash_len) for part in event_type.split(".")]

    return ".".join(parts)


def _encode_resource_id(resource_id, lazy_filter: bool = False):
    """
    Resource IDs have slashes to separate the parts,
    but we need to encode them for rabbitmq by replacing
    all slashes with dots, ignoring double slashes, and
    hashing each part to reduce its size.

    Two encoding shapes share most of this logic, controlled by
    ``lazy_filter``:

    * ``lazy_filter=False`` (publisher / EQ / GREEDY filter):
      Past ``routing_max_prefix_depth`` segments, the path is folded
      into (max_depth - 2) head segments + 1 middle hash + 1 last hash,
      keeping the final segment in its own slot so LAZY ``.*`` bindings
      can line up with it.

    * ``lazy_filter=True`` (LAZY filter prefix):
      Past (max_depth - 2) head segments, every remaining segment is
      folded into a single tail hash. The caller is expected to append
      ``.*`` to the returned key, and that wildcard takes the slot the
      publisher fills with its "last" hash.

    Both shapes share the same head-segment count, so the head hashes
    of a publisher routing key and a LAZY binding key for paths that
    share a prefix line up exactly.
    """
    # drop any trailing slashes from the resource_id
    resource_id = resource_id.rstrip("/")

    # get the path parts from the resource_id by splitting on "/" but not "//"
    parts = _split_ignore_double_slash(resource_id)

    max_depth = common_settings.routing_max_prefix_depth
    hash_len = common_settings.routing_hash_length

    parts = _fold_parts(parts, max_depth=max_depth, lazy_filter=lazy_filter)
    parts = [hash_string(part, length=hash_len) for part in parts]

    # combine the parts into a string with "." as a separator
    return ".".join(parts)


def _fold_parts(parts: list[str], *, max_depth: int, lazy_filter: bool) -> list[str]:
    """
    Restructure ``parts`` into the head / middle / last (or head / tail)
    shape described in ``_encode_resource_id``.

    Returns a list of strings ready to be individually hashed.
    """
    if max_depth <= 0:
        return parts

    # Both shapes use the same head boundary so the head segments of
    # publisher and LAZY-filter encodings line up byte-for-byte.
    head_end = max_depth - 2

    if lazy_filter:
        # head + 1 folded tail (no last-separate). The caller adds ".*"
        # to take the slot the publisher fills with its "last" hash.
        if len(parts) <= head_end:
            return parts
        head = parts[:head_end]
        tail = "/".join(parts[head_end:])
        return head + [tail]

    # Publisher / EQ / GREEDY: when the path fits the cap, emit every
    # segment individually. Past the cap, restructure into head + middle
    # + last so a LAZY filter's .* has a stable per-event slot to match.
    if len(parts) <= max_depth:
        return parts
    head = parts[:head_end]
    middle = "/".join(parts[head_end:-1])
    last = parts[-1]
    return head + [middle, last]


def _split_ignore_double_slash(text) -> list[str]:
    """
    Splits on "/" but not "//".
    Also, if the text starts with a "/",
    then add that to the first item in the returned list.

    examples:

        text="/a/b/c" => ["/a", "b", "c"]

        text="http://a/b/c" => ["http://a", "b", "c"]
    """
    if not text:
        return []

    parts = re.split(r"(?<!/)/(?!/)", text)

    # if resource_id started with a slash, add that to the first part
    if parts[0] == "":
        parts.pop(0)
        parts[0] = f"/{parts[0]}"

    return parts
