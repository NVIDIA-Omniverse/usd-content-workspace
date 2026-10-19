# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from notification_common.config import common_settings
from notification_common.errors.errors import ServerError
from pydantic import Field, model_validator
from pydantic_settings import SettingsConfigDict


class Settings(common_settings.CommonSettings):
    """
    Settings for the App.
    NOTE: all environment variables will be prefixed
          with "OMNI_EVENTS_"
    """

    grpc_port: str = "[::]:50051"
    rest_api_enabled: bool = True

    # The largest REST request body accepted, in bytes (default 4 MiB).
    # Deliberately well under RabbitMQ's 16 MiB `max_message_size` default,
    # because the published message is not the request body: OmniEvent
    # re-serializes it with `json.dumps` (ensure_ascii=True) and adds
    # `published_at` and `principal_identity`, so a 3-byte UTF-8 character
    # becomes 6 bytes and a 4-byte astral character becomes 12 -- a 4 MiB body
    # can publish as roughly 12 MiB. It is operator-settable because the safe
    # value depends on the broker's configured `max_message_size`. 4 MiB also
    # matches gRPC's default inbound cap (`grpc.max_receive_message_length`),
    # which this repo does not currently pin, so the two transports behave
    # alike out of the box. Read once at import, so a change needs a restart.
    rest_max_request_bytes: int = Field(default=4 * 1024 * 1024, gt=0)

    model_config = SettingsConfigDict(
        env_prefix=common_settings.CommonSettings.model_config["env_prefix"]
    )

    @model_validator(mode="after")
    def validate_auth_settings(self):
        """
        Validate authorization configuration is consistent.

        The permission service gate is independent: a disabled gate's
        endpoint is inert (no both-or-neither coupling). When the gate is
        enabled its endpoint must be set, since the permissions HTTP client
        otherwise crashes the publish path on a None endpoint.
        """
        if self.permissions_enabled and not self.permissions_endpoint:
            raise ServerError(
                "permissions_enabled is true but permissions_endpoint is "
                "not configured. Set permissions_endpoint, or disable the "
                "permission service gate (permissions_enabled=false)."
            )

        return self


settings = Settings()
