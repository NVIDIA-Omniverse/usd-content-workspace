# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Request limits shared by the REST and gRPC entry points.

Both APIs publish into the same RabbitMQ channel pool, so they have to agree
on how much work a single request may enqueue. REST applies these through its
Pydantic models; gRPC applies them in the servicer, refusing with
INVALID_ARGUMENT.
"""

import json
from typing import Optional

MAX_BATCH_EVENTS = 1000

# Lengths in characters, as Pydantic's ``max_length`` counts them.
MAX_EVENT_TYPE_LENGTH = 256
MAX_RESOURCE_ID_LENGTH = 2048
MAX_OCCURRED_AT_LENGTH = 64

MAX_EVENT_MESSAGE_BYTES = 64 * 1024


def message_size_violation(message: dict) -> Optional[str]:
    """Return why ``message`` is over the size limit, or ``None`` if it is not.

    The size is that of ``json.dumps(message)`` with the stdlib defaults, which
    is how ``OmniEvent`` serializes the message when it is published, so the
    limit bounds the bytes the message adds to the published event. The
    defaults include ``ensure_ascii=True``, which escapes every non-ASCII
    character, so the encoding is pure ASCII and its character count is its
    UTF-8 byte count.
    """
    size = len(json.dumps(message))
    if size <= MAX_EVENT_MESSAGE_BYTES:
        return None
    return (
        f"must be at most {MAX_EVENT_MESSAGE_BYTES} bytes when encoded as JSON, "
        f"but is {size}"
    )


def length_violation(value: str, limit: int) -> Optional[str]:
    """Return why ``value`` is longer than ``limit`` characters, or ``None``."""
    if len(value) <= limit:
        return None
    return f"must be at most {limit} characters, but is {len(value)}"
