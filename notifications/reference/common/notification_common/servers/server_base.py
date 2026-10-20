# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
from abc import ABC, abstractmethod


class ServerBase(ABC):
    @classmethod
    @abstractmethod
    async def start_server(cls, shutdown_event: asyncio.Event):
        pass
