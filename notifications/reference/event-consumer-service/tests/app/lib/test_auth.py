# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Unit tests for the explicit per-request authentication step.

The shared gRPC interceptor and REST middleware resolve the bearer token
through the configured ``principal_source`` before any handler runs, so
the transport is where a request's token is refused. ``authenticate`` is
kept behind that as belt and braces -- the house rule is to validate auth
at every API entry point and not only at the edge, and it is the check
that would still refuse a request if a route were accidentally exempted
from the transport's own list. These pin its whole contract: it resolves
the principal when authentication is on, it does nothing when
authentication is off, and a rejected token surfaces as the caller's
error rather than being swallowed.
"""

import pytest
from notification_common.auth import principal_id
from notification_common.errors.errors import AuthenticationError

from app.lib.auth import authenticate


@pytest.fixture
def resolve_principal(mocker):
    """
    Replace the principal resolution ``authenticate`` delegates to.

    Patched on ``notification_common.auth.principal_id`` -- where it lives
    -- rather than on the module under test, so the assertion survives the
    import in ``app/lib/auth.py`` being spelled differently.
    """

    def _patch(**kwargs):
        return mocker.patch.object(principal_id, "principal_id", **kwargs)

    return _patch


@pytest.mark.asyncio
class TestAuthenticate:
    async def test_resolves_the_principal_when_authentication_is_enabled(
        self, authentication_enabled, resolve_principal
    ):
        authentication_enabled(True)
        resolve = resolve_principal(return_value="a-principal")

        await authenticate()

        resolve.assert_awaited_once_with()

    async def test_does_not_resolve_the_principal_when_authentication_is_disabled(
        self, authentication_enabled, resolve_principal
    ):
        # There is no token to validate on a tokenless deployment, so the
        # step is a no-op rather than a failure.
        authentication_enabled(False)
        resolve = resolve_principal(return_value="a-principal")

        await authenticate()

        resolve.assert_not_called()

    async def test_propagates_an_authentication_error(
        self, authentication_enabled, resolve_principal
    ):
        # A token the resolution refuses -- an unresolvable one, or a cache
        # entry whose stored lifetime has lapsed -- raises out of here.
        # Swallowing it would let an unauthenticated request run the whole
        # RPC.
        authentication_enabled(True)
        resolve_principal(side_effect=AuthenticationError("Token expired"))

        with pytest.raises(AuthenticationError, match="Token expired"):
            await authenticate()
