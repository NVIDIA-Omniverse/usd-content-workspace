# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import AsyncMock, MagicMock

import aiohttp
import pytest
import pytest_asyncio

from notification_common.scaler.metrics_client import (
    MetricsClient,
    ChannelPoolMetrics,
)


@pytest.fixture
def metrics_client():
    return MetricsClient()


@pytest_asyncio.fixture
async def cleanup_client(metrics_client):
    yield
    await metrics_client.close()


@pytest.mark.asyncio
class TestGetPodMetrics:
    async def test_returns_metrics_on_success(
        self, metrics_client, mocker, cleanup_client
    ):
        mock_response = AsyncMock()
        mock_response.status = 200
        mock_response.json = AsyncMock(
            return_value={
                "utilization_percent": 75,
                "channels_in_use": 75,
                "channels_total": 100,
            }
        )

        mock_session = MagicMock()
        mock_session.get = MagicMock(
            return_value=AsyncMock(__aenter__=AsyncMock(return_value=mock_response))
        )

        mocker.patch("aiohttp.ClientSession", return_value=mock_session)

        result = await metrics_client.get_pod_metrics("pod-1", "10.0.0.1")

        assert result == ChannelPoolMetrics(
            pod_name="pod-1",
            pod_ip="10.0.0.1",
            utilization_percent=75,
            channels_in_use=75,
            channels_total=100,
        )

    async def test_returns_none_on_non_200_response(
        self, metrics_client, mocker, cleanup_client
    ):
        mock_response = AsyncMock()
        mock_response.status = 500

        mock_session = MagicMock()
        mock_session.get = MagicMock(
            return_value=AsyncMock(__aenter__=AsyncMock(return_value=mock_response))
        )

        mocker.patch("aiohttp.ClientSession", return_value=mock_session)

        result = await metrics_client.get_pod_metrics("pod-1", "10.0.0.1")

        assert result is None

    async def test_returns_none_on_connection_error(
        self, metrics_client, mocker, cleanup_client
    ):
        mock_session = MagicMock()
        mock_session.get = MagicMock(
            return_value=AsyncMock(
                __aenter__=AsyncMock(
                    side_effect=aiohttp.ClientError("Connection refused")
                )
            )
        )

        mocker.patch("aiohttp.ClientSession", return_value=mock_session)

        result = await metrics_client.get_pod_metrics("pod-1", "10.0.0.1")

        assert result is None

    async def test_handles_missing_fields_in_response(
        self, metrics_client, mocker, cleanup_client
    ):
        mock_response = AsyncMock()
        mock_response.status = 200
        mock_response.json = AsyncMock(return_value={})

        mock_session = MagicMock()
        mock_session.get = MagicMock(
            return_value=AsyncMock(__aenter__=AsyncMock(return_value=mock_response))
        )

        mocker.patch("aiohttp.ClientSession", return_value=mock_session)

        result = await metrics_client.get_pod_metrics("pod-1", "10.0.0.1")

        assert result == ChannelPoolMetrics(
            pod_name="pod-1",
            pod_ip="10.0.0.1",
            utilization_percent=0,
            channels_in_use=0,
            channels_total=0,
        )


@pytest.mark.asyncio
class TestClose:
    async def test_closes_session(self, metrics_client, mocker):
        mock_session = MagicMock()
        mock_session.closed = False
        mock_session.close = AsyncMock()
        metrics_client._session = mock_session

        await metrics_client.close()

        mock_session.close.assert_called_once()
        assert metrics_client._session is None
