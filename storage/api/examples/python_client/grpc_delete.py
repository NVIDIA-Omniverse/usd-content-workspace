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

import argparse
import sys

import grpc
import requests
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_pb2 import ResourceIdentity
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_service_pb2 import (
    DeleteRequest,
)

# start example
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_service_pb2_grpc import (
    FileObjectServiceStub,
)


def delete(
    grpc_address: str,
    resource_address: str,
):
    with grpc.insecure_channel(grpc_address) as channel:
        storage_api_server = FileObjectServiceStub(channel)
        try:
            reply = storage_api_server.Delete(DeleteRequest(resource_address=resource_address))
            print(f"Done deleting via grpc, file {resource_address}!")
        except grpc.RpcError as e:
            print(f"Failure to delete {resource_address}: {str(e)}")


# end example

if __name__ == "__main__":

    parser = argparse.ArgumentParser()
    parser.add_argument("resource_address", help="The resource address of the data object to delete")
    args = parser.parse_args()
    delete(
        "127.0.0.1:50051",
        resource_address=args.resource_address,
    )
