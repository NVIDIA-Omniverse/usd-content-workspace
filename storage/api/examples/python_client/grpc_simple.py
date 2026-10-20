# SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

import sys

import grpc
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_service_pb2 import (
    StatRequest,
)

# start example
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_service_pb2_grpc import (
    FileObjectServiceStub,
)


def stat(grpc_address: str, resource_address: str):
    with grpc.insecure_channel(grpc_address) as channel:
        storage_api_server = FileObjectServiceStub(channel)
        try:
            response = storage_api_server.Stat(StatRequest(resource_address=resource_address))
            print(f"{resource_address}: size {response.resource_info.metadata.data_object_size}")
        except grpc.RpcError as e:
            print(f"Failure to stat {resource_address}: {str(e)}")


# end example


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python grpc_simple.py <resource_address>")
        exit(1)
    stat(grpc_address="127.0.0.1:50051", resource_address=sys.argv[1])
