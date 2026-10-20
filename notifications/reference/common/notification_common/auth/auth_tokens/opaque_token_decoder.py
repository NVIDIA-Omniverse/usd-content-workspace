# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging
import time

from aiohttp import ClientError, ClientResponseError
from tenacity import (
    before_sleep_log,
    retry,
    retry_if_exception,
    stop_after_attempt,
    wait_random_exponential,
)

from notification_common.auth import openid_discovery
from notification_common.config.auth_settings import auth_settings
from notification_common.errors.errors import AuthenticationError, ServerError
from notification_common.http_client.http_client import http_client

logger = logging.getLogger(__name__)

_TOKEN_EXP_SECONDS = 300


async def get_user_id(token: str) -> tuple[str, float] | None:
    """
    Retrieves the user ID by calling the userinfo endpoint, with retries.
    Uses the session managed by the base class property.

    Since we have no way of knowing when an opaque token expires,
    we have to use our own expiration time that is a setting.
    """
    try:
        userinfo = await _fetch_userinfo_with_retry(token)
        token_exp = time.time() + _TOKEN_EXP_SECONDS

        user = userinfo.get(auth_settings.principal_id_claim)

        if user is None:
            return None

        return user, token_exp

    except ClientResponseError as e:
        logger.exception(
            f"[Auth] HTTP status error {e.status} calling userinfo endpoint: {e}"
        )

        if e.status == 401:
            raise AuthenticationError(
                "Unauthorized: The token is invalid or has expired"
            ) from e

        raise


def _should_retry_userinfo(exc: BaseException) -> bool:
    if isinstance(exc, ClientResponseError) and exc.status < 500:
        return False
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
    Fetches userinfo from the endpoint with retry logic for transient errors.
    """
    if http_client.session is None:
        raise ServerError("HTTP client is not initialized")

    userinfo_endpoint = await openid_discovery.get_userinfo_endpoint()
    if not userinfo_endpoint:
        raise Exception("userinfo_endpoint could not be fetched.")

    headers = {"Authorization": f"Bearer {token}"}

    logger.debug(f"[Auth] Calling userinfo endpoint: {userinfo_endpoint}")

    async with http_client.session.get(userinfo_endpoint, headers=headers) as response:
        response.raise_for_status()
        userinfo = await response.json()
        return userinfo
