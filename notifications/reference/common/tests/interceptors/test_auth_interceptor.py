# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import pytest
import grpc
from unittest.mock import AsyncMock, MagicMock
from contextvars import ContextVar

from notification_common.interceptors.auth_interceptor import (
    AuthInterceptor,
    _AUTH_HEADER_KEY,
)
from notification_common.auth import auth_context
from notification_common.auth import principal_id
from notification_common.auth.principal_source import PrincipalSource
from notification_common.config.auth_settings import auth_settings
from notification_common.errors.errors import AuthenticationError, ServerError
from notification_common.interceptors import metadata as interceptor_metadata
from notification_common.config.observability_config import EXCLUDE_METHODS

#: The principal a resolvable token resolves to.
A_PRINCIPAL = "a-principal"

#: The client-facing message an unresolvable token is refused with.
REFUSAL_MESSAGE = "Invalid or expired JWT"


def _warnings(caplog):
    """Return the WARNING records the interceptor emitted."""
    return [
        record
        for record in caplog.records
        if record.levelno == logging.WARNING
        and record.name == "notification_common.interceptors.auth_interceptor"
    ]


@pytest.fixture
def resolve_principal(mocker):
    """
    Replace the principal resolution the interceptor performs.

    Patched on ``notification_common.auth.principal_id`` -- the module object
    the interceptor imported -- so it stays in force however the interceptor
    spells the call.
    """

    def _patch(**kwargs):
        return mocker.patch.object(
            principal_id, "principal_id", autospec=True, **kwargs
        )

    return _patch


@pytest.fixture(autouse=True)
def mock_auth_token_var(mocker):
    mock_var = MagicMock(spec=ContextVar)
    mock_var.set = MagicMock()
    mocker.patch.object(
        auth_context,
        "auth_token_var",
        new=mock_var,
    )
    return mock_var


@pytest.fixture
def interceptor():
    return AuthInterceptor()


@pytest.fixture
def mock_request():
    return MagicMock()


@pytest.fixture
def mock_unary_method():
    return AsyncMock(return_value="unary_response")


@pytest.fixture
def mock_streaming_method():
    async def mock_async_generator():
        yield "stream1"
        yield "stream2"

    method = AsyncMock(return_value=mock_async_generator())
    return method


@pytest.fixture
def mock_token():
    return "test-token"


@pytest.fixture
def mock_auth_metadata_value(mock_token):
    return f"Bearer {mock_token}"


@pytest.fixture
def mock_metadata():
    return [(_AUTH_HEADER_KEY, "Bearer test-token")]


@pytest.fixture
def mock_context(mocker, mock_metadata):
    context = mocker.MagicMock(spec=grpc.aio.ServicerContext)
    context.invocation_metadata = mocker.MagicMock()
    context.invocation_metadata.return_value = mock_metadata
    context.abort = AsyncMock(side_effect=Exception("Mock Abort Called"))
    return context


