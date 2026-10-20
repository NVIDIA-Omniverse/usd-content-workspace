# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

from grpc import ServicerContext
from grpc_interceptor import AsyncExceptionToStatusInterceptor
from grpc_interceptor.exceptions import GrpcException

from notification_common.errors.error_details import ErrorDetails

logger = logging.getLogger(__name__)


class ExceptionInterceptor(AsyncExceptionToStatusInterceptor):
    async def handle_exception(
        self,
        error: Exception,
        request_or_iterator,
        context: ServicerContext,
        method_name: str,
    ):
        # noinspection PyBroadException
        try:
            logger.exception(error)
            raise error

        except GrpcException as error:
            await context.abort(error.status_code, error.details)

        except Exception:
            if type(error).__name__ == "AbortError":
                pass
            else:
                error_details = ErrorDetails(error)
                await context.abort(error_details.grpc_code, error_details.message)
