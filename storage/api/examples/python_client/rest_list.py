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
from typing import (
    Iterable,
    Optional,
    TypedDict,
)
from urllib.parse import quote_plus

from requests import Session

REST_SERVICE_ENDPOINT = os.environ.get("TEST_STORAGE_API_REST_ENDPOINT", "http://localhost:8011")
REST_SERVICE_ADDRESS = f"{REST_SERVICE_ENDPOINT}/v1alpha"


class Metadata(TypedDict):
    data_object_size: int
    last_modified_timestamp: Optional[str]


class ListItem(TypedDict):
    resource_address: str
    resource_identity: str
    metadata: Metadata


class FolderAddress(TypedDict):
    uri: str


class ListFileFoldersResponse(TypedDict):
    subfolder_addresses: list[FolderAddress]
    sub_resource_addresses: list[str]


class ListStatFileFoldersResponse(TypedDict):
    subfolder_addresses: list[FolderAddress]
    entries: list[ListItem]


# start example
def list_file_folders(resource_address: str, stat: bool = False, max_page_size: int | None = None):
    with Session() as session:
        if stat:
            for list_stat_response in list_stat_file_folders_paginated(session, resource_address, max_page_size):
                handle_list_stat_file_folders_response(list_stat_response)
        else:
            for list_response in list_file_folders_paginated(session, resource_address, max_page_size):
                handle_list_file_folders_response(list_response)


def fetch_paginated(session: Session, url: str, max_page_size: int | None) -> Iterable[dict]:
    query_params = {}
    if max_page_size:
        query_params["max_page_size"] = max_page_size

    while True:
        response = session.get(url, params=query_params)
        response.raise_for_status()

        response_json = response.json()
        yield response_json

        if continuation_handle := response_json.get("next_continuation_handle"):
            query_params["continuation_handle"] = continuation_handle
        else:
            break


def list_file_folders_paginated(
    session: Session,
    address: str,
    max_page_size: int | None,
) -> Iterable[ListFileFoldersResponse]:
    return fetch_paginated(
        session,
        f"{REST_SERVICE_ADDRESS}/filefolder/list/{quote_plus(address)}",
        max_page_size,
    )  # type: ignore[return-value]


def list_stat_file_folders_paginated(
    session: Session,
    address: str,
    max_page_size: int | None,
) -> Iterable[ListStatFileFoldersResponse]:
    return fetch_paginated(
        session,
        f"{REST_SERVICE_ADDRESS}/filefolder/liststat/{quote_plus(address)}",
        max_page_size,
    )  # type: ignore[return-value]


# end example


def handle_list_file_folders_response(response: ListFileFoldersResponse):
    for resource_address in response["sub_resource_addresses"]:
        print(f"folder; address={resource_address!r}")

    for folder_address in response["subfolder_addresses"]:
        print(f"data object; address={folder_address['uri']!r}")


def handle_list_stat_file_folders_response(response: ListStatFileFoldersResponse):
    for entry in response["entries"]:
        print(
            f"data object; address={entry['resource_address']!r}, "
            f"identity={entry['resource_identity']!r}, "
            f"size={entry['metadata']['data_object_size']}, "
            f"last_modified={entry['metadata']['last_modified_timestamp']!r}"
        )

    for folder_address in response["subfolder_addresses"]:
        print(f"folder; address={folder_address['uri']!r}")


def main():
    parser = ArgumentParser()
    parser.add_argument("resource_address", type=str)
    parser.add_argument("--stat", default=False, action="store_true")
    parser.add_argument("--max-page-size", default=None, type=int)

    args = parser.parse_args()

    list_file_folders(args.resource_address, args.stat, args.max_page_size)


if __name__ == "__main__":
    main()
