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
import sys
from argparse import ArgumentParser
from typing import (
    IO,
    TypedDict,
)
from urllib.parse import quote_plus

import requests
from utils import (
    SupportsRead,
    get_content_length,
    part_count_for_multipart_upload,
    split_contents_for_multipart_upload,
    upload_part,
)

REST_SERVICE_ENDPOINT = os.environ.get("TEST_STORAGE_API_REST_ENDPOINT", "http://localhost:8011")
BASE_URL = f"{REST_SERVICE_ENDPOINT}/v1alpha"


class HttpHeader(TypedDict):
    name: str
    value: str


class WriteRedirectParams(TypedDict):
    method: str
    redirect_target_url: str
    additional_headers: list[HttpHeader]
    completion_header_names: list[str]


class MultipartUploadParams(TypedDict):
    upload_id: str
    first_part_write_redirect: WriteRedirectParams
    minimum_size_per_part: int | None
    maximum_size_per_part: int | None
    maximum_parts_number: int | None


class PreparePartResponse(TypedDict):
    redirect: WriteRedirectParams


class CompletedUploadPart(TypedDict):
    part_number: int
    additional_headers: list[HttpHeader]


class WriteResponse(TypedDict):
    multipart: MultipartUploadParams | None
    redirect: WriteRedirectParams | None


# start example
def upload(address: str, upload_preference: str | None, content: IO[bytes]):
    """Upload a file object at the specified address using REST interface."""

    content_length = get_content_length(content)
    if not upload_preference:
        upload_preference = _get_upload_preference(address, content_length)

    write_response = requests.put(
        f"{BASE_URL}/fileobject/by-address/{quote_plus(address)}",
        data=content,
        headers={
            "Expect": "100-continue",
        },
        params={
            key: value
            for key, value in (
                ("data_object_size", content_length),
                ("upload_preference", upload_preference),
            )
            if value is not None
        },
    )
    write_response.raise_for_status()

    if write_response.status_code == 201:
        return

    if write_response.status_code == 300:
        content.seek(0)

        write_response_json: WriteResponse = write_response.json()
        if (redirect := write_response_json.get("redirect")) is not None:
            return _write_via_redirect(address, redirect, content)
        if (multipart := write_response_json.get("multipart")) is not None:
            return _write_via_multipart(address, multipart, content)

    raise Exception("PUT write got unexpected response.")


# end example


# start redirect write example
def _write_via_redirect(address: str, params: WriteRedirectParams, content: IO[bytes]):
    response = requests.request(
        url=params["redirect_target_url"],
        method=params["method"].upper(),
        headers={h["name"]: h["value"] for h in params["additional_headers"]},
        data=content,
    )
    response.raise_for_status()

    # Optionally, obtain the resource information of the just uploaded file object
    # This is not required to store the data, but helpful to keep the reference to the stored object.
    requested_headers = {name.lower() for name in params["completion_header_names"]}
    completion = requests.post(
        f"{BASE_URL}/fileobject/by-address/{quote_plus(address)}/redirect/complete",
        json={
            "additional_headers": [
                {"name": response_key, "value": response.headers[response_key]}
                for response_key in response.headers
                if response_key.lower() in requested_headers
            ]
        },
    )
    completion.raise_for_status()


# end redirect write example


# start multipart write example
def _write_via_multipart(address: str, params: MultipartUploadParams, content: IO[bytes]):
    # Build a list of redirect URLs to upload the parts to. The first we got from the CreateMultipartUploadResponse
    redirects = [params["first_part_write_redirect"]]

    # Calculate the number of parts, and retrieve the pre-signed URLs from the service to use for individual part upload
    total_part_count, part_size, content_length = part_count_for_multipart_upload(
        content, params["minimum_size_per_part"], params["maximum_size_per_part"]
    )
    if total_part_count > 1:
        response = requests.post(
            f"{BASE_URL}/fileobject/by-address/{quote_plus(address)}/multipart/prepare",
            json={
                "upload_id": params["upload_id"],
                "part_number": 1,
                "part_count": total_part_count - 1,
            },
        )
        if response.status_code != 200:
            print(response)
            raise Exception("POST multipart prepare got unexpected response.")

        redirects.extend(response.json()["part_write_redirects"])

    response = requests.post(
        f"{BASE_URL}/fileobject/by-address/{quote_plus(address)}/multipart/complete",
        json={
            "upload_id": params["upload_id"],
            "parts": [
                _write_part(part_number, redirects[part_number], part)
                for part_number, part in enumerate(
                    split_contents_for_multipart_upload(
                        content,
                        max_part_size=params.get("maximum_size_per_part"),
                        min_part_size=params.get("minimum_size_per_part"),
                        max_parts=params.get("maximum_parts_number"),
                    ),
                )
            ],
        },
    )
    if response.status_code != 200:
        raise Exception("Post multipart complete got unexpected response.")


def _write_part(part_number: int, redirect, content: SupportsRead) -> CompletedUploadPart:
    return {
        "part_number": part_number,
        "additional_headers": [
            {
                "name": name,
                "value": value,
            }
            for name, value in upload_part(
                redirect["redirect_target_url"],
                redirect["method"],
                content,
                upload_headers=dict([(h["name"], h["value"]) for h in redirect["additional_headers"]]),
                return_headers=redirect["completion_header_names"],
            )
        ],
    }


# end multipart write example


# start upload preference example
def _get_upload_preference(address: str, size: int) -> str | None:
    response = requests.get(f"{BASE_URL}/fileobject/upload-options/by-address/{quote_plus(address)}")
    response.raise_for_status()

    for interval in response.json()["write_type_intervals"]:
        min_size = interval.get("minimum_data_object_size")
        max_size = interval.get("maximum_data_object_size")
        if min_size and min_size > size:
            continue

        if max_size and max_size < size:
            continue

        return interval["preferred_upload_method"]

    return None


# end upload preference example


def main():
    parser = ArgumentParser()
    parser.add_argument("resource_address", type=str)
    parser.add_argument("file", type=str)
    parser.add_argument("--upload-preference", default=None, type=str)

    args = parser.parse_args()

    try:
        with open(args.file, mode="rb") as content:
            upload(args.resource_address, args.upload_preference, content)
    except Exception as exception:
        print(exception)
        sys.exit(1)


if __name__ == "__main__":
    main()
