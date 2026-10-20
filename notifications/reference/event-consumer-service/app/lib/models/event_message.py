# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import time
from typing import Optional


class EventMessage:
    def __init__(
        self,
        message: dict,
        processing_started_at: Optional[float] = None,
        delivery_tag: Optional[int] = None,
    ):
        self.message = message
        # Tracks when the event was first received from the message broker
        self.processing_started_at = processing_started_at or time.perf_counter()
        # Set when using client acks for durable consumption
        self.delivery_tag = delivery_tag

    @property
    def event_type(self) -> str:
        """Get the event type from the message, defaulting to 'unknown'."""
        return self.message.get("event_type") or "unknown"

    @property
    def processing_duration_ms(self) -> float:
        """Calculate processing duration from start time to now in milliseconds."""
        return (time.perf_counter() - self.processing_started_at) * 1000
