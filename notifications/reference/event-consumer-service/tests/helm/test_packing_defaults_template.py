# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Rendered-value pins for the two packing-contract chart values.

Epic 3 Task 2 flips the two EXISTING chart values that would otherwise
shadow the code-default flip: `rabbitmq.channel_client_limit` 1 -> 8 and
`rabbitmq.readiness_min_available_channels` 20 -> 80. These two numbers ARE
the packing contract: `channel_pool_size * channel_client_limit` is the
per-pod soft threshold (readiness + autoscaler basis) and
`readiness_min_available_channels` is the reserve subtracted from it. A
silent revert of either value re-creates the exact pre-Bee behaviour
(100-stream soft threshold, one client's scene-load exhausts a pod) with no
test failure anywhere -- until this module, no helm-template test pinned
either value.

This module also renders `helm/local-values.yaml` -- the overlay `/deploy-local`
ships -- which no other helm test does. Its deliberate dev-scale divergences
(`channel_client_limit: 1`, `readiness_min_available_channels: 3`,
`channel_pool_size: 8`, `channel_pool_min_size: 5`) are load-bearing: they are
what makes local scale-up/scale-down observable on a tiny pool, and an
accidental inheritance of the new stock defaults there would silently make
local runs unrepresentative.
"""

import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"
# The overlay lives at helm/local-values.yaml, NOT inside the chart directory.
LOCAL_VALUES = Path(__file__).resolve().parents[2] / "helm" / "local-values.yaml"

SETTINGS_CONFIGMAP_TEMPLATE = "templates/settings-configmap.yaml"

ENV_CLIENT_LIMIT = "OMNI_EVENTS_RABBITMQ_CHANNEL_CLIENT_LIMIT"
ENV_READINESS_RESERVE = "OMNI_EVENTS_READINESS_MIN_AVAILABLE_CHANNELS"
ENV_POOL_SIZE = "OMNI_EVENTS_RABBITMQ_CHANNEL_POOL_SIZE"
ENV_POOL_MIN_SIZE = "OMNI_EVENTS_RABBITMQ_CHANNEL_POOL_MIN_SIZE"

# Stock production defaults after Task 2's flip. WHY 8 and 80: the soft
# threshold is channel_pool_size 100 x channel_client_limit 8 = 800 streams/pod,
# and the readiness reserve was re-sized 20 -> 80 by the Epic's pinned decision
# so it stays ~10% of that 800 capacity rather than falling to 2.5%. A future
# reader can tell a deliberate retune from a regression by these two numbers.
STOCK_CLIENT_LIMIT = "8"
STOCK_READINESS_RESERVE = "80"

# local-values.yaml dev-scale overrides (helm/local-values.yaml:93-113).
LOCAL_CLIENT_LIMIT = "1"
LOCAL_READINESS_RESERVE = "3"
LOCAL_POOL_SIZE = "8"
LOCAL_POOL_MIN_SIZE = "5"

# Sentinels distinct from every default in play (1, 3, 5, 8, 75, 80, 100), so a
# hardcoded literal in the template would pass the default pins but fail these.
OVERRIDE_CLIENT_LIMIT = "13"
OVERRIDE_READINESS_RESERVE = "47"

pytestmark = pytest.mark.skipif(
    shutil.which("helm") is None,
    reason="helm CLI not available",
)


def _render(*extra_set: str, show_only: str | None = None, values_file=None) -> str:
    args = ["helm", "template", "ec", str(CHART_PATH), "--namespace", "storage-api"]
    if values_file is not None:
        args.extend(["-f", str(values_file)])
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


def _settings_configmap_data(*extra_set: str, values_file=None) -> dict:
    """Return the settings ConfigMap's `data` mapping from a scoped render.

    Parsed structurally rather than scanned as a substring: a scan cannot see
    the YAML type of the rendered value nor tell an absent key from an empty
    one.
    """
    out = _render(
        *extra_set,
        show_only=SETTINGS_CONFIGMAP_TEMPLATE,
        values_file=values_file,
    )
    docs = [d for d in yaml.safe_load_all(out) if d]
    assert len(docs) == 1, f"expected exactly one document, got {len(docs)}:\n{out}"
    assert docs[0]["kind"] == "ConfigMap", f"unexpected kind:\n{out}"
    return docs[0]["data"]


class TestStockPackingDefaults:
    """The stock render pins the two flipped packing-contract values."""

    def test_client_limit_is_eight(self):
        # 100 pool_size x 8 client_limit = the 800 streams/pod soft threshold.
        assert _settings_configmap_data()[ENV_CLIENT_LIMIT] == STOCK_CLIENT_LIMIT

    def test_client_limit_is_a_string(self):
        assert isinstance(_settings_configmap_data()[ENV_CLIENT_LIMIT], str)

    def test_readiness_reserve_is_eighty(self):
        # Re-sized 20 -> 80 so the reserve stays ~10% of the 800-slot capacity.
        assert (
            _settings_configmap_data()[ENV_READINESS_RESERVE] == STOCK_READINESS_RESERVE
        )

    def test_readiness_reserve_is_a_string(self):
        assert isinstance(_settings_configmap_data()[ENV_READINESS_RESERVE], str)


class TestStockValuesFollowTheirValuesKeys:
    """Each row reads its values key, not a literal that matches the default.

    Without these, a hardcoded literal in the template would pass the default
    pins above.
    """

    def test_client_limit_follows_its_values_key(self):
        data = _settings_configmap_data(
            f"rabbitmq.channel_client_limit={OVERRIDE_CLIENT_LIMIT}"
        )

        assert data[ENV_CLIENT_LIMIT] == OVERRIDE_CLIENT_LIMIT

    def test_readiness_reserve_follows_its_values_key(self):
        data = _settings_configmap_data(
            f"rabbitmq.readiness_min_available_channels={OVERRIDE_READINESS_RESERVE}"
        )

        assert data[ENV_READINESS_RESERVE] == OVERRIDE_READINESS_RESERVE


class TestLocalValuesOverlayKeepsDevScale:
    """The `/deploy-local` overlay keeps its walkable tiny-pool dev scale."""

    def test_client_limit_is_one(self):
        data = _settings_configmap_data(values_file=LOCAL_VALUES)

        assert data[ENV_CLIENT_LIMIT] == LOCAL_CLIENT_LIMIT

    def test_readiness_reserve_is_three(self):
        data = _settings_configmap_data(values_file=LOCAL_VALUES)

        assert data[ENV_READINESS_RESERVE] == LOCAL_READINESS_RESERVE

    def test_pool_size_is_eight(self):
        # The other half of the same dev-scale story: an 8-slot pool.
        data = _settings_configmap_data(values_file=LOCAL_VALUES)

        assert data[ENV_POOL_SIZE] == LOCAL_POOL_SIZE

    def test_pool_min_size_is_five(self):
        data = _settings_configmap_data(values_file=LOCAL_VALUES)

        assert data[ENV_POOL_MIN_SIZE] == LOCAL_POOL_MIN_SIZE


class TestLocalValuesOverlayDivergesFromStock:
    """Fails if someone "tidies up" the overlay to inherit stock defaults.

    Deleting the overlay's overrides so it inherits the new production defaults
    is exactly the silent regression this class exists to catch.
    """

    def test_client_limit_differs_from_stock(self):
        overlay = _settings_configmap_data(values_file=LOCAL_VALUES)
        stock = _settings_configmap_data()

        assert overlay[ENV_CLIENT_LIMIT] != stock[ENV_CLIENT_LIMIT]

    def test_readiness_reserve_differs_from_stock(self):
        overlay = _settings_configmap_data(values_file=LOCAL_VALUES)
        stock = _settings_configmap_data()

        assert overlay[ENV_READINESS_RESERVE] != stock[ENV_READINESS_RESERVE]
