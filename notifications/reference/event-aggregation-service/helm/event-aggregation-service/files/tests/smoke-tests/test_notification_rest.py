# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import pytest
from notification_rest_helpers import (
    run_batch_publish_events_rest,
    run_publish_event_rest,
)

logger = logging.getLogger(__name__)


@pytest.mark.rest
def test_publish_event_rest(
    require_notification_rest_endpoint,
    resource_base_url,
    http_headers,
    service_endpoints,
):
    """Test REST PublishEvent operation."""
    aggregation = service_endpoints.get("http_notification_aggregation")
    if not aggregation:
        pytest.skip("http_notification_aggregation not configured")

    run_publish_event_rest(aggregation, resource_base_url, http_headers)


@pytest.mark.rest
def test_batch_publish_events_rest(
    require_notification_rest_endpoint,
    resource_base_url,
    http_headers,
    service_endpoints,
):
    """Test REST BatchPublishEvents operation."""
    aggregation = service_endpoints.get("http_notification_aggregation")
    if not aggregation:
        pytest.skip("http_notification_aggregation not configured")

    run_batch_publish_events_rest(aggregation, resource_base_url, http_headers)
