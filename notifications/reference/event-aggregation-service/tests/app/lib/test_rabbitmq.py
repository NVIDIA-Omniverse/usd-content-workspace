# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from app.lib.rabbitmq import publisher
from app.lib.rabbitmq.publishing_client import PublishingClient


def test_publisher_is_instance_of_class():
    assert isinstance(publisher, PublishingClient)
