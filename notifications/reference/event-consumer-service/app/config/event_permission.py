# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from pydantic import BaseModel


class EventPermission(BaseModel):
    """
    Fine-grained permissions may be
    needed, based on the messages for a given event_type.

    Fine-grained permissions are specified per event_type.

    The action is used in the permissions service api call,
    along with the resource_id which comes from the event.
    """

    event_type: str
    action: str
