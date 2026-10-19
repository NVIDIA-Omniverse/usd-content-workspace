# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from async_lru import alru_cache

from notification_common.auth.auth_context import auth_token_var
from notification_common.config import metadata
from notification_common.config.common_settings import common_settings
from notification_common.permissions.models import (
    AuthorizationRequest,
    Action,
    AuthorizationDecision,
    Resource,
)
from notification_common.permissions.permissions_http_client import authorize


async def can(action: str, resource: Resource = None) -> bool:
    if not common_settings.permissions_enabled:
        return True

    token = auth_token_var.get()
    return await _cached_can(token, action, resource)


@alru_cache(ttl=common_settings.permissions_ttl_seconds)
async def _cached_can(token: str, action: str, resource: Resource = None) -> bool:
    authorization_request = AuthorizationRequest(
        action=Action(service=metadata.project_name(), name=action),
        token=token,
    )

    if resource:
        authorization_request.resource = resource

    auth_response = await authorize(authorization_request)

    return auth_response.decision == AuthorizationDecision.ALLOW
