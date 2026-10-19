# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

from notification_common.config.common_settings import common_settings


def configure_logging():
    logging.getLogger("aio_pika").propagate = False
    logging.getLogger("aiormq").propagate = False

    log_fmt = "%(asctime)s - " + logging.BASIC_FORMAT
    # trace_id/span_id are only populated when tracing is active.
    if common_settings.tracing_enabled:
        log_fmt = f"[trace_id=%(otelTraceID)s span_id=%(otelSpanID)s] {log_fmt}"

    logging.basicConfig(
        level=common_settings.log_level,
        format=log_fmt,
    )
