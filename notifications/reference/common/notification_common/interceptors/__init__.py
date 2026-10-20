# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""

Interceptors for gRPC services.
"""

from notification_common.interceptors.metrics.server_metrics_interceptor import (
    ServerMetricsInterceptor,
)

__all__ = ["ServerMetricsInterceptor"]
