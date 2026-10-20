# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import importlib
import os

import pytest

from notification_common.auth.principal_source import PrincipalSource
from notification_common.config import auth_settings
from notification_common.config import common_settings
from notification_common.errors.errors import ServerError

# Env vars this module mutates. The autouse fixture saves/restores each so
# there is no cross-test leakage.
_MANAGED_ENV_VARS = (
    "OMNI_EVENTS_PERMISSIONS_ENDPOINT",
    "OMNI_EVENTS_PERMISSIONS_ENABLED",
    "OMNI_EVENTS_PRINCIPAL_SOURCE",
    "OMNI_EVENTS_USER_INFO_SERVICE_URI",
    "OMNI_EVENTS_JWT_AUDIENCES",
    "OMNI_EVENTS_OPENID_CONFIGURATION_URI",
    "OMNI_EVENTS_ALLOW_UNVERIFIED_JWT",
)


@pytest.fixture(autouse=True)
def before():
    originals = {var: os.environ.get(var) for var in _MANAGED_ENV_VARS}

    # Start each test from a clean, fully-defaulted baseline. The default
    # principal_source (jwtUnverified) is refused without the
    # allow_unverified_jwt opt-in, and the module-level singleton is built on
    # reload, so the baseline selects the tokenless source instead.
    for var in _MANAGED_ENV_VARS:
        os.environ.pop(var, None)
    os.environ["OMNI_EVENTS_PERMISSIONS_ENDPOINT"] = ""
    os.environ["OMNI_EVENTS_PERMISSIONS_ENABLED"] = "false"
    os.environ["OMNI_EVENTS_PRINCIPAL_SOURCE"] = "disabled"
    importlib.reload(common_settings)
    importlib.reload(auth_settings)

    yield

    for var, value in originals.items():
        if value is not None:
            os.environ[var] = value
        else:
            os.environ.pop(var, None)

    importlib.reload(common_settings)
    importlib.reload(auth_settings)


@pytest.fixture
def default_auth_env():
    """Leave every auth setting at its default, as an unconfigured deploy does."""
    os.environ.pop("OMNI_EVENTS_PRINCIPAL_SOURCE", None)
    os.environ.pop("OMNI_EVENTS_ALLOW_UNVERIFIED_JWT", None)


class TestPrincipalSource:
    def test_field_default_is_jwt_unverified(self):
        field = auth_settings.AuthSettings.model_fields["principal_source"]
        assert field.default == PrincipalSource.JWT_UNVERIFIED

    def test_default_settings_raise_server_error(self, default_auth_env):
        with pytest.raises(ServerError, match="allow_unverified_jwt"):
            auth_settings.AuthSettings()

    def test_default_settings_fail_the_module_import(self, default_auth_env):
        with pytest.raises(ServerError, match="allow_unverified_jwt"):
            importlib.reload(auth_settings)

    def test_populated_from_env(self):
        os.environ["OMNI_EVENTS_PRINCIPAL_SOURCE"] = "disabled"
        importlib.reload(auth_settings)

        settings = auth_settings.AuthSettings()
        assert settings.principal_source == PrincipalSource.DISABLED

    def test_out_of_enum_value_raises(self):
        with pytest.raises(
            ValueError,
            match=(
                "principal_source\n  Input should be 'disabled', "
                "'jwtUnverified', 'jwt', 'opaque' or 'userInfoService'"
            ),
        ):
            os.environ["OMNI_EVENTS_PRINCIPAL_SOURCE"] = "not-a-mode"
            importlib.reload(auth_settings)


class TestAllowUnverifiedJwt:
    def test_defaults_to_false(self):
        settings = auth_settings.AuthSettings()
        assert settings.allow_unverified_jwt is False

    def test_populated_from_env(self):
        os.environ["OMNI_EVENTS_PRINCIPAL_SOURCE"] = "jwtUnverified"
        os.environ["OMNI_EVENTS_ALLOW_UNVERIFIED_JWT"] = "true"
        importlib.reload(auth_settings)

        assert auth_settings.auth_settings.allow_unverified_jwt is True

    def test_module_imports_with_jwt_unverified_and_opt_in(self):
        os.environ["OMNI_EVENTS_PRINCIPAL_SOURCE"] = "jwtUnverified"
        os.environ["OMNI_EVENTS_ALLOW_UNVERIFIED_JWT"] = "true"
        importlib.reload(auth_settings)

        assert (
            auth_settings.auth_settings.principal_source
            == PrincipalSource.JWT_UNVERIFIED
        )


class TestUserInfoServiceUri:
    def test_populated_from_env(self):
        os.environ["OMNI_EVENTS_PRINCIPAL_SOURCE"] = "userInfoService"
        os.environ["OMNI_EVENTS_USER_INFO_SERVICE_URI"] = "https://uis.example"
        importlib.reload(auth_settings)

        settings = auth_settings.AuthSettings()
        assert settings.user_info_service_uri == "https://uis.example"


class TestAuthenticationEnabled:
    @pytest.mark.parametrize(
        "principal_source, expected",
        [
            (PrincipalSource.DISABLED, False),
            (PrincipalSource.JWT_UNVERIFIED, True),
            (PrincipalSource.JWT, True),
            (PrincipalSource.OPAQUE, True),
            (PrincipalSource.USER_INFO_SERVICE, True),
        ],
    )
    def test_enabled_unless_disabled(self, principal_source, expected):
        settings = auth_settings.AuthSettings(
            principal_source=principal_source,
            openid_configuration_uri="test_endpoint",
            jwt_audiences="test_audience",
            user_info_service_uri="https://uis.example",
            allow_unverified_jwt=True,
        )
        assert settings.authentication_enabled() is expected


