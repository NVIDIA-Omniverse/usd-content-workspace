# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import AsyncMock, MagicMock

import pytest

from notification_common.auth import openid_discovery
from notification_common.config.auth_settings import auth_settings


@pytest.fixture
def mock_discovery_endpoint_url() -> str:
    return "http://fake-auth.com/.well-known/openid-configuration"


@pytest.fixture(autouse=True)
def mock_settings(mocker, mock_discovery_endpoint_url: str):
    mocker.patch.object(
        auth_settings,
        "openid_configuration_uri",
        mock_discovery_endpoint_url,
    )


@pytest.fixture
def mock_generic_http_response():
    response = MagicMock()
    response.json = AsyncMock()
    response.status = 200
    return response


@pytest.fixture
def mock_http_client_session(mocker, mock_generic_http_response):
    session_mock = MagicMock()
    session_mock.get = AsyncMock(return_value=mock_generic_http_response)

    mocker.patch.object(openid_discovery.http_client, "session", new=session_mock)
    return session_mock


@pytest.fixture(autouse=True)
def clear_openid_cache():
    openid_discovery._openid_discovery.cache_clear()
    openid_discovery._openid_discovery._LRUCacheWrapper__first_loop = None


@pytest.fixture
def userinfo_endpoint():
    return "http://example.com/userinfo"


@pytest.mark.asyncio
class TestOpenIDDiscovery:
    async def test_returns_discovery_result(
        self,
        mock_http_client_session,
        mock_generic_http_response,
        jwks_uri,
        userinfo_endpoint,
    ):
        expected_json = {
            "jwks_uri": jwks_uri,
            "userinfo_endpoint": userinfo_endpoint,
        }
        mock_generic_http_response.json.return_value = expected_json

        result = await openid_discovery._openid_discovery()

        assert result == expected_json

    async def test_get_called_with_expected_endpoint(
        self,
        mock_http_client_session,
        mock_generic_http_response,
        mock_discovery_endpoint_url,
    ):
        mock_generic_http_response.json.return_value = {
            "jwks_uri": "test-jwks-uri",
        }

        await openid_discovery._openid_discovery()

        mock_http_client_session.get.assert_awaited_once_with(
            mock_discovery_endpoint_url
        )

    async def test_raises_error_if_http_client_not_initialized(self, mocker):
        mocker.patch.object(openid_discovery.http_client, "session", new=None)
        with pytest.raises(Exception, match="HTTP client is not initialized"):
            await openid_discovery._openid_discovery()

    async def test_raises_error_if_empty_json_response(
        self,
        mock_http_client_session,
        mock_generic_http_response,
    ):
        mock_generic_http_response.json.return_value = None

        with pytest.raises(
            Exception, match="No data returned from openid discovery endpoint"
        ):
            await openid_discovery._openid_discovery()


@pytest.mark.asyncio
class TestGetJwksUri:
    async def test_returns_uri_from_discovery_result(self, mocker, jwks_uri):
        mock_discovery_result = {
            "jwks_uri": jwks_uri,
            "other_key": "value",
        }
        mocker.patch.object(
            openid_discovery,
            "_openid_discovery",
            return_value=mock_discovery_result,
        )

        uri = await openid_discovery.get_jwks_uri()
        assert uri == jwks_uri

    async def test_returns_none_if_missing_key(self, mocker):
        mock_discovery_result = {"other_key": "value"}
        mocker.patch.object(
            openid_discovery,
            "_openid_discovery",
            return_value=mock_discovery_result,
        )

        uri = await openid_discovery.get_jwks_uri()
        assert uri is None

    async def test_raises_error_if_jwks_discovery_fails(self, mocker):
        mocker.patch.object(
            openid_discovery,
            "_openid_discovery",
            side_effect=Exception("Discovery failed"),
        )
        with pytest.raises(Exception, match="Discovery failed"):
            await openid_discovery.get_jwks_uri()


@pytest.mark.asyncio
class TestGetUserinfoEndpoint:
    async def test_returns_endpoint_from_discovery_result(
        self, mocker, userinfo_endpoint
    ):
        mock_discovery_result = {
            "userinfo_endpoint": userinfo_endpoint,
            "other_key": "value",
        }
        mocker.patch.object(
            openid_discovery,
            "_openid_discovery",
            return_value=mock_discovery_result,
        )

        endpoint = await openid_discovery.get_userinfo_endpoint()
        assert endpoint == userinfo_endpoint

    async def test_returns_none_if_missing_key(self, mocker):
        mock_discovery_result = {"other_key": "value"}
        mocker.patch.object(
            openid_discovery,
            "_openid_discovery",
            return_value=mock_discovery_result,
        )

        endpoint = await openid_discovery.get_userinfo_endpoint()
        assert endpoint is None

    async def test_raises_error_if_endpoint_discovery_fails(self, mocker):
        mocker.patch.object(
            openid_discovery,
            "_openid_discovery",
            side_effect=Exception("Discovery failed"),
        )
        with pytest.raises(Exception, match="Discovery failed"):
            await openid_discovery.get_userinfo_endpoint()
