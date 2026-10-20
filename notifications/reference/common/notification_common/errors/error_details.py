# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

from grpc import StatusCode

from pydantic import ValidationError, BaseModel

from notification_common.errors.errors import (
    NotificationBaseException,
    ServerError,
)


class ErrorDetailsModel(BaseModel):
    message: str
    status_code: int
    grpc_code: StatusCode


class ErrorDetails:
    def __init__(self, error: Exception):
        super().__init__()
        self.status_code = self._status_code(error)
        self.message = self._message(error)

    @property
    def grpc_code(self) -> StatusCode:
        """gRPC status code from http status code"""
        http_to_grpc_status = {
            400: StatusCode.INVALID_ARGUMENT,
            401: StatusCode.UNAUTHENTICATED,
            403: StatusCode.PERMISSION_DENIED,
            404: StatusCode.NOT_FOUND,
            422: StatusCode.INVALID_ARGUMENT,
            500: StatusCode.INTERNAL,
            503: StatusCode.RESOURCE_EXHAUSTED,
        }

        return http_to_grpc_status.get(self.status_code, StatusCode.UNKNOWN)

    def to_model(self):
        return ErrorDetailsModel(
            message=self.message,
            status_code=self.status_code,
            grpc_code=self.grpc_code,
        )

    def _message(self, e: Exception) -> str:
        if self.status_code == 500 and not isinstance(e, NotificationBaseException):
            return "Internal Server Error"

        """ Get the message for the error details"""
        if isinstance(e, ValidationError):
            return ErrorDetails._pydantic_friendly_error(e)

        if isinstance(e, NotificationBaseException):
            # return client_message to users, not internal_message
            return e.user_message

        return str(e)

    @staticmethod
    def _pydantic_friendly_error(error: ValidationError) -> str:
        if not isinstance(error, ValidationError):
            raise ServerError("Error must be a ValidationError")

        errors = []

        for err in error.errors():
            loc = ".".join(str(part) for part in err["loc"])
            msg = err["msg"]
            msg = f"{loc}: {msg}" if loc and loc != "0" else msg

            errors.append(msg)

        return " - ".join(errors)

    @staticmethod
    def _status_code(e: Exception) -> int:
        """Get the status code for the error details"""
        if isinstance(e, ValidationError) or isinstance(e, ValueError):
            return 400

        if isinstance(e, NotificationBaseException):
            return e.status_code

        if isinstance(e, PermissionError):
            return 403

        return 500
