# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import os

from notification_common.auth.principal_source import PrincipalSource


def mock_auth_settings(
    principal_source: PrincipalSource = PrincipalSource.JWT,
    user_info_service_uri: str = "test-user-info-service-endpoint",
):
    """
    Flip auth "on" for tests by setting the OMNI_EVENTS_* env vars.

    The default zero-arg call selects the token-requiring ``jwt``
    principal source (the faithful successor to the OpenID-era fixture:
    it preserves the JWKS/OIDC config the fixture already set) plus
    permissions enabled.

    Pass ``principal_source=PrincipalSource.USER_INFO_SERVICE`` to
    exercise the user-info-service identity path;
    ``user_info_service_uri`` is then written so that source's validator
    passes. The OIDC config is set unconditionally so callers can switch
    the principal source without losing the jwt/opaque prerequisites.

    Passing ``principal_source=PrincipalSource.JWT_UNVERIFIED`` also sets
    the ``allow_unverified_jwt`` opt-in that mode requires; any other
    source clears it.
    """
    os.environ["OMNI_EVENTS_PRINCIPAL_SOURCE"] = principal_source.value
    if principal_source == PrincipalSource.JWT_UNVERIFIED:
        os.environ["OMNI_EVENTS_ALLOW_UNVERIFIED_JWT"] = "true"
    else:
        os.environ.pop("OMNI_EVENTS_ALLOW_UNVERIFIED_JWT", None)
    # token_verification_type is the OIDC datum distinguishing jwt vs
    # opaque (SR-1.5); retained from the OpenID-era fixture.
    os.environ["OMNI_EVENTS_TOKEN_VERIFICATION_TYPE"] = "jwt"
    os.environ["OMNI_EVENTS_OPENID_CONFIGURATION_URI"] = "test-openid-endpoint"
    os.environ["OMNI_EVENTS_JWT_AUDIENCES"] = "test-jwt-audience"
    os.environ["OMNI_EVENTS_USER_INFO_SERVICE_URI"] = user_info_service_uri
    os.environ["OMNI_EVENTS_PERMISSIONS_ENABLED"] = "true"
