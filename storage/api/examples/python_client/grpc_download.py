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

import grpc
import requests
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_service_pb2 import (
    DownloadPreference,
    ReadFromAddressRequest,
)

# start example
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_service_pb2_grpc import (
    FileObjectServiceStub,
)


def download(
    grpc_address: str,
    resource_address: str,
    destination_file_name: str,
    download_preference: DownloadPreference,
):
    with grpc.insecure_channel(grpc_address) as channel:
        storage_api_server = FileObjectServiceStub(channel)
        try:
            with open(destination_file_name, "wb") as destination_file:
                for reply in storage_api_server.ReadFromAddress(
                    ReadFromAddressRequest(resource_address=resource_address, download_preference=download_preference)
                ):
                    if reply.HasField("resource_info"):
                        print(f"Found {resource_address}, downloading {reply.resource_info.metadata.data_object_size} bytes")
                    elif reply.HasField("chunk"):
                        destination_file.write(reply.chunk.chunk)
                    elif reply.HasField("redirect"):
                        redirected_response = requests.get(reply.redirect.redirect_target_url, stream=True)
                        if redirected_response.ok:
                            for chunk in redirected_response.iter_content(chunk_size=None):
                                destination_file.write(chunk)
                            print(f"Done downloading from webserver file {destination_file_name} written!")
                            return
                        else:
                            raise Exception(f"{reply.redirect.redirect_target_url} failed to download")
                    else:
                        raise Exception(f"Unexpected reply {reply}")
                print(f"Done downloading via grpc, file {destination_file_name} written!")
        except grpc.RpcError as e:
            print(f"Failure to download {resource_address}: {str(e)}")


# end example


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("resource_address", help="The resource address of the data object to download")
    parser.add_argument("output", help="The name of the output file")
    parser.add_argument("--download-preference", help="File system directory to serve", choices=["body", "redirect"])
    args = parser.parse_args()
    preference: DownloadPreference = DownloadPreference.DOWNLOAD_PREFERENCE_BODY
    if args.download_preference == "body":
        preference = DownloadPreference.DOWNLOAD_PREFERENCE_BODY
    elif args.download_preference == "redirect":
        preference = DownloadPreference.DOWNLOAD_PREFERENCE_REDIRECT
    download(
        grpc_address="127.0.0.1:50051",
        resource_address=args.resource_address,
        destination_file_name=args.output,
        download_preference=preference,
    )
