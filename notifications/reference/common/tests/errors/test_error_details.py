# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from grpc import StatusCode
from pydantic import ValidationError, BaseModel, model_validator

from notification_common.errors.error_details import ErrorDetails, ErrorDetailsModel
from notification_common.errors.errors import (
    AuthorizationError,
    AuthenticationError,
    QueueNotFoundError,
    ChannelNotAvailableError,
    ServerError,
    ClientFacingServerError,
)
from tests.conftest import validation_error


class TestInit:
    def test_creates_instance_with_correct_code(self, mocker):
        mocker.patch.object(ErrorDetails, "_status_code", return_value=100)
        assert ErrorDetails(Exception()).status_code == 100

    def test_creates_instance_with_correct_message(self, mocker):
        mocker.patch.object(ErrorDetails, "_message", return_value="test-message")
        assert ErrorDetails(Exception()).message == "test-message"

    def test_sets_status_before_msg_and_doesnt_raise_attribute_error(self):
        ErrorDetails(Exception())


class TestGrpcCode:
    @pytest.mark.parametrize(
        "http_code, expected",
        [
            (400, StatusCode.INVALID_ARGUMENT),
            (401, StatusCode.UNAUTHENTICATED),
            (403, StatusCode.PERMISSION_DENIED),
            (404, StatusCode.NOT_FOUND),
            (500, StatusCode.INTERNAL),
            (503, StatusCode.RESOURCE_EXHAUSTED),
            (888, StatusCode.UNKNOWN),
        ],
    )
    def test_returns_correct_grpc_code_for_http_code(self, http_code, expected, mocker):
        mocker.patch.object(ErrorDetails, "_status_code", return_value=http_code)
        mocker.patch.object(ErrorDetails, "_message", return_value="")

        assert ErrorDetails(Exception()).grpc_code == expected


class TestToModel:
    def test_returns_model_instance_with_correct_data(self):
        model = ErrorDetails(QueueNotFoundError("queue-name")).to_model()

        assert model == ErrorDetailsModel(
            message="Queue not found",
            status_code=404,
            grpc_code=StatusCode.NOT_FOUND,
        )


class TestMessage:
    @pytest.mark.parametrize(
        "error, message",
        [
            (QueueNotFoundError("queue-name"), "Queue not found"),
            (AuthorizationError("test-message", "internal-message"), "test-message"),
            (validation_error(), "service: Field required - name: Field required"),
            (ServerError("internal-message"), "Internal Server Error"),
            (
                ClientFacingServerError("user-message"),
                "Internal Server Error: user-message",
            ),
            (ValueError("test-message"), "test-message"),
            (PermissionError("test-message"), "test-message"),
            (KeyError("m1", "m2"), "Internal Server Error"),
        ],
    )
    def test_returns_correct_message_for_exception_typ(self, error, message):
        assert ErrorDetails(error).message == message


class TestPydanticFriendlyError:
    def test_creates_user_friendly_error_message(self):
        expected = "service: Field required - name: Field required"

        assert ErrorDetails._pydantic_friendly_error(validation_error()) == expected

    def test_handles_case_where_error_doesnt_have_loc_field(self):
        class TestModel(BaseModel):
            @model_validator(mode="after")
            def test(self):
                raise ValueError("test-err")

        with pytest.raises(ValidationError) as e:
            TestModel()

        assert ErrorDetails._pydantic_friendly_error(e.value) == "Value error, test-err"


class TestStatusCode:
    @pytest.mark.parametrize(
        "error, code",
        [
            (validation_error(), 400),
            (ValueError("m"), 400),
            (AuthenticationError("m", "im"), 401),
            (AuthorizationError("m", "im"), 403),
            (PermissionError("test-message"), 403),
            (QueueNotFoundError("qn"), 404),
            (KeyError("m1", "m2"), 500),
            (ChannelNotAvailableError(), 503),
        ],
    )
    def test_returns_correct_code_based_on_exception(self, error, code):
        assert ErrorDetails._status_code(error) == code
