# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest

from notification_common.auth.principal_source import PrincipalSource
from notification_common.auth.token_verification_type import (
    TokenVerificationType,
)
from notification_common.auth.auth_tokens import (
    jwt_decoder,
    jwt_unverified_decoder,
    opaque_token_decoder,
    user_info_service_decoder,
)
from notification_common.auth.auth_tokens import token_decoder
from notification_common.auth.auth_tokens.token_decoder import (
    decoder,
)
from notification_common.config.auth_settings import AuthSettings, auth_settings


class TestDecoder:
    @pytest.mark.parametrize(
        ("principal_source", "expected_module"),
        [
            (PrincipalSource.JWT, jwt_decoder),
            (PrincipalSource.OPAQUE, opaque_token_decoder),
            (PrincipalSource.JWT_UNVERIFIED, jwt_unverified_decoder),
            (PrincipalSource.USER_INFO_SERVICE, user_info_service_decoder),
        ],
    )
    def test_returns_backend_for_principal_source(
        self, mocker, principal_source, expected_module
    ):
        mocker.patch.object(
            auth_settings,
            "principal_source",
            principal_source,
        )
        # noinspection PyUnresolvedReferences
        assert decoder().get_user_id.__module__ == expected_module.__name__

    def test_opted_in_jwt_unverified_uses_the_unverified_decoder(self, mocker):
        settings = AuthSettings(
            principal_source=PrincipalSource.JWT_UNVERIFIED,
            allow_unverified_jwt=True,
        )
        mocker.patch.object(token_decoder, "auth_settings", settings)
        # noinspection PyUnresolvedReferences
        assert decoder().get_user_id.__module__ == jwt_unverified_decoder.__name__


class TestTokenVerificationTypeRetention:
    def test_retains_jwt_and_opaque_members(self):
        # SR-1.5: token_verification_type no longer drives dispatch, but the
        # enum is retained as the OIDC config datum distinguishing the jwt
        # validating decoder from the opaque introspection decoder. Deleting
        # the enum (or these members) must fail CI.
        assert (TokenVerificationType.JWT, TokenVerificationType.OPAQUE) == (
            TokenVerificationType("jwt"),
            TokenVerificationType("opaque"),
        )
