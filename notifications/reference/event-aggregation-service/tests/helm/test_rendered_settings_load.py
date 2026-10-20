# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""If the chart renders, the service's settings load.

The settings ConfigMap's auth helpers fail the render for every auth and
permissions configuration the service's settings validators refuse, so such
a mistake surfaces at `helm template` / `helm upgrade` rather than as a
crash-looping pod. The exact form of each CORS origin is the exception: only
the service checks it, at startup.

This module checks that promise from the other side: it renders the ConfigMap
for valid value combinations and loads the rendered OMNI_EVENTS_* data into
the settings models the service constructs at startup, with every other
OMNI_EVENTS_* variable removed from the environment.
"""

import json
import os

import pytest
from notification_common.config import auth_settings

from app.config.settings import Settings
from tests.helm.conftest import requires_helm, settings_data

pytestmark = requires_helm

OPENID = (
    "openId.openIdConfigurationUri=https://idp.example.com/.well-known/openid-configuration",
    "openId.clientRegistrations[0].name=default",
    "openId.clientRegistrations[0].clientId=default-client",
)

VALID_COMBINATIONS = {
    "deployment-openid": (
        "httpProxy.enabled=true",
        "openId.enabled=true",
        *OPENID,
        "permissions.enabled=true",
    ),
    "openid-with-cors-origins": (
        *OPENID,
        "rest.corsAllowOrigins={https://a.example.com,https://b.example.com}",
    ),
    "additional-audience-only": (
        "openId.openIdConfigurationUri=https://idp.example.com/x",
        "openId.additionalJwtAudience[0]=extra",
    ),
    "explicit-jwt": ("principalSource=jwt", *OPENID),
    "opaque": ("principalSource=opaque", "openId.openIdConfigurationUri=https://x"),
    "user-info-service": ("principalSource=userInfoService",),
    "jwt-unverified-opt-in": (
        "principalSource=jwtUnverified",
        "allowUnverifiedJwt=true",
        "permissions.enabled=true",
    ),
    "disabled": ("principalSource=disabled",),
}


@pytest.fixture
def rendered_data(request, monkeypatch) -> dict:
    """Replace every OMNI_EVENTS_* variable with the rendered ConfigMap data."""
    for key in [k for k in os.environ if k.startswith("OMNI_EVENTS_")]:
        monkeypatch.delenv(key)
    data = settings_data(*VALID_COMBINATIONS[request.param], base=())
    for key, value in data.items():
        if key.startswith("OMNI_EVENTS_"):
            monkeypatch.setenv(key, value)
    return data


@pytest.mark.parametrize("rendered_data", VALID_COMBINATIONS, indirect=True)
class TestRenderedSettingsLoad:
    def test_settings_load_the_rendered_cors_origins(self, rendered_data):
        settings = Settings()

        expected = json.loads(rendered_data["OMNI_EVENTS_CORS_ALLOW_ORIGINS"])
        assert settings.cors_allow_origins == expected

    def test_auth_settings_load_the_rendered_principal_source(
        self, rendered_data, mocker
    ):
        # AuthSettings reads permissions_enabled from the module's
        # common_settings singleton rather than from the environment, so it is
        # handed the settings loaded from the same rendered data.
        mocker.patch.object(auth_settings, "common_settings", Settings())

        loaded = auth_settings.AuthSettings()

        expected = rendered_data["OMNI_EVENTS_PRINCIPAL_SOURCE"]
        assert loaded.principal_source.value == expected
