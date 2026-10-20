# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import MagicMock

import jwt
import pytest
from jwt.exceptions import InvalidTokenError

from notification_common.auth import openid_discovery
from notification_common.auth.auth_tokens import jwt_decoder
from notification_common.auth.auth_tokens import jwt_unverified_decoder

# noinspection PyProtectedMember
from notification_common.auth.auth_tokens.jwt_unverified_decoder import (
    get_user_id,
    _decode_token,
    _TOKEN_EXP_SECONDS,
)
from notification_common.config.auth_settings import auth_settings
from notification_common.errors.errors import AuthenticationError

#: The wall clock the derived-lifetime rows freeze.
NOW = 1000.0

#: An `exp` the token carries. It is in the past relative to NOW, which is the
#: interesting case: this mode accepts an expired token by design.
TOKEN_EXP = 500.0


@pytest.fixture
def mock_jwt_decode(mocker):
    return mocker.patch.object(jwt, "decode", autospec=True)


@pytest.mark.asyncio
class TestDecodeToken:
    async def test_decodes_without_signature_verification(
        self,
        mock_jwt_decode: MagicMock,
        test_token: str,
    ):
        _decode_token(test_token)

        mock_jwt_decode.assert_called_once_with(
            test_token, options={"verify_signature": False}
        )

    async def test_does_not_pass_signing_key(
        self,
        mock_jwt_decode: MagicMock,
        test_token: str,
    ):
        _decode_token(test_token)

        _args, _kwargs = mock_jwt_decode.call_args
        # The validating decoder passes the signing key positionally as the
        # second arg; the no-verify path must not.
        assert len(_args) == 1

    async def test_does_not_pass_algorithms(
        self,
        mock_jwt_decode: MagicMock,
        test_token: str,
    ):
        _decode_token(test_token)

        _args, kwargs = mock_jwt_decode.call_args
        assert "algorithms" not in kwargs

    async def test_does_not_pass_audience(
        self,
        mock_jwt_decode: MagicMock,
        test_token: str,
    ):
        _decode_token(test_token)

        _args, kwargs = mock_jwt_decode.call_args
        assert "audience" not in kwargs

    async def test_does_not_construct_jwks_client(
        self,
        mocker,
        mock_jwt_decode: MagicMock,
        test_token: str,
    ):
        mock_jwk_client = mocker.patch.object(jwt, "PyJWKClient", autospec=True)

        _decode_token(test_token)

        mock_jwk_client.assert_not_called()

    async def test_does_not_call_oidc_discovery(
        self,
        mocker,
        mock_jwt_decode: MagicMock,
        test_token: str,
    ):
        mock_get_jwks_uri = mocker.patch.object(
            openid_discovery, "get_jwks_uri", autospec=True
        )

        _decode_token(test_token)

        mock_get_jwks_uri.assert_not_awaited()

    async def test_returns_decoded_payload(
        self,
        mock_jwt_decode: MagicMock,
        test_token: str,
        test_user_id: str,
    ):
        payload = {"sub": test_user_id, "exp": 12345.0}
        mock_jwt_decode.return_value = payload

        assert _decode_token(test_token) == payload

    async def test_raises_authentication_error_when_decode_fails(
        self,
        mock_jwt_decode: MagicMock,
        test_token: str,
    ):
        mock_jwt_decode.side_effect = InvalidTokenError("malformed")

        with pytest.raises(AuthenticationError, match="Invalid or expired JWT"):
            _decode_token(test_token)


