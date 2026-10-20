# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Unit tests for ``CacheScope``.

The scope pairs a broker queue name with the presenting principal's
reference and keeps the two roles apart: the key addresses storage and is
never exported, the ``queue_ref`` is exported and carries no principal
component. The pairing is a type rather than a composite string precisely
so a caller cannot hand the composite to ``queue_ref_for_name`` and get the
reconnect token back out -- that failure is pinned here directly.
"""

import dataclasses
import re

import pytest

from app.lib.permissions.storage_permissions.cache_scope import CacheScope
from app.lib.queue_names import (
    NON_DURABLE_SUFFIX,
    QUEUE_NAME_PREFIX,
    queue_ref_for_name,
    redact_principal_id,
    redact_reconnect_token,
)
from tests.fixtures.reconnect_token import NON_DURABLE_QUEUE_NAME, RECONNECT_TOKEN

#: The bare reference shape a scope's ``queue_ref`` must have, so it joins
#: the same stream's span attribute by exact match.
REDACTED_PATTERN = r"^sha256-[0-9a-f]{12}$"

PRINCIPAL_REF = redact_principal_id("a-principal")
OTHER_PRINCIPAL_REF = redact_principal_id("another-principal")

DURABLE_QUEUE_NAME = f"{QUEUE_NAME_PREFIX}:my-app-queue"


@pytest.fixture
def scope():
    return CacheScope(NON_DURABLE_QUEUE_NAME, PRINCIPAL_REF)


class TestKey:
    """
    The value that addresses this scope's storage: every Redis key, the
    CacheWriter registry entry, the ContextVar the mutators read, and the
    metrics depth baseline.
    """

    def test_joins_the_queue_name_and_the_principal_reference(self, scope):
        assert scope.key == f"{NON_DURABLE_QUEUE_NAME}:{PRINCIPAL_REF}"

    def test_keeps_the_queue_name_verbatim(self, scope):
        # The key has to address the broker's own queue, so the name half
        # must never be redacted on the way in.
        assert scope.key.startswith(NON_DURABLE_QUEUE_NAME)

    def test_differs_for_a_second_principal_on_the_same_queue(self, scope):
        other = CacheScope(NON_DURABLE_QUEUE_NAME, OTHER_PRINCIPAL_REF)

        assert other.key != scope.key

    def test_is_shared_by_two_streams_of_the_same_principal(self, scope):
        # The supported same-principal fan-out is unaffected: two streams
        # from one principal still share one cache and one writer.
        same = CacheScope(NON_DURABLE_QUEUE_NAME, PRINCIPAL_REF)

        assert same.key == scope.key


class TestQueueRef:
    """
    The only value that is written down. Queue-only, with no principal
    component, so the two scopes of one queue report the same reference and
    a metric series still joins the stream's spans by exact match.
    """

    def test_renders_the_bare_reference_for_the_queues_token(self, scope):
        assert scope.queue_ref == redact_reconnect_token(RECONNECT_TOKEN)

    def test_renders_the_documented_shape(self, scope):
        assert re.fullmatch(REDACTED_PATTERN, scope.queue_ref)

    def test_does_not_contain_the_reconnect_token(self, scope):
        assert RECONNECT_TOKEN not in scope.queue_ref

    def test_does_not_contain_the_principal_reference(self, scope):
        # A principal component in an exported attribute answers no
        # operator question and breaks the exact-match join.
        assert PRINCIPAL_REF not in scope.queue_ref

    def test_is_identical_for_two_principals_on_one_queue(self, scope):
        other = CacheScope(NON_DURABLE_QUEUE_NAME, OTHER_PRINCIPAL_REF)

        assert other.queue_ref == scope.queue_ref

    def test_leaves_a_durable_queue_name_unchanged(self):
        # A durable name carries no token and is its own reference.
        scope = CacheScope(DURABLE_QUEUE_NAME, PRINCIPAL_REF)

        assert scope.queue_ref == DURABLE_QUEUE_NAME


class TestKeyIsNotExportable:
    """
    Why the pairing is a type and not a composite string.

    A scope key does not end in ``:nd``, and ``queue_ref_for_name`` decides
    by shape, so handing it the key returns it UNCHANGED -- reconnect token
    and all. Every log field, metric attribute and task name built that way
    would carry the raw token. Holding the halves separately is what makes
    that mistake unrepresentable; these tests pin the failure the type
    prevents so the reason for the type survives a refactor.
    """

    def test_queue_ref_for_name_returns_the_key_verbatim(self, scope):
        assert queue_ref_for_name(scope.key) == scope.key

    def test_the_key_run_through_queue_ref_for_name_still_leaks_the_token(self, scope):
        assert RECONNECT_TOKEN in queue_ref_for_name(scope.key)

    def test_the_key_does_not_end_in_the_non_durable_suffix(self, scope):
        assert not scope.key.endswith(NON_DURABLE_SUFFIX)


class TestFrozen:
    """
    Derived once when the consumer is constructed and read for the life of
    the stream. A scope that changed under a live stream would strand the
    Redis entries already written under the old key and make the writer
    registered under it unreachable.
    """

    def test_queue_name_cannot_be_reassigned(self, scope):
        with pytest.raises(dataclasses.FrozenInstanceError):
            scope.queue_name = "event-queue:someone-elses-token:nd"

    def test_principal_ref_cannot_be_reassigned(self, scope):
        with pytest.raises(dataclasses.FrozenInstanceError):
            scope.principal_ref = OTHER_PRINCIPAL_REF

    def test_is_hashable_so_it_can_key_a_dict(self, scope):
        assert {scope: "state"}[CacheScope(NON_DURABLE_QUEUE_NAME, PRINCIPAL_REF)]
