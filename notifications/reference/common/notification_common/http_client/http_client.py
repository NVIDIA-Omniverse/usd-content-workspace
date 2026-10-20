# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from contextlib import asynccontextmanager
import logging
import aiohttp

logger = logging.getLogger(__name__)


"""
Users of this module are meant to use the
http_client instance directly.

If calling this from outside of the
application context (i.e. tests or a script) then
the open_session contextmanager can be used.
"""


class HttpClient:
    """
    Class for managing and aiohttp session
    """

    def __init__(self):
        self.session: aiohttp.ClientSession | None = None

    async def open(self) -> None:
        if not self.session:
            self.session = aiohttp.ClientSession()

    async def close(self) -> None:
        if self.session and not self.session.closed:
            await self.session.close()
            self.session = None


http_client = HttpClient()


@asynccontextmanager
async def open_session():
    """Only meant to be used outside of app, like in scripts and tests"""
    await http_client.open()
    yield
    await http_client.close()
