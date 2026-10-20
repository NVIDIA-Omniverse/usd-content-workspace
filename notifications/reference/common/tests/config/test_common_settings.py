# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import importlib
import os
from pathlib import Path

import pytest

from notification_common.config import (
    auth_settings,
    common_settings,
    notification_base_settings,
)
from notification_common.config.common_settings import (
    CommonSettings,
    validate_cors_origins,
)
from notification_common.errors.errors import ServerError


@pytest.fixture
def rabbitmq_uri_initial_value():
    return "amqp://127.0.0.1:5672"


# Env vars this file's tests mutate or depend on. The shared
# mock_auth_settings fixture sets some of these (e.g.
# OMNI_EVENTS_PERMISSIONS_ENABLED=true) in os.environ at conftest import
# time, so the fixture below clears them in SETUP to give every test a
# clean, fully-defaulted environment regardless of test order, then
# restores the original ambient values on teardown so later test files
# still see the shared auth-on state.
_MANAGED_ENV_VARS = (
    "OMNI_EVENTS_RABBITMQ_URI",
    "OMNI_EVENTS_OPENID_ENABLED",
    "OMNI_EVENTS_PERMISSIONS_ENDPOINT",
    "OMNI_EVENTS_PERMISSIONS_ENABLED",
    "OMNI_EVENTS_RABBITMQ_DURABLE_CHANNEL_POOL_SIZE",
    "OMNI_EVENTS_RABBITMQ_CHANNEL_OVERFLOW_ENABLED",
)


@pytest.fixture(autouse=True)
def reset_settings(rabbitmq_uri_initial_value, mocker):
    """
    Give each test a clean, fully-defaulted settings environment, then
    restore the ambient (shared mock_auth_settings) env state afterward.
    """
    saved_env = {name: os.environ.get(name) for name in _MANAGED_ENV_VARS}

    for name in _MANAGED_ENV_VARS:
        os.environ.pop(name, None)

    importlib.reload(common_settings)
    importlib.reload(auth_settings)

    yield

    mocker.patch("builtins.open", side_effect=FileNotFoundError)

    for name, value in saved_env.items():
        if value is None:
            os.environ.pop(name, None)
        else:
            os.environ[name] = value

    CommonSettings.read_secrets()

    importlib.reload(common_settings)
    importlib.reload(auth_settings)


class TestReadSecrets:

    def test_does_nothing_if_secrets_json_file_does_not_exist(
        self, rabbitmq_uri_initial_value
    ):
        importlib.reload(common_settings)
        uri = common_settings.common_settings.rabbitmq_uri
        assert uri == rabbitmq_uri_initial_value

    def test_gets_secrets_out_of_json_file(self, rabbitmq_uri_initial_value, mocker):
        mocker.patch(
            "builtins.open",
            return_value=open(
                Path(__file__).parent.parent / "fixtures/test_secrets.json"
            ),
        )
        importlib.reload(notification_base_settings)
        importlib.reload(common_settings)
        assert common_settings.common_settings.rabbitmq_uri == "some-new-value"


class TestPermissionsEnabled:

    def test_defaults_to_false(self):
        assert CommonSettings().permissions_enabled is False

    def test_env_var_override_enables(self):
        os.environ["OMNI_EVENTS_PERMISSIONS_ENABLED"] = "true"
        importlib.reload(common_settings)
        assert common_settings.common_settings.permissions_enabled is True


class TestDurableChannelPoolSettings:
    """Behavior-preserving defaults for the durable / DLQ-browse pool."""

    def test_pool_size_defaults_to_100(self):
        assert CommonSettings().rabbitmq_durable_channel_pool_size == 100

    def test_pool_min_size_defaults_to_0(self):
        assert CommonSettings().rabbitmq_durable_channel_pool_min_size == 0

    def test_prefetch_count_defaults_to_500(self):
        assert CommonSettings().rabbitmq_durable_channel_prefetch_count == 500

    def test_wait_ms_defaults_to_500(self):
        assert CommonSettings().rabbitmq_durable_channel_wait_ms == 500

    def test_client_limit_defaults_to_1(self):
        assert CommonSettings().rabbitmq_durable_channel_client_limit == 1

    def test_env_var_override_sets_pool_size(self):
        os.environ["OMNI_EVENTS_RABBITMQ_DURABLE_CHANNEL_POOL_SIZE"] = "250"
        importlib.reload(common_settings)
        settings = common_settings.common_settings
        assert settings.rabbitmq_durable_channel_pool_size == 250


class TestNonDurableChannelPoolSettings:
    """
    Shipped defaults for the pre-existing (now non-durable) channel pool.
    Epic t1.tj2.xn landed the packing flip: client_limit 1 -> 8 and prefetch
    500 -> 15. The remaining values are unchanged (pool_size 100,
    pool_min_size 75, wait_ms 500, overflow_enabled True); a failure on those
    would be a silent drift, not an expected update.
    """

    def test_pool_size_defaults_to_100(self):
        assert CommonSettings().rabbitmq_channel_pool_size == 100

    def test_pool_min_size_defaults_to_75(self):
        assert CommonSettings().rabbitmq_channel_pool_min_size == 75

    def test_prefetch_count_defaults_to_15(self):
        assert CommonSettings().rabbitmq_channel_prefetch_count == 15

    def test_wait_ms_defaults_to_500(self):
        assert CommonSettings().rabbitmq_channel_wait_ms == 500

    def test_client_limit_defaults_to_8(self):
        assert CommonSettings().rabbitmq_channel_client_limit == 8

    def test_overflow_defaults_to_enabled(self):
        assert CommonSettings().rabbitmq_channel_overflow_enabled is True

    def test_env_var_override_disables_overflow(self):
        os.environ["OMNI_EVENTS_RABBITMQ_CHANNEL_OVERFLOW_ENABLED"] = "false"
        importlib.reload(common_settings)
        settings = common_settings.common_settings
        assert settings.rabbitmq_channel_overflow_enabled is False


