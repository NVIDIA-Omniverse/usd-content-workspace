# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import os

import pytest

from notification_common.auth.principal_source import PrincipalSource
from notification_common.test_tools.mock_auth_settings import mock_auth_settings

_MANAGED_ENV_VARS = (
    "OMNI_EVENTS_PRINCIPAL_SOURCE",
    "OMNI_EVENTS_ALLOW_UNVERIFIED_JWT",
    "OMNI_EVENTS_TOKEN_VERIFICATION_TYPE",
    "OMNI_EVENTS_OPENID_CONFIGURATION_URI",
    "OMNI_EVENTS_JWT_AUDIENCES",
    "OMNI_EVENTS_USER_INFO_SERVICE_URI",
    "OMNI_EVENTS_PERMISSIONS_ENABLED",
)


@pytest.fixture(autouse=True)
def restore_env(monkeypatch):
    """Record the env vars mock_auth_settings writes so they are restored."""
    for var in _MANAGED_ENV_VARS:
        # setenv records the original value, or its absence, for teardown;
        # delenv alone records nothing for an unset variable.
        monkeypatch.setenv(var, "")
        monkeypatch.delenv(var)


class TestMockAuthSettings:
    def test_jwt_unverified_sets_the_opt_in(self):
        mock_auth_settings(PrincipalSource.JWT_UNVERIFIED)

        assert os.environ["OMNI_EVENTS_ALLOW_UNVERIFIED_JWT"] == "true"

    def test_other_sources_clear_the_opt_in(self):
        os.environ["OMNI_EVENTS_ALLOW_UNVERIFIED_JWT"] = "true"

        mock_auth_settings(PrincipalSource.JWT)

        assert "OMNI_EVENTS_ALLOW_UNVERIFIED_JWT" not in os.environ
