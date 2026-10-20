# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import time
from unittest.mock import AsyncMock, MagicMock

import pytest
import pytest_asyncio

from notification_common.scaler.k8s_client import (
    K8sClient,
    K8sClientError,
    PodInfo,
)


@pytest.fixture
def k8s_client():
    return K8sClient()


@pytest_asyncio.fixture
async def cleanup_client(k8s_client):
    yield
    await k8s_client.close()


@pytest.fixture
def mock_k8s_session(k8s_client, mocker):
    """
    Fixture that patches the K8s client to use a mock session.

    Returns a factory function that creates mock sessions with the specified
    HTTP method (get/patch), response, or side_effect for errors.
    """

    def _create_mock_session(
        method: str = "get",
        status: int = 200,
        json_data: dict | None = None,
        text_data: str | None = None,
        side_effect: Exception | None = None,
    ) -> MagicMock:
        mock_session = MagicMock()

        if side_effect:
            # For timeout or other errors
            mock_method = MagicMock(
                return_value=AsyncMock(__aenter__=AsyncMock(side_effect=side_effect))
            )
        else:
            # For normal responses
            mock_response = AsyncMock()
            mock_response.status = status
            if json_data is not None:
                mock_response.json = AsyncMock(return_value=json_data)
            if text_data is not None:
                mock_response.text = AsyncMock(return_value=text_data)
            mock_method = MagicMock(
                return_value=AsyncMock(__aenter__=AsyncMock(return_value=mock_response))
            )

        setattr(mock_session, method, mock_method)

        mocker.patch.object(k8s_client, "_read_token", return_value="test-token")
        mocker.patch.object(k8s_client, "_create_ssl_context", return_value=None)
        mocker.patch("aiohttp.ClientSession", return_value=mock_session)

        return mock_session

    return _create_mock_session


class TestReadToken:
    def test_raises_error_when_token_file_not_found(self, mocker, tmp_path):
        non_existent_path = tmp_path / "nonexistent" / "token"
        mocker.patch(
            "notification_common.scaler.k8s_client._TOKEN_PATH",
            non_existent_path,
        )

        with pytest.raises(K8sClientError) as exc:
            K8sClient._read_token()

        assert "Service account token not found" in str(exc.value)


class TestReadNamespace:
    def test_returns_namespace_from_file(self, mocker, tmp_path):
        namespace_file = tmp_path / "namespace"
        namespace_file.write_text("my-namespace\n")
        mocker.patch(
            "notification_common.scaler.k8s_client._NAMESPACE_PATH",
            namespace_file,
        )

        result = K8sClient.read_namespace()

        assert result == "my-namespace"

    def test_raises_error_when_namespace_file_not_found(self, mocker, tmp_path):
        non_existent_path = tmp_path / "nonexistent" / "namespace"
        mocker.patch(
            "notification_common.scaler.k8s_client._NAMESPACE_PATH",
            non_existent_path,
        )

        with pytest.raises(K8sClientError) as exc:
            K8sClient.read_namespace()

        assert "Namespace not found" in str(exc.value)


