# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

_json = {
    "sub_resource_addresses": ["a", "b"],
    "subfolder_addresses": ["dir_a", "dir_b"],
}


class MockHttpResponse:
    # noinspection PyDefaultArgument
    def __init__(self, json_response: dict = _json, status=200, text="test-msg"):
        self.json_response = json_response
        self.status = status
        self.text_response = text

    async def json(self) -> dict:
        return self.json_response

    async def text(self) -> str:
        return self.text_response
