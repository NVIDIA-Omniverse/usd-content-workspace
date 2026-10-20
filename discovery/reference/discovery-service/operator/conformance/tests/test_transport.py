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

import requests
import pytest

from discovery_conformance.transport import get


def test_transport_disables_netrc_and_redirects(monkeypatch):
    observed = {}
    def fake_get(self, url, **kwargs):
        observed.update(kwargs)
        observed["trust_env"] = self.trust_env
        observed["proxies"] = self.proxies.copy()
        return requests.Response()
    monkeypatch.setattr(requests.Session, "get", fake_get)
    monkeypatch.setattr(requests.utils, "get_environ_proxies", lambda url: {"https": "http://proxy.test"})
    get("https://example.test", token="explicit-token", timeout=3)
    assert observed["trust_env"] is False
    assert observed["allow_redirects"] is False
    assert observed["headers"]["Authorization"] == "Bearer explicit-token"
    assert observed["proxies"] == {"https": "http://proxy.test"}
    assert observed["verify"] is True


@pytest.mark.parametrize("url, options", [
    ("http://example.test", {"token": "secret"}),
    ("https://example.test", {"ca_bundle": False}),
])
def test_unsafe_transport_rejected_before_request(monkeypatch, url, options):
    monkeypatch.setattr(requests.Session, "get", lambda *args, **kwargs: pytest.fail("Unsafe request"))
    with pytest.raises(ValueError):
        get(url, **options)
