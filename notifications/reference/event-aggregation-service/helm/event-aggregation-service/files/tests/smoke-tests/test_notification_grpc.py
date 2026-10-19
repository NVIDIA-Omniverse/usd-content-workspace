# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging

import pytest
from notification_grpc_helpers import (
    run_batch_publish_events_grpc,
    run_publish_event_grpc,
)

logger = logging.getLogger(__name__)


@pytest.mark.grpc
def test_publish_event_grpc(
    notification_aggregation_grpc_stub,
    grpc_metadata,
    resource_base_url,
):
    """Test gRPC PublishEvent operation."""
    run_publish_event_grpc(
        resource_base_url, notification_aggregation_grpc_stub, grpc_metadata
    )


@pytest.mark.grpc
def test_batch_publish_events_grpc(
    notification_aggregation_grpc_stub,
    grpc_metadata,
    resource_base_url,
):
    """Test gRPC BatchPublishEvents operation."""
    run_batch_publish_events_grpc(
        resource_base_url, notification_aggregation_grpc_stub, grpc_metadata
    )