class TestDefaultSignalFlagsToTelemetry:
    """metrics_enabled / tracing_enabled fall back to telemetry_enabled."""

    def test_both_default_to_false(self):
        settings = CommonSettings()
        assert settings.metrics_enabled is False
        assert settings.tracing_enabled is False

    def test_unset_signals_inherit_legacy_telemetry_enabled(self):
        settings = CommonSettings(telemetry_enabled=True)
        assert settings.metrics_enabled is True
        assert settings.tracing_enabled is True

    def test_explicit_signal_overrides_legacy(self):
        settings = CommonSettings(telemetry_enabled=True, metrics_enabled=False)
        assert settings.metrics_enabled is False
        assert settings.tracing_enabled is True

    def test_signal_can_enable_without_legacy(self):
        settings = CommonSettings(tracing_enabled=True)
        assert settings.tracing_enabled is True
        assert settings.metrics_enabled is False


class TestParseCorsAllowOrigins:
    """Parsing OMNI_EVENTS_CORS_ALLOW_ORIGINS into the origin allow-list."""

    @pytest.fixture(autouse=True)
    def restore_cors_env(self, monkeypatch):
        monkeypatch.delenv("OMNI_EVENTS_CORS_ALLOW_ORIGINS", raising=False)

    def test_defaults_to_empty(self):
        assert CommonSettings().cors_allow_origins == []

    def test_parses_comma_separated_env(self, monkeypatch):
        monkeypatch.setenv(
            "OMNI_EVENTS_CORS_ALLOW_ORIGINS",
            " https://a.example.com , https://b.example.com,",
        )
        assert CommonSettings().cors_allow_origins == [
            "https://a.example.com",
            "https://b.example.com",
        ]

    def test_parses_json_array_env(self, monkeypatch):
        monkeypatch.setenv(
            "OMNI_EVENTS_CORS_ALLOW_ORIGINS", '[" https://a.example.com"]'
        )
        assert CommonSettings().cors_allow_origins == ["https://a.example.com"]

    def test_empty_env_is_empty(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_CORS_ALLOW_ORIGINS", "")
        assert CommonSettings().cors_allow_origins == []


class TestValidateCorsAllowOrigins:
    """The field validator applies validate_cors_origins when settings load."""

    def test_refused_origin_in_env_fails_at_load(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_CORS_ALLOW_ORIGINS", "null")
        with pytest.raises(ServerError, match="must not contain 'null'"):
            CommonSettings()


class TestValidateCorsOrigins:
    """Normalizing and refusing entries of the CORS origin allow-list."""

    def test_trims_and_lower_cases(self):
        assert validate_cors_origins([" HTTPS://App.Example.com:8443 "]) == [
            "https://app.example.com:8443"
        ]

    def test_drops_blank_entries(self):
        assert validate_cors_origins(["", "  ", "https://a.example.com"]) == [
            "https://a.example.com"
        ]

    @pytest.mark.parametrize("origin", ["*", "null", " NULL "])
    def test_refuses_origins_unsafe_with_credentials(self, origin):
        with pytest.raises(ServerError, match="must not contain"):
            validate_cors_origins([origin])

    @pytest.mark.parametrize(
        "origin",
        [
            "https://app.example.com/",
            "https://app.example.com/path",
            "app.example.com",
            "ftp://app.example.com",
            "https://app.example.com?x=1",
            "https://*.example.com",
            "https://user:pw@app.example.com",
            "https://app.example.com:",
            "https://app.example.com:abc",
            "https://ex\u00e4mple.com",
            "http://app.example.com.",
            "https://app.example.com:443",
            "http://app.example.com:80",
            "https://app.example.com:080",
            "https://app.example.com:0",
            "https://app.example.com:99999",
            "https://[::ffff:127.0.0.1]",
            "https://[2001:0db8::1]",
            "https://[:]",
            "https://999.999.999.999",
            "https://1.2.3",
            "https://[1:0:0:2::3:4]",
            "https://[1::1:1:1:1:1:1]",
            "https://0x7f.0.0.1",
            "https://example.123",
        ],
    )
    def test_refuses_entries_that_are_not_origins(self, origin):
        with pytest.raises(ServerError, match="is not written the way browsers send"):
            validate_cors_origins([origin])

    @pytest.mark.parametrize(
        "origin",
        [
            "http://localhost:3000",
            "https://[::1]:8443",
            "https://xn--exmple-cua.com",
            "https://a_b.example.com",
            "http://127.0.0.1:8080",
            "https://[::ffff:7f00:1]",
            "https://[2001:db8::1]:8443",
            "https://[2001:db8:0:1::1]",
            "https://[1::2:0:0:3:4]",
            "https://[1:0:1:1:1:1:1:1]",
            "https://-foo.example.com",
            "http://app.example.com:443",
        ],
    )
    def test_accepts_bare_origins(self, origin):
        assert validate_cors_origins([origin]) == [origin]

    def test_refuses_a_single_string(self):
        with pytest.raises(ServerError, match="not a single string"):
            validate_cors_origins("https://a.example.com")
