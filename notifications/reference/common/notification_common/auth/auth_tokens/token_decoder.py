# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from types import SimpleNamespace
from typing import Protocol

from notification_common.auth.principal_source import PrincipalSource
from notification_common.auth.auth_tokens import (
    jwt_decoder,
    jwt_unverified_decoder,
    opaque_token_decoder,
    user_info_service_decoder,
)
from notification_common.config.auth_settings import auth_settings


class TokenDecoder(Protocol):
    async def get_user_id(self, token: str) -> tuple[str, float] | None:
        """Meant to be implemented as a function in a module"""
        ...


# The identity backend is selected by the first-class principal_source enum.
# token_verification_type is no longer the dispatch key; it is retained only as
# the OIDC config datum distinguishing the jwt validating decoder from the
# opaque introspection decoder (SR-1.5).
_DECODER_MODULES = {
    PrincipalSource.JWT: jwt_decoder,
    PrincipalSource.OPAQUE: opaque_token_decoder,
    PrincipalSource.JWT_UNVERIFIED: jwt_unverified_decoder,
    PrincipalSource.USER_INFO_SERVICE: user_info_service_decoder,
}


def decoder() -> TokenDecoder:
    """
    Return a decoder module based on the configured principal source.

    decoder() is only called after the disabled short-circuit in
    principal_id(), so PrincipalSource.DISABLED is never expected here. An
    unexpected source raises rather than falling back to a tokenless or
    cross-source decoder, so there is no silent cross-source fallback.
    """

    decoder_module = _DECODER_MODULES.get(auth_settings.principal_source)
    if decoder_module is None:
        raise ValueError(
            "Unsupported principal source for token decoding: "
            f"{auth_settings.principal_source}"
        )

    # noinspection PyTypeChecker
    return SimpleNamespace(
        get_user_id=decoder_module.get_user_id,
    )
