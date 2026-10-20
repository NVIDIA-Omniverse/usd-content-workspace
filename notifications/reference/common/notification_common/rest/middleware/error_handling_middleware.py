# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Middleware for handling exceptions and returning standardized errors."""

import logging

from fastapi import Request, HTTPException
from fastapi.responses import JSONResponse
from notification_common.errors.error_details import ErrorDetails

logger = logging.getLogger(__name__)


async def error_handling_middleware(request: Request, call_next):
    """
    Catches exceptions, returning standardized JSON error responses.
    HTTPExceptions are re-raised to be handled by FastAPI's default
    exception handlers.
    """
    try:
        return await call_next(request)
    except HTTPException as http_exc:
        # Let FastAPI handle its own HTTPExceptions
        raise http_exc
    except Exception as e:
        logger.exception(
            f"Received exception: {e} "
            f"while handling request: {request.method} {request.url.path}"
        )

        exception_details = ErrorDetails(e)

        return JSONResponse(
            status_code=exception_details.status_code,
            content={"detail": exception_details.message},
        )
