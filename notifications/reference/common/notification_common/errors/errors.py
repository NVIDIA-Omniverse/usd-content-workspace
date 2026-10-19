# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0


class NotificationBaseException(Exception):
    """
    We want to be able to have a user-friendly message
    as well as an internal message that will only be logged

    :param client_message: The user-friendly message.
    :param internal_message: A message that will only be logged.
    """

    def __init__(
        self,
        client_message: str,
        internal_message: str | None = None,
    ):
        self.user_message = client_message

        if not internal_message:
            message = self.user_message
        else:
            message = f"{client_message} - {internal_message}"

        super().__init__(message)

    @property
    def status_code(self) -> int:
        raise NotImplementedError("status_code must be implemented by subclass")

    def __str__(self) -> str:
        return f"{self.__class__.__name__}: {super().__str__()}"


class QueueNotFoundError(NotificationBaseException):
    """Raised when a specific queue is not found."""

    status_code = 404

    def __init__(self, queue_name: str):
        super().__init__(
            "Queue not found", internal_message=f"queue_name: {queue_name}"
        )


class ChannelNotAvailableError(NotificationBaseException):
    """Raised when the channel pool is exhausted."""

    status_code = 503

    def __init__(self):
        super().__init__(
            "Channel pool is exhausted",
        )


class AuthenticationError(NotificationBaseException):
    status_code = 401
    pass


class AuthorizationError(NotificationBaseException):
    status_code = 403
    pass


class BadRequestError(NotificationBaseException):
    status_code = 400


class ServerError(NotificationBaseException):
    status_code = 500

    def __init__(self, message: str = None):
        super().__init__("Internal Server Error", message)


class ClientFacingServerError(NotificationBaseException):
    status_code = 500

    def __init__(self, message: str = None):
        super().__init__(f"Internal Server Error: {message}")


class RabbitmqShutdownError(ServerError):
    def __init__(self):
        super().__init__(
            "The service was shut down before the RabbitMQ operation could complete"
        )


class UnprocessableError(NotificationBaseException):
    status_code = 422
    pass


NotificationValidationError = UnprocessableError
