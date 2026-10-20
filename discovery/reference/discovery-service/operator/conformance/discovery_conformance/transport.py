# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

"""Explicit HTTP authentication without ambient netrc credentials."""

import requests
from urllib.parse import urlsplit


def get(url, *, token=None, ca_bundle=True, timeout=10):
    if ca_bundle is False or (token is not None and urlsplit(url).scheme != "https"):
        raise ValueError("TLS verification and encrypted bearer credentials are required")
    headers = {"Accept": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    with requests.Session() as session:
        # Preserve proxy routing without allowing requests to load .netrc auth.
        session.trust_env = False
        session.proxies.update(requests.utils.get_environ_proxies(url))
        return session.get(url, headers=headers, verify=ca_bundle,
                           timeout=timeout, allow_redirects=False)
