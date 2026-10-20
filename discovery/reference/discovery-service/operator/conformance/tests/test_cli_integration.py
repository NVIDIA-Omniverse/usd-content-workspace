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

import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
import subprocess
import sys
from threading import Thread
from xml.etree import ElementTree as ET

import pytest


@pytest.fixture
def http_backend(monkeypatch):
    requests_seen = []

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            requests_seen.append(self.path)
            payload = {
                "schema-version": 2,
                "services": [{"name": "fileobject", "capability_source": "grpc"}],
            }
            status = 200
            if self.path == "/bad":
                payload["schema-version"] = 1
            elif self.path == "/rest":
                payload = {"services": []}
            elif self.path == "/redirect":
                status = 302
            body = json.dumps(payload).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Cache-Control", "must-revalidate, max-age=30")
            self.send_header("Content-Length", str(len(body)))
            if status == 302:
                self.send_header("Location", "/valid")
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    monkeypatch.setenv("NO_PROXY", "127.0.0.1")
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{server.server_port}", requests_seen
    server.shutdown()
    server.server_close()
    thread.join()


def run_cli(*args):
    return subprocess.run([sys.executable, "-m", "discovery_conformance", *args],
                          env=os.environ.copy(), capture_output=True, text=True, timeout=10)


def test_real_http_cli_and_reports(http_backend, tmp_path):
    base, seen = http_backend
    json_report, junit_report = tmp_path / "result.json", tmp_path / "result.xml"
    result = run_cli("--catalog-url", base + "/valid", "--expect-service", "fileobject",
                     "--rest-capabilities-url", base + "/rest",
                     "--json-report", str(json_report), "--junit-report", str(junit_report))
    assert result.returncode == 0, result.stdout + result.stderr
    report = json.loads(json_report.read_text())
    assert report["exit_code"] == 0
    assert all(check["status"] == "PASS" for check in report["checks"])
    assert report["specs"]["discovery_protos_revision"]
    assert ET.parse(junit_report).getroot().attrib["failures"] == "0"
    assert seen == ["/valid", "/rest"]


@pytest.mark.parametrize("path", ["/bad", "/redirect"])
def test_real_http_negative_paths(http_backend, path):
    base, seen = http_backend
    result = run_cli("--catalog-url", base + path)
    assert result.returncode == 1
    assert "FAIL" in result.stdout
    assert seen == [path]


def test_real_http_missing_expected_service(http_backend):
    base, _ = http_backend
    result = run_cli("--catalog-url", base + "/valid", "--expect-service", "not-deployed")
    assert result.returncode == 1
    assert "catalog.inventory" in result.stdout
