# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

# SECURITY: This module decodes the JWT *payload* for principal-id extraction
# only. It performs NO token validation — it does NOT verify the signature,
# issuer, audience, or expiry. Token *validity* is delegated upstream to a
# component (e.g. Envoy) that is trusted to have already validated the bearer
# token before this decoder is consulted. This decoder MUST only be used in the
# `jwtUnverified` principal-source mode, where a validated token is guaranteed
# to be present; using it anywhere a token's validity is not already guaranteed
# would create a path that trusts unvalidated, attacker-controllable claims.
# Re-verifying locally would duplicate the upstream component's contract and
# create a second, divergent trust anchor, so it is intentionally omitted here.
#
# The same reasoning governs the expiry value this decoder returns. Because the
# mode makes no expiry decision, it must not write one into a cache that another
# layer enforces: returning the token's own `exp` would let the caching layer
# refuse, on a later call, a token this decoder deliberately accepted. The
# second element of the returned tuple is therefore a *cache lifetime* -- how
# long the resolved principal may be reused -- and not a statement about the
# token's validity, matching what the opaque and user-info-service decoders
# return for tokens whose expiry they likewise cannot judge.

import logging
import time

import jwt

from notification_common.config.auth_settings import auth_settings
from notification_common.errors.errors import AuthenticationError

logger = logging.getLogger(__name__)

_TOKEN_EXP_SECONDS = 300


async def get_user_id(token: str) -> tuple[str, float] | None:
    decoded_token = _decode_token(token)
    principal_id = decoded_token.get(auth_settings.principal_id_claim)
    exp = decoded_token.get("exp")

    # `exp` stays required: a token without one is not accepted here, so the
    # set of tokens this mode admits is unchanged. Its value is not returned,
    # because this mode does not enforce expiry.
    if principal_id is None or exp is None:
        return None

    return principal_id, time.time() + _TOKEN_EXP_SECONDS


def _decode_token(token: str) -> dict:
    try:
        # No signature, audience, or expiry verification. Disabling signature
        # verification in PyJWT also disables audience and expiry checks by
        # default; this mode intentionally accepts expired tokens because the
        # upstream component is the trust anchor.
        return jwt.decode(token, options={"verify_signature": False})

    except Exception as e:
        raise AuthenticationError("Invalid or expired JWT") from e
