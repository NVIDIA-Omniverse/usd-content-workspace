# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest

from notification_common.auth.auth_context import auth_token_var
from notification_common.config.common_settings import common_settings
from notification_common.errors.errors import AuthenticationError
from notification_common.permissions import permissions_client
from notification_common.permissions.models import (
    AuthorizationRequest,
    Action,
    AuthorizationResponse,
    AuthorizationDecision,
    PermissionAuthenticationError,
)


@pytest.fixture(autouse=True)
def setup_tests(test_token):
    permissions_client._cached_can.cache_clear()
    permissions_client._cached_can._LRUCacheWrapper__first_loop = None
    reset_token = auth_token_var.set(test_token)

    yield

    auth_token_var.reset(reset_token)


@pytest.fixture
def mock_authorize(mocker):
    return mocker.patch.object(permissions_client, "authorize")


@pytest.mark.asyncio
class TestCan:
    async def test_returns_true_if_authorization_disabled(self, mock_authorize):
        """Test that permissions check is skipped when authorization is disabled."""
        common_settings.permissions_enabled = False

        assert await permissions_client.can("test-action") is True
        mock_authorize.assert_not_called()

    async def test_endpoint_not_set_does_not_short_circuit(self, mock_authorize):
        """Gating is keyed on permissions_enabled, not on the endpoint
        being set, so an unset endpoint must not skip the check, and an
        ALLOW decision must surface as True."""
        common_settings.permissions_enabled = True
        common_settings.permissions_endpoint = None
        mock_authorize.return_value = AuthorizationResponse(
            decision=AuthorizationDecision.ALLOW
        )

        assert await permissions_client.can("test-action") is True
        mock_authorize.assert_called_once()

    async def test_deny_with_endpoint_not_set_returns_false(self, mock_authorize):
        """With no endpoint configured but permissions enabled, a DENY
        decision must surface as False (no endpoint short-circuit)."""
        common_settings.permissions_enabled = True
        common_settings.permissions_endpoint = None
        mock_authorize.return_value = AuthorizationResponse(
            decision=AuthorizationDecision.DENY
        )

        # Use a distinct token to avoid colliding with any cached ALLOW result.
        deny_token = auth_token_var.set("deny-no-endpoint-token")
        try:
            assert await permissions_client.can("test-action") is False
            mock_authorize.assert_called_once()
        finally:
            auth_token_var.reset(deny_token)

    async def test_makes_call_with_http_client(self, test_token, mock_authorize):
        await permissions_client.can("test-action")

        mock_authorize.assert_called_once_with(
            AuthorizationRequest(
                action=Action(name="test-action", service="notification-common"),
                token=test_token,
            )
        )

    async def test_returns_true_when_result_is_allow(self, mock_authorize):
        mock_authorize.return_value = AuthorizationResponse(
            decision=AuthorizationDecision.ALLOW
        )

        assert await permissions_client.can("test-action") is True

    async def test_returns_raises_authentication_error_with_bad_auth(
        self, mock_authorize
    ):
        mock_authorize.side_effect = PermissionAuthenticationError("error")

        with pytest.raises(AuthenticationError):
            await permissions_client.can("test-action")

    async def test_returns_cached_results(self, mock_authorize):
        mock_authorize.return_value = AuthorizationResponse(
            decision=AuthorizationDecision.ALLOW
        )
        await permissions_client.can("test-action")

        # do it again without sleeping and the result should be the same
        mock_authorize.return_value = AuthorizationResponse(
            decision=AuthorizationDecision.DENY
        )
        assert await permissions_client.can("test-action") is True

    async def test_different_tokens_get_separate_cache_entries(self, mock_authorize):
        """
        The cache must be keyed per-token so that
        different users get independent authorization results.
        """
        mock_authorize.return_value = AuthorizationResponse(
            decision=AuthorizationDecision.ALLOW
        )
        assert await permissions_client.can("test-action") is True

        # Switch to a different user's token
        other_token = auth_token_var.set("other-user-token")
        mock_authorize.return_value = AuthorizationResponse(
            decision=AuthorizationDecision.DENY
        )

        try:
            result = await permissions_client.can("test-action")
            assert result is False
            assert mock_authorize.call_count == 2
        finally:
            auth_token_var.reset(other_token)