class TestValidateTokenSettingsNegative:
    # (principal_source, kwargs, match) -- match names the offending setting
    # in the raised ServerError message.
    negative_cases = [
        (
            PrincipalSource.DISABLED,
            {"permissions_enabled": True},
            {},
            "permissions_enabled requires authentication",
        ),
        (
            PrincipalSource.JWT_UNVERIFIED,
            {},
            {"allow_unverified_jwt": False},
            "Set principal_source to jwt with openid_configuration_uri",
        ),
        (
            PrincipalSource.JWT,
            {},
            {"openid_configuration_uri": "", "jwt_audiences": "aud"},
            "openid_configuration_uri is required when principal_source is jwt",
        ),
        (
            PrincipalSource.JWT,
            {},
            {"openid_configuration_uri": "test_endpoint", "jwt_audiences": ""},
            "jwt_audiences is required when principal_source is jwt",
        ),
        (
            PrincipalSource.OPAQUE,
            {},
            {"openid_configuration_uri": ""},
            "openid_configuration_uri is required when principal_source is opaque",
        ),
        (
            PrincipalSource.USER_INFO_SERVICE,
            {},
            {"user_info_service_uri": ""},
            "user_info_service_uri is required when principal_source is "
            "userInfoService",
        ),
    ]

    @pytest.mark.parametrize(
        "principal_source, common_overrides, auth_kwargs, expected_match",
        negative_cases,
    )
    def test_raises_server_error(
        self, mocker, principal_source, common_overrides, auth_kwargs, expected_match
    ):
        permissions_enabled = common_overrides.get("permissions_enabled", False)
        os.environ["OMNI_EVENTS_PERMISSIONS_ENABLED"] = (
            "true" if permissions_enabled else "false"
        )
        importlib.reload(common_settings)
        # Hand the validator the reloaded common settings without reloading
        # auth_settings, whose module-level singleton would itself be refused
        # under some of these combinations.
        mocker.patch.object(
            auth_settings, "common_settings", common_settings.common_settings
        )

        with pytest.raises(ServerError, match=expected_match):
            auth_settings.AuthSettings(principal_source=principal_source, **auth_kwargs)


class TestValidateTokenSettingsPositive:
    def test_jwt_passes_with_uri_and_audiences(self):
        auth_settings.AuthSettings(
            principal_source=PrincipalSource.JWT,
            openid_configuration_uri="test_endpoint",
            jwt_audiences="test_audience",
        )

    def test_opaque_passes_with_uri(self):
        auth_settings.AuthSettings(
            principal_source=PrincipalSource.OPAQUE,
            openid_configuration_uri="test_endpoint",
        )

    def test_user_info_service_passes_with_uri(self):
        auth_settings.AuthSettings(
            principal_source=PrincipalSource.USER_INFO_SERVICE,
            user_info_service_uri="https://uis.example",
        )

    def test_jwt_unverified_passes_with_opt_in_and_no_oidc_config(self):
        settings = auth_settings.AuthSettings(
            principal_source=PrincipalSource.JWT_UNVERIFIED,
            allow_unverified_jwt=True,
            openid_configuration_uri="",
            jwt_audiences="",
            user_info_service_uri="",
        )
        assert settings.principal_source == PrincipalSource.JWT_UNVERIFIED

    def test_disabled_passes_with_no_oidc_config(self):
        settings = auth_settings.AuthSettings(
            principal_source=PrincipalSource.DISABLED,
            openid_configuration_uri="",
            jwt_audiences="",
            user_info_service_uri="",
        )
        assert settings.authentication_enabled() is False


class TestParsedJwtAudiences:
    def test_parses_single_audience(self):
        settings = auth_settings.AuthSettings(
            principal_source=PrincipalSource.JWT,
            jwt_audiences="audience_one",
            openid_configuration_uri="test_endpoint",
        )
        assert settings.parsed_jwt_audiences == ["audience_one"]

    def test_parses_multiple_comma_separated_audiences(self):
        settings = auth_settings.AuthSettings(
            principal_source=PrincipalSource.JWT,
            jwt_audiences="aud_a,aud_b,aud_c",
            openid_configuration_uri="test_endpoint",
        )
        assert settings.parsed_jwt_audiences == ["aud_a", "aud_b", "aud_c"]

    def test_strips_whitespace_from_audiences(self):
        settings = auth_settings.AuthSettings(
            principal_source=PrincipalSource.JWT,
            jwt_audiences=" aud_a , aud_b ",
            openid_configuration_uri="test_endpoint",
        )
        assert settings.parsed_jwt_audiences == ["aud_a", "aud_b"]

    def test_returns_empty_list_for_empty_string(self):
        settings = auth_settings.AuthSettings(
            principal_source=PrincipalSource.DISABLED,
            jwt_audiences="",
        )
        assert settings.parsed_jwt_audiences == []

    def test_ignores_empty_entries_from_trailing_commas(self):
        settings = auth_settings.AuthSettings(
            principal_source=PrincipalSource.JWT,
            jwt_audiences="aud_a,,aud_b,",
            openid_configuration_uri="test_endpoint",
        )
        assert settings.parsed_jwt_audiences == ["aud_a", "aud_b"]
