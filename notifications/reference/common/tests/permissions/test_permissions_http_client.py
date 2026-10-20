# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import AsyncMock, MagicMock

import aiohttp
import pytest

from notification_common.config.common_settings import common_settings
from notification_common.permissions import permissions_http_client
from notification_common.permissions.models import (
    Action,
    AuthorizationBatch,
    AuthorizationBatchCondition,
    AuthorizationBatchRequest,
    AuthorizationDecision,
    AuthorizationRequest,
    PermissionInvocationError,
    PermissionResponseError,
    PermissionAuthenticationError,
    Principal,
    Resource,
    PermissionAuthorizationError,
)
from notification_common.http_client.http_client import http_client
from notification_common.permissions.permissions_http_client import (
    authorize,
    batch_authorize,
)


@pytest.fixture(autouse=True)
def mock_logger(mocker):
    mocker.patch.object(permissions_http_client, "logger")


@pytest.fixture
def mock_resource() -> Resource:
    return Resource(
        id="Astronaut.usd",
        type="File",
    )


@pytest.fixture
def mock_request(mock_resource: Resource) -> AuthorizationRequest:
    return AuthorizationRequest(
        principal=Principal(sub="user1", info=None),
        action=Action(service="tags", name="get"),
        resource=mock_resource,
        context={"time": "2024-01-01T00:00:00Z"},
        token="test-token",
    )


@pytest.fixture
def mock_response():
    response = AsyncMock(spec=aiohttp.ClientResponse)
    response.raise_for_status = MagicMock()
    response.json = AsyncMock(return_value={"decision": "allow"})
    return response


@pytest.fixture
def mock_session(mock_response):
    session = AsyncMock(spec=aiohttp.ClientSession)
    request = AsyncMock()
    request.__aenter__.return_value = mock_response
    request.__aexit__.return_value = None
    # noinspection PyTestUnpassedFixture
    session.request.return_value = request
    session.close = AsyncMock()
    session.closed = False
    return session


@pytest.fixture(autouse=True)
def mock_client_session(mocker, mock_session: AsyncMock):
    mocker.patch.object(
        http_client,
        "session",
        new=mock_session,
    )
    mocker.patch.object(
        common_settings,
        "permissions_endpoint",
        "test-permissions-endpoint",
    )
    yield


@pytest.mark.asyncio
class TestAuthorize:
    async def test_calls_request_with_correct_args(
        self,
        mock_session: AsyncMock,
        mock_response: AsyncMock,
        mock_request: AuthorizationRequest,
    ):
        mock_response.json.return_value = {
            "decision": "allow",
            "reason": "Explicit allow",
        }

        await authorize(mock_request)

        mock_session.request.assert_called_once_with(
            "POST",
            "test-permissions-endpoint/v1beta/authorization/",
            json=mock_request.model_dump(exclude_none=True, exclude={"token"}),
            headers=({"Authorization": "Bearer test-token"}),
        )

    @pytest.mark.parametrize(
        "response_data,expected_decision,expected_reason",
        [
            (
                {"decision": "allow", "reason": "Explicit allow"},
                AuthorizationDecision.ALLOW,
                "Explicit allow",
            ),
            (
                {"decision": "deny", "reason": "Explicit deny"},
                AuthorizationDecision.DENY,
                "Explicit deny",
            ),
        ],
    )
    async def test_authorization_response_returns_decision(
        self,
        mock_response: AsyncMock,
        mock_request: AuthorizationRequest,
        response_data: dict,
        expected_decision: AuthorizationDecision,
        expected_reason: str,
    ):
        mock_response.json.return_value = response_data

        result = await authorize(mock_request)

        assert result.decision == expected_decision
        assert result.reason == expected_reason

    @pytest.mark.parametrize(
        "error_code, error_type",
        [
            (401, PermissionAuthenticationError),
            (403, PermissionAuthorizationError),
        ],
    )
    async def test_raises_auth_error(
        self,
        error_code: int,
        error_type,
        mock_session: AsyncMock,
        mock_request: AuthorizationRequest,
    ):
        error_response = AsyncMock(spec=aiohttp.ClientResponse)
        # noinspection PyTypeChecker
        error_response.raise_for_status.side_effect = aiohttp.ClientResponseError(
            request_info=MagicMock(),
            history=(),
            status=error_code,
        )

        request = AsyncMock()
        request.__aenter__.return_value = error_response
        request.__aexit__.return_value = None
        mock_session.request.return_value = request

        with pytest.raises(error_type):
            await authorize(mock_request)

    @pytest.mark.parametrize(
        "response_data,expected_error_message",
        [
            (["not a dict"], "Input should be a valid dict"),
            ({"reason": "some reason"}, "Field required"),
            ({"decision": "invalid"}, "Input should be 'deny', 'allow' or 'skip'"),
        ],
        ids=["invalid_type", "missing_decision", "invalid_decision"],
    )
    async def test_raises_errors_with_correct_messages(
        self,
        mock_response: AsyncMock,
        mock_request: AuthorizationRequest,
        response_data: dict | list,
        expected_error_message: str,
    ):
        mock_response.json.return_value = response_data

        with pytest.raises(PermissionResponseError) as exc_info:
            await authorize(mock_request)
        assert expected_error_message in str(exc_info.value)


