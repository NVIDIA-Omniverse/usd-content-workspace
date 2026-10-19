# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""gRPC server interceptors for cross-cutting concerns.

This module provides interceptors that handle common functionality across
all gRPC service methods, such as exception logging and error handling.
"""

import logging

import grpc
from grpc_interceptor import ServerInterceptor
from local_filesystem_service.backends.storage_backend_interface import ReadOnlyError

logger = logging.getLogger(__name__)


class ExceptionLoggingInterceptor(ServerInterceptor):
    """Interceptor that logs unhandled exceptions from gRPC methods.

    This interceptor catches all exceptions that escape from gRPC servicer
    methods, logs them with full stack traces, and converts them to appropriate
    gRPC errors. This provides a single point for exception logging without
    needing to wrap every service method.

    Expected gRPC aborts (via context.abort()) are passed through unchanged.
    Only unexpected exceptions are caught and logged.
    """

    def intercept(self, method, request, context, method_name):
        """Intercept a gRPC method call and handle exceptions.

        Args:
            method: The servicer method being called
            request: The request message
            context: The gRPC ServicerContext
            method_name: Name of the method being called

        Returns:
            The response from the method, or raises a gRPC error

        Raises:
            Exception: Expected abort from context.abort() or unexpected exceptions
        """
        try:
            return method(request, context)
        except ReadOnlyError as e:
            # Read-only rejection raised at the backend mutation boundary. Map it to
            # FAILED_PRECONDITION centrally so every mutation RPC (including multipart
            # completion, folder, and metadata mutations) is covered consistently.
            logger.info(f"Rejected mutation in {method_name}: {e}")
            context.set_code(grpc.StatusCode.FAILED_PRECONDITION)
            context.set_details(str(e))
            raise
        except Exception as e:
            # context.abort() raises a plain Exception after setting the error code
            # Check if an error code was already set on the context
            if context.code() is not None:
                # This is an expected abort - the code and details were already set
                # Just re-raise to let gRPC handle it
                raise

            # Truly unexpected exception - log with full stack trace
            logger.exception(f"Unexpected error in {method_name}: {e}")
            # Set error on context
            context.set_code(grpc.StatusCode.INTERNAL)
            context.set_details(f"Internal server error: {type(e).__name__}")
            # Re-raise to let gRPC handle it
            raise
