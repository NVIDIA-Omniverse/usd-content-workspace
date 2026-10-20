# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import importlib
import os

import pytest
import redis.asyncio as redis

from notification_common.config import common_settings as settings_module
from notification_common.config.common_settings import common_settings
from notification_common.redis import redis_client


@pytest.fixture(autouse=True)
def mock_redis_uri():
    os.environ["OMNI_EVENTS_REDIS_URI"] = "redis://localhost:6379"
    importlib.reload(settings_module)
    importlib.reload(redis_client)

    yield

    os.environ.pop("OMNI_EVENTS_REDIS_URI")
    importlib.reload(settings_module)
    importlib.reload(redis_client)


def test_it_sets_redis_client_value():
    client = redis_client.redis_client
    assert isinstance(client, redis.Redis)
    assert "host=localhost" in str(client)
    assert "port=6379" in str(client)


@pytest.mark.asyncio
class TestCloseRedisClient:
    async def test_closes_redis_client(self, mocker):
        spy = mocker.patch.object(redis_client.redis_client, "aclose")
        await redis_client.close_redis_client()
        spy.assert_called_once_with()

    async def test_closes_decoding_redis_client(self, mocker):
        spy = mocker.patch.object(redis_client.decoding_redis_client, "aclose")
        await redis_client.close_redis_client()
        spy.assert_called_once_with()

    async def test_does_noting_if_no_redis_client_exists(self, mocker):
        redis_client.redis_client = None
        redis_client.decoding_redis_client = None
        spy = mocker.patch.object(redis.Redis, "aclose")
        await redis_client.close_redis_client()
        spy.assert_not_called()


class TestGetRedisClient(object):
    @staticmethod
    def assert_client_created_with_correct_args(
        redis_uri, mocker, host="host", port=1234, password=None, ssl=False, decode=True
    ):
        mock_redis = mocker.patch.object(redis, "Redis")
        # noinspection PyProtectedMember
        redis_client._get_redis_client(redis_uri, decode_responses=decode)

        mock_redis.assert_called_once_with(
            host=host,
            port=port,
            password=password,
            ssl=ssl,
            socket_connect_timeout=10,
            decode_responses=decode,
        )

    def tests_makes_and_returns_redis_client(self, mocker):
        mocker.patch.object(redis, "Redis", return_value="client")
        assert redis_client._get_redis_client(common_settings.redis_uri) == "client"

    def tests_handles_uri_with_scheme(self, mocker):
        self.assert_client_created_with_correct_args("redis://host:1234", mocker)

    def tests_handles_uri_without_scheme(self, mocker):
        self.assert_client_created_with_correct_args("host:1234", mocker)

    @pytest.mark.parametrize("decode_responses", [True, False])
    def test_conditionally_decodes_responses(self, decode_responses, mocker):
        self.assert_client_created_with_correct_args(
            "host:1234", mocker, decode=decode_responses
        )

    def tests_handles_azure_cache_for_redis_conn_strings(self, mocker):
        self.assert_client_created_with_correct_args(
            "host:1234,password=password,ssl=True,abortConnect=False",
            mocker,
            password="password",
            ssl=True,
        )

    def tests_handles_uri_with_name_and_password(self, mocker):
        self.assert_client_created_with_correct_args(
            "username:test-pwd@host:1234",
            mocker,
            password="test-pwd",
        )

    def tests_handles_uri_with_scheme_name_and_password(self, mocker):
        self.assert_client_created_with_correct_args(
            "redis://username:test-pwd@host:1234",
            mocker,
            password="test-pwd",
        )
