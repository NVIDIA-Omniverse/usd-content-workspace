# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Middleware for extracting and validating Bearer tokens from requests."""

import logging
from typing import List

from fastapi import Request
from fastapi.responses import JSONResponse
from notification_common.auth import auth_context
from notification_common.auth import principal_id
from notification_common.errors.errors import AuthenticationError

logger = logging.getLogger(__name__)


def create_auth_token_middleware(excluded_paths: List[str] | None = None):
    """
    Create an auth token middleware with configurable excluded paths.

    :param excluded_paths: List of paths to exclude from auth checks
    :return: Middleware function
    """
    if excluded_paths is None:
        excluded_paths = [
            "/api/v1/health",
            "/api/v1beta/health",
            "/docs",
            "/docs/oauth2-redirect",
            "/openapi.json",
            "/redoc",
        ]

    async def auth_token_middleware(request: Request, call_next):
        """
        Extract the Bearer token from the "Authorization" header, set it in
        context, and resolve it to a principal.

        Extracting the token proves nothing about it, so the token is also
        resolved here, through the configured ``principal_source``, before
        the request reaches a route. For ``jwt``, ``opaque`` and
        ``userInfoService`` that resolution verifies the token;
        ``jwtUnverified`` decodes the payload without verifying it and
        deliberately accepts expired tokens.

        An unresolvable token is refused with a 401. Any other failure --
        notably a ``ServerError`` from an unreachable identity provider or
        user-info service -- propagates to the error handling middleware, so
        an outage is not reported to callers as a bad token, with one
        documented residual: under ``jwt``, a malformed or empty JWKS
        document is indistinguishable from an unknown key id and still
        surfaces as a 401 (see ``jwt_decoder._decode_token``).
        """
        if request.url.path in excluded_paths:
            return await call_next(request)

        auth_header = request.headers.get("Authorization", "")

        if not auth_header.startswith("Bearer "):
            logger.warning("Authorization header does not start with Bearer")
            return JSONResponse(
                status_code=401,
                content={"detail": "Invalid Authorization header"},
            )

        token = auth_header[len("Bearer "):]  # fmt: skip

        auth_context.auth_token_var.set(token)

        try:
            await principal_id.principal_id()
        except AuthenticationError as e:
            logger.warning(f"Rejecting request with an unresolvable token: {e}")
            return JSONResponse(status_code=401, content={"detail": e.user_message})

        return await call_next(request)

    return auth_token_middleware
