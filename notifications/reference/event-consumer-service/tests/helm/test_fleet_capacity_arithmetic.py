# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""SR-7.3 / PRD AC-2 fleet-capacity arithmetic over rendered chart values.

This file IS the SR-7.3 record: it asserts that a >=12,000 concurrent
non-durable stream fleet target is reachable by changing helm VALUES ONLY --
packing density (`rabbitmq.channel_client_limit`) and/or
`channelPoolAutoscaling.maxReplicas` -- with no code change. It is a pure
arithmetic assertion over rendered ConfigMap values plus the real cap function,
never a load test. A test is the right form because the claim is about the
CHART, is cheap to evaluate, and is exactly the claim that silently rots when a
default moves. Every quantity is DERIVED from the render; hardcoding 800/720/10
would make the test a tautology that survives a defaults change.

The arithmetic (re-derived from the landed code):

- Stock render after Task 2: channel_pool_size 100 x channel_client_limit 8 =
  800 streams/pod soft threshold; minus the 80-slot readiness reserve = 720/pod
  on the autoscaler's conservative (scaling-signal) basis.
  OMNI_EVENTS_SCALER_MAX_REPLICAS renders "10" on the enabled branch.
- Stock fleet = 800 x 10 = 8,000 (admitted basis) / 7,200 (usable basis).
  Stock does NOT meet 12,000 -- the SR is about reachability via values, not
  about the shipped default. Asserting the shortfall is the honest form of the
  record; it is what makes the reachability routes meaningful.
- Route A (raise the ceiling): channelPoolAutoscaling.maxReplicas=17 ->
  720 x 17 = 12,240 usable and 800 x 17 = 13,600 admitted, clearing 12,000 on
  BOTH bases. (maxReplicas 15 gives 800 x 15 = 12,000 exactly on the admitted
  basis but only 15 x 720 = 10,800 on the scaling-signal basis, so 17 is the
  value that clears both -- both bases are asserted so the record is
  unambiguous.)
- Route B (raise density): channel_client_limit=16 -> (1,600 - 80) x 10 =
  15,200 usable and 1,600 x 10 = 16,000 admitted.
- Route C (autoscaling disabled): channelPoolAutoscaling.enabled=false plus
  replicaCount=17 -> the divisor row falls back to replicaCount, so the same
  arithmetic holds on that branch (720 x 17 = 12,240).

The memory cap must not bind below any of these routes. At the stock budget
1,024 MiB and post-Task-2 prefetch 15: per_stream_worst_case_bytes(15) =
15*1800 + 5120 = 32,120 B; 1024*1024*1024 // 32,120 = 33,429 fleet streams; the
per-pod cap divides that by the route's max-replicas divisor and stays far above
each route's soft threshold (route A/C: 33,429 // 17 = 1,966 > 800; route B:
33,429 // 10 = 3,342 > 1,600). These tests call the SHIPPED
`memory_safe_stream_cap()` rather than re-implementing the formula -- a
re-implementation would pass forever after the formula changed.

Metrics ceiling: Epic 7's Task 3 REMOVED the `le=10000` bound on
`ChannelPoolMetricsResponse.channels_in_use`/`channels_total`
(app/rest/models/channel_pool_metrics.py:21-44 records the reasoning), so the
former ceiling that could 500 the autoscaler feed is moot. The surviving pin is
a soft-threshold-sanity claim: each route's surface value is accepted by the
response model without a ValidationError.

