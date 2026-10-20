# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from pydantic import model_validator

from notification_common.auth.principal_source import PrincipalSource
from notification_common.auth.token_verification_type import (
    TokenVerificationType,
)
from notification_common.config.common_settings import (
    NotificationBaseSettings,
    common_settings,
)
from notification_common.errors.errors import ServerError


class AuthSettings(NotificationBaseSettings):
    """
    Auth settings for the App.
    NOTE: all environment variables will be prefixed
          with "OMNI_EVENTS_"
    """

    # principal_source selects how identity is derived and whether a token
    # is required. It is the single first-class gate for authentication
    # behavior.
    principal_source: PrincipalSource = PrincipalSource.JWT_UNVERIFIED
    # allow_unverified_jwt is the explicit opt-in for the jwtUnverified
    # principal source, which reads the principal from a JWT without checking
    # its signature, issuer, audience, or expiry. It is safe only when every
    # path to the service passes a proxy that has already verified the token,
    # including in-cluster callers that could otherwise reach the service
    # directly, so it defaults to false and the validator below refuses
    # jwtUnverified (including the default principal_source) unless it is set.
    allow_unverified_jwt: bool = False
    # user_info_service_uri is consulted only when principal_source is
    # USER_INFO_SERVICE.
    user_info_service_uri: str = ""
    # openid_enabled is retained as an infra-only field. It is no longer
    # read by authentication_enabled() or the validator.
    openid_enabled: bool = False
    # token_verification_type is the OIDC datum distinguishing jwt vs opaque
    # decoders; it no longer independently gates behavior.
    token_verification_type: TokenVerificationType = TokenVerificationType.JWT
    openid_configuration_uri: str = ""
    jwt_audiences: str = ""
    auth_context_ttl: int = 1800  # 30 minutes
    principal_id_claim: str = "sub"

    @property
    def parsed_jwt_audiences(self) -> list[str]:
        """
        Parse the comma-separated jwt_audiences string
        into a list of audience values.
        """
        return [a.strip() for a in self.jwt_audiences.split(",") if a.strip()]

    def authentication_enabled(self) -> bool:
        """Check if authentication is enabled."""
        return self.principal_source != PrincipalSource.DISABLED

    @model_validator(mode="after")
    def validate_token_settings(self):
        if (
            common_settings.permissions_enabled
            and self.principal_source == PrincipalSource.DISABLED
        ):
            raise ServerError(
                "permissions_enabled requires authentication, but "
                "principal_source is disabled. Either set a non-disabled "
                "principal_source or disable permissions_enabled."
            )

        if (
            self.principal_source == PrincipalSource.JWT_UNVERIFIED
            and not self.allow_unverified_jwt
        ):
            raise ServerError(
                "principal_source is jwtUnverified (also the default when "
                "OMNI_EVENTS_PRINCIPAL_SOURCE is unset), which accepts a "
                "bearer token without verifying its signature, issuer, "
                "audience, or expiry. Set principal_source to jwt with "
                "openid_configuration_uri and jwt_audiences so the service "
                "verifies tokens itself, or to disabled (with "
                "permissions_enabled false) for a local tokenless run. Set "
                "allow_unverified_jwt "
                "(OMNI_EVENTS_ALLOW_UNVERIFIED_JWT=true) only when every path "
                "to the service verifies tokens first, including in-cluster "
                "callers that reach it without passing through a proxy."
            )

        if self.principal_source == PrincipalSource.JWT:
            if not self.openid_configuration_uri:
                raise ServerError(
                    "openid_configuration_uri is required "
                    "when principal_source is jwt"
                )
            if not self.parsed_jwt_audiences:
                raise ServerError(
                    "jwt_audiences is required when principal_source is jwt"
                )
        elif self.principal_source == PrincipalSource.OPAQUE:
            if not self.openid_configuration_uri:
                raise ServerError(
                    "openid_configuration_uri is required "
                    "when principal_source is opaque"
                )
        elif self.principal_source == PrincipalSource.USER_INFO_SERVICE:
            if not self.user_info_service_uri:
                raise ServerError(
                    "user_info_service_uri is required "
                    "when principal_source is userInfoService"
                )

        return self


auth_settings = AuthSettings()
