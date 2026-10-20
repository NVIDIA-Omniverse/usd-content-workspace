# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import pytest

from nvidia.omniverse.discovery.capabilities.v2alpha import (
    capabilities_pb2,
    capabilities_service_pb2,
)

from notification_common.grpc.capabilities_servicer import CapabilitiesServicer


@pytest.fixture
def service_entry():
    return capabilities_pb2.ServiceEntry(service_name="test-service")


@pytest.fixture
def servicer(service_entry):
    return CapabilitiesServicer(service_entry)


class TestListServices:
    @pytest.mark.asyncio
    async def test_returns_response_with_the_configured_entry_and_no_paging(
        self, servicer, service_entry
    ):
        response = await servicer.ListServices(
            capabilities_service_pb2.ListServicesRequest(), None
        )
        assert response == capabilities_service_pb2.ListServicesResponse(
            services=[service_entry],
            next_page_token="",
        )