@pytest.mark.asyncio
class TestAuthInterceptor:
    class TestIntercept:
        @pytest.fixture(autouse=True)
        def mock_set_auth_context(self, mocker, interceptor):
            return mocker.patch.object(
                interceptor,
                "_set_auth_context",
                new_callable=AsyncMock,
            )

        @pytest.mark.parametrize("excluded_method_name", EXCLUDE_METHODS)
        async def test_intercept_excluded_method_unary_bypasses_auth(
            self,
            interceptor: AuthInterceptor,
            mock_unary_method: AsyncMock,
            mock_request: MagicMock,
            mock_context: MagicMock,
            mock_set_auth_context: AsyncMock,
            excluded_method_name: str,
        ):
            response = await interceptor.intercept(
                mock_unary_method, mock_request, mock_context, excluded_method_name
            )

            mock_set_auth_context.assert_not_called()
            mock_unary_method.assert_awaited_once_with(mock_request, mock_context)
            assert response == "unary_response"

        @pytest.mark.parametrize("excluded_method_name", EXCLUDE_METHODS)
        async def test_intercept_excluded_method_streaming_bypasses_auth(
            self,
            interceptor: AuthInterceptor,
            mock_streaming_method: AsyncMock,
            mock_request: MagicMock,
            mock_context: MagicMock,
            mock_set_auth_context: AsyncMock,
            excluded_method_name: str,
        ):
            iterator = await interceptor.intercept(
                mock_streaming_method, mock_request, mock_context, excluded_method_name
            )

            mock_set_auth_context.assert_not_called()
            mock_streaming_method.assert_awaited_once_with(mock_request, mock_context)

            results = []
            async for item in iterator:
                results.append(item)
            assert results == ["stream1", "stream2"]

        async def test_calls_set_auth_context(
            self,
            interceptor,
            mock_unary_method,
            mock_request,
            mock_context,
            mock_set_auth_context,
            mocker,
        ):
            await interceptor.intercept(
                mock_unary_method, mock_request, mock_context, "TestMethod"
            )
            mock_set_auth_context.assert_awaited_once_with(mock_context)

        async def test_handles_unary_response(
            self, interceptor, mock_unary_method, mock_request, mock_context, mocker
        ):
            response = await interceptor.intercept(
                mock_unary_method, mock_request, mock_context, "TestMethod"
            )
            mock_unary_method.assert_awaited_once_with(mock_request, mock_context)
            assert response == "unary_response"

        async def test_intercept_handles_streaming_response(
            self, interceptor, mock_streaming_method, mock_request, mock_context, mocker
        ):
            iterator = await interceptor.intercept(
                mock_streaming_method, mock_request, mock_context, "TestMethod"
            )
            results = []
            async for item in iterator:
                results.append(item)
            mock_streaming_method.assert_awaited_once_with(mock_request, mock_context)
            assert results == ["stream1", "stream2"]

        async def test_intercept_rejects_before_invoking_method(
            self,
            interceptor,
            mock_unary_method,
            mock_request,
            mock_context,
            mock_set_auth_context,
        ):
            """A tokenless request aborts at call entry before the wrapped
            method runs.

            _set_auth_context aborts UNAUTHENTICATED for a tokenless request;
            the abort propagates out of intercept and the wrapped method is
            never invoked.
            """
            mock_set_auth_context.side_effect = Exception("Mock Abort Called")

            with pytest.raises(Exception, match="Mock Abort Called"):
                await interceptor.intercept(
                    mock_unary_method, mock_request, mock_context, "TestMethod"
                )

            mock_set_auth_context.assert_awaited_once_with(mock_context)
            mock_unary_method.assert_not_called()

    class TestSetAuthContext:
        @pytest.fixture(autouse=True)
        def mock_extract_auth_token(self, mocker, interceptor, mock_token):
            return mocker.patch.object(
                interceptor,
                "_extract_auth_token",
                new_callable=AsyncMock,
                return_value=mock_token,
            )

        @pytest.fixture(autouse=True)
        def resolvable_token(self, resolve_principal):
            """
            Resolve every token in this class unless a row says otherwise.

            The module-level fixture swaps ``auth_context.auth_token_var`` for
            a MagicMock, so the real resolution would read an unset ContextVar
            and raise ``LookupError`` rather than exercising anything. These
            rows are about extraction and the context var; the rows about the
            resolution itself are in ``TestTokenResolution`` below, which
            re-patches this to fail.
            """
            return resolve_principal(return_value=A_PRINCIPAL)

        async def test_returns_unauthorized_no_metadata(
            self,
            interceptor,
            mock_context,
        ):
            mock_context.invocation_metadata.return_value = None

            with pytest.raises(Exception, match="Mock Abort Called"):
                await interceptor._set_auth_context(mock_context)

            mock_context.abort.assert_awaited_once_with(
                grpc.StatusCode.UNAUTHENTICATED, "No metadata provided"
            )

        async def test_calls_extract_auth_token(
            self,
            interceptor,
            mock_context,
            mock_metadata,
        ):
            await interceptor._set_auth_context(mock_context)

            interceptor._extract_auth_token.assert_awaited_once_with(
                mock_metadata, mock_context
            )

        async def test_auth_context_token_set(
            self,
            interceptor,
            mock_context,
            mock_auth_token_var,
            mock_token,
        ):
            await interceptor._set_auth_context(mock_context)

            mock_auth_token_var.set.assert_called_once_with(mock_token)

        class TestTokenResolution:
            """
            Extracting the token proves nothing about it, so the interceptor
            also resolves it -- through the configured principal source --
            before any handler runs. An unresolvable token is the caller's
            fault and is refused; anything else is ours and must not be
            reported to the caller as a bad token.
            """

            async def test_resolves_the_extracted_token(
                self, interceptor, mock_context, resolvable_token
            ):
                # assert_awaited, not assert_called: an un-awaited coroutine
                # would check nothing while still recording the call.
                await interceptor._set_auth_context(mock_context)

                resolvable_token.assert_awaited_once_with()

            async def test_does_not_abort_a_resolvable_token(
                self, interceptor, mock_context
            ):
                await interceptor._set_auth_context(mock_context)

                mock_context.abort.assert_not_called()

            class TestWhenTheTokenIsUnresolvable:
                @pytest.fixture(autouse=True)
                def unresolvable_token(self, resolvable_token):
                    # Reconfigure the mock the enclosing class installed
                    # rather than patching over it: stacking a second
                    # autospec patch on an attribute that is already a mock
                    # is not something `mock` will build a spec from.
                    resolvable_token.side_effect = AuthenticationError(REFUSAL_MESSAGE)
                    return resolvable_token

                async def test_aborts_unauthenticated(self, interceptor, mock_context):
                    with pytest.raises(Exception, match="Mock Abort Called"):
                        await interceptor._set_auth_context(mock_context)

                    mock_context.abort.assert_awaited_once_with(
                        grpc.StatusCode.UNAUTHENTICATED, REFUSAL_MESSAGE
                    )

                async def test_logs_one_warning(
                    self, interceptor, mock_context, real_get_logger, caplog
                ):
                    # ``real_get_logger`` puts the stdlib ``logging.getLogger``
                    # back for the duration; without it caplog installs its
                    # handler on a MagicMock and sees none of these records.
                    with pytest.raises(Exception, match="Mock Abort Called"):
                        await interceptor._set_auth_context(mock_context)

                    assert len(_warnings(caplog)) == 1

                async def test_logs_no_traceback(
                    self, interceptor, mock_context, real_get_logger, caplog
                ):
                    # A refused token is routine traffic, not a service fault:
                    # logger.exception here would put a stack trace in the log
                    # for every caller who sent a stale token.
                    with pytest.raises(Exception, match="Mock Abort Called"):
                        await interceptor._set_auth_context(mock_context)

                    assert _warnings(caplog)[0].exc_info is None

            class TestWhenTheResolutionFails:
                """A ServerError is an outage of ours, not a bad token."""

                @pytest.fixture(autouse=True)
                def failing_resolution(self, resolvable_token):
                    resolvable_token.side_effect = ServerError(
                        "identity provider unreachable"
                    )
                    return resolvable_token

                async def test_propagates_the_server_error(
                    self, interceptor, mock_context
                ):
                    with pytest.raises(ServerError, match="Internal Server Error"):
                        await interceptor._set_auth_context(mock_context)

                async def test_does_not_abort_unauthenticated(
                    self, interceptor, mock_context
                ):
                    # Aborting here would tell the caller their token was bad
                    # and invite them to fetch a new one, which cannot help.
                    with pytest.raises(ServerError):
                        await interceptor._set_auth_context(mock_context)

                    mock_context.abort.assert_not_called()

    @pytest.mark.asyncio
    class TestExtractAuthToken:
        @pytest.fixture(autouse=True)
        def mock_extract_metadata_value(self, mocker, mock_auth_metadata_value):
            return mocker.patch.object(
                interceptor_metadata,
                "extract_metadata_value",
                return_value=mock_auth_metadata_value,
            )

        @pytest.fixture(autouse=True)
        def mock_metadata_string_value(self, mocker, mock_auth_metadata_value):
            return mocker.patch.object(
                interceptor_metadata,
                "metadata_string_value",
                return_value=mock_auth_metadata_value,
            )

        async def test_returns_token(
            self,
            interceptor,
            mock_token,
            mock_metadata,
            mock_context,
        ):
            token = await interceptor._extract_auth_token(mock_metadata, mock_context)
            assert token == mock_token

        async def test_calls_extract_metadata_value_with_correct_args(
            self,
            interceptor,
            mock_metadata,
            mock_context,
            mock_extract_metadata_value,
        ):
            await interceptor._extract_auth_token(mock_metadata, mock_context)
            mock_extract_metadata_value.assert_called_once_with(
                mock_metadata, _AUTH_HEADER_KEY
            )

        async def test_calls_metadata_string_value_with_correct_args(
            self,
            interceptor,
            mock_auth_metadata_value,
            mock_metadata_string_value,
        ):
            await interceptor._extract_auth_token(mock_metadata, mock_context)
            mock_metadata_string_value.assert_called_once_with(mock_auth_metadata_value)

        async def test_returns_unauthenticated_if_no_auth_token(
            self,
            interceptor,
            mock_context,
            mock_extract_metadata_value,
        ):
            mock_extract_metadata_value.return_value = None

            with pytest.raises(Exception, match="Mock Abort Called"):
                await interceptor._extract_auth_token(mock_metadata, mock_context)

            mock_context.abort.assert_awaited_once_with(
                grpc.StatusCode.UNAUTHENTICATED, "No authentication token provided"
            )

        async def test_returns_unauthenticated_if_token_is_not_string(
            self,
            interceptor,
            mock_context,
            mock_metadata_string_value,
        ):
            mock_metadata_string_value.side_effect = UnicodeDecodeError(
                "utf-8", b"", 0, 0, "Mock UnicodeDecodeError"
            )

            with pytest.raises(Exception, match="Mock Abort Called"):
                await interceptor._extract_auth_token(mock_metadata, mock_context)

            mock_context.abort.assert_awaited_once_with(
                grpc.StatusCode.UNAUTHENTICATED, "Invalid token encoding"
            )

        async def test_returns_unauthenticated_if_token_is_not_bearer(
            self,
            interceptor,
            mock_context,
            mock_metadata_string_value,
        ):
            mock_metadata_string_value.return_value = "Not a Bearer token"

            with pytest.raises(Exception, match="Mock Abort Called"):
                await interceptor._extract_auth_token(mock_metadata, mock_context)

            mock_context.abort.assert_awaited_once_with(
                grpc.StatusCode.UNAUTHENTICATED,
                "Invalid JWT format: Missing Bearer prefix",
            )


