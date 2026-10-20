# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
import aiohttp
from unittest.mock import AsyncMock, MagicMock

from notification_common.auth.auth_tokens import opaque_token_decoder

# noinspection PyProtectedMember
from notification_common.auth.auth_tokens.opaque_token_decoder import (
    _fetch_userinfo_with_retry,
    get_user_id,
)
from notification_common.config.auth_settings import auth_settings
from notification_common.errors.errors import AuthenticationError
from notification_common.http_client.http_client import http_client


@pytest.fixture(autouse=True)
def mock_logger(mocker):
    mocked_logger = mocker.patch.object(opaque_token_decoder, "logger")
    return mocked_logger


@pytest.fixture
def userinfo_endpoint() -> str:
    return "https://fake-userinfo.com/userinfo"


@pytest.fixture
def mock_userinfo_response_data(test_user_id: str) -> dict:
    return {"sub": test_user_id, "name": "Test User"}


@pytest.mark.asyncio
class TestGetUserId:

    @pytest.fixture
    def mock_fetch_userinfo(self, mocker):
        return mocker.patch.object(
            opaque_token_decoder,
            "_fetch_userinfo_with_retry",
            autospec=True,
        )

    async def test_success_returns_user_id(
        self,
        mock_fetch_userinfo: AsyncMock,
        test_token: str,
        mock_userinfo_response_data: dict,
    ):
        mock_fetch_userinfo.return_value = mock_userinfo_response_data

        user_id_result = await get_user_id(test_token)

        assert user_id_result is not None
        assert user_id_result[0] == mock_userinfo_response_data["sub"]

    async def test_returns_configured_claim_when_principal_id_claim_overridden(
        self,
        mocker,
        mock_fetch_userinfo: AsyncMock,
        test_token: str,
        test_user_id: str,
    ):
        mocker.patch.object(auth_settings, "principal_id_claim", "oid")
        expected_oid = test_user_id
        mock_fetch_userinfo.return_value = {
            "oid": expected_oid,
            "sub": "should-not-be-used",
            "name": "Test User",
        }

        user_id_result = await get_user_id(test_token)

        assert user_id_result is not None
        assert user_id_result[0] == expected_oid

    async def test_default_claim_still_returns_sub(
        self,
        mock_fetch_userinfo: AsyncMock,
        test_token: str,
        mock_userinfo_response_data: dict,
    ):
        assert auth_settings.principal_id_claim == "sub"
        mock_fetch_userinfo.return_value = mock_userinfo_response_data

        user_id_result = await get_user_id(test_token)

        assert user_id_result is not None
        assert user_id_result[0] == mock_userinfo_response_data["sub"]

    async def test_fetch_userinfo_called_with_correct_parameters(
        self,
        mock_fetch_userinfo: AsyncMock,
        test_token: str,
        userinfo_endpoint: str,
    ):
        mock_fetch_userinfo.return_value = {"sub": "some-user-id"}

        await get_user_id(test_token)

        mock_fetch_userinfo.assert_awaited_once_with(test_token)

    async def test_missing_sub_claim_returns_none(
        self,
        mock_fetch_userinfo: AsyncMock,
        test_token: str,
    ):
        mock_fetch_userinfo.return_value = {}

        user_id_result = await get_user_id(test_token)

        assert user_id_result is None

    async def test_http_error_raises_exception_and_logs(
        self,
        mock_fetch_userinfo: AsyncMock,
        mock_logger,
        test_token: str,
    ):
        error_status = 500
        log_message = f"HTTP status error {error_status}"
        # noinspection PyTypeChecker
        error = aiohttp.ClientResponseError(
            request_info=MagicMock(),
            history=(),
            status=error_status,
            message=f"{error_status} Error",
        )
        mock_fetch_userinfo.side_effect = error

        with pytest.raises(aiohttp.ClientResponseError):
            await get_user_id(test_token)

        assert any(
            log_message in call.args[0] for call in mock_logger.exception.call_args_list
        ), f"Expected log message containing '{log_message}'"

    async def test_401_http_error_raises_authentication_error(
        self,
        mock_fetch_userinfo: AsyncMock,
        test_token: str,
    ):
        error_status = 401
        # noinspection PyTypeChecker
        error = aiohttp.ClientResponseError(
            request_info=MagicMock(),
            history=(),
            status=error_status,
            message=f"{error_status} Error",
        )
        mock_fetch_userinfo.side_effect = error

        with pytest.raises(
            AuthenticationError,
            match="Unauthorized: The token is invalid or has expired",
        ):
            await get_user_id(test_token)

    async def test_runtime_error_if_not_connected(self, mock_fetch_userinfo: AsyncMock):
        expected_error = ValueError("HTTP client is not initialized")
        mock_fetch_userinfo.side_effect = expected_error

        with pytest.raises(ValueError, match="HTTP client is not initialized"):
            await get_user_id("some-user-id")


