# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from datetime import datetime, timezone

import pytest
from fastapi.testclient import TestClient
from notification_common.auth import principal_id
from notification_common.rabbitmq.publisher import (
    SuccessfulPublishingResult,
    FailedPublishingResult,
)

from app.lib.rabbitmq import publishing_client
from app.rest.rest_server import rest_server


@pytest.fixture
def mock_principal():
    return "test_principal"


@pytest.fixture
def mock_auth_token():
    return "test_auth_token"


@pytest.fixture()
def metadata(mock_auth_token):
    return [("authorization", f"Bearer {mock_auth_token}")]


@pytest.fixture
def client():
    """Return a FastAPI test client."""
    return TestClient(rest_server)


@pytest.fixture
def post_raw_body(client):
    """Return a poster for a raw request body at a given path."""

    def _post(path, body, headers=None):
        return client.post(
            path,
            content=body,
            headers={"Content-Type": "application/json", **(headers or {})},
        )

    return _post


@pytest.fixture
def auth_headers(mock_auth_token):
    """Return authorization headers."""
    return {"Authorization": f"Bearer {mock_auth_token}"}


@pytest.fixture
def sample_event():
    """Return a sample event for testing."""
    return {
        "event_type": "test.event",
        "message": {"key": "value"},
        "occurred_at": datetime.now(timezone.utc).isoformat(),
        "resource": {"resource_id": "test/resource"},
    }


@pytest.fixture
def successful_result(mocked_omni_event):
    """Return a successful publishing result."""
    return SuccessfulPublishingResult(event=mocked_omni_event)


@pytest.fixture
def failed_result(mocked_omni_event):
    """Return a failed publishing result with a test failure reason."""
    return FailedPublishingResult(event=mocked_omni_event, reason="Test failure")


@pytest.fixture(autouse=True)
def setup_auth(mocker, mock_principal):
    mocker.patch.object(principal_id, "principal_id", return_value=mock_principal)
    mocker.patch.object(publishing_client, "raise_if_cant_publish")
