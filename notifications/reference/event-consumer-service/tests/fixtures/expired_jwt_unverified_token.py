# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
A real, already-expired JWT for the ``jwtUnverified`` mode.

``jwtUnverified`` is the opt-in ``principal_source`` (the library's built-in
default, refused without ``allow_unverified_jwt``): the token is validated
by the component in front of the service, and the service
only decodes the payload to read the principal claim. That decode
deliberately does not check ``exp``, so an expired-but-upstream-validated
token is one this mode is supposed to serve -- which is exactly the case
a test needs a genuine token for. A hand-rolled string cannot express it:
the value has to survive ``jwt.decode`` and carry both an ``exp`` and the
configured principal claim, because the decoder refuses a payload missing
either.

The token is signed with a throwaway HMAC secret. Nothing verifies the
signature in this mode, and nothing may: a test that could only pass
against a *validly* signed token would be pinning a guarantee this mode
does not make.
"""

import time

import jwt

#: The principal the token's ``sub`` claim carries. ``sub`` is
#: ``auth_settings.principal_id_claim``'s default, which is what the
#: shipped configuration uses.
EXPIRED_TOKEN_PRINCIPAL = "expired-token-principal"

#: How far in the past the token's own ``exp`` sits. An hour is far enough
#: that no plausible clock skew or slow test run puts it back in the
#: future, which would make an assertion about expired-token handling pass
#: for the wrong reason.
_EXPIRED_BY_SECONDS = 3600

#: The signing key. Its only requirement is to be long enough that PyJWT
#: does not warn about it: nothing in this mode reads the signature, so the
#: value is arbitrary.
_SIGNING_KEY = "test-only-signing-key-nothing-verifies-this-signature"


def make_expired_jwt_unverified_token(principal: str = EXPIRED_TOKEN_PRINCIPAL) -> str:
    """
    Build a JWT whose ``exp`` has already passed.

    :param principal: The value for the ``sub`` claim, i.e. the principal
        the token resolves to.
    :return: The encoded token, ready to present as a bearer credential.
    """
    return jwt.encode(
        {"sub": principal, "exp": int(time.time()) - _EXPIRED_BY_SECONDS},
        _SIGNING_KEY,
        algorithm="HS256",
    )
