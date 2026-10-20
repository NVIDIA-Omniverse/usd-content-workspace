# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

from notification_common.config import logging_config
from notification_common.config.common_settings import common_settings


class TestConfigureLogging:
    def test_adds_otel_ids_to_log_format(self, mocker):
        mocker.patch.object(common_settings, "tracing_enabled", True)
        mock_basic_config = mocker.patch("logging.basicConfig")

        logging_config.configure_logging()

        mock_basic_config.assert_called_with(
            level="INFO",
            format=(
                f"[trace_id=%(otelTraceID)s span_id=%(otelSpanID)s] "
                f"%(asctime)s - {logging.BASIC_FORMAT}"
            ),
        )

    def test_uses_basic_format_with_timestamp(self, mocker):
        mocker.patch.object(common_settings, "tracing_enabled", False)
        mock_basic_config = mocker.patch("logging.basicConfig")

        logging_config.configure_logging()

        mock_basic_config.assert_called_once_with(
            level="INFO", format=f"%(asctime)s - {logging.BASIC_FORMAT}"
        )

    def test_disables_pika_lib_noise(self, mocker):
        mock_get_logger = mocker.patch("logging.getLogger")

        logging_config.configure_logging()

        assert mock_get_logger("aio_pika").propagate is False
        assert mock_get_logger("aiormq").propagate is False
