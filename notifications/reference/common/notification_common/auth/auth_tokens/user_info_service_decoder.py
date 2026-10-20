# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
REST user-info-service token decoder.

Resolves a bearer token to a principal id by calling the deployed user-info
service over REST and forwarding the caller's token:

    GET {user_info_service_uri}/api/v1alpha/users/me
    Authorization: Bearer <token>

PRINCIPAL EQUIVALENCE
---------------------
The service returns a JSON body shaped like
``{id, display_name, email, username}`` (see the live_edit reference
``src/auth/rest.rs`` ``UserInfoDto`` ~270-278, ``RestUserinfoClient`` ~261-394).
The reference sets ``UserInfo.id = dto.id``; its JWT decoder sets
``UserInfo.id = oid (fallback sub)`` (``userinfo_jwt.rs:147``) and its gRPC
userinfo path sets ``id = response.user.id``. Therefore the response ``id``
field IS the ``principal_id_claim``/``oid`` value, and this module returns it
as the principal so the resolved principal matches what
``jwt_decoder.get_user_id`` would return for the equivalent JWT (which reads
``auth_settings.principal_id_claim``, configured to ``oid`` for this mode).
Only ``id`` is load-bearing for principal resolution; the other DTO fields are
ignored.

SECURITY
--------
Token material, the bearer header, the full URL, and backend response bodies
are never echoed into exception messages or logs (mirrors the reference's
URL/body scrubbing, ``classify_request_error`` ~147-180 and
``read_bounded_body`` ~182-215).
"""

import json
import logging
import time

from aiohttp import ClientError, ClientResponseError, ClientTimeout
from tenacity import (
    before_sleep_log,
    retry,
    retry_if_exception,
    stop_after_attempt,
    wait_random_exponential,
)

from notification_common.config.auth_settings import auth_settings
from notification_common.errors.errors import AuthenticationError, ServerError
from notification_common.http_client.http_client import http_client

logger = logging.getLogger(__name__)

# The user-info-service response carries no expiry, so (like the opaque-token
# decoder) we derive our own short-lived expiry.
_TOKEN_EXP_SECONDS = 300

# Cap the response body at 64 KiB to match the live_edit reference's
# USERINFO_MAX_RESPONSE_BYTES; a hostile backend must not be able to OOM the
# service by streaming an arbitrarily large body.
_MAX_RESPONSE_BYTES = 64 * 1024

_USERS_ME_PATH = "/api/v1alpha/users/me"

# Bound each request so a hung user-info backend cannot stall a request
# indefinitely. This is a fast internal identity lookup, so ~10s total is
# generous; the bounded per-attempt timeout is what makes the tenacity retry
# (wait_random_exponential(max=60)) meaningful — a timeout surfaces as an
# aiohttp.ClientError subclass and is retried, then mapped to ServerError.
_REQUEST_TIMEOUT_SECONDS = 10


async def get_user_id(token: str) -> tuple[str, float] | None:
    """
    Resolve a bearer token to ``(principal_id, expires_at)`` by calling the
    user-info service, with retries on transient failures.

    Returns ``None`` when the response omits the ``id`` field.

    Raises:
        AuthenticationError: the backend rejected the token (401/403).
        ServerError: the backend was unreachable or returned a fault
            (connect error / timeout / 429 / 5xx) or an oversized/malformed
            body. This is the distinct non-``AuthenticationError`` "unreachable"
            class; Task E owns the call-entry hard-fail semantics.
    """
    try:
        userinfo = await _fetch_userinfo_with_retry(token)
    except ClientResponseError as e:
        # Status-only message — never include the reason phrase, body, or URL.
        logger.warning("[Auth] user-info service returned HTTP %s", e.status)
        if e.status in (401, 403):
            raise AuthenticationError(
                "Unauthorized: The token is invalid or has expired"
            ) from e
        # 5xx / 429 / any other non-2xx is a backend fault, not an auth
        # decision -> distinct unreachable/server-error class.
        raise ServerError(f"user-info service returned HTTP {e.status}") from e
    except ClientError as e:
        # Connect error / timeout / transport error: scrub the exception so no
        # URL or token material leaks; surface only the exception type name.
        logger.warning("[Auth] user-info service unreachable: %s", type(e).__name__)
        raise ServerError(f"user-info service unreachable: {type(e).__name__}") from e

    principal_id = userinfo.get("id")
    if principal_id is None:
        return None

    return principal_id, time.time() + _TOKEN_EXP_SECONDS


def _should_retry_userinfo(exc: BaseException) -> bool:
    # Retry transient transport faults and 5xx/429; never retry 401/403 (or any
    # other < 500 status, matching opaque_token_decoder).
    if isinstance(exc, ClientResponseError):
        return exc.status >= 500 or exc.status == 429
    return isinstance(exc, ClientError)


@retry(
    stop=stop_after_attempt(4),
    wait=wait_random_exponential(multiplier=1, max=60),
    retry=retry_if_exception(_should_retry_userinfo),
    before_sleep=before_sleep_log(logger, logging.WARNING),
    reraise=True,
)
async def _fetch_userinfo_with_retry(token: str) -> dict:
    """
    GET the user-info service's ``/users/me`` endpoint with the forwarded
    bearer token, bounding the response body, and return the parsed JSON dict.
    """
    if http_client.session is None:
        raise ServerError("HTTP client is not initialized")

    # Trim a trailing slash so we don't emit a double slash before the path.
    base = auth_settings.user_info_service_uri.rstrip("/")
    url = f"{base}{_USERS_ME_PATH}"

    headers = {"Authorization": f"Bearer {token}"}

    timeout = ClientTimeout(total=_REQUEST_TIMEOUT_SECONDS)
    async with http_client.session.get(
        url, headers=headers, timeout=timeout
    ) as response:
        # raise_for_status() surfaces 4xx/5xx as ClientResponseError, which the
        # caller maps to AuthenticationError (401/403) or ServerError (else).
        response.raise_for_status()
        body = await _read_bounded_body(response)

    try:
        return json.loads(body)
    except ValueError as e:
        # A malformed body is a backend fault, not an auth decision. Never echo
        # the body itself.
        logger.warning(
            "[Auth] user-info service returned a malformed body: %s",
            type(e).__name__,
        )
        raise ServerError("user-info service returned a malformed body") from e


async def _read_bounded_body(response) -> bytes:
    """
    Read up to ``_MAX_RESPONSE_BYTES`` from the response body, raising
    ``ServerError`` the moment the accumulated size would exceed the cap.

    ``Content-Length`` is consulted as a fast-path rejection before any bytes
    are read, but a backend may omit it (chunked transfer-encoding), so the
    streaming cap is the correctness primitive — the header check is only an
    optimization and is never trusted alone. Never calls an unbounded
    ``response.json()`` / ``response.read()``.
    """
    content_length = response.content_length
    if content_length is not None and content_length > _MAX_RESPONSE_BYTES:
        raise ServerError(
            "user-info service response exceeded " f"{_MAX_RESPONSE_BYTES}-byte cap"
        )

    chunks = bytearray()
    async for chunk in response.content.iter_chunked(8192):
        if len(chunks) + len(chunk) > _MAX_RESPONSE_BYTES:
            raise ServerError(
                "user-info service response exceeded " f"{_MAX_RESPONSE_BYTES}-byte cap"
            )
        chunks.extend(chunk)
    return bytes(chunks)
