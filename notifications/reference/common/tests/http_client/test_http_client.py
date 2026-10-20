# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
import pytest_asyncio

from notification_common.http_client.http_client import HttpClient


@pytest.fixture
def client() -> HttpClient:
    return HttpClient()


@pytest_asyncio.fixture(autouse=True)
async def clos_session(client):
    yield

    if client.session and not client.session.closed:
        await client.close()


@pytest.mark.asyncio
class TestOpen:
    async def test_opens_session(self, client):
        assert client.session is None
        await client.open()
        # noinspection PyUnresolvedReferences
        assert client.session.closed is False


@pytest.mark.asyncio
class TestClose:
    async def test_closes_session_and_sets_it_to_none(self, client, mocker):
        await client.open()
        spy = mocker.spy(client.session, "close")

        await client.close()

        assert client.session is None
        spy.assert_called_once()