@pytest.mark.asyncio
class TestTokenResolutionThroughIntercept:
    """
    The resolution's effect on the RPC, seen from ``intercept``.

    ``TestSetAuthContext`` pins what the resolution does; these pin what that
    means for the wrapped method. They deliberately leave
    ``_set_auth_context`` real, because a patched one would make "the method
    never ran" true for reasons that have nothing to do with the token.
    """

    @pytest.fixture(autouse=True)
    def mock_extract_auth_token(self, mocker, interceptor, mock_token):
        return mocker.patch.object(
            interceptor,
            "_extract_auth_token",
            new_callable=AsyncMock,
            return_value=mock_token,
        )

    async def test_a_resolvable_token_invokes_the_method(
        self,
        interceptor,
        mock_unary_method,
        mock_request,
        mock_context,
        resolve_principal,
    ):
        resolve_principal(return_value=A_PRINCIPAL)

        await interceptor.intercept(
            mock_unary_method, mock_request, mock_context, "TestMethod"
        )

        mock_unary_method.assert_awaited_once_with(mock_request, mock_context)

    async def test_an_unresolvable_token_never_invokes_the_method(
        self,
        interceptor,
        mock_unary_method,
        mock_request,
        mock_context,
        resolve_principal,
    ):
        # The ordering pin: refusing after the handler ran would still reach
        # the caller as UNAUTHENTICATED, having already done the work.
        resolve_principal(side_effect=AuthenticationError(REFUSAL_MESSAGE))

        with pytest.raises(Exception, match="Mock Abort Called"):
            await interceptor.intercept(
                mock_unary_method, mock_request, mock_context, "TestMethod"
            )

        mock_unary_method.assert_not_called()

    @pytest.mark.parametrize("excluded_method_name", EXCLUDE_METHODS)
    async def test_an_excluded_method_resolves_no_token(
        self,
        interceptor,
        mock_unary_method,
        mock_request,
        mock_context,
        resolve_principal,
        excluded_method_name,
    ):
        # Health and reflection are reachable without a token, so the
        # resolution must not run for them either -- an excluded method that
        # resolved would fail on a request that carries no token at all.
        resolve = resolve_principal(return_value=A_PRINCIPAL)

        await interceptor.intercept(
            mock_unary_method, mock_request, mock_context, excluded_method_name
        )

        resolve.assert_not_called()


@pytest.mark.asyncio
class TestWhenAuthenticationIsDisabled:
    """
    With ``principal_source`` disabled the resolution is a no-op, so a request
    that carries a well-formed token is neither resolved nor refused.

    The server only installs this interceptor when authentication is enabled,
    so the combination cannot arise through the real server build. It is
    pinned on a directly constructed interceptor because that is the whole
    point: the refusal must be gated on the setting inside the resolution, not
    on the interceptor being left out of the chain.

    Nothing is patched over ``principal_id`` or the token extraction here --
    the real resolver runs and answers None off the disabled source, reached
    through the real extraction of a real ``Bearer`` metadata value.
    """

    @pytest.fixture(autouse=True)
    def authentication_disabled(self, mocker):
        mocker.patch.object(auth_settings, "principal_source", PrincipalSource.DISABLED)

    async def test_a_bearer_token_reaches_the_handler(
        self, interceptor, mock_unary_method, mock_request, mock_context
    ):
        # An abort would raise out of intercept before this assertion, so the
        # handler having run is also the "did not refuse" assertion.
        await interceptor.intercept(
            mock_unary_method, mock_request, mock_context, "TestMethod"
        )

        mock_unary_method.assert_awaited_once_with(mock_request, mock_context)
