# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import grpc
from grpc_interceptor import AsyncServerInterceptor
import logging
from typing import Any, Callable, Sequence, Tuple
from notification_common.auth import auth_context
from notification_common.auth import principal_id

from notification_common.config.observability_config import EXCLUDE_METHODS
from notification_common.errors.errors import AuthenticationError
from notification_common.interceptors import metadata as interceptor_metadata
from notification_common.interceptors.metadata import (
    GrpcMetadataKey,
    GrpcMetadataValue,
)

logger = logging.getLogger(__name__)

_AUTH_HEADER_KEY = "authorization"


class AuthInterceptor(AsyncServerInterceptor):
    async def intercept(
        self,
        method: Callable,
        request_or_iterator: Any,
        context: grpc.aio.ServicerContext,
        method_name: str,
    ):
        if method_name in EXCLUDE_METHODS:
            response_or_iterator = method(request_or_iterator, context)
            if not hasattr(response_or_iterator, "__aiter__"):
                return await response_or_iterator
            return self._intercept_streaming(response_or_iterator)

        await self._set_auth_context(context)

        response_or_iterator = method(request_or_iterator, context)
        if not hasattr(response_or_iterator, "__aiter__"):
            return await response_or_iterator

        return self._intercept_streaming(response_or_iterator)

    async def _set_auth_context(self, context: grpc.aio.ServicerContext):
        """
        Extract the caller's bearer token and resolve it to a principal.

        Extracting the token proves nothing about it, so the token is also
        resolved here, through the configured ``principal_source``, before
        any handler runs. For ``jwt``, ``opaque`` and ``userInfoService``
        that resolution verifies the token; ``jwtUnverified`` decodes the
        payload without verifying it and deliberately accepts expired
        tokens.

        An unresolvable token is refused with UNAUTHENTICATED. Any other
        failure -- notably a ``ServerError`` from an unreachable identity
        provider or user-info service -- propagates, so an outage is not
        reported to callers as a bad token, with one documented residual:
        under ``jwt``, a malformed or empty JWKS document is
        indistinguishable from an unknown key id and still surfaces as an
        UNAUTHENTICATED (see ``jwt_decoder._decode_token``).
        """
        metadata = context.invocation_metadata()

        if metadata is None:
            await context.abort(grpc.StatusCode.UNAUTHENTICATED, "No metadata provided")

        token = await self._extract_auth_token(metadata, context)  # type: ignore

        auth_context.auth_token_var.set(token)

        try:
            await principal_id.principal_id()
        except AuthenticationError as e:
            logger.warning(f"Rejecting request with an unresolvable token: {e}")
            await context.abort(grpc.StatusCode.UNAUTHENTICATED, e.user_message)

    @staticmethod
    async def _intercept_streaming(iterator):
        async for r in iterator:
            yield r

    @staticmethod
    async def _extract_auth_token(
        metadata: Sequence[Tuple[GrpcMetadataKey, GrpcMetadataValue]],
        context: grpc.aio.ServicerContext,
    ) -> str:

        auth_metadata_value = interceptor_metadata.extract_metadata_value(
            metadata, _AUTH_HEADER_KEY
        )

        if auth_metadata_value is None:
            await context.abort(
                grpc.StatusCode.UNAUTHENTICATED, "No authentication token provided"
            )

        try:
            auth_metadata = interceptor_metadata.metadata_string_value(
                auth_metadata_value
            )
        except UnicodeDecodeError:
            await context.abort(
                grpc.StatusCode.UNAUTHENTICATED, "Invalid token encoding"
            )

        if not auth_metadata.startswith("Bearer "):
            await context.abort(
                grpc.StatusCode.UNAUTHENTICATED,
                "Invalid JWT format: Missing Bearer prefix",
            )

        return auth_metadata[len("Bearer "):]  # fmt: skip
