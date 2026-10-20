# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Client for fetching channel pool metrics from service pods.
"""

import logging
from dataclasses import dataclass

import aiohttp

from notification_common.scaler.scaler_settings import scaler_settings

logger = logging.getLogger(__name__)


@dataclass
class ChannelPoolMetrics:
    """Metrics about a pod's channel pool utilization."""

    pod_name: str
    pod_ip: str
    utilization_percent: int
    channels_in_use: int
    channels_total: int


class MetricsClient:
    """
    Client for fetching channel pool metrics from pods.
    """

    def __init__(self):
        self._session: aiohttp.ClientSession | None = None

    async def _ensure_session(self) -> aiohttp.ClientSession:
        """Ensure we have an active aiohttp session."""
        if self._session is None or self._session.closed:
            timeout = aiohttp.ClientTimeout(
                total=scaler_settings.scaler_metrics_timeout_seconds
            )
            self._session = aiohttp.ClientSession(timeout=timeout)
        return self._session

    async def get_pod_metrics(
        self, pod_name: str, pod_ip: str
    ) -> ChannelPoolMetrics | None:
        """
        Fetch channel pool metrics from a single pod.

        :param pod_name: The name of the pod (for logging)
        :param pod_ip: The IP address of the pod
        :return: ChannelPoolMetrics if successful, None if the request fails
        """
        session = await self._ensure_session()

        url = (
            f"http://{pod_ip}:{scaler_settings.scaler_metrics_port}"
            f"{scaler_settings.scaler_metrics_path}"
        )

        try:
            async with session.get(url) as response:
                if response.status != 200:
                    logger.warning(
                        f"Failed to get metrics from pod {pod_name} ({pod_ip}): "
                        f"HTTP {response.status}"
                    )
                    return None

                data = await response.json()

                return ChannelPoolMetrics(
                    pod_name=pod_name,
                    pod_ip=pod_ip,
                    utilization_percent=data.get("utilization_percent", 0),
                    channels_in_use=data.get("channels_in_use", 0),
                    channels_total=data.get("channels_total", 0),
                )

        except aiohttp.ClientError as e:
            logger.warning(f"Failed to connect to pod {pod_name} ({pod_ip}): {e}")
            return None
        except Exception as e:
            logger.warning(f"Error getting metrics from pod {pod_name} ({pod_ip}): {e}")
            return None

    async def close(self) -> None:
        """Close the aiohttp session."""
        if self._session and not self._session.closed:
            await self._session.close()
            self._session = None
