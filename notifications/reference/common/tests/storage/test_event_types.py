# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest

from notification_common.storage.event_types import (
    STORAGE_CREATED,
    STORAGE_DELETED,
    STORAGE_DIR_DELETED,
    STORAGE_DIR_CREATED,
    STORAGE_EVENTS,
    is_storage_event,
)


def test_constants_have_correct_values():
    assert STORAGE_CREATED == "omni.storage.created"
    assert STORAGE_DELETED == "omni.storage.deleted"
    assert STORAGE_DIR_CREATED == "omni.storage.dir_created"
    assert STORAGE_DIR_DELETED == "omni.storage.dir_deleted"


def test_storage_events_constant_doesnt_include_synthetic_events():
    assert STORAGE_EVENTS == [
        STORAGE_CREATED,
        STORAGE_DELETED,
        STORAGE_DIR_CREATED,
        STORAGE_DIR_DELETED,
    ]


class TestIsStorageEvent:
    @pytest.mark.parametrize("event_type", STORAGE_EVENTS)
    def test_returns_true_for_storage_events(self, event_type):
        assert is_storage_event(event_type) is True

    def test_returns_false_for_non_storage_events(self):
        assert is_storage_event("event_type") is False