@pytest.mark.asyncio
class TestFetchUserinfoWithRetry:

    @pytest.fixture
    def mock_response(self, mock_userinfo_response_data: dict) -> AsyncMock:
        response = AsyncMock(spec=aiohttp.ClientResponse)
        response.status = 200
        response.raise_for_status = MagicMock()
        response.json = AsyncMock(return_value=mock_userinfo_response_data)
        return response

    @pytest.fixture
    def mock_session(self, mock_response: AsyncMock) -> AsyncMock:
        session = AsyncMock(spec=aiohttp.ClientSession)
        get_request_context = AsyncMock()
        get_request_context.__aenter__.return_value = mock_response
        get_request_context.__aexit__.return_value = None
        session.get.return_value = get_request_context

        session.close = AsyncMock()
        session.closed = False
        return session

    @pytest.fixture
    def mock_client_session(self, mocker, mock_session: AsyncMock):
        mocker.patch.object(
            http_client,
            "session",
            new=mock_session,
        )
        yield

    @pytest.fixture(autouse=True)
    def mock_async_sleep(self, mocker):
        """Patches asyncio.sleep to speed up tests involving backoff delays."""
        mock_sleep = mocker.patch("asyncio.sleep", return_value=None)
        return mock_sleep

    @pytest.fixture(autouse=True)
    def _setup_common_test_fixtures(
        self,
        mock_client_session: None,
        mock_session: AsyncMock,
        test_token: str,
        mock_get_effect: AsyncMock,
        mock_userinfo_response_data: dict,
        userinfo_endpoint: str,
        mocker,
    ):
        mocker.patch(
            "notification_common.auth.openid_discovery.get_userinfo_endpoint",
            return_value=userinfo_endpoint,
        )
        self.mock_session = mock_session
        self.token = test_token
        self.mock_get_effect = mock_get_effect
        self.mock_userinfo_response_data = mock_userinfo_response_data
        self.userinfo_endpoint = userinfo_endpoint

    @pytest.fixture
    def mock_get_effect(self, mock_userinfo_response_data: dict) -> AsyncMock:
        response = AsyncMock(spec=aiohttp.ClientResponse)
        response.status = 200
        response.json = AsyncMock(return_value=mock_userinfo_response_data)
        response.raise_for_status = MagicMock()  # Does not raise

        request_context = AsyncMock()
        request_context.__aenter__.return_value = response
        request_context.__aexit__.return_value = None
        return request_context

    @staticmethod
    def _create_http_error_effect(status_code: int, message: str) -> AsyncMock:
        """Helper to create a mock effect for an HTTP error response."""
        response_mock = AsyncMock(spec=aiohttp.ClientResponse)
        response_mock.status = status_code
        response_mock.message = message
        # noinspection PyTypeChecker
        response_mock.raise_for_status.side_effect = aiohttp.ClientResponseError(
            request_info=MagicMock(),
            history=(),
            status=status_code,
            message=message,
        )
        response_mock.json = AsyncMock(return_value={})  # Should not be called if error

        request_context_mock = AsyncMock()
        request_context_mock.__aenter__.return_value = response_mock
        request_context_mock.__aexit__.return_value = None
        return request_context_mock

    # noinspection PyMethodMayBeStatic
    def _create_client_error_effect(
        self, error_type: type[aiohttp.ClientError], message: str
    ) -> AsyncMock:
        """Helper to create a mock effect for a client-side network error."""
        request_context_mock = AsyncMock()
        request_context_mock.__aenter__.side_effect = error_type(message)
        request_context_mock.__aexit__.return_value = None
        return request_context_mock

    def _prepare_side_effects(self, side_effects_config: list):
        """Prepares and sets the side effects on the mock_session.get method."""
        actual_side_effects = []
        for config_item in side_effects_config:
            if config_item["type"] == "success":
                actual_side_effects.append(self.mock_get_effect)
            elif config_item["type"] == "http_error":
                actual_side_effects.append(
                    self._create_http_error_effect(
                        status_code=config_item["status"],
                        message=config_item.get("message", "HTTP Error"),
                    )
                )
            elif config_item["type"] == "client_error":
                actual_side_effects.append(
                    self._create_client_error_effect(
                        error_type=config_item["error_type"],
                        message=config_item.get("message", "Client Error"),
                    )
                )
        self.mock_session.get.side_effect = actual_side_effects
        self.mock_session.get.call_count = 0

    async def _assert_successful_run(self, expected_calls: int):
        """Asserts a scenario expected to succeed after retries."""
        userinfo_result = await _fetch_userinfo_with_retry(self.token)
        assert userinfo_result == self.mock_userinfo_response_data
        assert self.mock_session.get.call_count == expected_calls
        self.mock_session.get.assert_called_with(
            self.userinfo_endpoint, headers={"Authorization": f"Bearer {self.token}"}
        )

    async def _assert_failed_run(
        self,
        expected_calls: int,
        expected_exception: type[Exception],
        side_effects_config: list,
    ):
        """Asserts a scenario expected to fail with a specific exception."""
        with pytest.raises(expected_exception) as exc_info:
            await _fetch_userinfo_with_retry(self.token)
            if isinstance(exc_info.value, aiohttp.ClientResponseError):
                # noinspection PyUnresolvedReferences
                assert exc_info.value.status == side_effects_config[-1]["status"]
        assert self.mock_session.get.call_count == expected_calls

    async def test_success_on_first_attempt(self):
        side_effects_config = [{"type": "success"}]
        self._prepare_side_effects(side_effects_config)
        await self._assert_successful_run(expected_calls=1)

    async def test_server_error_503_retries_and_succeeds(self):
        side_effects_config = [
            {"type": "http_error", "status": 503},
            {"type": "http_error", "status": 503},
            {"type": "success"},
        ]
        self._prepare_side_effects(side_effects_config)
        await self._assert_successful_run(expected_calls=3)

    async def test_client_error_401_fails_no_retry(self):
        side_effects_config = [{"type": "http_error", "status": 401}]
        self._prepare_side_effects(side_effects_config)
        await self._assert_failed_run(
            expected_calls=1,
            expected_exception=aiohttp.ClientResponseError,
            side_effects_config=side_effects_config,
        )

    async def test_client_error_403_fails_no_retry(self):
        side_effects_config = [{"type": "http_error", "status": 403}]
        self._prepare_side_effects(side_effects_config)
        await self._assert_failed_run(
            expected_calls=1,
            expected_exception=aiohttp.ClientResponseError,
            side_effects_config=side_effects_config,
        )

    async def test_network_error_retries_and_succeeds(self):
        side_effects_config = [
            {"type": "client_error", "error_type": aiohttp.ClientConnectionError},
            {"type": "success"},
        ]
        self._prepare_side_effects(side_effects_config)
        await self._assert_successful_run(expected_calls=2)

    async def test_server_error_500_fails_after_max_retries(self):
        side_effects_config = [
            {"type": "http_error", "status": 500},
            {"type": "http_error", "status": 500},
            {"type": "http_error", "status": 500},
            {"type": "http_error", "status": 500},
        ]
        self._prepare_side_effects(side_effects_config)
        await self._assert_failed_run(
            expected_calls=4,
            expected_exception=aiohttp.ClientResponseError,
            side_effects_config=side_effects_config,
        )

    async def test_network_error_fails_after_max_retries(self):
        side_effects_config = [
            {"type": "client_error", "error_type": aiohttp.ClientOSError},
            {"type": "client_error", "error_type": aiohttp.ClientOSError},
            {"type": "client_error", "error_type": aiohttp.ClientOSError},
            {"type": "client_error", "error_type": aiohttp.ClientOSError},
        ]
        self._prepare_side_effects(side_effects_config)
        await self._assert_failed_run(
            expected_calls=4,
            expected_exception=aiohttp.ClientOSError,
            side_effects_config=side_effects_config,
        )
