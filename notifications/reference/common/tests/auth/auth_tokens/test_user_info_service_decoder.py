# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import json
import logging
from unittest.mock import AsyncMock, MagicMock

import aiohttp
import pytest
from aiohttp import ClientTimeout
from tenacity import wait_none

from notification_common.auth.auth_tokens import user_info_service_decoder

# noinspection PyProtectedMember
from notification_common.auth.auth_tokens.user_info_service_decoder import (
    _MAX_RESPONSE_BYTES,
    _REQUEST_TIMEOUT_SECONDS,
    _TOKEN_EXP_SECONDS,
    _USERS_ME_PATH,
    get_user_id,
)
from notification_common.config.auth_settings import auth_settings
from notification_common.errors.errors import AuthenticationError, ServerError
from notification_common.http_client.http_client import http_client

USER_INFO_URI = "https://user-info.example.com"
EXPECTED_URL = f"{USER_INFO_URI}{_USERS_ME_PATH}"

# Captured at import time, before this module's autouse ``mock_logger`` swaps
# the decoder's logger for a MagicMock. The caplog-based tests restore it.
_REAL_DECODER_LOGGER = user_info_service_decoder.logger


@pytest.fixture(autouse=True)
def mock_logger(mocker):
    return mocker.patch.object(user_info_service_decoder, "logger")


@pytest.fixture(autouse=True)
def mock_user_info_service_uri(mocker):
    """Pin the configured base URI for all tests."""
    mocker.patch.object(auth_settings, "user_info_service_uri", USER_INFO_URI)


@pytest.fixture(autouse=True)
def neutralize_retry_wait(mocker):
    """
    Keep retryable-error tests fast: zero out the tenacity backoff wait on the
    decorated coroutine so no real sleeping happens between attempts.
    """
    mocker.patch.object(
        user_info_service_decoder._fetch_userinfo_with_retry.retry,
        "wait",
        new=wait_none(),
    )


def _make_response(
    *,
    status: int = 200,
    body: bytes = b'{"id": "user-from-decoder"}',
    content_length: int | None = None,
    raise_status_error: int | None = None,
    chunk_size: int = 8192,
) -> AsyncMock:
    """
    Build an AsyncMock ClientResponse whose ``content.iter_chunked`` streams the
    given body and whose ``raise_for_status`` optionally raises.
    """
    response = AsyncMock(spec=aiohttp.ClientResponse)
    response.status = status
    response.content_length = len(body) if content_length is None else content_length

    if raise_status_error is not None:
        # noinspection PyTypeChecker
        response.raise_for_status = MagicMock(
            side_effect=aiohttp.ClientResponseError(
                request_info=MagicMock(),
                history=(),
                status=raise_status_error,
                message=f"{raise_status_error} Error",
            )
        )
    else:
        response.raise_for_status = MagicMock()

    async def _iter_chunked(size):
        for start in range(0, len(body), size):
            end = start + size
            yield body[start:end]

    content = MagicMock()
    content.iter_chunked = lambda size: _iter_chunked(size)
    response.content = content
    return response


def _request_ctx(response: AsyncMock) -> AsyncMock:
    ctx = AsyncMock()
    ctx.__aenter__.return_value = response
    ctx.__aexit__.return_value = None
    return ctx


def _client_error_ctx(error: BaseException) -> AsyncMock:
    ctx = AsyncMock()
    ctx.__aenter__.side_effect = error
    ctx.__aexit__.return_value = None
    return ctx


@pytest.fixture
def mock_session(mocker):
    session = AsyncMock(spec=aiohttp.ClientSession)
    session.closed = False
    mocker.patch.object(http_client, "session", new=session)
    return session


