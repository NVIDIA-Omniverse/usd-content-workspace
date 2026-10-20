# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import importlib
import os

import pytest
from notification_common.errors.errors import ServerError
from pydantic import ValidationError

from app.config import settings
from tests.conftest import reset_env_vars

FOUR_MEBIBYTES = 4194304


@pytest.fixture(autouse=True)
def reset_settings():
    """
    Put the settings back to normal after tests
    """
    reset_env_vars()
    yield
    reset_env_vars()


def test_sets_settings_from_env():
    os.environ["OMNI_EVENTS_GRPC_PORT"] = "new value"
    importlib.reload(settings)
    assert settings.settings.grpc_port == "new value"


class TestValidatePermissionsSettings:
    """
    Covers the single permission-service gate of validate_auth_settings.
    Aggregation has exactly one gate: when permissions_enabled is true,
    permissions_endpoint must be set, otherwise the permissions HTTP client
    crashes the publish path on a None endpoint. A disabled gate leaves its
    endpoint inert (no error). There is no storage-permissions gate.

    Enabling the gate requires a non-disabled principal_source, since the
    AuthSettings validator rejects permissions_enabled=true together with
    principal_source=disabled (the tokenless reseed reset_env_vars seeds).
    """

    def test_raises_when_permissions_enabled_but_no_endpoint(self):
        os.environ["OMNI_EVENTS_PRINCIPAL_SOURCE"] = "jwtUnverified"
        os.environ["OMNI_EVENTS_PERMISSIONS_ENABLED"] = "true"
        os.environ["OMNI_EVENTS_PERMISSIONS_ENDPOINT"] = ""

        with pytest.raises(
            ServerError,
            match="permissions_enabled is true but permissions_endpoint",
        ):
            importlib.reload(settings)

    def test_passes_when_permissions_enabled_with_endpoint(self):
        os.environ["OMNI_EVENTS_PRINCIPAL_SOURCE"] = "jwtUnverified"
        os.environ["OMNI_EVENTS_PERMISSIONS_ENABLED"] = "true"
        os.environ["OMNI_EVENTS_PERMISSIONS_ENDPOINT"] = (
            "https://permissions.example.com"
        )
        importlib.reload(settings)

        assert settings.settings.permissions_enabled is True

    def test_passes_when_permissions_disabled_with_endpoint_set(self):
        # A disabled gate leaves its endpoint inert: no ServerError even
        # with permissions_endpoint populated.
        os.environ["OMNI_EVENTS_PERMISSIONS_ENABLED"] = "false"
        os.environ["OMNI_EVENTS_PERMISSIONS_ENDPOINT"] = (
            "https://permissions.example.com"
        )
        importlib.reload(settings)

        assert settings.settings.permissions_enabled is False
        assert settings.settings.permissions_endpoint == (
            "https://permissions.example.com"
        )


class TestRestMaxRequestBytes:
    """
    Covers the REST body cap: its default and its gt=0 constraint.

    The field is constructed directly rather than through an env var and a
    module reload, so a rejected value cannot leak into the process
    environment of a later test under the random-order runner. The default is
    asserted as a plain byte count -- the same number the chart renders --
    rather than as the 4 * 1024 * 1024 expression the model is written with.
    """

    def test_defaults_to_four_mebibytes(self):
        assert settings.Settings().rest_max_request_bytes == FOUR_MEBIBYTES

    @pytest.mark.parametrize("value", [0, -1], ids=["zero", "negative"])
    def test_rejects_a_non_positive_cap(self, value):
        with pytest.raises(ValidationError, match="greater than 0"):
            settings.Settings(rest_max_request_bytes=value)


def test_default_deploy_posture_loads_without_crash_loop():
    # Crash-loop regression guard: the DEFAULT-deploy posture is the gate
    # off (permissions_enabled unset/false), no permissions_endpoint, and
    # principal_source at its default. settings must reload cleanly with no
    # startup ServerError and no endpoint required.
    os.environ.pop("OMNI_EVENTS_PERMISSIONS_ENABLED", None)
    os.environ.pop("OMNI_EVENTS_PRINCIPAL_SOURCE", None)
    os.environ["OMNI_EVENTS_PERMISSIONS_ENDPOINT"] = ""

    importlib.reload(settings)

    assert settings.settings.permissions_enabled is False
