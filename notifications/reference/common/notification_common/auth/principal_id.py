# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging
import time
from contextvars import ContextVar
from typing import NamedTuple

from notification_common.auth.auth_context import auth_token_var
from notification_common.auth.auth_tokens.token_decoder import decoder
from notification_common.config.auth_settings import auth_settings
from notification_common.errors.errors import AuthenticationError

logger = logging.getLogger(__name__)


class PrincipalContext(NamedTuple):
    principal_id: str
    expires_at: float


_PRINCIPAL_KEY = "principal_id_context_var"

_principal_id_var: ContextVar[PrincipalContext] = ContextVar(_PRINCIPAL_KEY)


async def principal_id() -> str | None:
    """
    Resolve the principal for the bearer token in
    `auth_token_var`, caching it for the life of the
    current request or stream context.

    The auth interceptor and the REST auth middleware
    call this once per unary request, on every
    non-excluded request of either transport. Any
    in-service caller that later needs the principal
    calls it too and is answered from the per-context
    cache. On a long-lived stream it is called once at
    stream setup, where the token we receive is never
    updated for the life of the stream.

    See if we have a principal_id already cached.
    If we don't, then decode the token and cache
    the principal id, then return it.

    We store an expiration time as part of the cached
    value so we know when the cache entry gets stale.
    That value means "when this cache entry goes
    stale", not "when the token stops being valid".

    Only the `jwt` source judges a token's expiry, and
    it does so up front: its decoder refuses an already
    expired token rather than reporting one. The
    `opaque`, `userInfoService` and `jwtUnverified`
    sources cannot or deliberately do not judge expiry,
    so each returns a short derived cache lifetime
    instead.

    A principal claim that is present but empty is
    refused as unresolved: it raises the same
    `AuthenticationError` an absent claim does, so the
    caller sees a 401 (REST) / UNAUTHENTICATED (gRPC).
    An empty claim carries no identity, so accepting it
    would stamp `principal_identity=""` on published
    events and collapse every such caller into a single
    per-principal cache scope downstream. Refusing it
    surfaces a misconfigured `principal_id_claim`
    setting instead of hiding it behind an empty
    identity.

    If authentication is disabled, return None.
    """
    # If authentication is disabled, there's no token to get a principal_id from
    if not auth_settings.authentication_enabled():
        return None

    p_id = _cached_principal_id()

    # `None` is the cache miss, so test for the miss rather than for
    # truthiness. A truthiness test would re-resolve any falsy cached principal
    # on every call, costing a second decode (a JWKS verification, or a retried
    # user-info round trip) per request. An empty principal is refused below
    # before it is ever cached, so no falsy principal reaches the cache today;
    # the explicit `is not None` keeps that hole closed regardless.
    if p_id is not None:
        return p_id

    # the token should already be in a ContextVar
    token = auth_token_var.get()

    resolved = await decoder().get_user_id(token)
    if resolved is None:
        # The token decoded/resolved but carried no principal id (e.g. the
        # configured principal_id_claim or the user-info `id` field was absent).
        # On an authenticated stream this is an authentication failure, not a
        # silent None principal, so hard-fail at call entry rather than letting
        # the tuple unpacking raise a raw TypeError.
        raise AuthenticationError("Token did not resolve to a principal id")

    p_id, expires_at = resolved

    if not p_id:
        # The claim was present but empty (every decoder passes `""` through,
        # refusing only an absent claim). An empty principal is no identity at
        # all, so it is refused exactly as an absent one is -- and refused
        # here, before the cache write, so `""` is never stamped onto an event
        # nor served to a later in-service caller from the cache. The
        # truthiness test also covers a `None` principal carried inside a
        # non-None tuple, which would otherwise cache as an unauthenticated
        # principal indistinguishable from authentication being disabled.
        raise AuthenticationError("Token did not resolve to a principal id")

    _principal_id_var.set(PrincipalContext(p_id, expires_at))

    return p_id


def _cached_principal_id() -> str | None:
    """
    If the principal id is already set in the context,
    then check expiration.
    If it's expired, then raise a PermissionError.
    Otherwise, return it.

    If not found in the cache, then return None.
    """
    try:
        context = _principal_id_var.get()
    except LookupError:
        return None

    if context.expires_at < time.time():
        raise AuthenticationError("Token expired")

    return context.principal_id
