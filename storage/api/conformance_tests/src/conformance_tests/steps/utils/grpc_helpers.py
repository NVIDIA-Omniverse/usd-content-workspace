# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

import grpc


def map_grpc_codes(grpc_code):
    status_code_map = {
        "NOT_FOUND": grpc.StatusCode.NOT_FOUND,
        "PERMISSION_DENIED": grpc.StatusCode.PERMISSION_DENIED,
        "INVALID_ARGUMENT": grpc.StatusCode.INVALID_ARGUMENT,
        "FAILED_PRECONDITION": grpc.StatusCode.FAILED_PRECONDITION,
        "ALREADY_EXISTS": grpc.StatusCode.ALREADY_EXISTS,
        "UNIMPLEMENTED": grpc.StatusCode.UNIMPLEMENTED,
        "UNKNOWN": grpc.StatusCode.UNKNOWN,
    }

    if grpc_code not in status_code_map:
        raise ValueError(f"Unmapped grpc Status code: {grpc_code}, please add to map")

    return status_code_map[grpc_code]


def map_download_preference(download_preference, scenario_state):
    if download_preference == "body":
        return scenario_state.sapi_test_client.fileobject_pb2(
            scenario_state.version_under_test["fileobject"]
        ).DownloadPreference.DOWNLOAD_PREFERENCE_BODY
    elif download_preference == "redirect":
        return scenario_state.sapi_test_client.fileobject_pb2(
            scenario_state.version_under_test["fileobject"]
        ).DownloadPreference.DOWNLOAD_PREFERENCE_REDIRECT
    else:
        raise ValueError(f"unknown download preference, must be one of body or redirect: {download_preference}")
