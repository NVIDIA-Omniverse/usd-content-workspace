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

# start example


def delete(
    example_server_address: str,
    resource_address: str,
):
    api_url = example_server_address + f"/by-address/{urllib.parse.quote_plus(resource_address)}"
    response = requests.delete(api_url)
    if response.status_code == 204:
        print(f"Done deleting via rest, file {resource_address}!")
    else:
        print(f"Error deleting {resource_address}, result code is {response.status_code}")
        print(response.json())
        raise Exception("Could not delete")


# end example

if __name__ == "__main__":

    parser = argparse.ArgumentParser()
    parser.add_argument("server_address", help="The REST endpoint of the fileobject service, e.g. http://127.0.0.1:8011/fileobject")
    parser.add_argument("resource_address", help="The resource address of the data object to download")
    args = parser.parse_args()

    delete(
        example_server_address=args.server_address,
        resource_address=args.resource_address,
    )
