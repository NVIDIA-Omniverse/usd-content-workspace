# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from typing import Optional

from async_lru import alru_cache

from notification_common.config.auth_settings import auth_settings
from notification_common.http_client.http_client import http_client


async def get_jwks_uri() -> Optional[str]:
    json = await _openid_discovery()
    return json.get("jwks_uri")


async def get_userinfo_endpoint() -> Optional[str]:
    json = await _openid_discovery()
    return json.get("userinfo_endpoint")


@alru_cache(maxsize=None)
async def _openid_discovery() -> dict:
    if not http_client.session:
        raise Exception("HTTP client is not initialized")

    response = await http_client.session.get(auth_settings.openid_configuration_uri)
    json = await response.json()

    if not json:
        raise Exception(
            "No data returned from openid discovery endpoint "
            f"{auth_settings.openid_configuration_uri}"
        )

    return json