class TestParsePods:
    def test_parses_pods_correctly(self):
        data = {
            "items": [
                {
                    "metadata": {"name": "pod-1"},
                    "status": {
                        "podIP": "10.0.0.1",
                        "phase": "Running",
                        "conditions": [{"type": "Ready", "status": "True"}],
                    },
                },
                {
                    "metadata": {"name": "pod-2"},
                    "status": {
                        "podIP": "10.0.0.2",
                        "phase": "Running",
                        "conditions": [{"type": "Ready", "status": "False"}],
                    },
                },
            ]
        }

        result = K8sClient._parse_pods(data)

        assert len(result) == 2
        assert result[0] == PodInfo(
            name="pod-1", ip="10.0.0.1", ready=True, phase="Running"
        )
        assert result[1] == PodInfo(
            name="pod-2", ip="10.0.0.2", ready=False, phase="Running"
        )

    def test_skips_pods_without_ip(self):
        data = {
            "items": [
                {
                    "metadata": {"name": "pod-1"},
                    "status": {
                        "podIP": "",
                        "phase": "Pending",
                        "conditions": [],
                    },
                },
            ]
        }

        result = K8sClient._parse_pods(data)

        assert len(result) == 0

    def test_handles_empty_items(self):
        data = {"items": []}

        result = K8sClient._parse_pods(data)

        assert len(result) == 0

    def test_marks_pod_terminating_when_deletion_timestamp_present(self):
        # A pod on SIGTERM keeps phase=Running with a podIP for its whole
        # preStop + drain window; its only distinguishing signal is the
        # metadata.deletionTimestamp the API stamps on deletion. That must set
        # PodInfo.terminating so the scaler can drop it from the capacity sum.
        data = {
            "items": [
                {
                    "metadata": {
                        "name": "pod-1",
                        "deletionTimestamp": "2026-08-19T12:00:00Z",
                    },
                    "status": {
                        "podIP": "10.0.0.1",
                        "phase": "Running",
                        "conditions": [{"type": "Ready", "status": "True"}],
                    },
                },
            ]
        }

        result = K8sClient._parse_pods(data)

        assert len(result) == 1
        assert result[0].terminating is True

    def test_pod_not_terminating_when_deletion_timestamp_absent(self):
        # The common case: no deletionTimestamp key at all -> live pod.
        data = {
            "items": [
                {
                    "metadata": {"name": "pod-1"},
                    "status": {
                        "podIP": "10.0.0.1",
                        "phase": "Running",
                        "conditions": [{"type": "Ready", "status": "True"}],
                    },
                },
            ]
        }

        result = K8sClient._parse_pods(data)

        assert len(result) == 1
        assert result[0].terminating is False


@pytest.mark.asyncio
class TestListPods:
    async def test_returns_pods_on_success(
        self, k8s_client, mock_k8s_session, cleanup_client
    ):
        mock_k8s_session(
            method="get",
            status=200,
            json_data={
                "items": [
                    {
                        "metadata": {"name": "pod-1"},
                        "status": {
                            "podIP": "10.0.0.1",
                            "phase": "Running",
                            "conditions": [{"type": "Ready", "status": "True"}],
                        },
                    }
                ]
            },
        )

        result = await k8s_client.list_pods("default", "app=test")

        assert len(result) == 1
        assert result[0].name == "pod-1"

    async def test_raises_error_on_api_failure(
        self, k8s_client, mock_k8s_session, cleanup_client
    ):
        mock_k8s_session(method="get", status=403, text_data="Forbidden")

        with pytest.raises(K8sClientError) as exc:
            await k8s_client.list_pods("default", "app=test")

        assert "Failed to list pods: 403" in str(exc.value)

    async def test_raises_error_on_timeout(
        self, k8s_client, mock_k8s_session, cleanup_client
    ):
        mock_k8s_session(method="get", side_effect=TimeoutError("Connection timed out"))

        with pytest.raises(K8sClientError) as exc:
            await k8s_client.list_pods("default", "app=test")

        assert "Timeout connecting to Kubernetes API" in str(exc.value)


@pytest.mark.asyncio
class TestGetDeploymentReplicas:
    async def test_returns_replica_count(
        self, k8s_client, mock_k8s_session, cleanup_client
    ):
        mock_k8s_session(method="get", status=200, json_data={"spec": {"replicas": 3}})

        result = await k8s_client.get_deployment_replicas("default", "my-deployment")

        assert result == 3

    async def test_raises_error_on_timeout(
        self, k8s_client, mock_k8s_session, cleanup_client
    ):
        mock_k8s_session(method="get", side_effect=TimeoutError("Connection timed out"))

        with pytest.raises(K8sClientError) as exc:
            await k8s_client.get_deployment_replicas("default", "my-deployment")

        assert "Timeout connecting to Kubernetes API" in str(exc.value)


