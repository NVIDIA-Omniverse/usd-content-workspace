# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
The shipped packing-defaults contract (Epic 3, t1.tj2.xn).

After the packing flip the four numbers below stop being incidental tuning and
become a published contract that three downstream surfaces consume as a SET:

- Task 3 templates their env vars into ``settings-configmap.yaml`` /
  ``values.yaml``;
- Task 5 documents them in the README defaults table (PRD AC-9);
- Epic 6's autoscaler retune arithmetic assumes soft threshold 800, reserve 80
  and 720 usable slots.

The library's ``TestNonDurableChannelPoolSettings`` pins two of them one repo
away and ``test_settings.py`` pins the third, but nothing anywhere asserts the
shipped TRIPLE as a set, nor multiplies ``pool_size x client_limit`` to 800 or
subtracts the reserve to 720. This file owns exactly that: the packing triple
(plus the reserve) and the 800 / 80 / 720 arithmetic. A change to any number
here is a deliberate migration -- chart value, README row and an upgrade note --
not a refactor.

This file spans the library settings module (``CommonSettings``) and the
service settings module (``Settings``), so ``tests/app/`` is the nearest common
ancestor -- the same placement justification
``tests/app/test_capacity_surface_consistency.py`` records.

Option (a) shipped: the flip lives in the ``CommonSettings`` process-globals;
the non-durable pool carries NO per-pool ``ChannelPoolConfig`` override.

Deliberate non-duplication. The computed memory-safe cap at the shipped prefetch
(3,342) is owned by ``tests/app/lib/rabbitmq/test_stream_memory_cap.py``; the
shipped shed line is owned by ``tests/app/lib/rabbitmq/test_capacity_guard.py``;
the durable-side defaults are owned by the library's
``TestDurableChannelPoolSettings``. This file restates NONE of that cap or
shed arithmetic -- it holds only the packing triple and the 800 / 80 / 720
products and differences.

Every value is read from ``model_fields`` (the DECLARED default), never from a
resolved ``settings`` instance: a developer's untracked ``.env`` re-seeds these
vars on every reload, so a resolved-value assertion would be green in CI and red
on that machine.
"""

from notification_common.config.common_settings import CommonSettings

from app.config import settings

# The shipped packing threshold and usable-capacity numbers, as independently
# reasoned literals rather than a recomputation of the multiplication the tests
# check. Derivation (checked by hand, not in code):
#   soft packing threshold = pool_size 100 x client_limit 8 = 800 streams/pod
#   usable autoscaler capacity = soft threshold 800 - reserve 80 = 720 slots
SOFT_PACKING_THRESHOLD = 800
USABLE_AUTOSCALER_CAPACITY = 720


def _common_default(field_name):
    return CommonSettings.model_fields[field_name].default


def _service_default(field_name):
    return settings.Settings.model_fields[field_name].default


class TestShippedPackingDefaults:
    """
    The four declared defaults the chart and README publish.

    A drift in any one of them fails here first, before the single-value tests
    one repo away happen to move.
    """

    def test_client_limit_default_is_8(self):
        assert _common_default("rabbitmq_channel_client_limit") == 8

    def test_prefetch_count_default_is_15(self):
        assert _common_default("rabbitmq_channel_prefetch_count") == 15

    def test_pool_size_default_is_100(self):
        assert _common_default("rabbitmq_channel_pool_size") == 100

    def test_readiness_reserve_default_is_80(self):
        assert _service_default("readiness_min_available_channels") == 80


class TestPackingArithmeticContract:
    """
    The products and differences Task 4, Task 5 and Epic 6 consume.

    Computed from the DECLARED defaults and compared against independently
    reasoned literals, so a drift in a default OR in the arithmetic contract
    is caught here.
    """

    def test_soft_packing_threshold_is_800(self):
        # SR-1.4's ">= 800 streams/pod", stated as an equality.
        pool_size = _common_default("rabbitmq_channel_pool_size")
        client_limit = _common_default("rabbitmq_channel_client_limit")

        assert pool_size * client_limit == SOFT_PACKING_THRESHOLD

    def test_soft_packing_threshold_meets_the_sr_1_4_floor(self):
        # The SR-1.4 floor stated as a floor, not only as an equality: the
        # packing product must be at least 800 streams/pod.
        pool_size = _common_default("rabbitmq_channel_pool_size")
        client_limit = _common_default("rabbitmq_channel_client_limit")

        assert pool_size * client_limit >= 800

    def test_usable_autoscaler_capacity_is_720(self):
        # Soft threshold minus the readiness reserve: the capacity the scaling
        # signal steers on, which Task 4 / Task 5 / Epic 6 all consume.
        pool_size = _common_default("rabbitmq_channel_pool_size")
        client_limit = _common_default("rabbitmq_channel_client_limit")
        reserve = _service_default("readiness_min_available_channels")

        soft_threshold = pool_size * client_limit

        assert soft_threshold - reserve == USABLE_AUTOSCALER_CAPACITY


class TestPackingFieldNameContract:
    """
    The field NAMES are a helm/README contract (SR-7.2).

    The pydantic-settings env source iterates ``model_fields`` only, so an
    ``OMNI_EVENTS_``-prefixed variable with no matching field is silently
    ignored -- neither read nor rejected. Rename one of these and an operator's
    tuned value is silently dropped while the default takes effect, so a rename
    is a deliberate migration (chart key + README + upgrade note), not a
    refactor.
    """

    def test_non_durable_packing_field_names_exist(self):
        expected = {
            "rabbitmq_channel_client_limit",
            "rabbitmq_channel_prefetch_count",
        }
        assert expected <= set(CommonSettings.model_fields)

    def test_readiness_reserve_field_name_exists(self):
        assert "readiness_min_available_channels" in settings.Settings.model_fields

    def test_service_and_library_share_the_env_prefix(self):
        # The OMNI_EVENTS_ vars Task 3 templates must reach BOTH classes; they
        # only do while the two settings classes share an env_prefix.
        assert (
            settings.Settings.model_config["env_prefix"]
            == CommonSettings.model_config["env_prefix"]
        )
