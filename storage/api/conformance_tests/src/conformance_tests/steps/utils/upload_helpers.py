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

from typing import (
    Dict,
    Iterable,
    List,
)

import requests


def split_into_chunks(data: bytes, min_chunk_size: int | None, max_chunk_size: int | None, max_chunks: int | None) -> Iterable[bytes]:
    """Split contents of a data object into at least two chunks fitting the limits."""
    if len(data) == 0:
        return

    min_chunk_size = min_chunk_size or 0
    max_chunk_size = max_chunk_size or max(len(data), min_chunk_size)

    chunk_base_size = max(min_chunk_size, len(data) // 2)
    chunks = (len(data) + chunk_base_size - 1) // chunk_base_size
    chunk_extra = len(data) % chunk_base_size
    assert max_chunk_size >= chunk_base_size + 1
    assert chunks >= 2
    assert not max_chunks or chunks <= max_chunks

    offset = 0
    for idx in range(chunks):
        last = offset + chunk_base_size + (1 if idx <= chunk_extra else 0)
        yield data[offset:last]
        offset = last


def perform_http_upload(data: bytes, method: str, url: str, headers: dict[str, str] | None) -> requests.Response:
    """Upload a data object via a redirect."""
    if isinstance(headers, list):
        headers = {h["name"]: h["value"] for h in headers}

    response = requests.request(url=url, method=method, headers=headers, data=data)
    assert 200 <= response.status_code < 300

    return response


def upload_part_to_http_server(
    data: bytes, method: str, url: str, header_names: List[str], upload_headers: Dict[str, str]
) -> Dict[str, str]:
    """Upload a part of a data object returning the values of the asked headers."""

    response = requests.request(url=url, method=method, data=data, headers=upload_headers)
    assert 200 <= response.status_code < 300

    # Create lowercase mapping for case-insensitive header matching
    response_headers_lower = {key.lower(): key for key in response.headers}

    return {
        header_name: response.headers[response_headers_lower[header_name.lower()]]
        for header_name in header_names
        if header_name.lower() in response_headers_lower
    }
