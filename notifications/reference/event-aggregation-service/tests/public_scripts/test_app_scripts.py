# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import os

import pytest

from public_scripts import app_scripts


@pytest.fixture
def app_start(mocker):
    return mocker.patch.object(app_scripts, "app_start")


@pytest.fixture
def environ(mocker):
    """An ``os.environ`` restored after the test, since ``start`` writes to it."""
    return mocker.patch.dict(os.environ)


@pytest.fixture
def no_rest_port(environ):
    environ.pop("OMNI_EVENTS_REST_PORT", None)


class TestStart:
    def test_defaults_the_rest_port(self, app_start, no_rest_port):
        app_scripts.start()
        assert os.environ["OMNI_EVENTS_REST_PORT"] == "8001"

    def test_keeps_a_configured_rest_port(self, app_start, environ):
        environ["OMNI_EVENTS_REST_PORT"] = "9000"
        app_scripts.start()
        assert os.environ["OMNI_EVENTS_REST_PORT"] == "9000"

    def test_launches_the_service(self, app_start, no_rest_port):
        app_scripts.start()
        app_start.assert_called_once_with()