def _assert_get_call(
    mock_session: AsyncMock, *, expected_url: str, expected_headers: dict
) -> None:
    """
    Assert the single ``session.get`` call forwarded the expected positional URL
    and ``headers``, and that an explicit per-request ``timeout`` was passed as a
    ``ClientTimeout`` whose total is bound to the source constant (so the
    hardening stays regression-proof without hardcoding the magnitude here).
    """
    assert mock_session.get.call_count == 1
    args, kwargs = mock_session.get.call_args
    assert args == (expected_url,)
    assert kwargs["headers"] == expected_headers

    timeout = kwargs["timeout"]
    assert isinstance(timeout, ClientTimeout)
    assert timeout.total == _REQUEST_TIMEOUT_SECONDS


@pytest.mark.asyncio
class TestBearerForwarding:
    async def test_forwards_bearer_token_to_configured_users_me_uri(
        self, mock_session: AsyncMock, test_token: str
    ):
        mock_session.get.return_value = _request_ctx(_make_response())

        await get_user_id(test_token)

        _assert_get_call(
            mock_session,
            expected_url=EXPECTED_URL,
            expected_headers={"Authorization": f"Bearer {test_token}"},
        )

    async def test_calls_only_the_configured_uri_no_fallback(
        self, mock_session: AsyncMock, test_token: str
    ):
        mock_session.get.return_value = _request_ctx(_make_response())

        await get_user_id(test_token)

        assert mock_session.get.call_count == 1

    async def test_passes_explicit_per_request_timeout(
        self, mock_session: AsyncMock, test_token: str
    ):
        # A hung backend must not stall a request indefinitely: every request
        # carries an explicit ClientTimeout bound to the source constant.
        mock_session.get.return_value = _request_ctx(_make_response())

        await get_user_id(test_token)

        _, kwargs = mock_session.get.call_args
        timeout = kwargs["timeout"]
        assert isinstance(timeout, ClientTimeout)
        assert timeout.total == _REQUEST_TIMEOUT_SECONDS

    async def test_trims_trailing_slash_from_configured_uri(
        self, mocker, mock_session: AsyncMock, test_token: str
    ):
        mocker.patch.object(auth_settings, "user_info_service_uri", USER_INFO_URI + "/")
        mock_session.get.return_value = _request_ctx(_make_response())

        await get_user_id(test_token)

        _assert_get_call(
            mock_session,
            expected_url=EXPECTED_URL,
            expected_headers={"Authorization": f"Bearer {test_token}"},
        )


@pytest.mark.asyncio
class TestPrincipalExtraction:
    async def test_returns_id_field_as_principal(
        self, mock_session: AsyncMock, test_token: str, test_user_id: str
    ):
        body = json.dumps({"id": test_user_id}).encode()
        mock_session.get.return_value = _request_ctx(_make_response(body=body))

        result = await get_user_id(test_token)

        assert result is not None
        assert result[0] == test_user_id

    async def test_returns_expiry_300_seconds_out(
        self, mocker, mock_session: AsyncMock, test_token: str, test_user_id: str
    ):
        mocker.patch.object(user_info_service_decoder.time, "time", return_value=1000.0)
        body = json.dumps({"id": test_user_id}).encode()
        mock_session.get.return_value = _request_ctx(_make_response(body=body))

        result = await get_user_id(test_token)

        assert result[1] == 1000.0 + _TOKEN_EXP_SECONDS

    async def test_ignores_decoy_fields_and_uses_id_only(
        self, mock_session: AsyncMock, test_token: str, test_user_id: str
    ):
        # A body carrying sub/oid decoys must not override the load-bearing id.
        body = json.dumps(
            {
                "id": test_user_id,
                "sub": "should-not-be-used",
                "oid": "should-not-be-used-either",
            }
        ).encode()
        mock_session.get.return_value = _request_ctx(_make_response(body=body))

        result = await get_user_id(test_token)

        assert result[0] == test_user_id

    async def test_missing_id_field_returns_none(
        self, mock_session: AsyncMock, test_token: str
    ):
        body = json.dumps({"sub": "no-id-here", "name": "Test User"}).encode()
        mock_session.get.return_value = _request_ctx(_make_response(body=body))

        result = await get_user_id(test_token)

        assert result is None