@pytest.mark.asyncio
class TestBatchAuthorization:
    @pytest.fixture
    def mock_batch_request(self, mock_resource: Resource) -> AuthorizationBatchRequest:
        actions = [
            Action(service="storage", name="read"),
            Action(service="storage", name="write"),
            Action(service="storage", name="delete"),
        ]
        return AuthorizationBatchRequest(
            token="test-token",
            batches=[
                AuthorizationBatch(
                    principal=Principal(sub="user1", info=None),
                    actions=actions,
                    resource=mock_resource,
                    context=None,
                )
            ],
            condition=AuthorizationBatchCondition.AND,
        )

    async def test_returns_all_authorization_decisions(
        self,
        mock_response: AsyncMock,
        mock_batch_request: AuthorizationBatchRequest,
    ):
        mock_response.json.return_value = {
            "decisions": [
                {
                    "test:read": {"decision": "allow"},
                    "test:write": {
                        "decision": "deny",
                        "reason": "Not allowed",
                    },
                    "test:delete": {"decision": "allow"},
                }
            ],
            "summary": {"decision": "deny", "reason": "At least one action was denied"},
        }

        result = await batch_authorize(mock_batch_request)

        assert result.summary is not None
        assert result.summary.decision == AuthorizationDecision.DENY
        assert result.summary.reason == "At least one action was denied"

        assert len(result.decisions) == 1
        decisions = result.decisions[0]
        assert decisions["test:read"].decision == AuthorizationDecision.ALLOW
        assert decisions["test:write"].decision == AuthorizationDecision.DENY
        assert decisions["test:delete"].decision == AuthorizationDecision.ALLOW

    async def test_calls_request_with_correct_args(
        self,
        mock_session: AsyncMock,
        mock_response: AsyncMock,
        mock_batch_request: AuthorizationBatchRequest,
    ):
        mock_response.json.return_value = {
            "decisions": [
                {
                    "test:read": {"decision": "allow"},
                    "test:write": {
                        "decision": "deny",
                        "reason": "Not allowed",
                    },
                    "test:delete": {"decision": "allow"},
                }
            ],
            "summary": {"decision": "deny", "reason": "At least one action was denied"},
        }

        await batch_authorize(mock_batch_request)

        mock_session.request.assert_called_once_with(
            "POST",
            "test-permissions-endpoint/v1beta/authorization/batch/",
            json=mock_batch_request.model_dump(
                exclude_none=True, exclude={"token", "condition"}
            )
            | {"condition": "and"},
            headers=({"Authorization": "Bearer test-token"}),
        )

    @pytest.mark.parametrize(
        "response_data,expected_error_message",
        [
            # Invalid response type
            (["not a dict"], "Input should be a valid dict"),
            # Missing required fields
            ({"summary": {"decision": "deny"}}, "Field required"),
            # Invalid decisions type
            (
                {
                    "decisions": "not a list",
                    "summary": {"decision": "deny"},
                },
                "Input should be a valid list",
            ),
            # Invalid summary type
            (
                {
                    "decisions": [],
                    "summary": "not a dict",
                },
                "Input should be a valid dict",
            ),
            # Invalid decision in summary
            (
                {
                    "decisions": [],
                    "summary": {"decision": "invalid"},
                },
                "Input should be 'deny', 'allow' or 'skip'",
            ),
            # Invalid decision in decisions
            (
                {
                    "decisions": [{"action": {"decision": "invalid"}}],
                },
                "Input should be 'deny', 'allow' or 'skip'",
            ),
        ],
        ids=[
            "invalid_type",
            "missing_decisions_field",
            "invalid_decisions_type",
            "invalid_summary_type",
            "invalid_summary_decision",
            "invalid_decision_in_decisions",
        ],
    )
    async def test_raises_errors_with_correct_messages(
        self,
        mock_response: AsyncMock,
        mock_batch_request: AuthorizationBatchRequest,
        response_data: dict | list,
        expected_error_message: str,
        mock_client_session,
    ):
        mock_response.json.return_value = response_data

        with pytest.raises(PermissionResponseError) as exc_info:
            await batch_authorize(mock_batch_request)
        assert expected_error_message in str(exc_info.value)


@pytest.mark.asyncio
class TestRetryBehavior:
    @pytest.fixture
    def error_request(self, error_code: int) -> AsyncMock:
        error_response = AsyncMock(spec=aiohttp.ClientResponse)
        error_response.status = error_code
        # noinspection PyTypeChecker
        error_response.raise_for_status.side_effect = aiohttp.ClientResponseError(
            request_info=MagicMock(),
            history=(),
            status=error_code,
        )

        request = AsyncMock()
        request.__aenter__.return_value = error_response
        request.__aexit__.return_value = None
        return request

    @pytest.fixture
    def success_request(self, mock_response: AsyncMock) -> AsyncMock:
        request = AsyncMock()
        request.__aenter__.return_value = mock_response
        request.__aexit__.return_value = None
        return request

    @pytest.mark.parametrize(
        "error_code, expected_retries,expect_success",
        [
            # 5xx error - retries and succeeds
            (503, 2, True),
            # 4xx error - no retry, raises error
            (400, 1, False),
        ],
        ids=["server_error_retry", "client_error_no_retry"],
    )
    async def test_retry_attempts(
        self,
        error_code,
        mock_session: AsyncMock,
        mock_request: AuthorizationRequest,
        expected_retries: int,
        expect_success: bool,
        error_request: AsyncMock,
        success_request: AsyncMock,
        mock_client_session,
    ):
        mock_session.request.side_effect = [error_request, success_request]

        if expect_success:
            result = await authorize(mock_request)
            assert result.decision == AuthorizationDecision.ALLOW
        else:
            with pytest.raises(PermissionInvocationError):
                await authorize(mock_request)

        assert mock_session.request.call_count == expected_retries
