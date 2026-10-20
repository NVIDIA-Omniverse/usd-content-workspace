# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import AsyncMock

import pytest
from grpc import StatusCode
from grpc_interceptor.exceptions import GrpcException

from notification_common.errors.error_details import ErrorDetails
from notification_common.errors.errors import (
    AuthenticationError,
    NotificationBaseException,
)
from notification_common.interceptors import exception_interceptor
from notification_common.interceptors.exception_interceptor import ExceptionInterceptor


@pytest.fixture(autouse=True)
def mock_logger(mocker):
    return mocker.patch.object(exception_interceptor, "logger")


@pytest.fixture
def mock_context():
    return AsyncMock()


class AbortError(Exception):
    pass


@pytest.mark.asyncio
class TestHandleException:
    @pytest.mark.parametrize(
        "error",
        [
            Exception(),
            NotificationBaseException(""),
            GrpcException(),
        ],
    )
    async def test_logs_all_exception_types(self, mock_context, mock_logger, error):
        await ExceptionInterceptor().handle_exception(
            Exception("m"), object(), mock_context, "test_method"
        )

        mock_logger.exception.assert_called_once()

    async def test_doesnt_call_abort_for_abort_error(self, mock_context, mock_logger):
        await ExceptionInterceptor().handle_exception(
            AbortError(), object(), mock_context, "test_method"
        )

        mock_context.abort.assert_not_called()

    async def test_calls_abort_for_grpc_exception(self, mock_context, mock_logger):
        await ExceptionInterceptor().handle_exception(
            GrpcException("d", StatusCode.NOT_FOUND),
            object(),
            mock_context,
            "test_method",
        )

        mock_context.abort.assert_called_once_with(StatusCode.NOT_FOUND, "d")

    class TestWhenErrorIsNotGrpcExceptionOrAbortError:
        async def test_uses_error_details_to_abort(
            self, mock_context, mock_logger, mocker
        ):
            mocker.patch.object(ErrorDetails, "grpc_code", StatusCode.UNAUTHENTICATED)
            mocker.patch.object(ErrorDetails, "_message", return_value="test-m")

            await ExceptionInterceptor().handle_exception(
                AuthenticationError("ae"), object(), mock_context, "test_method"
            )

            mock_context.abort.assert_called_once_with(
                StatusCode.UNAUTHENTICATED, "test-m"
            )
