# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

# Regression guard for the deliberate two-Service topology introduced in
# commit 0d69f8d ("preserve streaming connections when pod reaches
# capacity"). The consumer serves hours-long streams, and its non-durable
# admission is two-tier. A pod flips its readiness probe to not-ready once
# free stream slots fall below the readiness reserve
# (rabbitmq.readiness_min_available_channels), just short of its soft
# threshold (rabbitmq.channel_pool_size x rabbitmq.channel_client_limit).
# That flip is a SCALING SIGNAL, not a refusal: such a pod keeps admitting,
# packing pinned non-durable streams into the overflow band above the soft
# threshold. Re-routable streams (all REST/SSE, plus gRPC consume carrying
# the X-Via-Proxy stamp) are refused earlier, at the shed threshold
# (rabbitmq.max_re_routable_streams_per_pod), which is what reserves that
# band for pinned traffic; the memory-safe cap is the one hard refusal, and
# it applies to everyone non-durable, pinned included.
#
# A normal readiness-gated Service would evict a not-ready pod from Envoy's
# EDS cluster, tearing down its existing streams (RST_STREAM) and cutting
# its pinned clients off from the burst headroom they cannot be rescheduled
# to reach -- a pinned connection is bound to its pod for life, so scaling
# up cannot rescue it. The fix is two Services with opposite readiness
# semantics: the main Service stays readiness gated (internal traffic + the
# discovery operator's capability query), while a sibling
# publishNotReadyAddresses Service fronts the HTTPProxy so not-ready pods
# stay in EDS, keeping their streams alive and themselves reachable for
# pinned-burst absorption. Collapsing the two reintroduces that bug, so
# these tests pin the load-bearing invariants. The chart's service-envoy.yaml
# comment states the rationale.

import shutil
import subprocess
from pathlib import Path

import pytest

from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"

ENVOY_NAME = "event-consumer-service-envoy"

pytestmark = pytest.mark.skipif(
    shutil.which("helm") is None,
    reason="helm CLI not available",
)


def _render(*extra_set: str, show_only: str | None = None) -> str:
    args = ["helm", "template", "ec", str(CHART_PATH), "--namespace", "storage-api"]
    if show_only is not None:
        args.extend(["--show-only", show_only])
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


class TestEnvoyServiceIsLoadBearing:
    @pytest.fixture
    def output(self):
        return _render(
            "httpProxy.enabled=true",
            "httpProxy.fqdn.domain=example.com",
            show_only="templates/service-envoy.yaml",
        )

    def test_publishes_not_ready_addresses(self, output):
        # THE load-bearing property: without it, a pod that went
        # not-ready at the readiness reserve leaves Envoy's EDS cluster --
        # its existing streams are torn down, and its pinned clients lose
        # the overflow band they cannot be rescheduled to reach.
        assert "publishNotReadyAddresses: true" in output

    def test_named_as_the_envoy_service(self, output):
        assert f"name: {ENVOY_NAME}" in output

    def test_not_annotated_for_discovery(self, output):
        # The discovery annotation belongs on the readiness-gated main
        # Service, never on the not-ready-publishing envoy Service.
        assert "discovery.omniverse.nvidia.com/" not in output


class TestEnvoyServiceGatedOnIngress:
    def test_absent_when_proxy_disabled(self):
        # No external ingress -> no envoy Service to render.
        output = _render("httpProxy.enabled=false")
        assert ENVOY_NAME not in output


class TestMainServiceStaysReadinessGated:
    def test_main_service_does_not_publish_not_ready(self):
        # Asserts the spec field specifically -- the main Service's own
        # comment mentions the word "publishNotReadyAddresses", so a bare
        # substring check would false-match.
        output = _render(show_only="templates/service.yaml")
        assert "publishNotReadyAddresses: true" not in output


class TestProxyRoutesThroughEnvoy:
    @pytest.fixture
    def output(self):
        return _render(
            "httpProxy.enabled=true",
            "httpProxy.fqdn.domain=example.com",
            show_only="templates/http-proxy.yaml",
        )

    def test_route_backends_target_envoy(self, output):
        assert f"name: {ENVOY_NAME}" in output

    def test_no_route_backend_targets_main_service(self, output):
        # A route backend targeting the bare (readiness-gated) main
        # Service would sever live streams the moment a pod went not-ready
        # at the readiness reserve, and would strand the pinned bursts that
        # pod is still meant to absorb. The envoy backend is "...-envoy",
        # so an exact line match distinguishes them.
        backend_lines = [
            line.strip() for line in output.splitlines() if "- name:" in line
        ]
        assert "- name: event-consumer-service" not in backend_lines
