# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
The shared queue-id and reconnect-token bounds, and the sample values that
violate them.

Both bounds are enforced on four surfaces -- gRPC create / consume / update
/ delete and their REST counterparts -- and the byte-budget arithmetic that
justifies them only holds if every one of those surfaces applies the SAME
rule. Keeping the sample values in one place is what lets each surface's
test suite assert against the same inputs instead of each inventing its own
"probably malformed" string.

The bounds themselves are pinned against the real validators in
``tests/app/lib/rabbitmq/test_rabbitmq_base.py``, which asserts that a
value at the bound is accepted and one character over is rejected, and that
the reconnect token's alphabet is exactly printable ASCII.
"""

from app.lib.rabbitmq.rabbitmq_base import RECONNECT_TOKEN_MAX_LENGTH

#: The upper bound written into ``QUEUE_ID_PATTERN``'s ``{1,200}``. There is
#: no constant in the source to import -- the bound lives inside the regex.
MAX_QUEUE_ID_LENGTH = 200

#: The two ways a client-supplied queue id goes wrong: it breaks the
#: alphabet, or it breaks the length bound. ``:`` is the interesting
#: character because it is the separator inside the broker queue name.
MALFORMED_QUEUE_IDS = ["bad:id", "a" * (MAX_QUEUE_ID_LENGTH + 1)]
MALFORMED_QUEUE_ID_IDS = ["contains-colon", "exceeds-max-length"]

MAX_LENGTH_RECONNECT_TOKEN = "a" * RECONNECT_TOKEN_MAX_LENGTH
OVER_LONG_RECONNECT_TOKEN = "a" * (RECONNECT_TOKEN_MAX_LENGTH + 1)

#: The three ways a reconnect token goes wrong. Its alphabet is deliberately
#: wider than a queue id's -- printable ASCII, because a token is opaque to
#: the client and an unrecognised one of legal shape has to reach the lookup
#: and fail as "queue not found" -- so what is left to reject is the length
#: bound and the characters a broker queue name cannot carry at all.
#:
#: Both non-ASCII samples stay inside latin-1, because the header carrier
#: (``Last-Event-ID``) cannot transmit anything else and a test driving that
#: surface would fail on the encoding rather than on the validation.
NON_ASCII_RECONNECT_TOKEN = "café"
SPACE_CONTAINING_RECONNECT_TOKEN = "has spaces"
MALFORMED_RECONNECT_TOKENS = [
    OVER_LONG_RECONNECT_TOKEN,
    NON_ASCII_RECONNECT_TOKEN,
    SPACE_CONTAINING_RECONNECT_TOKEN,
]
MALFORMED_RECONNECT_TOKEN_IDS = [
    "exceeds-max-length",
    "non-ascii",
    "contains-space",
]
