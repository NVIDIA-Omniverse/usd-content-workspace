# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import AsyncMock, Mock

import pytest_asyncio


@pytest_asyncio.fixture
async def mock_connection():
    """Mock RabbitMQ connection."""
    return Mock()


@pytest_asyncio.fixture
async def mock_channel_factory():
    """Mock RabbitMQ channel factory that creates fresh channels."""

    def create_mock_channel():
        channel = AsyncMock()
        channel.is_closed = False
        channel.set_qos = AsyncMock()
        channel.close = AsyncMock()
        return channel

    return create_mock_channel


@pytest_asyncio.fixture
async def mock_connection_with_factory(mock_connection, mock_channel_factory):
    """Mock connection that creates fresh channels on each call."""
    mock_connection.channel = AsyncMock(side_effect=mock_channel_factory)
    return mock_connection
