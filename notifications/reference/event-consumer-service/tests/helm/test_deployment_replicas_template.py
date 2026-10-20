# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

# Regression pin for the guard at templates/deployment.yaml:12-14, which
# omits spec.replicas from the service Deployment whenever the channel-pool
# autoscaler owns it. The failure mode is silent: if the chart ever emits
# spec.replicas while autoscaling is enabled, every routine `helm upgrade`
# writes the chart's replicaCount (default 1) back over whatever the
# autoscaler had scaled the Deployment to. Nothing errors -- the manifest is
# valid, the upgrade succeeds, and the pods simply go away. Applied during an
# incident that is a capacity cut delivered by a deploy, which is exactly the
# 2026-08-11 failure mode this work exists to prevent. See SDD SR-5.6 and
# PRD AC-5 ("a helm upgrade against a deployment scaled above the chart
# default leaves the replica count unchanged").
#
# Renders are scoped with --show-only templates/deployment.yaml on purpose:
# a whole-chart render also contains the AUTOSCALER Deployment, which
# hardcodes replicas: 1, so an unscoped check would match the wrong document.

import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"

# The chart default at values.yaml:10; channelPoolAutoscaling.enabled
# defaults to true at values.yaml:566.
DEFAULT_REPLICA_COUNT = 1
OVERRIDE_REPLICA_COUNT = 4

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


def _deployment(*extra_set: str) -> dict:
    out = _render(*extra_set, show_only="templates/deployment.yaml")
    docs = [d for d in yaml.safe_load_all(out) if d]
    assert len(docs) == 1, f"expected exactly one document, got {len(docs)}:\n{out}"
    assert docs[0]["kind"] == "Deployment", f"unexpected kind:\n{out}"
    return docs[0]


class TestReplicasOmittedWhenAutoscalingEnabled:
    def test_omitted_in_default_render(self):
        doc = _deployment()

        assert "replicas" not in doc["spec"]

    def test_omitted_even_when_replica_count_is_overridden(self):
        # An operator with a stale replicaCount in their values file must
        # not fight the autoscaler -- the strong form of AC-5, and the case
        # a naive "always template replicaCount" refactor would break.
        doc = _deployment(f"replicaCount={OVERRIDE_REPLICA_COUNT}")

        assert "replicas" not in doc["spec"]


class TestReplicasRenderedWhenAutoscalingDisabled:
    def test_renders_chart_default_replica_count(self):
        doc = _deployment("channelPoolAutoscaling.enabled=false")

        assert doc["spec"]["replicas"] == DEFAULT_REPLICA_COUNT

    def test_renders_overridden_replica_count(self):
        # Proves the rendered number follows the values key rather than a
        # hardcoded literal, locking the disabled branch both ways.
        doc = _deployment(
            "channelPoolAutoscaling.enabled=false",
            f"replicaCount={OVERRIDE_REPLICA_COUNT}",
        )

        assert doc["spec"]["replicas"] == OVERRIDE_REPLICA_COUNT
