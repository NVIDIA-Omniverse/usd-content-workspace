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

import os
from argparse import ArgumentParser

from grpc import (
    Channel,
    insecure_channel,
)
from nvidia.omniverse.storage.filefolder.v1alpha.filefolder_service_pb2 import (
    FolderAddress,
    ListRequest,
    ListResponse,
    ListStatRequest,
    ListStatResponse,
)
from nvidia.omniverse.storage.filefolder.v1alpha.filefolder_service_pb2_grpc import (
    FileFolderServiceStub,
)

GRPC_SERVICE_ADDRESS = os.environ.get("TEST_STORAGE_API_GRPC_ENDPOINT", "localhost:50051")


# start example
def list_file_folders(channel: Channel, resource_address: str, stat: bool = False):
    client = FileFolderServiceStub(channel)

    if stat:
        for list_stat_message in client.ListStat(ListStatRequest(folder=FolderAddress(uri=resource_address))):
            handle_list_stat_message(list_stat_message)
    else:
        for list_message in client.List(ListRequest(folder=FolderAddress(uri=resource_address))):
            handle_list_message(list_message)


# end example


def handle_list_message(response: ListResponse):
    for resource_address in response.sub_resource_addresses:
        print(f"data object; address={resource_address!r}")

    for folder_address in response.subfolder_addresses:
        print(f"folder; address={folder_address.uri!r}")


def handle_list_stat_message(response: ListStatResponse):
    for entry in response.entries:
        print(
            f"data object; address={entry.resource_address!r}, "
            f"identity={entry.resource_info.resource_identity.encoded_identity!r}, "
            f"size={entry.resource_info.metadata.data_object_size}, "
            f"last_modified={entry.resource_info.metadata.last_modified_timestamp.ToDatetime()}"
        )

    for resource_address in response.subfolder_addresses:
        print(f"folder; address={resource_address!r}")


def main():
    parser = ArgumentParser()
    parser.add_argument("resource_address", type=str)
    parser.add_argument("--stat", default=False, action="store_true")

    args = parser.parse_args()

    with insecure_channel(GRPC_SERVICE_ADDRESS) as channel:
        list_file_folders(channel, args.resource_address, args.stat)


if __name__ == "__main__":
    main()
