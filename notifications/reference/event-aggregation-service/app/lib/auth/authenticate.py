# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Explicit per-request authentication for the service's API entry points.

The gRPC ``AuthInterceptor`` and the REST auth middleware stash the bearer
token on the request context and resolve it to a principal before any handler
runs, refusing a request whose token is unresolvable or whose principal claim
is empty.

The service still names the step and calls it explicitly rather than letting
the principal arrive as a side effect of stamping ``principal_identity`` onto
the published event: relying on a stamping detail to establish the request's
principal is exactly the kind of implicit coupling that breaks silently the
moment the stamping changes. So every authenticated entry point invokes
:func:`authenticate`, and a deployment that runs with authentication on but the
permissions service off still obtains the principal.

What resolution buys depends on the configured ``principal_source``: for
``jwt``, ``opaque`` and ``userInfoService`` it is a verification, while
``jwtUnverified`` is a payload decode that trusts an upstream validator and
deliberately accepts expired tokens.

This call is the single resolution *this service* performs: callers receive
the principal back from it and stamp that value, rather than resolving the
token a second time. It is not the request's first resolution -- the transport
layer above has already resolved the token -- so it is answered from
``notification_common.auth.principal_id``'s per-request cache instead of
decoding again. That is safe because every decoder reports a cache lifetime of
its own rather than the token's expiry, so the cache is not making an
authentication decision. Being cache-answered does not make the call
redundant: that cached principal is where the value stamped onto
``OmniEvent.principal_identity`` comes from. A second in-service resolution
would be answered from the same cache without re-exercising the configured
source, which is why the service must not write one.
"""

from notification_common.auth import principal_id

UNDETERMINED = "undetermined"


async def authenticate() -> str:
    """
    Resolve the request's bearer token to the calling principal.

    Resolving the principal is what exercises the configured principal
    source, so a malformed or unresolvable token is refused rather than
    being discovered later (or never). That refusal happens in the
    interceptor and the middleware, before any handler runs, and this call
    is answered from ``principal_id``'s per-request cache. An *expired*
    token is refused only where the configured source checks expiry --
    under ``jwtUnverified`` it is not, by design, because the upstream
    validator is the trust anchor.

    A request that carries no token at all reaches
    ``auth_token_var.get()`` with nothing set and raises ``LookupError``,
    which surfaces as a 500 rather than a 401. That is unreachable in a
    deployment, because the interceptor and the middleware both refuse a
    request with no ``Authorization`` header before any handler runs.

    :return: The resolved principal. :data:`UNDETERMINED` is returned only
        when the resolver returns ``None``, which it does when authentication
        is disabled and there is no token to resolve.
    """
    principal = await principal_id.principal_id()

    return UNDETERMINED if principal is None else principal
