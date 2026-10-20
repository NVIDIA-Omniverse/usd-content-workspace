# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import datetime
import json

from notification_common.errors.errors import BadRequestError
from notification_common.storage.event_types import is_storage_event


class OmniEvent:
    def __init__(
        self,
        event_type: str,
        occurred_at: str,
        resource_id: str,
        message: dict,
        principal_identity: str | None = None,
    ):
        self.event_type = event_type
        self.occurred_at = occurred_at
        self.resource_id = resource_id or ""
        self.message = message
        self.principal_identity = principal_identity
        super().__init__()

        self._validate()

    def encoded_for_publish(self) -> bytes:
        return str(self).encode("utf-8")

    def _validate(self):
        if not self.occurred_at:
            raise BadRequestError("occurred_at must be specified")

        if not self.message:
            raise BadRequestError("message must be specified")

        if not self.event_type:
            raise BadRequestError("event_type must be specified")

        if is_storage_event(self.event_type) and not self.resource_id:
            raise BadRequestError("resource_id must be specified for storage events")

    def __str__(self) -> str:
        """
        Build a string representation of an event for publishing to rabbitmq.
        """
        published_at = (
            datetime.datetime.now(datetime.UTC).isoformat().replace("+00:00", "Z")
        )

        data = {
            "event_type": self.event_type,
            "occurred_at": self.occurred_at,
            "resource_id": self.resource_id,
            "message": self.message,
            "published_at": published_at,
        }

        if self.principal_identity:
            data["principal_identity"] = self.principal_identity

        return json.dumps(data)
