# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Common test functions for Event Aggregation Service REST smoke tests."""

import logging
import random
import string
from datetime import datetime, timezone

import requests
from utils import assert_response, timeout_rest_request

logger = logging.getLogger(__name__)


def get_publish_event_url(http_endpoint):
    """Get URL for publishing a single event."""
    return f"{http_endpoint}/api/v1beta/events"


def get_batch_publish_events_url(http_endpoint):
    """Get URL for batch publishing events."""
    return f"{http_endpoint}/api/v1beta/events/batch"


def run_publish_event_rest(aggregation_endpoint, resource_base_url, http_headers):
    """Publish a storage created event via REST."""
    url = get_publish_event_url(aggregation_endpoint)

    file_name = "".join(random.choices(string.ascii_letters + string.digits, k=5))
    resource_address = f"{resource_base_url}{file_name}"

    occurred_at = datetime.now(timezone.utc).isoformat()

    payload = {
        "event": {
            "event_type": "omni.storage.created",
            "message": {
                "action": "created",
                "resource_address": resource_address,
                "timestamp": occurred_at,
            },
            "occurred_at": occurred_at,
            "resource": {"resource_id": resource_address},
        }
    }

    response = timeout_rest_request(
        requests.post,
        url,
        json=payload,
        headers=http_headers,
    )

    assert_response(response, 200, "PublishEvent(REST)")

    result = response.json().get("result", {})
    assert result.get(
        "success"
    ), f"Publish failed: {result.get('failure_reason', 'Unknown')}"


def run_batch_publish_events_rest(
    aggregation_endpoint, resource_base_url, http_headers, count=3
):
    """Batch publish multiple events via REST."""
    url = get_batch_publish_events_url(aggregation_endpoint)
    occurred_at = datetime.now(timezone.utc).isoformat()

    events = []
    for i in range(count):
        file_name = "".join(random.choices(string.ascii_letters + string.digits, k=5))
        resource_address = f"{resource_base_url}{file_name}"

        event = {
            "event_type": "test.batch.publish",
            "message": {
                "resource_address": resource_address,
                "event_index": i,
                "timestamp": occurred_at,
            },
            "occurred_at": occurred_at,
            "resource": {"resource_id": resource_address},
        }
        events.append(event)

    payload = {"events": events}

    response = timeout_rest_request(
        requests.post,
        url,
        json=payload,
        headers=http_headers,
    )

    assert_response(response, 200, "BatchPublishEvents(REST)")

    data = response.json()
    results = data.get("results", [])
    results_count = len(results)

    assert results_count == count, f"Expected {count} results, got {results_count}"

    failed_count = 0
    for i, result in enumerate(results):
        if not result.get("success"):
            failure_reason = result.get("failure_reason", "No failure reason provided")
            logger.error(f"  Result {i} failed: {failure_reason}")
            failed_count += 1

    assert failed_count == 0, f"{failed_count} out of {results_count} events failed"

    logger.info(f"✅ All {results_count} events published successfully")
