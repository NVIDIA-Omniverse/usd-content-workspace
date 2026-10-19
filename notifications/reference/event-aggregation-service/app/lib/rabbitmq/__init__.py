# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from notification_common.rabbitmq.publisher import Publisher

from app.lib.rabbitmq.publishing_client import PublishingClient

rabbitmq_connector = Publisher()
publisher = PublishingClient(rabbitmq_connector)

__all__ = ["rabbitmq_connector", "publisher"]