@pytest.mark.asyncio
class TestGetUserId:
    @pytest.fixture(autouse=True)
    def mock_internal_decode_token(self, mocker):
        return mocker.patch.object(
            jwt_unverified_decoder,
            "_decode_token",
            autospec=True,
        )

    @pytest.fixture
    def frozen_clock(self, mocker):
        """Freeze the clock the derived cache lifetime is measured from."""
        mocker.patch.object(jwt_unverified_decoder.time, "time", return_value=NOW)
        return NOW

    async def test_returns_principal_and_lifetime_tuple(
        self,
        frozen_clock: float,
        mock_internal_decode_token: MagicMock,
        test_token: str,
        test_user_id: str,
    ):
        mock_internal_decode_token.return_value = {
            "sub": test_user_id,
            "exp": TOKEN_EXP,
        }

        assert await get_user_id(test_token) == (
            test_user_id,
            frozen_clock + _TOKEN_EXP_SECONDS,
        )

    @pytest.mark.parametrize(
        "token_exp",
        [1, TOKEN_EXP, NOW + 10_000.0],
        ids=["long_expired", "expired", "far_future"],
    )
    async def test_lifetime_is_independent_of_the_tokens_own_exp(
        self,
        frozen_clock: float,
        mock_internal_decode_token: MagicMock,
        test_token: str,
        test_user_id: str,
        token_exp: float,
    ):
        # The second element is a cache lifetime, not the token's expiry.
        # Returning the token's own `exp` let the caching layer refuse, on a
        # later call, a token this decoder had deliberately accepted -- which
        # is exactly what an expired token does under this mode, since it
        # makes no expiry judgement at all.
        mock_internal_decode_token.return_value = {
            "sub": test_user_id,
            "exp": token_exp,
        }

        result = await get_user_id(test_token)

        assert result[1] == frozen_clock + _TOKEN_EXP_SECONDS

    async def test_returns_none_when_exp_absent(
        self,
        mock_internal_decode_token: MagicMock,
        test_token: str,
        test_user_id: str,
    ):
        mock_internal_decode_token.return_value = {"sub": test_user_id}

        assert await get_user_id(test_token) is None

    async def test_returns_none_when_principal_claim_absent(
        self,
        mock_internal_decode_token: MagicMock,
        test_token: str,
    ):
        mock_internal_decode_token.return_value = {"exp": 12345.0}

        assert await get_user_id(test_token) is None

    async def test_uses_overridden_principal_claim(
        self,
        mocker,
        mock_internal_decode_token: MagicMock,
        test_token: str,
        test_user_id: str,
    ):
        mocker.patch.object(auth_settings, "principal_id_claim", "oid")
        mock_internal_decode_token.return_value = {
            "oid": test_user_id,
            "sub": "should-not-be-used",
            "exp": 12345.0,
        }

        result = await get_user_id(test_token)

        assert result is not None
        assert result[0] == test_user_id

    async def test_uses_default_sub_claim(
        self,
        mock_internal_decode_token: MagicMock,
        test_token: str,
        test_user_id: str,
    ):
        assert auth_settings.principal_id_claim == "sub"
        mock_internal_decode_token.return_value = {
            "sub": test_user_id,
            "exp": 12345.0,
        }

        result = await get_user_id(test_token)

        assert result is not None
        assert result[0] == test_user_id

    async def test_propagates_authentication_error_from_decode(
        self,
        mock_internal_decode_token: MagicMock,
        test_token: str,
    ):
        mock_internal_decode_token.side_effect = AuthenticationError(
            "Invalid or expired JWT"
        )

        with pytest.raises(AuthenticationError):
            await get_user_id(test_token)


@pytest.mark.asyncio
class TestPrincipalEquivalenceWithValidatingDecoder:
    """The no-verify decoder must extract the SAME principal id as the
    validating decoder for an identical payload + principal claim, so durable
    queue names stay stable across principal-source modes."""

    @pytest.mark.parametrize(
        "principal_claim, payload",
        [
            (
                "sub",
                {"sub": "user-from-decoder", "exp": 12345.0},
            ),
            (
                "oid",
                {"oid": "user-from-decoder", "sub": "decoy", "exp": 12345.0},
            ),
        ],
    )
    async def test_principals_match(
        self,
        mocker,
        test_token: str,
        principal_claim: str,
        payload: dict,
    ):
        mocker.patch.object(auth_settings, "principal_id_claim", principal_claim)

        # Drive both decoders off the SAME decoded payload.
        mocker.patch.object(jwt, "decode", autospec=True, return_value=payload)
        mocker.patch.object(
            jwt_decoder, "_decode_token", autospec=True, return_value=payload
        )

        unverified_result = await get_user_id(test_token)
        verified_result = await jwt_decoder.get_user_id(test_token)

        assert unverified_result is not None
        assert verified_result is not None
        assert unverified_result[0] == verified_result[0]
