# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import aiohttp
from tenacity import (
    retry,
    retry_if_exception,
    stop_after_attempt,
    wait_random_exponential,
)

from notification_common.config.common_settings import common_settings
from notification_common.http_client.http_client import http_client
from notification_common.permissions.models import (
    AuthorizationBatchRequest,
    AuthorizationBatchResponse,
    AuthorizationRequest,
    AuthorizationResponse,
    PermissionClientError,
    PermissionInvocationError,
    PermissionResponseError,
    PermissionAuthenticationError,
    AuthorizationRequestBase,
    PermissionAuthorizationError,
)

logger = logging.getLogger(__name__)


def _handle_errors(func):
    """Decorator to handle common error cases for permission service requests."""

    async def wrapper(*args, **kwargs):
        try:
            return await func(*args, **kwargs)
        except PermissionClientError:
            raise
        except aiohttp.ClientResponseError as e:
            if e.status == 401:
                raise PermissionAuthenticationError("Authentication failed") from e
            elif e.status == 403:
                raise PermissionAuthorizationError("Authorization failed") from e
            raise PermissionInvocationError(
                "Failed to invoke permission service"
            ) from e
        except (
            aiohttp.ClientError,
            aiohttp.ServerTimeoutError,
            aiohttp.ServerConnectionError,
            aiohttp.ServerDisconnectedError,
        ) as e:
            raise PermissionInvocationError(
                "Failed to invoke permission service"
            ) from e
        except ValueError as e:
            raise PermissionResponseError(str(e)) from e
        except Exception as e:
            raise PermissionInvocationError(
                "Unexpected error while invoking permission service"
            ) from e

    return wrapper


@_handle_errors
async def authorize(request: AuthorizationRequest) -> AuthorizationResponse:
    """Check if an action is authorized."""
    data = await _request(path="/v1beta/authorization/", request=request)

    return AuthorizationResponse.model_validate(data)


@_handle_errors
async def batch_authorize(
    request: AuthorizationBatchRequest,
) -> AuthorizationBatchResponse:
    """Check if multiple actions are authorized."""
    data = await _request(path="/v1beta/authorization/batch/", request=request)

    return AuthorizationBatchResponse.model_validate(data)


def _should_retry_request(exc: BaseException) -> bool:
    if isinstance(exc, aiohttp.ClientResponseError) and exc.status < 500:
        return False
    return isinstance(exc, (aiohttp.ClientError, aiohttp.ServerTimeoutError))


@retry(
    stop=stop_after_attempt(4),
    wait=wait_random_exponential(multiplier=1, max=60),
    retry=retry_if_exception(_should_retry_request),
    reraise=True,
)
async def _request(path: str, request: AuthorizationRequestBase) -> dict:
    if http_client.session is None:
        raise PermissionInvocationError("HTTP client is not initialized")

    base_url = common_settings.permissions_endpoint.rstrip("/")
    url = f"{base_url}{path}"

    headers = {"Authorization": f"Bearer {request.token}"}
    request_body = request.model_dump(mode="json", exclude_none=True, exclude={"token"})

    async with http_client.session.request(
        "POST", url, json=request_body, headers=headers
    ) as response:
        response.raise_for_status()
        return await response.json()
