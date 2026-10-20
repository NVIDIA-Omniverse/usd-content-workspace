# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

STORAGE_CREATED = "omni.storage.created"
STORAGE_DELETED = "omni.storage.deleted"
STORAGE_DIR_CREATED = "omni.storage.dir_created"
STORAGE_DIR_DELETED = "omni.storage.dir_deleted"

STORAGE_EVENTS = [
    STORAGE_CREATED,
    STORAGE_DELETED,
    STORAGE_DIR_CREATED,
    STORAGE_DIR_DELETED,
]


def is_storage_event(event_type: str):
    return event_type in STORAGE_EVENTS
