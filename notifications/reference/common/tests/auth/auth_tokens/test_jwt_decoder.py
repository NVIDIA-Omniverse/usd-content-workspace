# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import AsyncMock, MagicMock

import jwt
import pytest
import pytest_asyncio
from jwt.exceptions import InvalidTokenError

from notification_common.auth import openid_discovery
from notification_common.auth.auth_tokens import jwt_decoder

# noinspection PyProtectedMember
from notification_common.auth.auth_tokens.jwt_decoder import (
    get_user_id,
    _decode_token,
    _jwks_client,
)
from notification_common.config.auth_settings import auth_settings
from notification_common.errors.errors import (
    AuthenticationError,
    ServerError,
)


@pytest.fixture
def mock_jwk_client(mocker):
    return mocker.patch.object(jwt, "PyJWKClient", autospec=True)


@pytest.mark.asyncio
class TestJwksClient:
    @pytest.fixture(autouse=True)
    def clear_jwks_client_cache(self):
        _jwks_client.cache_clear()
        _jwks_client._LRUCacheWrapper__first_loop = None

    @pytest.fixture
    def mock_get_jwks_uri(self, mocker, jwks_uri):
        return mocker.patch.object(
            openid_discovery,
            "get_jwks_uri",
            return_value=jwks_uri,
        )

    @pytest.fixture
    def mock_jwk_client_constructor(self, mocker):
        return mocker.patch("jwt.PyJWKClient", autospec=True)

    async def test_initializes_jwk_client_correctly(
        self,
        mock_get_jwks_uri: AsyncMock,
        mock_jwk_client_constructor: MagicMock,
        jwks_uri: str,
    ):
        jwk_client_instance = await _jwks_client()

        mock_get_jwks_uri.assert_awaited_once()
        mock_jwk_client_constructor.assert_called_once_with(
            jwks_uri,
            lifespan=auth_settings.auth_context_ttl,
        )

        assert jwk_client_instance == mock_jwk_client_constructor.return_value

    async def test_raises_server_error_if_no_jwks_uri(self, mock_get_jwks_uri):
        mock_get_jwks_uri.return_value = None
        with pytest.raises(ServerError, match="jwks_uri could not be fetched."):
            await _jwks_client()

    async def test_returns_cached_client_on_subsequent_calls(
        self,
        mock_get_jwks_uri: AsyncMock,
        mock_jwk_client_constructor: MagicMock,
    ):
        client1 = await _jwks_client()
        client2 = await _jwks_client()

        assert client1 is client2
        mock_get_jwks_uri.assert_awaited_once()
        mock_jwk_client_constructor.assert_called_once()


@pytest.mark.asyncio
class TestDecodeToken:
    @pytest.fixture(autouse=True)
    def mock_jwt_decode_lib(self, mocker):
        return mocker.patch.object(jwt, "decode", autospec=True)

    @pytest.fixture
    def mock_jwk_client_instance(self, mocker):
        mock_client = AsyncMock(spec=jwt.PyJWKClient)

        mock_signing_key_obj = mocker.MagicMock(spec=jwt.PyJWK)
        mock_signing_key_obj.key = "test-key"
        mock_signing_key_obj.algorithm_name = "RS256"

        mock_client.get_signing_key_from_jwt = mocker.MagicMock(
            return_value=mock_signing_key_obj
        )
        return mock_client

    @pytest_asyncio.fixture(autouse=True)
    async def mock_jwks_client(self, mocker, mock_jwk_client_instance):
        mock_client_func = mocker.patch.object(
            jwt_decoder,
            "_jwks_client",
            return_value=mock_jwk_client_instance,
        )
        return await mock_client_func()

    async def test_decode_invoked_with_expected_parameters(
        self,
        mock_jwt_decode_lib: MagicMock,
        mock_jwks_client: AsyncMock,
        test_token: str,
    ):
        await _decode_token(test_token)

        mock_jwks_client.get_signing_key_from_jwt.assert_called_once_with(test_token)
        signing_key_mock = mock_jwks_client.get_signing_key_from_jwt.return_value

        mock_jwt_decode_lib.assert_called_once_with(
            test_token,
            signing_key_mock.key,
            algorithms=[signing_key_mock.algorithm_name],
            audience=auth_settings.parsed_jwt_audiences,
        )

    async def test_get_signing_key_invoked_with_token(
        self,
        mock_jwks_client: AsyncMock,
        test_token: str,
    ):
        await _decode_token(test_token)
        mock_jwks_client.get_signing_key_from_jwt.assert_called_once_with(test_token)

    async def test_raises_authentication_error_when_error_caught_getting_signing_key(
        self, mock_jwks_client: MagicMock, test_token: str
    ):
        mock_jwks_client.get_signing_key_from_jwt.side_effect = RuntimeError("Fail")

        with pytest.raises(AuthenticationError):
            await _decode_token(test_token)

    async def test_raises_authentication_error_when_error_caught_while_decoding(
        self,
        mock_jwt_decode_lib: MagicMock,
        test_token: str,
    ):
        mock_jwt_decode_lib.side_effect = InvalidTokenError("Fail")

        with pytest.raises(AuthenticationError, match="Invalid or expired JWT"):
            await _decode_token(test_token)

    class TestWhenTheSigningKeyLookupFails:
        """
        Whose fault the failure is decides the status the caller sees.

        Failing to *reach* the IdP is ours, so it is a ServerError; an
        unrecognised key id is attacker-controllable and stays the caller's
        401. A residual remains and is pinned here so it stays visible: a
        malformed or empty JWKS document raises the same PyJWKClientError an
        unknown key id does, so it is type-indistinguishable and still
        surfaces as a 401.
        """

        async def test_a_connection_failure_is_a_server_error(
            self, mock_jwks_client: AsyncMock, test_token: str
        ):
            mock_jwks_client.get_signing_key_from_jwt.side_effect = (
                jwt.exceptions.PyJWKClientConnectionError("jwks host unreachable")
            )

            with pytest.raises(ServerError, match="Internal Server Error"):
                await _decode_token(test_token)

        async def test_an_unknown_key_id_is_an_authentication_error(
            self, mock_jwks_client: AsyncMock, test_token: str
        ):
            # PyJWKClientError is the parent of the connection error above.
            # Widening the carve-out to it would turn an attacker-controllable
            # unknown `kid` into a 500.
            mock_jwks_client.get_signing_key_from_jwt.side_effect = (
                jwt.exceptions.PyJWKClientError('Unable to find a signing key "abc"')
            )

            with pytest.raises(AuthenticationError, match="Invalid or expired JWT"):
                await _decode_token(test_token)


