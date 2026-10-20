# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Common test functions for Event Consumer Service REST smoke tests."""

import json
import logging
import time
import urllib.parse

import requests
from utils import (
    assert_response,
    generate_queue_id,
    retry_on_503_rest,
    timeout_rest_request,
)

logger = logging.getLogger(__name__)


def get_create_durable_queue_url(http_endpoint):
    """Get URL for creating durable queue."""
    return f"{http_endpoint}/api/v1beta/queues/durable"


def get_update_durable_queue_url(http_endpoint):
    """Get URL for updating durable queue."""
    return f"{http_endpoint}/api/v1beta/queues/durable"


def get_delete_durable_queue_url(http_endpoint, queue_id):
    """Get URL for deleting durable queue."""
    queue_id_encoded = urllib.parse.quote(queue_id, safe="")
    return f"{http_endpoint}/api/v1beta/queues/durable?queue_id={queue_id_encoded}"


def get_stream_events_url(http_endpoint, filter_groups=None, reconnect_token=None):
    """Get URL for streaming non-durable events."""
    url = f"{http_endpoint}/api/v1beta/events/stream"
    params = []
    if filter_groups:
        filter_groups_json = json.dumps(filter_groups)
        params.append(
            f"filter_groups={urllib.parse.quote(filter_groups_json, safe='')}"
        )
    if reconnect_token:
        params.append(f"reconnect_token={urllib.parse.quote(reconnect_token, safe='')}")
    if params:
        url += "?" + "&".join(params)
    return url


def build_filter_groups_rest(resource_base_url, filter_type="starts_with_lazy"):
    """Build filter groups for event type omni.storage.created (REST)."""
    return [
        {
            "event_type": "omni.storage.created",
            "filters": [{"filter_type": filter_type, "resource_id": resource_base_url}],
        }
    ]


def create_durable_queue_rest(consumer_endpoint, resource_base_url, http_headers):
    """Create a durable queue and return the queue_id.

    Retries on 503 and on connection failure, which a rolling upgrade can
    produce transiently. The call supplies its own queue_id so a retry is
    ensure-exists rather than a second queue: the server returns the
    existing queue with created=false, which is why 200 is accepted
    alongside the 201 a genuinely new queue gets.
    """
    url = get_create_durable_queue_url(consumer_endpoint)
    queue_id = generate_queue_id()
    payload = {
        "queue_id": queue_id,
        "filter_groups": build_filter_groups_rest(resource_base_url),
    }
    response = retry_on_503_rest(
        timeout_rest_request, requests.post, url, json=payload, headers=http_headers
    )
    assert_response(response, [200, 201], "CreateDurableQueue")
    return response.json().get("queue_id")


def update_durable_queue_rest(
    consumer_endpoint, queue_id, resource_base_url, http_headers
):
    """Update a durable queue (change filter type from lazy to greedy).

    Retries on 503 and on connection failure, as create does.
    """
    url = get_update_durable_queue_url(consumer_endpoint)
    payload = {
        "queue_id": queue_id,
        "current_filter_groups": build_filter_groups_rest(
            resource_base_url, "starts_with_lazy"
        ),
        "new_filter_groups": build_filter_groups_rest(
            resource_base_url, "starts_with_greedy"
        ),
    }
    response = retry_on_503_rest(
        timeout_rest_request, requests.patch, url, json=payload, headers=http_headers
    )
    assert_response(response, 200, "UpdateDurableQueue")


def delete_durable_queue_rest(consumer_endpoint, queue_id, http_headers):
    """Delete a durable queue. Retry up to 30 seconds if it fails.

    Retrying is safe here because this caller tolerates a replay, not
    because a failed attempt is known to be undelivered. A terminating
    pod can accept the delete, process it, and then lose the connection
    before the response is read, which looks from here exactly like a
    delete that never arrived. Delete is the one operation a replay is
    not naturally idempotent for: the replay finds the queue already
    gone and the server answers 404.

    So a 404 is accepted as success ONLY once at least one earlier
    attempt has been made, mirroring the gRPC helper's
    ``not_found_after_retry_ok``. A 404 on the first attempt is still
    a failure, so a genuinely wrong queue_id stays a real failure.

    The budget is wall-clock (30 seconds); exhausting it raises an
    AssertionError naming the last failure.
    """
    url = get_delete_durable_queue_url(consumer_endpoint, queue_id)
    last_error = None
    retried = False
    start_time = time.time()
    while time.time() - start_time < 30:
        try:
            response = timeout_rest_request(requests.delete, url, headers=http_headers)
            if response.status_code == 200:
                return
            if retried and response.status_code == 404:
                logger.info(
                    "Consumer REST delete returned 404 after a retry. "
                    "Treating an earlier attempt as having succeeded."
                )
                return
            last_error = f"Status {response.status_code}: {response.text}"
        except Exception as e:
            last_error = str(e)
        time.sleep(2)
        retried = True
    raise AssertionError(f"Failed to delete queue after 30s: {last_error}")


def _connect_to_sse_stream(url, http_headers, timeout=15):
    """Establish connection to SSE stream with retry for 503 errors."""

    def _connect():
        response = requests.get(
            url,
            headers={**http_headers, "Accept": "text/event-stream"},
            stream=True,
            timeout=(5, timeout),
        )
        if response.status_code != 200:
            raise AssertionError(
                f"Stream connection failed with status "
                f"{response.status_code}: {response.text}"
            )
        return response

    return retry_on_503_rest(_connect)


def get_reconnect_token(url, http_headers):
    """Get reconnect_token from Last-Event-ID by connecting briefly."""
    max_duration = 5
    start_time = time.time()
    try:
        response = _connect_to_sse_stream(url, http_headers, timeout=max_duration)
        for line in response.iter_lines(decode_unicode=True):
            if time.time() - start_time > max_duration:
                return None
            if line and line.startswith("id:"):
                return line[3:].strip()
        return None
    except requests.exceptions.Timeout:
        return None
    except Exception:
        raise
