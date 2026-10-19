# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import jwt
from async_lru import alru_cache

from notification_common.auth import openid_discovery
from notification_common.config.auth_settings import auth_settings
from notification_common.errors.errors import (
    AuthenticationError,
    ServerError,
)

logger = logging.getLogger(__name__)


async def get_user_id(token: str) -> tuple[str, float] | None:
    decoded_token = await _decode_token(token)
    principal_id = decoded_token.get(auth_settings.principal_id_claim)
    exp = decoded_token.get("exp")

    if principal_id is None or exp is None:
        return None

    return principal_id, float(exp)


async def _decode_token(token: str) -> dict:
    """
    Verify the token against the IdP's signing keys and return its claims.

    Failing to reach the IdP is our fault, not the caller's, so a connection
    failure while fetching the JWKS becomes a `ServerError` rather than being
    reported to the caller as a bad token.

    A residual remains: a malformed JWKS document raises the same
    `PyJWKClientError` that an unknown `kid` does (PyJWT `PyJWKClient`'s JWKS
    payload check versus `get_signing_key`), and an empty one raises
    `PyJWKSetError`, so neither is type-distinguishable from a bad token and
    both still surface as a 401. Widening the carve-out to `PyJWKClientError`
    is not the fix, because an unknown `kid` is attacker-controllable and must
    stay a 401.
    """
    jwk_client = await _jwks_client()

    try:
        signing_key: jwt.PyJWK = jwk_client.get_signing_key_from_jwt(token)

        return jwt.decode(
            token,
            signing_key.key,
            algorithms=[signing_key.algorithm_name],
            audience=auth_settings.parsed_jwt_audiences,
        )

    except jwt.exceptions.PyJWKClientConnectionError as e:
        raise ServerError("Could not fetch the IdP signing keys") from e

    except Exception as e:
        raise AuthenticationError("Invalid or expired JWT") from e


@alru_cache()
async def _jwks_client():
    jwks_uri = await openid_discovery.get_jwks_uri()
    if not jwks_uri:
        raise ServerError("jwks_uri could not be fetched.")
    return jwt.PyJWKClient(jwks_uri, lifespan=auth_settings.auth_context_ttl)
