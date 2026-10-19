# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from opentelemetry import metrics
from opentelemetry.instrumentation import grpc as otel_grpc_instrumentation

from notification_common.config.auth_settings import auth_settings
from notification_common.config.common_settings import common_settings
from notification_common.config.metadata import project_name
from notification_common.config import observability_config
from notification_common.interceptors import (
    auth_interceptor,
    exception_interceptor,
)
from notification_common.interceptors.metrics import server_metrics_interceptor


async def get_interceptors():
    interceptors = []

    if common_settings.tracing_enabled:
        interceptors.append(
            otel_grpc_instrumentation.aio_server_interceptor(
                filter_=observability_config.create_instrumentation_exclude_filter()
            )
        )

    if common_settings.metrics_enabled:
        meter = metrics.get_meter_provider().get_meter(project_name())
        interceptors.append(server_metrics_interceptor.ServerMetricsInterceptor(meter))

    interceptors.append(exception_interceptor.ExceptionInterceptor())

    # Only add auth interceptor if authentication is enabled
    if auth_settings.authentication_enabled():
        interceptors.append(auth_interceptor.AuthInterceptor())

    return interceptors
