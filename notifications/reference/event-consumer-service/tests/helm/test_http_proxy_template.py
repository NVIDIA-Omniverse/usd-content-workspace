# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import shutil
import subprocess
from pathlib import Path

import pytest

from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"

pytestmark = pytest.mark.skipif(
    shutil.which("helm") is None,
    reason="helm CLI not available",
)


def _render(*extra_set: str) -> str:
    args = [
        "helm",
        "template",
        "ec",
        str(CHART_PATH),
        "--namespace",
        "storage-api",
    ]
    # Before the caller's own overrides: helm takes the last --set, so an
    # explicit policyHook setting in extra_set still wins.
    disable_policy_hook(args)
    select_tokenless_auth(args)
    for kv in extra_set:
        args.extend(["--set", kv])
    result = subprocess.run(args, capture_output=True, text=True)
    assert result.returncode == 0, (
        f"helm template failed:\nstdout:\n{result.stdout}\n" f"stderr:\n{result.stderr}"
    )
    return result.stdout


class TestHttpProxyDisabled:
    def test_renders(self):
        _render()


class TestHttpProxyEnabled:
    # Regression guard for the Go template scope bug fixed in
    # http-proxy.yaml where `.Release.Namespace` inside a `range`
    # block resolved against the loop variable instead of the root
    # context, causing a nil-pointer render failure.
    @pytest.fixture
    def output(self):
        return _render(
            "httpProxy.enabled=true",
            "httpProxy.fqdn.domain=example.com",
        )

    def test_emits_rest_proxy(self, output):
        assert "name: ec-event-consumer-service-rest" in output

    def test_emits_grpc_proxy(self, output):
        assert "name: ec-event-consumer-service-grpc" in output

    def test_namespace_resolves_to_release_namespace(self, output):
        # Both HTTPProxy resources must carry the release namespace.
        assert output.count("namespace: storage-api") >= 2
