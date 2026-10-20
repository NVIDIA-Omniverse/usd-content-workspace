# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import json
import os
from pathlib import Path

from dotenv import load_dotenv

# noinspection PyProtectedMember
from pydantic_settings import BaseSettings, SettingsConfigDict


class NotificationBaseSettings(BaseSettings):
    """
    Used by all of our settings classes as a base class.
    Provides some common functionality like reading secrets in NVCF
    and loading dotenv.
    """

    model_config = SettingsConfigDict(env_prefix="OMNI_EVENTS_")

    @staticmethod
    def load_dot_env():
        load_dotenv(dotenv_path=Path.cwd() / ".env")

    @staticmethod
    def read_secrets():
        """
        In NVCF secrets get put in a JSON file
        mounted into the container at /var/secrets/secrets.json
        """
        try:
            json_file = open("/var/secrets/secrets.json")
            json_dict = json.load(json_file)

            for key in json_dict:
                os.environ[key] = json_dict[key]

        except FileNotFoundError:
            # If the file isn't there, then we're not running in NVCF,
            # or we're in NVCF, and the secrets aren't set correctly,
            # in which case the pydantic code will catch that error.
            return


NotificationBaseSettings.load_dot_env()
NotificationBaseSettings.read_secrets()