@pytest.mark.asyncio
class TestScaleDeployment:
    async def test_scales_deployment_successfully(
        self, k8s_client, mock_k8s_session, cleanup_client
    ):
        mock_k8s_session(method="patch", status=200)

        # Should not raise
        await k8s_client.scale_deployment("default", "my-deployment", 5)

    async def test_raises_error_on_scale_failure(
        self, k8s_client, mock_k8s_session, cleanup_client
    ):
        mock_k8s_session(method="patch", status=403, text_data="Forbidden")

        with pytest.raises(K8sClientError) as exc:
            await k8s_client.scale_deployment("default", "my-deployment", 5)

        assert "Failed to scale deployment: 403" in str(exc.value)

    async def test_raises_error_on_timeout(
        self, k8s_client, mock_k8s_session, cleanup_client
    ):
        mock_k8s_session(
            method="patch", side_effect=TimeoutError("Connection timed out")
        )

        with pytest.raises(K8sClientError) as exc:
            await k8s_client.scale_deployment("default", "my-deployment", 5)

        assert "Timeout connecting to Kubernetes API" in str(exc.value)


@pytest.mark.asyncio
class TestPatchPodAnnotation:
    async def test_patches_annotation_successfully(
        self, k8s_client, mock_k8s_session, cleanup_client
    ):
        mock_session = mock_k8s_session(method="patch", status=200)

        await k8s_client.patch_pod_annotation(
            "default", "pod-1", "my-annotation", "my-value"
        )

        # Verify the patch was called with the correct data
        mock_session.patch.assert_called_once()
        call_args = mock_session.patch.call_args
        assert "pods/pod-1" in call_args[0][0]
        assert call_args[1]["json"] == {
            "metadata": {"annotations": {"my-annotation": "my-value"}}
        }

    async def test_raises_error_on_patch_failure(
        self, k8s_client, mock_k8s_session, cleanup_client
    ):
        mock_k8s_session(method="patch", status=403, text_data="Forbidden")

        with pytest.raises(K8sClientError) as exc:
            await k8s_client.patch_pod_annotation(
                "default", "pod-1", "my-annotation", "my-value"
            )

        assert "Failed to patch pod annotation: 403" in str(exc.value)

    async def test_raises_error_on_timeout(
        self, k8s_client, mock_k8s_session, cleanup_client
    ):
        mock_k8s_session(
            method="patch", side_effect=TimeoutError("Connection timed out")
        )

        with pytest.raises(K8sClientError) as exc:
            await k8s_client.patch_pod_annotation(
                "default", "pod-1", "my-annotation", "my-value"
            )

        assert "Timeout connecting to Kubernetes API" in str(exc.value)


@pytest.mark.asyncio
class TestClose:
    async def test_closes_session(self, k8s_client):
        mock_session = MagicMock()
        mock_session.closed = False
        mock_session.close = AsyncMock()
        k8s_client._session = mock_session

        await k8s_client.close()

        mock_session.close.assert_called_once()
        assert k8s_client._session is None


@pytest.mark.asyncio
class TestEnsureSession:
    """
    Guards the token-rotation timing of `_ensure_session`.

    `_token_read_time` is deliberately initialized to 0 and is NOT cooldown
    state, so the scaler's "initialize time-based guards to process start"
    rule must not spread to it. These tests pin the two invariants that a
    well-meaning sweep would silently break.
    """

    async def test_reads_token_on_first_call_even_when_timestamp_is_fresh(
        self, k8s_client, mock_k8s_session, cleanup_client
    ):
        mock_session = mock_k8s_session()
        # Simulate a process-start init: the timestamp is not expired, so only
        # the "no session yet" arm can drive the token read.
        k8s_client._token_read_time = time.time()

        session = await k8s_client._ensure_session()

        k8s_client._read_token.assert_called_once()
        assert session is mock_session

    async def test_token_older_than_refresh_interval_rebuilds_session(
        self, k8s_client, mock_k8s_session, cleanup_client
    ):
        new_session = mock_k8s_session()
        old_session = MagicMock()
        old_session.closed = False
        old_session.close = AsyncMock()
        k8s_client._session = old_session
        k8s_client._token_read_time = time.time() - (
            K8sClient._TOKEN_REFRESH_INTERVAL + 1
        )

        session = await k8s_client._ensure_session()

        old_session.close.assert_called_once()
        k8s_client._read_token.assert_called_once()
        assert session is new_session
