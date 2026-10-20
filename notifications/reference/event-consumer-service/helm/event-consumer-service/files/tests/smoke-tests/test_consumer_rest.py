# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import pytest
from consumer_rest_helpers import (
    build_filter_groups_rest,
    create_durable_queue_rest,
    delete_durable_queue_rest,
    get_reconnect_token,
    get_stream_events_url,
    update_durable_queue_rest,
)

logger = logging.getLogger(__name__)


@pytest.mark.rest
def test_create_and_delete_durable_queue_rest(
    rest_consumer_endpoint,
    resource_base_url,
    http_headers,
):
    """Create a durable queue and delete it."""
    queue_id = None
    try:
        logger.info("Creating durable queue...")
        queue_id = create_durable_queue_rest(
            rest_consumer_endpoint, resource_base_url, http_headers
        )
        assert queue_id, "Queue ID should not be empty"
        logger.info("CreateDurableQueue succeeded - Queue ID: %s", queue_id)

        logger.info("Deleting durable queue...")
        delete_durable_queue_rest(rest_consumer_endpoint, queue_id, http_headers)
        logger.info("DeleteDurableQueue succeeded")
        queue_id = None
    finally:
        if queue_id:
            try:
                delete_durable_queue_rest(
                    rest_consumer_endpoint, queue_id, http_headers
                )
            except Exception:
                pass


@pytest.mark.rest
def test_update_durable_queue_rest(
    rest_consumer_endpoint,
    resource_base_url,
    http_headers,
):
    """Create durable queue, update filters, then delete."""
    queue_id = None
    try:
        queue_id = create_durable_queue_rest(
            rest_consumer_endpoint, resource_base_url, http_headers
        )
        assert queue_id, "Queue ID should not be empty"

        update_durable_queue_rest(
            rest_consumer_endpoint, queue_id, resource_base_url, http_headers
        )
        logger.info("UpdateDurableQueue succeeded")

        delete_durable_queue_rest(rest_consumer_endpoint, queue_id, http_headers)
        queue_id = None
    finally:
        if queue_id:
            try:
                delete_durable_queue_rest(
                    rest_consumer_endpoint, queue_id, http_headers
                )
            except Exception:
                pass


@pytest.mark.rest
def test_non_durable_stream_reconnect_token_rest(
    rest_consumer_endpoint,
    resource_base_url,
    http_headers,
):
    """Connect to non-durable stream and verify we get a reconnect token."""
    filter_groups = build_filter_groups_rest(resource_base_url)
    url = get_stream_events_url(rest_consumer_endpoint, filter_groups=filter_groups)
    reconnect_token = get_reconnect_token(url, http_headers)
    assert reconnect_token, "Failed to get reconnect token from stream"
