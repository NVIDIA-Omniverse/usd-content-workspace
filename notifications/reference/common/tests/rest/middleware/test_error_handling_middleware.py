# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import json
from unittest.mock import AsyncMock, MagicMock

import pytest
import pytest_asyncio
from fastapi import HTTPException, Request
from fastapi.responses import JSONResponse

from notification_common.errors.error_details import ErrorDetails
from notification_common.rest.middleware import (
    error_handling_middleware as error_handling_middleware_module,
)


@pytest_asyncio.fixture
async def mock_request():
    mock = MagicMock(spec=Request)
    mock.method = "GET"
    mock.url = MagicMock()
    mock.url.path = "/test/path"
    return mock


@pytest_asyncio.fixture
async def mock_call_next():
    mock = AsyncMock()
    mock.return_value = JSONResponse(content={"message": "Success"}, status_code=200)
    return mock


@pytest.mark.asyncio
class TestErrorHandlingMiddleware:

    async def test_request_passes_through_if_no_exception(
        self, mock_request, mock_call_next
    ):
        response = await error_handling_middleware_module.error_handling_middleware(
            mock_request, mock_call_next
        )

        mock_call_next.assert_called_once_with(mock_request)
        assert response.status_code == 200

        body_content = json.loads(response.body)
        assert body_content == {"message": "Success"}

    async def test_http_exception_is_reraised(self, mock_request, mock_call_next):
        custom_exception = HTTPException(status_code=418, detail="I'm a teapot")
        mock_call_next.side_effect = custom_exception

        with pytest.raises(HTTPException) as exc_info:
            await error_handling_middleware_module.error_handling_middleware(
                mock_request, mock_call_next
            )

        assert exc_info.value == custom_exception

    async def test_json_response_message_contents_for_exception(
        self,
        mock_request,
        mock_call_next,
        mocker,
    ):
        test_exception = Exception("Something unexpected happened")
        mock_call_next.side_effect = test_exception

        mocked_error_detail = ErrorDetails(ValueError("Mocked Internal Error"))

        mock_get_detail = mocker.patch.object(
            error_handling_middleware_module,
            "ErrorDetails",
            return_value=mocked_error_detail,
        )

        response = await error_handling_middleware_module.error_handling_middleware(
            mock_request, mock_call_next
        )

        mock_get_detail.assert_called_once_with(test_exception)

        assert response.status_code == mocked_error_detail.status_code

        body_content = json.loads(response.body)
        assert body_content == {"detail": mocked_error_detail.message}
