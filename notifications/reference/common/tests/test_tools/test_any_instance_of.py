# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest

from notification_common.rabbitmq import routing
from notification_common.test_tools.any_instance_of import AnyInstanceOf


def test_allows_asserting_mock_called_with_any_instance_of(mocker):
    spy = mocker.patch.object(routing, "routing_key")
    routing.routing_key("a", "b")
    spy.assert_called_once_with(AnyInstanceOf(str), AnyInstanceOf(str))


def test_fails_tests_if_not_called_with_correct_type(mocker):
    spy = mocker.patch.object(routing, "routing_key")
    routing.routing_key("a", "b")

    with pytest.raises(AssertionError):
        spy.assert_called_once_with(AnyInstanceOf(list), AnyInstanceOf(str))
