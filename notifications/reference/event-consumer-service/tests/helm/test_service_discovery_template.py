# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import shutil
import subprocess
from pathlib import Path

import pytest

from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"

REST_KEY = "discovery.omniverse.nvidia.com/external-rest-endpoint"
GRPC_KEY = "discovery.omniverse.nvidia.com/external-grpc-endpoint"

pytestmark = pytest.mark.skipif(
    shutil.which("helm") is None,
    reason="helm CLI not available",
)


def _render_service(*extra_set: str) -> str:
    args = [
        "helm",
        "template",
        "ec",
        str(CHART_PATH),
        "--namespace",
        "storage-api",
        "--show-only",
        "templates/service.yaml",
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


class TestExternalEndpointsWithProxy:
    # Regression guard: the discovery-service-operator resolves a
    # service's external URL by matching an HTTPProxy whose route backend
    # name equals this Service's name, which can never match here because
    # ingress routes through the separate envoy Service. service.yaml
    # bridges that by deriving external-*-endpoint annotations from
    # httpProxy.fqdn. If these disappear, the consumer silently drops out
    # of the operator's external /api/v2/services list. The design rationale is
    # in the source repository's internal design doc, which is not published.
    @pytest.fixture
    def output(self):
        return _render_service(
            "httpProxy.enabled=true",
            "httpProxy.tls.enabled=true",
            "httpProxy.fqdn.domain=example.com",
        )

    def test_emits_external_rest_endpoint(self, output):
        assert f"{REST_KEY}: https://event-consumer.example.com" in output

    def test_emits_external_grpc_endpoint(self, output):
        assert f"{GRPC_KEY}: https://grpc.event-consumer.example.com" in output


class TestExternalEndpointsSchemeFollowsTls:
    def test_scheme_is_http_when_tls_disabled(self):
        output = _render_service(
            "httpProxy.enabled=true",
            "httpProxy.tls.enabled=false",
            "httpProxy.fqdn.domain=example.com",
        )
        assert f"{REST_KEY}: http://event-consumer.example.com" in output
        assert f"{GRPC_KEY}: http://grpc.event-consumer.example.com" in output


class TestExternalEndpointsWithoutProxy:
    # With no external ingress there is no external face to advertise, so
    # the derived annotations must not be emitted.
    def test_no_external_endpoints_when_proxy_disabled(self):
        output = _render_service("httpProxy.enabled=false")
        assert REST_KEY not in output
        assert GRPC_KEY not in output


class TestExternalEndpointOverride:
    # An explicit value in service.annotations wins over the derived one,
    # so an operator can pin a non-standard URL without losing the other.
    def test_user_value_overrides_derived_grpc(self):
        output = _render_service(
            "httpProxy.enabled=true",
            "httpProxy.tls.enabled=true",
            "httpProxy.fqdn.domain=example.com",
            f"service.annotations.{GRPC_KEY.replace('.', chr(92) + '.')}"
            "=https://custom.example.com",
        )
        assert f"{GRPC_KEY}: https://custom.example.com" in output
        # The non-overridden REST endpoint is still derived.
        assert f"{REST_KEY}: https://event-consumer.example.com" in output
