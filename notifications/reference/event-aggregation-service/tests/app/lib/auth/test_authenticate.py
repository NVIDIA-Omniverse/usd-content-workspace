# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Unit tests for the explicit per-request authentication step.

``authenticate`` replaced ``get_principal_or_undetermined``, whose name said
"stamp a principal" while its only other effect -- resolving, and so
validating, the caller's token -- was an unnamed side effect that the entry
points invoked incidentally. These pin the whole contract of the named step:
the resolved principal comes back, an absent principal (authentication
disabled) becomes ``UNDETERMINED``, and a refusal from the resolver is the
caller's error rather than something the helper absorbs.
"""

import pytest
from notification_common.auth import principal_id
from notification_common.errors.errors import AuthenticationError

from app.lib.auth.authenticate import UNDETERMINED, authenticate


@pytest.fixture
def resolve_principal(mocker):
    """
    Replace the principal resolution ``authenticate`` delegates to.

    Patched on ``notification_common.auth.principal_id`` -- where it lives --
    rather than on the module under test, so the assertion survives the import
    in ``app/lib/auth/authenticate.py`` being spelled differently.
    """

    def _patch(**kwargs):
        return mocker.patch.object(
            principal_id, "principal_id", autospec=True, **kwargs
        )

    return _patch


@pytest.mark.asyncio
class TestAuthenticate:
    async def test_returns_the_resolved_principal(self, resolve_principal):
        resolve_principal(return_value="a-principal")

        assert await authenticate() == "a-principal"

    async def test_awaits_the_resolver(self, resolve_principal):
        # assert_awaited, not assert_called: an un-awaited coroutine would
        # resolve nothing while still recording the call, and resolving is
        # what exercises the configured principal source.
        resolve = resolve_principal(return_value="a-principal")

        await authenticate()

        resolve.assert_awaited_once_with()

    async def test_returns_undetermined_when_the_resolver_returns_none(
        self, resolve_principal
    ):
        # The resolver answers None only when authentication is disabled, so
        # there is no token and no principal to stamp.
        resolve_principal(return_value=None)

        assert await authenticate() == UNDETERMINED

    async def test_does_not_swallow_an_authentication_error(self, resolve_principal):
        # A malformed or unresolvable token raises out of the resolution.
        # Absorbing it here would let an unauthenticated request publish.
        resolve_principal(side_effect=AuthenticationError("Invalid or expired JWT"))

        with pytest.raises(AuthenticationError, match="Invalid or expired JWT"):
            await authenticate()
