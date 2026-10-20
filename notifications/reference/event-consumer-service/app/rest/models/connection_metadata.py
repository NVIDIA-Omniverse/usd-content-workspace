# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import time
from dataclasses import dataclass, field


@dataclass
class ConnectionMetadata:
    """
    Metadata for an SSE connection
    """

    endpoint: str
    start_time: float = field(default_factory=time.time)
    events_sent_count: int = field(default=0)
