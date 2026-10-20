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

from io import (
    SEEK_CUR,
    SEEK_END,
    SEEK_SET,
)
from typing import (
    IO,
    Dict,
    Iterable,
    Protocol,
)

import requests


class SupportsRead(Protocol):
    def read(self, length: int = ..., /) -> bytes: ...


class IOSlice:
    """A read-only slice of an underlying file object."""

    def __init__(self, inner: IO[bytes], first: int, last: int):
        self._inner = inner
        self._first = first
        self._last = last
        self._pos = first

    def tell(self) -> int:
        return self._pos - self._first

    def seek(self, pos: int, whence: int = SEEK_SET) -> int:
        if whence == SEEK_SET:
            next_pos = self._first + pos
        elif whence == SEEK_CUR:
            next_pos = self._pos + pos
        elif whence == SEEK_END:
            next_pos = self._last + pos
        else:
            raise ValueError("Unsupported seek method.")

        if next_pos > self._last or next_pos < self._first:
            raise ValueError("Offset is out of bounds.")

        self._pos = next_pos

        return next_pos - self._first

    def seekable(self) -> bool:
        return True

    def readable(self) -> bool:
        return True

    def read(self, n: int = -1) -> bytes:
        self._inner.seek(self._pos, SEEK_SET)
        if n < 0:
            data = self._inner.read(self._last - self._pos)
        else:
            data = self._inner.read(min(self._last - self._pos, n))

        self._pos += len(data)

        return data


def get_content_length(content: IO[bytes]) -> int:
    """Return the length of a file content."""

    first_pos = content.tell()
    content.seek(0, SEEK_END)
    last_pos = content.tell()
    content.seek(first_pos)

    return last_pos - first_pos


def upload_part(
    url: str, method: str, content: SupportsRead, upload_headers: Dict[str, str], return_headers: list[str]
) -> list[tuple[str, str]]:
    """Upload a part returning selected response headers."""

    response = requests.request(url=url, method=method, data=content, headers=upload_headers)
    response.raise_for_status()

    # Create lowercase mapping for case-insensitive header matching
    response_headers_lower = {key.lower(): key for key in response.headers}
    return [
        (header, response.headers[response_headers_lower[header.lower()]])
        for header in return_headers
        if header.lower() in response_headers_lower
    ]


def part_count_for_multipart_upload(
    content: IO[bytes],
    min_part_size: int | None,
    max_part_size: int | None,
) -> tuple[int, int, int]:
    content_length = get_content_length(content)
    part_size = _get_part_size(content_length, min_part_size, max_part_size)
    return content_length // part_size, part_size, content_length


def split_contents_for_multipart_upload(
    content: IO[bytes],
    min_part_size: int | None,
    max_part_size: int | None,
    max_parts: int | None,
) -> Iterable[SupportsRead]:
    """Split contents of a single file into multiple chunks, fitting the multipart upload constraints."""

    parts, part_size, content_length = part_count_for_multipart_upload(content, min_part_size, max_part_size)
    if max_parts and parts > max_parts:
        raise RuntimeError("The maximum number of parts exceeded.")

    extra = content_length % part_size
    extra_per_part = (extra + parts - 1) // parts

    first = 0
    for _ in range(parts):
        last = first + part_size
        if extra:
            current_part_extra = min(extra, extra_per_part)
            last += current_part_extra
            extra -= current_part_extra

        yield IOSlice(content, first, last)

        first = last

    assert last == content_length


def _get_part_size(content_length: int, min_part_size: int | None, max_part_size: int | None) -> int:
    if max_part_size:
        if min_part_size:
            if min_part_size * 2 > max_part_size:
                raise RuntimeError("The maximum size of a part is expected to be at least twice as large as the minimum.")

            return max(min_part_size, content_length // 2)
        else:
            return min(max_part_size // 2, content_length // 2)
    else:
        if min_part_size:
            return max(min_part_size, content_length // 2)
        else:
            return content_length // 2
