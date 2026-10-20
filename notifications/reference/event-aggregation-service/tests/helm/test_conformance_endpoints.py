# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""The conformance chart's default REST endpoints hit the port the service serves.

The service chart's Service exposes REST on `rest.port`; the conformance chart
points its Job at a hard-coded URL. Rendering both keeps the two defaults from
drifting apart again (the conformance chart once targeted :8001 while the
Service served :8000).
"""

import subprocess
from urllib.parse import urlsplit

import pytest

from tests.helm.conftest import CHART_PATH, render, requires_helm, single_document

pytestmark = requires_helm

CONFORMANCE_CHART = CHART_PATH.parent / "event-aggregation-conformance-tests"


@pytest.fixture(scope="module")
def service_rest_port() -> int:
    service = single_document(render(show_only="templates/service.yaml"), "Service")
    return next(p["port"] for p in service["spec"]["ports"] if p["name"] == "http")


@pytest.fixture(scope="module")
def conformance_env() -> dict[str, str]:
    result = subprocess.run(
        [
            "helm",
            "template",
            "ct",
            str(CONFORMANCE_CHART),
            "--show-only",
            "templates/job.yaml",
        ],
        capture_output=True,
        text=True,
        check=True,
    )
    job = single_document(result.stdout, "Job")
    container = job["spec"]["template"]["spec"]["containers"][0]
    return {entry["name"]: entry.get("value") for entry in container["env"]}


class TestConformanceEndpoints:
    def test_rest_endpoint_uses_the_service_rest_port(
        self, conformance_env, service_rest_port
    ):
        url = conformance_env["CONFORMANCE_REST_ENDPOINT"]
        assert urlsplit(url).port == service_rest_port

    def test_health_endpoint_uses_the_service_rest_port(
        self, conformance_env, service_rest_port
    ):
        url = conformance_env["CONFORMANCE_HEALTH_ENDPOINT"]
        assert urlsplit(url).port == service_rest_port