@pytest.mark.asyncio
class TestResponseReadCap:
    async def test_cap_constant_is_64_kib(self):
        assert _MAX_RESPONSE_BYTES == 64 * 1024

    async def test_content_length_over_cap_raises_server_error_unparsed(
        self, mock_session: AsyncMock, test_token: str
    ):
        # An oversized advertised Content-Length is rejected before any parse.
        response = _make_response(content_length=_MAX_RESPONSE_BYTES + 1)
        mock_session.get.return_value = _request_ctx(response)

        with pytest.raises(ServerError):
            await get_user_id(test_token)

    async def test_streamed_body_over_cap_raises_server_error(
        self, mock_session: AsyncMock, test_token: str
    ):
        # Chunked transfer (no Content-Length) that streams past the cap is
        # rejected by the streaming primitive, not the header fast-path.
        oversized = b"x" * (_MAX_RESPONSE_BYTES + 100)
        response = _make_response(body=oversized, content_length=None, chunk_size=8192)
        response.content_length = None
        mock_session.get.return_value = _request_ctx(response)

        with pytest.raises(ServerError):
            await get_user_id(test_token)


@pytest.mark.asyncio
class TestStatusErrorMapping:
    @pytest.mark.parametrize("status", [401, 403])
    async def test_auth_statuses_raise_authentication_error_no_retry(
        self, mock_session: AsyncMock, test_token: str, status: int
    ):
        response = _make_response(raise_status_error=status)
        mock_session.get.return_value = _request_ctx(response)

        with pytest.raises(AuthenticationError):
            await get_user_id(test_token)

        # 401/403 are non-retryable: exactly one call.
        assert mock_session.get.call_count == 1

    @pytest.mark.parametrize("status", [429, 500, 503])
    async def test_server_statuses_raise_server_error_after_retries(
        self, mock_session: AsyncMock, test_token: str, status: int
    ):
        mock_session.get.side_effect = [
            _request_ctx(_make_response(raise_status_error=status)) for _ in range(4)
        ]

        with pytest.raises(ServerError):
            await get_user_id(test_token)

        # 429/5xx are retryable up to stop_after_attempt(4).
        assert mock_session.get.call_count == 4

    @pytest.mark.parametrize(
        "error",
        [
            aiohttp.ClientConnectionError("connect refused"),
            aiohttp.ServerTimeoutError("timed out"),
        ],
        ids=["connect_error", "timeout"],
    )
    async def test_transport_errors_raise_server_error_after_retries(
        self, mock_session: AsyncMock, test_token: str, error: BaseException
    ):
        mock_session.get.side_effect = [_client_error_ctx(error) for _ in range(4)]

        with pytest.raises(ServerError):
            await get_user_id(test_token)

        assert mock_session.get.call_count == 4

    async def test_malformed_body_raises_server_error(
        self, mock_session: AsyncMock, test_token: str
    ):
        response = _make_response(body=b"not json{{{")
        mock_session.get.return_value = _request_ctx(response)

        with pytest.raises(ServerError):
            await get_user_id(test_token)


@pytest.mark.asyncio
class TestNoTokenLeakage:
    async def test_auth_error_message_omits_token(
        self, mock_session: AsyncMock, test_token: str
    ):
        response = _make_response(raise_status_error=401)
        mock_session.get.return_value = _request_ctx(response)

        with pytest.raises(AuthenticationError) as exc_info:
            await get_user_id(test_token)

        assert test_token not in str(exc_info.value)

    @pytest.mark.parametrize(
        "error",
        [
            aiohttp.ClientConnectionError("connect refused"),
            aiohttp.ServerTimeoutError("timed out"),
        ],
        ids=["connect_error", "timeout"],
    )
    async def test_unreachable_error_message_omits_token(
        self, mock_session: AsyncMock, test_token: str, error: BaseException
    ):
        mock_session.get.side_effect = [_client_error_ctx(error) for _ in range(4)]

        with pytest.raises(ServerError) as exc_info:
            await get_user_id(test_token)

        assert test_token not in str(exc_info.value)

    async def test_server_status_error_message_omits_token(
        self, mock_session: AsyncMock, test_token: str
    ):
        mock_session.get.side_effect = [
            _request_ctx(_make_response(raise_status_error=503)) for _ in range(4)
        ]

        with pytest.raises(ServerError) as exc_info:
            await get_user_id(test_token)

        assert test_token not in str(exc_info.value)


