# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging
import os

import aio_pika
import pytest
import pytest_asyncio
from pydantic import ValidationError

from notification_common.config.common_settings import common_settings
from notification_common.config.metadata import project_name, version_number
from notification_common.permissions.models import Action
from notification_common.test_tools.mock_auth_settings import mock_auth_settings

mock_auth_settings()

#: ``logging.getLogger`` as the stdlib defines it, captured at import time --
#: before ``before_each`` patches the name for every test.
_REAL_GET_LOGGER = logging.getLogger

os.environ["OMNI_EVENTS_PERMISSIONS_ENDPOINT"] = "test-permissions-endpoint"


# noinspection PyArgumentList
def validation_error():
    with pytest.raises(ValidationError) as e:
        Action()
    return e.value


@pytest.fixture(autouse=True)
def reset_environment():
    common_settings.permissions_endpoint = os.environ.get(
        "OMNI_EVENTS_PERMISSIONS_ENDPOINT", "test-permissions-endpoint"
    )
    common_settings.permissions_enabled = True
    yield
    common_settings.permissions_endpoint = os.environ.get(
        "OMNI_EVENTS_PERMISSIONS_ENDPOINT", "test-permissions-endpoint"
    )
    common_settings.permissions_enabled = True


@pytest.fixture
def real_get_logger(mocker):
    """
    Put the real ``logging.getLogger`` back so ``caplog`` captures anything.

    ``before_each`` replaces ``logging.getLogger`` with a MagicMock for every
    test, and pytest resolves the root logger through that name when it
    attaches -- and, through pytest 9.0, when it detaches -- the handler that
    backs ``caplog``. With the name mocked the handler goes onto a MagicMock
    instead of the real root, so nothing is captured and ``caplog.records``
    comes back empty. Up to pytest 9.0 the mismatched detach silently left the
    setup-phase handler on the real root, which is what these tests were
    reading; 9.1 removes it from the logger it was actually added to, and the
    reads come back empty.

    Request this alongside ``caplog`` in any test that asserts on log records.
    With it in place, ``caplog.set_level`` and ``caplog.at_level`` work too.
    """
    mocker.patch.object(logging, "getLogger", new=_REAL_GET_LOGGER)
    return _REAL_GET_LOGGER


@pytest.fixture
def test_token() -> str:
    return "test-auth-token"


@pytest_asyncio.fixture(autouse=True)
async def before_each(mocker):
    """
    Some things that need to be done before each test
    """
    # This is async just so that aio-pika code doesn't raise warning
    # about no existing event loop.
    mocker.patch.object(aio_pika, "connect_robust", autospec=True)

    mocker.patch.object(logging, "getLogger")

    # Clear our function caches
    version_number.cache_clear()
    project_name.cache_clear()
