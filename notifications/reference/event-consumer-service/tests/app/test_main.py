# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest
from notification_common.servers import start_servers

from app import main
from app.grpc.grpc_server import GrpcServer
from app.lib.rabbitmq import rabbitmq_connector


@pytest.mark.asyncio
class TestRun:
    @pytest.mark.parametrize("uses_rest", [True, False])
    async def test_calls_start_servers(self, uses_rest, mocker):
        # Import RestServer locally to match the import context in main.py
        from app.rest.rest_server import RestServer

        # Create a mock settings object with the desired rest_api_enabled value
        mock_settings = mocker.MagicMock()
        mock_settings.rest_api_enabled = uses_rest
        mocker.patch("app.main.settings", mock_settings, create=True)

        # Also patch where it's imported inside main.run()
        mocker.patch("app.config.settings.settings.rest_api_enabled", uses_rest)

        spy = mocker.patch.object(start_servers, "start_servers")
        await main.run()

        rest_server_arg = RestServer if uses_rest else None

        spy_args = spy.call_args[0]

        assert spy_args[0] == rabbitmq_connector
        assert spy_args[1] == GrpcServer

        if rest_server_arg is None:
            assert spy_args[2] is None
        else:
            # Checking name because occasionally the class reference comparison
            # would fail due to import context differences
            assert (
                spy_args[2] == rest_server_arg
                or spy_args[2].__name__ == rest_server_arg.__name__
            )


@pytest.mark.asyncio
class TestRunScaler:
    async def test_calls_run_autoscaler(self, mocker):
        from notification_common import scaler

        mock_run_autoscaler = mocker.patch.object(scaler, "run_autoscaler")

        await main.run_scaler()

        mock_run_autoscaler.assert_called_once()
