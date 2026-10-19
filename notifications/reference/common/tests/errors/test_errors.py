# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest

from notification_common.errors.errors import (
    AuthenticationError,
    NotificationBaseException,
    QueueNotFoundError,
    AuthorizationError,
    ServerError,
    UnprocessableError,
    RabbitmqShutdownError,
    NotificationValidationError,
    BadRequestError,
    ClientFacingServerError,
)


class TestNotificationBaseException:
    def test_uses_class_name_and_message_in_str_method(self):
        expected = "NotificationBaseException: test-msg"
        assert str(NotificationBaseException("test-msg")) == expected

    def test_uses_internal_message_in_str_method_if_present(self):
        expected = "NotificationBaseException: test-msg - internal-msg"
        assert str(NotificationBaseException("test-msg", "internal-msg")) == expected

    def test_has_user_message_prop(self):
        error = NotificationBaseException("test-msg", "internal-msg")
        assert error.user_message == "test-msg"

    def test_calling_status_code_on_base_class_errors(self):
        with pytest.raises(
            NotImplementedError, match="status_code must be implemented by subclass"
        ):
            # noinspection PyStatementEffect
            NotificationBaseException("").status_code


class TestQueueNotFoundError:
    def test_has_404_status_code(self):
        assert QueueNotFoundError("").status_code == 404

    def test_has_correct_user_message(self):
        assert QueueNotFoundError("").user_message == "Queue not found"

    def test_uses_internal_message_in_str(self):
        expected = "QueueNotFoundError: Queue not found - queue_name: test-name"
        assert str(QueueNotFoundError("test-name")) == expected

    def test_is_instance_notification_base(self):
        assert isinstance(QueueNotFoundError(""), NotificationBaseException)


@pytest.mark.parametrize(
    "error_type, code",
    [
        (AuthenticationError, 401),
        (AuthorizationError, 403),
        (ServerError, 500),
        (UnprocessableError, 422),
        (NotificationValidationError, 422),
        (BadRequestError, 400),
    ],
)
def test_error_types_contain_correct_status_codes(error_type, code):
    assert error_type("").status_code == code
    assert isinstance(error_type(""), NotificationBaseException)


class TestRabbitmqShutdownError:
    def test_has_500_status_code(self):
        assert RabbitmqShutdownError().status_code == 500

    def test_is_instance_notification_base(self):
        assert isinstance(RabbitmqShutdownError(), ServerError)

    def test_has_default_message(self):
        with pytest.raises(
            RabbitmqShutdownError,
            match=(
                "The service was shut down before the RabbitMQ "
                "operation could complete"
            ),
        ):
            raise RabbitmqShutdownError()


class TestServerError:
    def test_has_correct_user_message(self):
        assert ServerError("some-message").user_message == "Internal Server Error"

    def test_has_correct_internal_message(self):
        with pytest.raises(ServerError, match="Internal Server Error - some-message"):
            raise ServerError("some-message")


class TestClientFacingServerError:
    def test_has_correct_user_message(self):
        assert (
            ClientFacingServerError("some-message").user_message
            == "Internal Server Error: some-message"
        )

    def test_has_correct_internal_message(self):
        with pytest.raises(
            ClientFacingServerError,
            match="ClientFacingServerError: Internal Server Error: some-message",
        ):
            raise ClientFacingServerError("some-message")
