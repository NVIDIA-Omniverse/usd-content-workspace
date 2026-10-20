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
from xml.etree import ElementTree as ET

from discovery_conformance.reports import exit_code, write_reports


def test_exit_codes():
    assert exit_code([]) == 3
    for status, expected in [("PASS", 0), ("FAIL", 1), ("SKIP", 3), ("INCONCLUSIVE", 3)]:
        assert exit_code([{"status": status}]) == expected
    assert exit_code([{"status": "PASS"}, {"status": "INCONCLUSIVE"}]) == 3
    assert exit_code([{"status": "PASS"}, {"status": "FAIL"}]) == 1


def test_reports(tmp_path):
    checks = [{"id": status, "status": status, "message": "safe <message>"}
              for status in ("PASS", "FAIL", "SKIP", "INCONCLUSIVE")]
    json_file, xml_file = tmp_path / "result.json", tmp_path / "result.xml"
    write_reports(checks, {"version": "test"}, json_file, xml_file)
    assert json.loads(json_file.read_text())["exit_code"] == 1
    root = ET.parse(xml_file).getroot()
    assert root.attrib == {"name": "discovery-conformance", "tests": "4", "failures": "1", "errors": "1", "skipped": "1"}
    assert root.find("testcase/failure").attrib["message"] == "safe <message>"
