# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import json

from notification_common.rabbitmq.omni_event import OmniEvent

from app.lib.limits import (
    MAX_EVENT_MESSAGE_BYTES,
    length_violation,
    message_size_violation,
)


class TestMessageSizeViolation:
    def test_accepts_a_message_at_the_limit(self, message_of_size):
        assert message_size_violation(message_of_size(MAX_EVENT_MESSAGE_BYTES)) is None

    def test_rejects_a_message_over_the_limit(self, message_of_size):
        violation = message_size_violation(message_of_size(MAX_EVENT_MESSAGE_BYTES + 1))

        assert violation == (
            f"must be at most {MAX_EVENT_MESSAGE_BYTES} bytes when encoded as JSON, "
            f"but is {MAX_EVENT_MESSAGE_BYTES + 1}"
        )

    def test_counts_non_ascii_as_escaped(self):
        # json.dumps escapes the one character U+00E9 as the six characters
        # \u00e9. A sixth of the limit in that character escapes to 65532
        # bytes, and the 9-byte {"k": ""} wrapper takes it to 65541, over it.
        message = {"k": chr(0xE9) * (MAX_EVENT_MESSAGE_BYTES // 6)}

        assert message_size_violation(message) is not None

    def test_measures_the_message_as_the_published_event_encodes_it(self):
        # The limit is only meaningful if it measures the message the way the
        # published event serializes it, so the measured encoding must appear
        # verbatim in what OmniEvent publishes.
        message = {"k": chr(0xE9), "n": [1, 2.5, None, True]}
        event = OmniEvent(
            event_type="t",
            occurred_at="2024-10-23T00:00:00Z",
            resource_id="",
            message=message,
        )

        assert json.dumps(message) in str(event)


class TestLengthViolation:
    def test_accepts_a_value_at_the_limit(self):
        assert length_violation("x" * 8, 8) is None

    def test_rejects_a_value_over_the_limit(self):
        assert length_violation("x" * 9, 8) == "must be at most 8 characters, but is 9"