Verify-only: the OMNI_EVENTS_SCALER_MAX_REPLICAS both-branch pin, the
X-Via-Proxy retention pin, and the Contour retryPolicy / loadBalancerPolicy
pins already exist in test_scaler_max_replicas_divisor_template.py and
test_http_proxy_streaming_template.py; this Task does not duplicate them.
"""

import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

from app.lib.rabbitmq import stream_memory_cap
from app.rest.models.channel_pool_metrics import ChannelPoolMetricsResponse
from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"

SETTINGS_CONFIGMAP_TEMPLATE = "templates/settings-configmap.yaml"

FLEET_TARGET_STREAMS = 12_000  # PRD G-2.

ENV_POOL_SIZE = "OMNI_EVENTS_RABBITMQ_CHANNEL_POOL_SIZE"
ENV_CLIENT_LIMIT = "OMNI_EVENTS_RABBITMQ_CHANNEL_CLIENT_LIMIT"
ENV_READINESS_RESERVE = "OMNI_EVENTS_READINESS_MIN_AVAILABLE_CHANNELS"
ENV_SCALER_MAX_REPLICAS = "OMNI_EVENTS_SCALER_MAX_REPLICAS"
ENV_BUDGET = "OMNI_EVENTS_BROKER_STREAM_MEMORY_BUDGET_MIB"
ENV_PREFETCH = "OMNI_EVENTS_RABBITMQ_CHANNEL_PREFETCH_COUNT"
ENV_CAP_OVERRIDE = "OMNI_EVENTS_MAX_NON_DURABLE_STREAMS_PER_POD"

# The two values-only reachability routes plus the autoscaling-disabled branch.
SET_MAX_REPLICAS_17 = "channelPoolAutoscaling.maxReplicas=17"
SET_CLIENT_LIMIT_16 = "rabbitmq.channel_client_limit=16"
DISABLE_AUTOSCALING = "channelPoolAutoscaling.enabled=false"
SET_REPLICA_COUNT_17 = "replicaCount=17"

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


def _settings_configmap_data(*extra_set: str) -> dict:
    out = _render(*extra_set, show_only=SETTINGS_CONFIGMAP_TEMPLATE)
    docs = [d for d in yaml.safe_load_all(out) if d]
    assert len(docs) == 1, f"expected exactly one document, got {len(docs)}:\n{out}"
    assert docs[0]["kind"] == "ConfigMap", f"unexpected kind:\n{out}"
    return docs[0]["data"]


def _per_pod_soft_threshold(data: dict) -> int:
    """channel_pool_size x channel_client_limit, the admitted-basis threshold."""
    return int(data[ENV_POOL_SIZE]) * int(data[ENV_CLIENT_LIMIT])


def _per_pod_usable(data: dict) -> int:
    """Soft threshold less the readiness reserve, the scaling-signal basis."""
    return _per_pod_soft_threshold(data) - int(data[ENV_READINESS_RESERVE])


class TestStockFleetCapacity:
    """Records where the shipped default sits: below the 12,000 target.

    Asserting the shortfall is deliberate -- it is what makes the reachability
    routes below meaningful rather than accidental.
    """

    def test_soft_threshold_is_eight_hundred(self):
        assert _per_pod_soft_threshold(_settings_configmap_data()) == 800

    def test_usable_is_seven_twenty(self):
        assert _per_pod_usable(_settings_configmap_data()) == 720

    def test_stock_fleet_usable_is_below_the_target(self):
        data = _settings_configmap_data()
        fleet_usable = _per_pod_usable(data) * int(data[ENV_SCALER_MAX_REPLICAS])

        assert fleet_usable < FLEET_TARGET_STREAMS


class TestTargetReachableByRaisingMaxReplicas:
    """Route A: raise the fleet ceiling, stock packing density."""

    def test_reachable_on_the_usable_basis(self):
        data = _settings_configmap_data(SET_MAX_REPLICAS_17)
        fleet = _per_pod_usable(data) * int(data[ENV_SCALER_MAX_REPLICAS])

        assert fleet >= FLEET_TARGET_STREAMS

    def test_reachable_on_the_admitted_basis(self):
        data = _settings_configmap_data(SET_MAX_REPLICAS_17)
        fleet = _per_pod_soft_threshold(data) * int(data[ENV_SCALER_MAX_REPLICAS])

        assert fleet >= FLEET_TARGET_STREAMS


class TestTargetReachableByRaisingPackingDensity:
    """Route B: denser packing, stock fleet size."""

    def test_reachable_on_the_usable_basis(self):
        data = _settings_configmap_data(SET_CLIENT_LIMIT_16)
        fleet = _per_pod_usable(data) * int(data[ENV_SCALER_MAX_REPLICAS])

        assert fleet >= FLEET_TARGET_STREAMS

    def test_reachable_on_the_admitted_basis(self):
        data = _settings_configmap_data(SET_CLIENT_LIMIT_16)
        fleet = _per_pod_soft_threshold(data) * int(data[ENV_SCALER_MAX_REPLICAS])

        assert fleet >= FLEET_TARGET_STREAMS


class TestTargetReachableWithAutoscalingDisabled:
    """Route C: with autoscaling off, replicaCount IS the divisor."""

    def test_divisor_row_renders_replica_count(self):
        data = _settings_configmap_data(DISABLE_AUTOSCALING, SET_REPLICA_COUNT_17)

        assert data[ENV_SCALER_MAX_REPLICAS] == "17"

    def test_reachable_on_the_usable_basis(self):
        data = _settings_configmap_data(DISABLE_AUTOSCALING, SET_REPLICA_COUNT_17)
        fleet = _per_pod_usable(data) * int(data[ENV_SCALER_MAX_REPLICAS])

        assert fleet >= FLEET_TARGET_STREAMS

    def test_reachable_on_the_admitted_basis(self):
        data = _settings_configmap_data(DISABLE_AUTOSCALING, SET_REPLICA_COUNT_17)
        fleet = _per_pod_soft_threshold(data) * int(data[ENV_SCALER_MAX_REPLICAS])

        assert fleet >= FLEET_TARGET_STREAMS


# Each route: (label, extra --set args). The memory-cap and metrics tests apply
# the SAME real cap function / response model to every demonstrated route.
ROUTES = [
    ("stock", ()),
    ("route_a_max_replicas", (SET_MAX_REPLICAS_17,)),
    ("route_b_density", (SET_CLIENT_LIMIT_16,)),
    ("route_c_disabled", (DISABLE_AUTOSCALING, SET_REPLICA_COUNT_17)),
]
ROUTE_IDS = [label for label, _ in ROUTES]


class TestMemoryCapDoesNotBindBelowTheDemonstratedRoutes:
    """The memory-safe per-pod cap sits above each route's soft threshold.

    Ties SR-7.3 to the ACTUAL refusal point rather than to a soft number nobody
    enforces. The shipped `memory_safe_stream_cap()` is driven from each route's
    RENDERED budget/prefetch/replicas (never a re-implemented formula); it is
    deliberately un-memoized, so patched settings take effect immediately.
    """

    @pytest.mark.parametrize("label, extra_set", ROUTES, ids=ROUTE_IDS)
    def test_cap_is_at_least_the_soft_threshold(self, mocker, label, extra_set):
        data = _settings_configmap_data(*extra_set)
        # The demonstrated routes leave the absolute override unset, so the cap
        # is COMPUTED; drive the real function from the rendered inputs.
        assert ENV_CAP_OVERRIDE not in data
        mocker.patch.object(
            stream_memory_cap.settings.settings,
            "max_non_durable_streams_per_pod",
            None,
        )
        mocker.patch.object(
            stream_memory_cap.settings.settings,
            "broker_stream_memory_budget_mib",
            int(data[ENV_BUDGET]),
        )
        mocker.patch.object(
            stream_memory_cap.settings.settings,
            "scaler_max_replicas",
            int(data[ENV_SCALER_MAX_REPLICAS]),
        )
        mocker.patch.object(
            stream_memory_cap.common_settings,
            "rabbitmq_channel_prefetch_count",
            int(data[ENV_PREFETCH]),
        )

        cap = stream_memory_cap.memory_safe_stream_cap()

        assert cap is not None
        assert cap >= _per_pod_soft_threshold(data)


class TestScalingSurfaceStaysWithinTheMetricsResponseBound:
    """Each route's surface value is accepted by the metrics response model.

    Epic 7's Task 3 removed the `le=10000` bound on channels_in_use /
    channels_total (the model comment records why: a bound a real deployment can
    exceed would 500 the feed and drop the pod from the autoscaler's view), so
    the former ceiling is moot. The surviving claim is that each demonstrated
    route's soft threshold is a value the response model validates without
    raising -- i.e. the route cannot silently take the autoscaler feed non-200.
    """

    @pytest.mark.parametrize("label, extra_set", ROUTES, ids=ROUTE_IDS)
    def test_response_model_accepts_the_route_surface(self, label, extra_set):
        data = _settings_configmap_data(*extra_set)
        soft = _per_pod_soft_threshold(data)

        # Constructing the model is the pin: a resurrected upper bound below the
        # route's soft threshold would raise ValidationError here.
        response = ChannelPoolMetricsResponse(
            utilization_percent=100,
            channels_in_use=soft,
            channels_total=soft,
        )

        assert response.channels_in_use == soft