@pytest.mark.asyncio
class TestNoTokenLeakageInLogs:
    """
    The error paths emit WARNING log records (status / unreachable). Assert the
    emitted records never carry the bearer token, an Authorization/Bearer header
    value, or the full request URL — so a log scrape can't recover secrets.
    """

    @pytest.fixture(autouse=True)
    def real_logger(self, mocker, real_get_logger):
        # Two autouse fixtures sabotage log capture here: the conftest
        # ``before_each`` patches ``logging.getLogger`` (undone by
        # ``real_get_logger``), and this module's ``mock_logger`` swaps the
        # decoder's logger for a MagicMock. Restore the real named logger too so
        # caplog can capture the records the decoder emits on its error paths.
        mocker.patch.object(
            user_info_service_decoder, "logger", new=_REAL_DECODER_LOGGER
        )
        return _REAL_DECODER_LOGGER

    def _assert_no_secrets(self, caplog, test_token: str) -> None:
        assert caplog.records, "expected at least one WARNING record"
        for record in caplog.records:
            message = record.getMessage()
            assert test_token not in message
            assert f"Bearer {test_token}" not in message
            assert "Authorization" not in message
            assert EXPECTED_URL not in message
        # caplog.text folds in the logger name / level formatting too.
        assert test_token not in caplog.text
        assert "Bearer " not in caplog.text
        assert "Authorization" not in caplog.text
        assert EXPECTED_URL not in caplog.text

    @pytest.mark.parametrize("status", [401, 403])
    async def test_auth_error_path_logs_no_secrets(
        self,
        caplog,
        real_logger,
        mock_session: AsyncMock,
        test_token: str,
        status: int,
    ):
        caplog.set_level(logging.WARNING, logger=real_logger.name)
        response = _make_response(raise_status_error=status)
        mock_session.get.return_value = _request_ctx(response)

        with pytest.raises(AuthenticationError):
            await get_user_id(test_token)

        # The emitted record is the status-only "returned HTTP %s" warning.
        assert any("returned HTTP" in r.getMessage() for r in caplog.records)
        self._assert_no_secrets(caplog, test_token)

    @pytest.mark.parametrize(
        "error",
        [
            aiohttp.ClientConnectionError("connect refused"),
            aiohttp.ServerTimeoutError("timed out"),
        ],
        ids=["connect_error", "timeout"],
    )
    async def test_unreachable_path_logs_no_secrets(
        self,
        caplog,
        real_logger,
        mock_session: AsyncMock,
        test_token: str,
        error: BaseException,
    ):
        caplog.set_level(logging.WARNING, logger=real_logger.name)
        mock_session.get.side_effect = [_client_error_ctx(error) for _ in range(4)]

        with pytest.raises(ServerError):
            await get_user_id(test_token)

        # The unreachable path emits "user-info service unreachable: %s".
        assert any("unreachable" in r.getMessage() for r in caplog.records)
        self._assert_no_secrets(caplog, test_token)


@pytest.mark.asyncio
class TestHttpClientNotInitialized:
    async def test_uninitialized_session_raises_server_error(
        self, mocker, test_token: str
    ):
        mocker.patch.object(http_client, "session", new=None)

        with pytest.raises(ServerError):
            await get_user_id(test_token)
