# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Kubernetes API client using aiohttp.

This module provides a lightweight client for interacting with the Kubernetes
API directly via REST, without requiring the kubernetes Python client library.
It uses service account credentials mounted in the pod.
"""

import logging
import ssl
from dataclasses import dataclass
from pathlib import Path

import aiohttp

logger = logging.getLogger(__name__)

# Kubernetes service account paths
_SERVICE_ACCOUNT_PATH = Path("/var/run/secrets/kubernetes.io/serviceaccount")
_TOKEN_PATH = _SERVICE_ACCOUNT_PATH / "token"
_CA_CERT_PATH = _SERVICE_ACCOUNT_PATH / "ca.crt"
_NAMESPACE_PATH = _SERVICE_ACCOUNT_PATH / "namespace"

# Kubernetes API server address (in-cluster)
_K8S_API_HOST = "https://kubernetes.default.svc"


@dataclass
class PodInfo:
    """Information about a Kubernetes pod."""

    name: str
    ip: str
    ready: bool
    phase: str
    # True when the pod carries a metadata.deletionTimestamp -- it is
    # terminating and its reported capacity must be excluded from the scaler's
    # fleet view. Defaults to False so a pod with no deletionTimestamp (the
    # common case) is treated as live.
    terminating: bool = False


class K8sClientError(Exception):
    """Exception raised for Kubernetes API errors."""

    pass


class K8sClient:
    """
    A lightweight Kubernetes API client.

    Uses the service account token mounted in the pod to authenticate
    with the Kubernetes API server.
    """

    # Refresh the token every 5 minutes to handle rotation
    _TOKEN_REFRESH_INTERVAL = 300

    # Timeout for K8s API requests (seconds)
    _REQUEST_TIMEOUT = 30

    def __init__(self):
        self._session: aiohttp.ClientSession | None = None
        self._token: str | None = None
        self._token_read_time: float = 0
        self._ssl_context: ssl.SSLContext | None = None

    async def _ensure_session(self) -> aiohttp.ClientSession:
        """Ensure we have an active aiohttp session with a fresh token."""
        import time

        now = time.time()
        token_age = now - self._token_read_time
        token_expired = token_age > self._TOKEN_REFRESH_INTERVAL

        if self._session is None or self._session.closed or token_expired:
            # Close existing session if token expired
            if self._session and not self._session.closed and token_expired:
                await self._session.close()

            self._token = self._read_token()
            self._token_read_time = now
            self._ssl_context = self._create_ssl_context()

            timeout = aiohttp.ClientTimeout(total=self._REQUEST_TIMEOUT)
            # Limit concurrent connections to avoid overwhelming the K8s API server
            connector = aiohttp.TCPConnector(ssl=self._ssl_context, limit=10)
            self._session = aiohttp.ClientSession(
                connector=connector,
                headers={"Authorization": f"Bearer {self._token}"},
                timeout=timeout,
            )

        return self._session

    @staticmethod
    def _read_token() -> str:
        """Read the service account token from the mounted path."""
        try:
            return _TOKEN_PATH.read_text().strip()
        except FileNotFoundError:
            raise K8sClientError(
                f"Service account token not found at {_TOKEN_PATH}. "
                "Are you running in a Kubernetes pod with a service account?"
            )

    @staticmethod
    def _create_ssl_context() -> ssl.SSLContext:
        """Create an SSL context with the Kubernetes CA certificate."""
        ssl_context = ssl.create_default_context()
        if _CA_CERT_PATH.exists():
            ssl_context.load_verify_locations(_CA_CERT_PATH)
        return ssl_context

    @staticmethod
    def read_namespace() -> str:
        """Read the namespace from the mounted service account path."""
        try:
            return _NAMESPACE_PATH.read_text().strip()
        except FileNotFoundError:
            raise K8sClientError(
                f"Namespace not found at {_NAMESPACE_PATH}. "
                "Are you running in a Kubernetes pod?"
            )

    async def list_pods(self, namespace: str, label_selector: str) -> list[PodInfo]:
        """
        List pods in a namespace matching a label selector.

        :param namespace: The Kubernetes namespace
        :param label_selector: Label selector string (e.g., "app=my-app")
        :return: List of PodInfo objects for ready, running pods
        """
        session = await self._ensure_session()

        url = (
            f"{_K8S_API_HOST}/api/v1/namespaces/{namespace}/pods"
            f"?labelSelector={label_selector}"
        )

        try:
            async with session.get(url) as response:
                if response.status != 200:
                    text = await response.text()
                    raise K8sClientError(
                        f"Failed to list pods: {response.status} - {text}"
                    )

                data = await response.json()
                return self._parse_pods(data)

        except TimeoutError as e:
            raise K8sClientError(f"Timeout connecting to Kubernetes API: {e}")
        except aiohttp.ClientError as e:
            raise K8sClientError(f"Failed to connect to Kubernetes API: {e}")

    @staticmethod
    def _parse_pods(data: dict) -> list[PodInfo]:
        """Parse the pod list response from Kubernetes API."""
        pods = []

        for item in data.get("items", []):
            metadata = item.get("metadata", {})
            status = item.get("status", {})

            name = metadata.get("name", "")
            pod_ip = status.get("podIP", "")
            phase = status.get("phase", "")

            # A pod with a deletionTimestamp is terminating: it stays
            # phase=Running with a podIP for its whole preStop + drain window
            # while refusing new work, so the autoscaler must not count its
            # reported capacity as usable fleet capacity.
            terminating = bool(metadata.get("deletionTimestamp"))

            # Check if pod is ready
            ready = False
            for condition in status.get("conditions", []):
                if condition.get("type") == "Ready":
                    ready = condition.get("status") == "True"
                    break

            # Only include pods with an IP address
            if pod_ip:
                pods.append(
                    PodInfo(
                        name=name,
                        ip=pod_ip,
                        ready=ready,
                        phase=phase,
                        terminating=terminating,
                    )
                )

        return pods

    async def get_deployment_replicas(
        self, namespace: str, deployment_name: str
    ) -> int:
        """
        Get the current replica count for a deployment.

        :param namespace: The Kubernetes namespace
        :param deployment_name: The name of the deployment
        :return: Current replica count
        """
        session = await self._ensure_session()

        url = (
            f"{_K8S_API_HOST}/apis/apps/v1/namespaces/{namespace}"
            f"/deployments/{deployment_name}"
        )

        try:
            async with session.get(url) as response:
                if response.status != 200:
                    text = await response.text()
                    raise K8sClientError(
                        f"Failed to get deployment: {response.status} - {text}"
                    )

                data = await response.json()
                return data.get("spec", {}).get("replicas", 0)

        except TimeoutError as e:
            raise K8sClientError(f"Timeout connecting to Kubernetes API: {e}")
        except aiohttp.ClientError as e:
            raise K8sClientError(f"Failed to connect to Kubernetes API: {e}")

    async def scale_deployment(
        self, namespace: str, deployment_name: str, replicas: int
    ) -> None:
        """
        Scale a deployment to a specified number of replicas.

        :param namespace: The Kubernetes namespace
        :param deployment_name: The name of the deployment
        :param replicas: The desired number of replicas
        """
        session = await self._ensure_session()

        url = (
            f"{_K8S_API_HOST}/apis/apps/v1/namespaces/{namespace}"
            f"/deployments/{deployment_name}/scale"
        )

        patch_data = {"spec": {"replicas": replicas}}

        headers = {"Content-Type": "application/merge-patch+json"}

        try:
            async with session.patch(url, json=patch_data, headers=headers) as response:
                if response.status not in (200, 201):
                    text = await response.text()
                    raise K8sClientError(
                        f"Failed to scale deployment: {response.status} - {text}"
                    )

                logger.info(
                    f"Scaled deployment {deployment_name} in {namespace} "
                    f"to {replicas} replicas"
                )

        except TimeoutError as e:
            raise K8sClientError(f"Timeout connecting to Kubernetes API: {e}")
        except aiohttp.ClientError as e:
            raise K8sClientError(f"Failed to connect to Kubernetes API: {e}")

    async def patch_pod_annotation(
        self, namespace: str, pod_name: str, annotation_key: str, annotation_value: str
    ) -> None:
        """
        Patch an annotation on a pod.

        :param namespace: The Kubernetes namespace
        :param pod_name: The name of the pod
        :param annotation_key: The annotation key to set
        :param annotation_value: The annotation value to set
        """
        session = await self._ensure_session()

        url = f"{_K8S_API_HOST}/api/v1/namespaces/{namespace}/pods/{pod_name}"

        patch_data = {"metadata": {"annotations": {annotation_key: annotation_value}}}

        headers = {"Content-Type": "application/merge-patch+json"}

        try:
            async with session.patch(url, json=patch_data, headers=headers) as response:
                if response.status not in (200, 201):
                    text = await response.text()
                    raise K8sClientError(
                        f"Failed to patch pod annotation: {response.status} - {text}"
                    )

                logger.debug(
                    f"Set {annotation_key}={annotation_value} on pod {pod_name}"
                )

        except TimeoutError as e:
            raise K8sClientError(f"Timeout connecting to Kubernetes API: {e}")
        except aiohttp.ClientError as e:
            raise K8sClientError(f"Failed to connect to Kubernetes API: {e}")

    async def close(self) -> None:
        """Close the aiohttp session."""
        if self._session and not self._session.closed:
            await self._session.close()
            self._session = None