@pytest.mark.asyncio
class TestGetUserId:
    @pytest.fixture(autouse=True)
    def clear_jwks_client_cache(self):
        _jwks_client.cache_clear()
        _jwks_client._LRUCacheWrapper__first_loop = None

    @pytest.fixture
    def mock_internal_decode_token(self, mocker):
        return mocker.patch.object(
            jwt_decoder,
            "_decode_token",
            autospec=True,
        )

    async def test_calls_decode_token(
        self,
        mock_internal_decode_token: AsyncMock,
        test_token: str,
        test_user_id: str,
    ):
        mock_payload = {
            "sub": test_user_id,
            "other": "data",
        }
        mock_internal_decode_token.return_value = mock_payload

        await get_user_id(test_token)

        mock_internal_decode_token.assert_awaited_once_with(test_token)

    async def test_returns_user_id_from_decoded_token(
        self,
        mock_internal_decode_token: AsyncMock,
        test_token: str,
        test_user_id: str,
    ):
        expected_sub = test_user_id
        mock_payload = {
            "sub": expected_sub,
            "exp": 12345.0,
            "other": "data",
        }
        mock_internal_decode_token.return_value = mock_payload

        user_id_result = await get_user_id(test_token)

        assert user_id_result is not None
        assert user_id_result[0] == expected_sub

    async def test_returns_configured_claim_when_principal_id_claim_overridden(
        self,
        mocker,
        mock_internal_decode_token: AsyncMock,
        test_token: str,
        test_user_id: str,
    ):
        mocker.patch.object(auth_settings, "principal_id_claim", "oid")
        expected_oid = test_user_id
        mock_payload = {
            "oid": expected_oid,
            "sub": "should-not-be-used",
            "exp": 12345.0,
        }
        mock_internal_decode_token.return_value = mock_payload

        user_id_result = await get_user_id(test_token)

        assert user_id_result is not None
        assert user_id_result[0] == expected_oid

    async def test_default_claim_still_returns_sub(
        self,
        mock_internal_decode_token: AsyncMock,
        test_token: str,
        test_user_id: str,
    ):
        assert auth_settings.principal_id_claim == "sub"
        expected_sub = test_user_id
        mock_payload = {
            "sub": expected_sub,
            "exp": 12345.0,
        }
        mock_internal_decode_token.return_value = mock_payload

        user_id_result = await get_user_id(test_token)

        assert user_id_result is not None
        assert user_id_result[0] == expected_sub

    async def test_returns_none_if_no_sub(
        self,
        mock_internal_decode_token: AsyncMock,
        test_token: str,
    ):
        mock_payload = {
            "nosub": "nodata",
            "exp": 12345.0,
            "other": "data",
        }
        mock_internal_decode_token.return_value = mock_payload

        user_id_result = await get_user_id(test_token)

        assert user_id_result is None

    async def test_raises_error_from_decode(
        self,
        mock_internal_decode_token: AsyncMock,
        test_token: str,
    ):
        mock_internal_decode_token.side_effect = InvalidTokenError("Fail")

        with pytest.raises(InvalidTokenError):
            await get_user_id(test_token)
