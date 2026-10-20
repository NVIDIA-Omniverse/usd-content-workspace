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

import pytest

from discovery_conformance.cli import main


@pytest.mark.parametrize("arguments", [[], ["--expect-service", "x"],
    ["--catalog-url", "https://user:secret@example.test/api/v2beta/services"],
    ["--catalog-url", "https://example.test", "--timeout", "nan"],
    ["--grpc-target", "https://example.test:443"],
    ["--grpc-target", "example.test:not-a-port"],
    ["--grpc-target", "example.test:0"],
    ["--grpc-target", "example.test:70000"],
    ["--grpc-target", "example.test:443", "--page-size", "2147483648"],
    ["--plaintext-grpc"], ["--catalog-url", "https://example.test", "--max-pages", "0"]])
def test_invalid_configuration(arguments):
    with pytest.raises(SystemExit) as error:
        main(arguments)
    assert error.value.code == 2


def test_token_never_sent_over_http(tmp_path):
    token = tmp_path / "token"
    token.write_text("sensitive-value")
    with pytest.raises(SystemExit) as error:
        main(["--catalog-url", "http://example.test/api/v2beta/services", "--token-file", str(token)])
    assert error.value.code == 2


def test_selected_catalog_and_report(monkeypatch, tmp_path):
    from discovery_conformance import catalog
    monkeypatch.setattr(catalog, "check_catalog", lambda *args, **kwargs: [
        {"id": "catalog.test", "status": "PASS", "message": "validated"}])
    assert main(["--catalog-url", "http://example.test/api/v2beta/services", "--json-report", str(tmp_path / "report.json")]) == 0
