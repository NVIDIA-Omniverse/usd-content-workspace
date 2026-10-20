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
import json
import urllib.parse

import requests


# start body example
def download_via_body(example_server_address: str, resource_address: str, destination_file_name: str):
    api_url = example_server_address + f"/by-address/{urllib.parse.quote_plus(resource_address)}?download_preference=body"
    response = requests.get(api_url, stream=True)
    if response.status_code == 200:
        with open(destination_file_name, "wb") as downloaded_file:
            for chunk in response.iter_content(chunk_size=None):
                downloaded_file.write(chunk)
        print(f"finished writing to file {destination_file_name}")
    else:
        print(f"Error downloading {resource_address}, result code is {response.status_code}")


# end body example


# start redirect example
def download_via_redirect(example_server_address: str, resource_address: str, destination_file_name: str):
    api_url = example_server_address + f"/by-address/{urllib.parse.quote_plus(resource_address)}?download_preference=redirect"
    response = requests.get(api_url, stream=True)
    if response.status_code == 300:
        redirection_properties = json.loads(response.content)
        redirected_response = requests.get(
            redirection_properties["redirect_target_url"], headers=redirection_properties["additional_headers"], stream=True
        )
        if redirected_response.status_code == 200:
            with open(destination_file_name, "wb") as downloaded_file:
                for chunk in redirected_response.iter_content(chunk_size=None):
                    downloaded_file.write(chunk)
            print(f"finished writing to file {destination_file_name}")
        else:
            print(f"Error downloading {resource_address} after redirection, result code is {redirected_response.status_code}")
    else:
        print(f"Error downloading {resource_address}, result code is {response.status_code}")


# end redirect example


# start complete download example
def download(example_server_address: str, resource_address: str, destination_file_name: str):
    api_url = example_server_address + f"/by-address/{urllib.parse.quote_plus(resource_address)}"
    response = requests.get(api_url, stream=True)
    if response.status_code == 200:
        with open(destination_file_name, "wb") as downloaded_file:
            for chunk in response.iter_content(chunk_size=None):
                downloaded_file.write(chunk)
        print(f"finished writing to file {destination_file_name}")
    elif response.status_code == 300:
        redirection_properties = json.loads(response.content)
        if redirection_properties["method"] == "get":
            redirected_response = requests.get(redirection_properties["redirect_target_url"], stream=True)
        elif redirection_properties["method"] == "post":
            redirected_response = requests.post(redirection_properties["redirect_target_url"], stream=True)
        else:
            raise RuntimeError(f"Download verb {redirection_properties['method']} not implemented")

        if redirected_response.status_code == 200:
            with open(destination_file_name, "wb") as downloaded_file:
                for chunk in redirected_response.iter_content(chunk_size=None):
                    downloaded_file.write(chunk)
            print(f"finished writing to file {destination_file_name}")
        else:
            print(f"Error downloading {resource_address} after redirection, result code is {redirected_response.status_code}")
    else:
        print(f"Error downloading {resource_address}, result code is {response.status_code}")


# end complete download example

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("server_address", help="The REST endpoint of the fileobject service, e.g. http://127.0.0.1:8011/fileobject")
    parser.add_argument("resource_address", help="The resource address of the data object to download")
    parser.add_argument("output", help="The name of the output file")
    parser.add_argument("--download_preference", help="File system directory to serve", choices=["body", "redirect"])
    args = parser.parse_args()

    if args.download_preference == "body":
        print("Downloading via response body...")
        download_via_body(
            example_server_address=args.server_address, resource_address=args.resource_address, destination_file_name=args.output
        )
    elif args.download_preference == "redirect":
        print("Downloading via redirect...")
        download_via_redirect(
            example_server_address=args.server_address, resource_address=args.resource_address, destination_file_name=args.output
        )
    else:
        print("Downloading via default method...")
        download(example_server_address=args.server_address, resource_address=args.resource_address, destination_file_name=args.output)
