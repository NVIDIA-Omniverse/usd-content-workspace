# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
The report of the storage events dropped against an unauthorized
storage-permissions cache entry.

Each unauthorized entry's unreported drop count is reported once, when the
entry leaves a stream's cache. These are the pieces the tests of every exit
point share: the report line's message, a distinctive count, and a builder
for an entry carrying that count. The ``drop_reports`` fixture that captures
the report lines is in ``tests/app/conftest.py``.
"""

from app.lib.permissions.storage_permissions.directory_contents import (
    DirectoryContents,
    ListingOutcome,
)

#: The message of the line that reports the events dropped against an
#: unauthorized entry as it leaves the stream's cache.
DROP_REPORT_MESSAGE = (
    "storage events were dropped for a directory the storage listing "
    "did not authorize"
)

#: A drop count no other value in these tests collides with, so a record
#: carrying it is a report of these drops.
DROPPED = 7


def unauthorized_entry_with_drops(
    dir_path: str,
    outcome: ListingOutcome | None = ListingOutcome.FORBIDDEN,
    dropped: int = DROPPED,
) -> DirectoryContents:
    """
    An unauthorized entry for ``dir_path`` with ``dropped`` unreported drops
    against it.

    ``outcome=None`` builds it the way Redis hydration does, which records
    no outcome.
    """
    if outcome is None:
        entry = DirectoryContents.from_redis(
            authorized=False,
            files=None,
            folders=None,
            expires_at=0,
            prev={},
            dir_path=dir_path,
        )
    else:
        entry = DirectoryContents(dir_path=dir_path, outcome=outcome)
    # Incremented as the readers in storage_permissions do.
    entry.dropped_events += dropped
    return entry
