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

"""Deterministic exit status and machine-readable reports."""

import json
from pathlib import Path
from xml.etree import ElementTree as ET


def exit_code(checks):
    if not checks:
        return 3
    statuses = {check["status"] for check in checks}
    if "FAIL" in statuses:
        return 1
    if "INCONCLUSIVE" in statuses or statuses <= {"SKIP"}:
        return 3
    return 0


def write_reports(checks, provenance, json_path=None, junit_path=None):
    if json_path:
        Path(json_path).write_text(json.dumps({
            "format_version": 1, "specs": provenance,
            "checks": checks, "exit_code": exit_code(checks),
        }, indent=2) + "\n", encoding="utf-8")
    if junit_path:
        suite = ET.Element("testsuite", name="discovery-conformance",
                           tests=str(len(checks)),
                           failures=str(sum(c["status"] == "FAIL" for c in checks)),
                           errors=str(sum(c["status"] == "INCONCLUSIVE" for c in checks)),
                           skipped=str(sum(c["status"] == "SKIP" for c in checks)))
        for check in checks:
            case = ET.SubElement(suite, "testcase", name=check["id"])
            tag = {"FAIL": "failure", "INCONCLUSIVE": "error", "SKIP": "skipped"}.get(check["status"])
            if tag:
                ET.SubElement(case, tag, message=check["message"])
            else:
                ET.SubElement(case, "system-out").text = check["message"]
        ET.ElementTree(suite).write(junit_path, encoding="utf-8", xml_declaration=True)
