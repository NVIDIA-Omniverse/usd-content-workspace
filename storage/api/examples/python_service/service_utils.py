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
import binascii
import json
import os

parser = argparse.ArgumentParser()
parser.add_argument("--directory", help="File system directory to serve")
parser.add_argument("--reflection", action="store_true", help="Enable GRPC reflection")
args = parser.parse_args()
if args.directory:
    STATIC_DIR = args.directory
else:
    STATIC_DIR = os.getenv("FILESERVICE_STATIC_DIR")
    if STATIC_DIR is None:
        print("STATIC_DIR environment variable not set and no directory to serve given as command line argument.")
        exit(1)


# start encode identity
def create_identity(relative_path: str, modification_time: str) -> str:
    return binascii.b2a_base64(json.dumps({"p": relative_path, "t": modification_time}).encode("utf-8")).decode("utf-8").strip()


# end encode identity


def path_from_identity(resource_identity: str) -> str:
    data = json.loads(binascii.a2b_base64(resource_identity).decode("utf-8"))
    return data["p"]
