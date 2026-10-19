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

import json
import sys
import urllib.parse

import requests


# start example
def stat(example_server_address: str, resource_address: str):
    api_url = example_server_address + f"/by-address/{urllib.parse.quote_plus(resource_address)}"
    response = requests.head(api_url)
    if response.status_code == 204:
        result = json.loads(response.headers["x-nvidia-omniverse-storage-metadata"])
        print(result)
    else:
        print(f"Error calling stat, result code is {response.status_code}")


# end example


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python rest_simple.py <resource_address>")
        exit(1)
    print(f"Running stat on {sys.argv[1]}")
    stat(example_server_address="http://127.0.0.1:8011/fileobject", resource_address=sys.argv[1])
