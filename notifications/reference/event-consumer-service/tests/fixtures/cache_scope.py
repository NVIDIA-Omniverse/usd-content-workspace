# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
The cache scope the storage-permissions tests share, and the principals it
is built from.

A storage-permissions cache is namespaced by the queue AND the principal
presenting the request, so most tests below the cache layer need a
``CacheScope`` rather than a bare queue name. Declaring the standard one
here keeps every module talking about the same pair: the suite's stand-in
non-durable broker queue name (which embeds
``tests.fixtures.reconnect_token.RECONNECT_TOKEN``) and the principal the
autouse ``mock_principal_id`` fixture in ``tests/conftest.py`` resolves.

``SECOND_PRINCIPAL`` is the other party in the isolation the design exists
for: a different authenticated principal presenting the same reconnect
token. It gets its own scope on the same queue, and the two must share a
``queue_ref`` while never sharing a ``key``.
"""

from app.lib.permissions.storage_permissions.cache_scope import CacheScope
from app.lib.queue_names import redact_principal_id
from tests.fixtures.reconnect_token import NON_DURABLE_QUEUE_NAME

#: The principal the autouse ``mock_principal_id`` fixture returns. Spelled
#: here as the same literal that fixture uses, so a scope built from this
#: module matches one a ``get_instance`` call under that fixture derives.
TEST_PRINCIPAL = "test_principal"

#: A second, different authenticated principal -- the one that attaches with
#: someone else's reconnect token.
SECOND_PRINCIPAL = "other_principal"

#: The redacted references those two principals render to. Derived rather
#: than spelled as literals: the digest construction is pinned in
#: ``tests/app/lib/test_queue_names.py``, and pinning it a second time here
#: would only couple every consumer of this module to it.
TEST_PRINCIPAL_REF = redact_principal_id(TEST_PRINCIPAL)
SECOND_PRINCIPAL_REF = redact_principal_id(SECOND_PRINCIPAL)

#: The standard scope: the suite's non-durable queue, as seen by the
#: principal the autouse fixture resolves.
CACHE_SCOPE = CacheScope(NON_DURABLE_QUEUE_NAME, TEST_PRINCIPAL_REF)

#: The same queue as seen by a second principal. Same ``queue_ref``,
#: different ``key``.
SECOND_CACHE_SCOPE = CacheScope(NON_DURABLE_QUEUE_NAME, SECOND_PRINCIPAL_REF)

#: What addresses the standard scope's storage: every Redis key, the writer
#: registry entry, and the ContextVar the mutators read.
CACHE_SCOPE_KEY = CACHE_SCOPE.key
