# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from opentelemetry import metrics
from opentelemetry.instrumentation import grpc as otel_grpc_instrumentation

from notification_common.auth.principal_source import PrincipalSource
from notification_common.config.common_settings import common_settings
from notification_common.interceptors import (
    auth_interceptor,
    exception_interceptor,
)
from notification_common.interceptors.interceptors import (
    get_interceptors,
)
from notification_common.interceptors.metrics import server_metrics_interceptor
from notification_common.config import observability_config


@pytest.mark.asyncio
class TestGetInterceptors:

    @pytest.fixture(autouse=True)
    def mock_get_meter(self, mocker):
        return mocker.patch.object(metrics.get_meter_provider(), "get_meter")

    @pytest.fixture(autouse=True)
    def mock_aio_server(self, mocker):
        return mocker.patch.object(
            otel_grpc_instrumentation,
            "aio_server_interceptor",
            return_value="test-aio-interceptor",
        )

    @pytest.fixture(autouse=True)
    def mock_server_metrics(self, mocker):
        return mocker.patch.object(
            server_metrics_interceptor,
            "ServerMetricsInterceptor",
            return_value="test-metrics-interceptor",
        )

    @pytest.fixture(autouse=True)
    def mock_exception_interceptor(self, mocker):
        return mocker.patch.object(
            exception_interceptor,
            "ExceptionInterceptor",
            return_value="test-exception-interceptor",
        )

    @pytest.fixture(autouse=True)
    def mock_auth_interceptor(self, mocker):
        return mocker.patch.object(
            auth_interceptor,
            "AuthInterceptor",
            return_value="test-auth-interceptor",
        )

    @pytest.fixture(autouse=True)
    def mock_project_name(self, mocker):
        return mocker.patch(
            "notification_common.interceptors.interceptors.project_name",
            return_value="test-project",
        )

    @pytest.fixture(autouse=True)
    def mock_common_settings(self, mocker):
        mocker.patch.object(common_settings, "metrics_enabled", False)
        mocker.patch.object(common_settings, "tracing_enabled", False)

    @pytest.mark.parametrize("is_enabled", [True, False])
    @pytest.mark.parametrize(
        "interceptor_name, flag",
        [
            ("test-metrics-interceptor", "metrics_enabled"),
            ("test-aio-interceptor", "tracing_enabled"),
        ],
    )
    async def test_conditionally_enables_interceptors(
        self, interceptor_name, flag, is_enabled, mocker
    ):
        mocker.patch.object(common_settings, flag, is_enabled)
        interceptors = await get_interceptors()

        result = interceptor_name in interceptors
        assert result is is_enabled

    @pytest.mark.parametrize(
        "principal_source, expected_in_interceptors",
        [
            (PrincipalSource.DISABLED, False),
            (PrincipalSource.JWT, True),
            (PrincipalSource.OPAQUE, True),
            (PrincipalSource.JWT_UNVERIFIED, True),
            (PrincipalSource.USER_INFO_SERVICE, True),
        ],
    )
    async def test_conditionally_adds_auth_interceptor(
        self, principal_source, expected_in_interceptors, mocker
    ):
        """Auth interceptor installs iff principal_source is not disabled.

        Pins the real install-gate semantics: the gate reads
        authentication_enabled(), which is principal_source != DISABLED.
        """
        from notification_common.interceptors import interceptors as interceptors_module

        mock_settings = mocker.Mock()
        mock_settings.principal_source = principal_source
        mock_settings.authentication_enabled.side_effect = (
            lambda: mock_settings.principal_source != PrincipalSource.DISABLED
        )
        mocker.patch.object(interceptors_module, "auth_settings", mock_settings)

        interceptors = await get_interceptors()
        result = "test-auth-interceptor" in interceptors
        assert result is expected_in_interceptors

    async def test_aio_server_interceptor_called_with_filter(
        self, mock_aio_server, mocker
    ):
        mocker.patch.object(common_settings, "tracing_enabled", True)

        mock_filter = mocker.MagicMock()
        mock_create_filter = mocker.patch.object(
            observability_config,
            "create_instrumentation_exclude_filter",
            return_value=mock_filter,
        )

        await get_interceptors()

        mock_create_filter.assert_called_once()
        mock_aio_server.assert_called_once_with(filter_=mock_filter)
