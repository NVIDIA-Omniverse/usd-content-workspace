# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
What a non-durable stream's storage-permissions cache is namespaced by.

A non-durable queue's id IS its reconnect token, so any authenticated
principal holding one can attach to that queue. Each of them is authorized
per event against its OWN permissions, but the storage-permissions cache
behind those checks used to be addressed by the broker queue name alone: a
second principal attaching inherited the first's cached directory listings,
and registering its CacheWriter under the same key drained and stopped the
first's. The namespace is therefore the QUEUE plus the PRESENTING PRINCIPAL,
which is what this class is. Two streams from the same principal still share
one scope, so the supported same-principal fan-out is unaffected.

The pairing is a type rather than a composite string on purpose. The two
components have opposite export rules -- the queue name yields the exported
``queue_ref``, the principal reference must never be exported -- and the
combined value must never be exported at all. A bare
``event-queue:<token>:nd:sha256-...`` string handed to
:func:`~app.lib.queue_names.queue_ref_for_name` comes back UNCHANGED, because
that function decides by shape and the composite no longer ends in ``:nd``;
every log field, metric attribute, and task name built that way would then
carry the raw reconnect token. Holding the parts separately makes that
mistake unrepresentable: :attr:`CacheScope.key` is the only value that
addresses storage, :attr:`CacheScope.queue_ref` is the only value that is
written down.

This module deliberately imports nothing but :mod:`app.lib.queue_names` and
the stdlib, so every layer that needs a scope -- the non-durable consumer,
the cache, the Redis Op builders, the writer -- can import it without
importing any of the others.
"""

from __future__ import annotations

from dataclasses import dataclass

from app.lib.queue_names import queue_ref_for_name


@dataclass(frozen=True)
class CacheScope:
    """
    The (queue, presenting principal) pair a storage-permissions cache and
    its CacheWriter are namespaced by.

    Frozen because it is derived once, when the consumer is constructed, and
    read for the life of the stream: a scope that changed under a live
    stream would leave already-written Redis entries stranded under the old
    key and the writer registered under it unreachable.

    :param queue_name: The broker queue name, exact. This is what addresses
        the queue, so it must stay verbatim -- it is never logged.
    :param principal_ref: The presenting principal, rendered by
        :func:`~app.lib.queue_names.redact_principal_id`, or
        :data:`~app.lib.queue_names.NO_PRINCIPAL` when authentication is
        disabled.
    """

    queue_name: str
    principal_ref: str

    @property
    def key(self) -> str:
        """
        The value that addresses this scope's storage: every Redis key, the
        CacheWriter registry entry, the ``_active_queue_id_var`` ContextVar
        that binds a request to that entry, and the metric depth baseline.

        Never export it -- it embeds the queue name, and a non-durable
        queue name embeds the reconnect token. Use :attr:`queue_ref`.
        """
        return f"{self.queue_name}:{self.principal_ref}"

    @property
    def queue_ref(self) -> str:
        """
        The reference to this scope's queue that is safe to write down.

        Queue-only, with no principal component, so it is byte-identical to
        the ``queue_ref`` the same stream's spans and the consumer's own log
        lines carry and the three still join by exact match. Two scopes on
        one queue therefore share a ``queue_ref``; that is the intent --
        they are two views of one stream.
        """
        return queue_ref_for_name(self.queue_name)
