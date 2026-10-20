# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""gRPC servicer implementing the Discovery CapabilitiesService.

The v2 Discovery Kubernetes operator enumerates services by calling
``ListServices`` on each server and building its registry from the
responses. This servicer advertises a single, statically-configured
``ServiceEntry`` describing the host service's gRPC and REST APIs,
matching the permission-service reference implementation: the response
carries exactly one entry and pagination is ignored.
"""

from nvidia.omniverse.discovery.capabilities.v2alpha import (
    capabilities_pb2,
    capabilities_service_pb2,
    capabilities_service_pb2_grpc,
)


class CapabilitiesServicer(capabilities_service_pb2_grpc.CapabilitiesServiceServicer):
    """Serves a fixed ``ServiceEntry`` describing this server's APIs."""

    def __init__(self, service_entry: capabilities_pb2.ServiceEntry):
        self._service_entry = service_entry

    async def ListServices(
        self,
        request: capabilities_service_pb2.ListServicesRequest,
        context,
    ) -> capabilities_service_pb2.ListServicesResponse:
        """Return the single configured ServiceEntry, ignoring pagination."""
        return capabilities_service_pb2.ListServicesResponse(
            services=[self._service_entry],
            next_page_token="",
        )
