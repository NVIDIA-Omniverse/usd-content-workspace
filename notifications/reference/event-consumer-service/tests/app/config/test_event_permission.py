# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import importlib
import json
import os

import pytest

from app.config import settings
from tests.conftest import reset_env_vars


@pytest.fixture(autouse=True)
def reset_settings():
    """
    Put the settings back to normal after tests
    """
    try:
        del os.environ["OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH"]
    except KeyError:
        pass

    yield
    reset_env_vars()


def load_settings(settings_dict: dict):
    os.environ["OMNI_EVENTS_EVENT_PERMISSIONS"] = json.dumps([settings_dict])
    importlib.reload(settings)


@pytest.fixture
def event_settings():
    return {
        "event_type": "test-event",
    }


@pytest.fixture
def _mock_permissions_with_grpc_endpoint(event_settings):
    event_settings["api_type"] = "grpc"
    event_settings["action"] = "read"

    load_settings(event_settings)
    return event_settings


@pytest.fixture
def _mock_permissions_with_rest_endpoint(event_settings):
    event_settings["api_type"] = "rest"
    event_settings["action"] = "read"

    load_settings(event_settings)
    return event_settings


def test_requires_event_type(event_settings):
    del event_settings["event_type"]
    with pytest.raises(ValueError, match="Field required"):
        load_settings(event_settings)
