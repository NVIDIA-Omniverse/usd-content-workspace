# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Explicit per-request authentication for the service's API entry points.

The authentication guarantee is at the transport: the shared gRPC
``AuthInterceptor`` and the shared REST auth middleware resolve the bearer
token through the configured ``principal_source`` before any handler runs, and
refuse a token that does not resolve with UNAUTHENTICATED (gRPC) or 401
(REST). That resolution verifies the token under ``jwt``, ``opaque`` and
``userInfoService``; under ``jwtUnverified`` it decodes the payload without
verifying it and deliberately accepts an expired token, leaving validity to
the upstream validator that fronts the service.

:func:`authenticate` is retained behind that as belt and braces. The house
rule is to validate auth at every API entry point and not only at the edge
(the source repository's internal engineering guide, which is not
published), so each authenticated entry point
names the step and calls it: it is the one check that would still refuse a
request if a future edit to the REST ``AUTH_EXCLUDED_PATHS`` or the
interceptor's excluded methods accidentally exempted a real route. That same
transport-level resolution is also what seeds the per-request cache a caller
reads when it needs the principal as a value, via
``principal_id.principal_id()``.

Resolution is cached per request context, so the resolutions a request makes
cost one decoder invocation between them, however many callers resolve.
"""

from notification_common.auth import principal_id
from notification_common.config.auth_settings import auth_settings


async def authenticate() -> None:
    """
    Re-check the request's bearer token, raising if it is not usable.

    This docstring is also the single statement of what *any* later
    in-request ``principal_id.principal_id()`` call does, which the
    in-service callers that resolve the principal as a value cite rather than
    restate.

    The transport layer has already resolved the token by the time an entry
    point calls this (see the module docstring), so this call and every later
    one read the per-request cache. A cached read is not unconditionally
    non-raising: it raises ``AuthenticationError("Token expired")`` -- instead
    of re-resolving -- whenever the stored ``expires_at`` has passed, and it
    does so regardless of ``principal_source``. What differs by source is what
    ``expires_at`` holds, and so what the raise reports:

    - under ``jwt`` it is the token's own ``exp``, so what is reported is a
      genuine token lapse;
    - under ``jwtUnverified``, ``opaque`` and ``userInfoService`` the token's
      own expiry is not judged at all, and the stored value is instead a
      derived cache lifetime -- 300 seconds, ``_TOKEN_EXP_SECONDS``, declared
      separately in each of notification_common's three non-``jwt`` decoders
      -- so what is reported is the cache entry going stale. That only
      happens when the work between the transport's resolution and the call
      outlasts that lifetime.

    The resolved principal itself is not needed by any caller here, so
    nothing is returned.

    When authentication is disabled there is no token to validate and this
    does nothing.
    """
    if not auth_settings.authentication_enabled():
        return

    await principal_id.principal_id()
