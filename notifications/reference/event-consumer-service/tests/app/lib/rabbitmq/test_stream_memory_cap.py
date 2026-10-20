# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
The computed memory-safe non-durable stream cap (SDD SR-3.5).

The cap's DEFAULT is computed, not constant, and that is the whole point: a
constant tuned for one prefetch becomes wildly wrong at another. At the
measured constants a stream's worst case is ~32 kB at the shipped prefetch of
15 but ~883 kB at prefetch 500, so a constant default sized for the shipped low
prefetch would be ~28x too permissive the moment an operator raised the prefetch
back up to 500. This file pins the arithmetic that keeps the default
self-consistent under tuning.

Expected values are LITERALS derived by hand from the landed constants in
``app/lib/rabbitmq/stream_memory_cap.py`` (``BYTES_PER_HELD_MESSAGE = 1800``,
``BYTES_PER_STREAM_CHANNEL_SHARE = 5120``), with the derivation shown in a
comment next to each. Nothing here re-implements
``budget // per_stream // replicas`` -- a test that recomputes the formula
passes against whatever bug the implementation has.

Neutralizer interaction
-----------------------
``tests/app/conftest.py`` carries an AUTOUSE fixture
(``non_binding_stream_memory_cap``) that patches the operator override
``max_non_durable_streams_per_pod`` to 1,000,000 for every test under
``tests/app/``, which short-circuits the computed path entirely. Every test in
this file therefore patches that override back to ``None`` locally, via the
autouse ``computed_cap_path`` fixture below. Without it the degenerate-budget
assertion in particular would pass VACUOUSLY (a 1,000,000 override trivially
violates a tiny budget), silently un-pinning the floor exception.

Machine independence
--------------------
An untracked local ``.env`` can set ``OMNI_EVENTS_RABBITMQ_CHANNEL_PREFETCH_
COUNT``, so no test here relies on the ambient prefetch: every case patches
``common_settings.rabbitmq_channel_prefetch_count`` (and the budget and the
replica divisor) explicitly.
"""

import logging

import pytest
from notification_common.config.common_settings import (
    CommonSettings,
    common_settings,
)

from app.config import settings
from app.lib.rabbitmq import stream_memory_cap
from app.lib.rabbitmq.rabbitmq_base import RabbitMQBase
from app.lib.rabbitmq.stream_memory_cap import (
    memory_safe_stream_cap,
    per_stream_worst_case_bytes,
)

# The two prefetch values the Epic contrasts: the shipped default (15, the
# packing work landed it) and the legacy value an operator can raise back to.
SHIPPED_PREFETCH = 15
LEGACY_PREFETCH = 500

# The shipped apportioning inputs (app/config/settings.py defaults).
DEFAULT_BUDGET_MIB = 1024
DEFAULT_MAX_REPLICAS = 10

_BYTES_PER_MIB = 1024 * 1024

# Worst-case bytes one stream can hold, at each prefetch. Derived by hand from
# the landed constants: prefetch x 1800 + 5120.
#   15  -> 15 x 1800 + 5120  =  27,000 + 5,120  =     32,120
#   500 -> 500 x 1800 + 5120 = 900,000 + 5,120  =    905,120
#   1000 -> 1000 x 1800 + 5120 = 1,800,000 + 5,120 = 1,805,120
PER_STREAM_BYTES_AT_SHIPPED_PREFETCH = 32_120
PER_STREAM_BYTES_AT_LEGACY_PREFETCH = 905_120
PER_STREAM_BYTES_AT_PREFETCH_1000 = 1_805_120

# Per-pod caps at the shipped budget (1024 MiB = 1,073,741,824 B) and the
# shipped divisor (10 replicas). Derived by hand:
#   prefetch 15:  1,073,741,824 / 32,120  = 33,429.9  -> fleet 33,429
#                 33,429 / 10             =  3,342.9  -> per pod 3,342
#   prefetch 500: 1,073,741,824 / 905,120 =  1,186.3  -> fleet  1,186
#                 1,186 / 10              =    118.6  -> per pod   118
CAP_AT_SHIPPED_PREFETCH = 3_342
CAP_AT_LEGACY_PREFETCH = 118

# Fleet cap at the legacy prefetch and the shipped budget, before the replica
# divisor is applied.
FLEET_CAP_AT_LEGACY_PREFETCH = 1_186

# The measured headroom claim this cap exists to preserve (recorded in the
# source repository's internal channel-capacity design record, which is not
# published): prefetch 15 buys
# "~28x more headroom" than prefetch 500. The bound is deliberately coarse --
# the exact ratio here is 3,342 / 118 = 28.32, but the point being pinned is
# the order of magnitude, not a value the constants do not really support.
MIN_HEADROOM_RATIO = 25
MAX_HEADROOM_RATIO = 32

# Distinctive values used to prove WHICH setting object the formula reads.
SENTINEL_DURABLE_PREFETCH = 7
SENTINEL_SERVICE_TWIN_PREFETCH = 3
SENTINEL_POOL_PREFETCH = 137
SENTINEL_POOL_PREFETCH_AFTER_RETUNE = 41

# Override values sitting well below and well above the computed default of
# 3,342, so "the override wins" cannot be mistaken for "the smaller of the two".
OVERRIDE_BELOW_COMPUTED = 5
OVERRIDE_ABOVE_COMPUTED = 100_000
OVERRIDE_WITH_UNBOUNDED_PREFETCH = 42

# A budget of 1 MiB does not divide evenly by any per-stream footprint here,
# which is what makes it useful for pinning the rounding direction.
UNEVEN_BUDGET_MIB = 1


@pytest.fixture(autouse=True)
def computed_cap_path(mocker):
    """
    Put every test in this file on the COMPUTED path, with a clean warn-once.

    Undoes the ``tests/app/conftest.py`` autouse neutralizer for this file only
    (see the module docstring) by patching the operator override back to
    ``None``; ``mocker`` applies patches in order, so this later patch wins.
    Also clears the ``@cache`` on the unbounded-prefetch warn-once helper on
    both sides of the test, so a warning emitted by another module cannot
    suppress the one asserted here, and vice versa.
    """
    mocker.patch.object(settings.settings, "max_non_durable_streams_per_pod", None)
    stream_memory_cap._warn_unbounded_prefetch_once.cache_clear()
    yield
    stream_memory_cap._warn_unbounded_prefetch_once.cache_clear()


@pytest.fixture
def cap_inputs(mocker):
    """
    Set the three inputs the computed cap reads, defaulting to the shipped
    configuration (prefetch 15).

    The prefetch is patched on the LIBRARY singleton
    (``common_settings``), which is the object the formula reads; the budget
    and the replica divisor are patched on the service ``Settings`` singleton.
    """

    def _configure(
        *,
        prefetch=SHIPPED_PREFETCH,
        budget_mib=DEFAULT_BUDGET_MIB,
        max_replicas=DEFAULT_MAX_REPLICAS,
    ):
        mocker.patch.object(
            common_settings, "rabbitmq_channel_prefetch_count", prefetch
        )
        mocker.patch.object(
            settings.settings, "broker_stream_memory_budget_mib", budget_mib
        )
        mocker.patch.object(settings.settings, "scaler_max_replicas", max_replicas)

    return _configure


class TestPerStreamWorstCaseBytes:
    """The per-stream footprint, pinned at the measured constants."""

    def test_channel_share_is_the_whole_footprint_at_zero_prefetch(self):
        # Isolates BYTES_PER_STREAM_CHANNEL_SHARE: with no prefetch window the
        # flat channel-share term is all that is left.
        assert per_stream_worst_case_bytes(0) == 5_120

    def test_footprint_at_the_shipped_prefetch(self):
        # 15 x 1800 + 5120 = 32,120
        assert per_stream_worst_case_bytes(SHIPPED_PREFETCH) == 32_120

    def test_footprint_at_the_legacy_prefetch(self):
        # 500 x 1800 + 5120 = 905,120
        assert per_stream_worst_case_bytes(LEGACY_PREFETCH) == 905_120


class TestPrefetchDivergence:
    """
    The reason the default is COMPUTED rather than constant.

    A stream's cost is dominated by its prefetch window, so the same budget
    buys wildly different stream counts at different prefetch settings. A
    constant default would be correct at exactly one of them.
    """

    @pytest.mark.parametrize(
        "prefetch, expected_cap",
        [
            (SHIPPED_PREFETCH, CAP_AT_SHIPPED_PREFETCH),
            (LEGACY_PREFETCH, CAP_AT_LEGACY_PREFETCH),
        ],
    )
    def test_cap_at_prefetch(self, cap_inputs, prefetch, expected_cap):
        cap_inputs(prefetch=prefetch)

        assert memory_safe_stream_cap() == expected_cap

    def test_low_prefetch_buys_roughly_the_measured_headroom_multiple(self, cap_inputs):
        """
        Lowering the prefetch buys ~28x more streams for the same budget.

        This is the failure mode the computed default prevents: a constant
        default sized at the shipped prefetch of 15 would be ~28x too permissive
        once an operator raised the prefetch back to 500.
        """
        cap_inputs(prefetch=SHIPPED_PREFETCH)
        cap_at_shipped_prefetch = memory_safe_stream_cap()
        cap_inputs(prefetch=LEGACY_PREFETCH)
        cap_at_legacy_prefetch = memory_safe_stream_cap()

        ratio = cap_at_shipped_prefetch / cap_at_legacy_prefetch

        assert MIN_HEADROOM_RATIO <= ratio <= MAX_HEADROOM_RATIO


class TestShippedDefaultCap:
    """
    The cap computed at the DECLARED library default is the number the README,
    the helm values, and the sizing docs all quote (3,342).

    Read the declared default from ``model_fields`` rather than the ambient
    resolved setting (the ``.env`` hazard the module docstring records), so a
    drift in the shipped prefetch default fails loudly right here.
    """

    def test_cap_at_the_declared_default_prefetch_is_3342(self, cap_inputs):
        declared_prefetch = CommonSettings.model_fields[
            "rabbitmq_channel_prefetch_count"
        ].default
        cap_inputs(
            prefetch=declared_prefetch,
            budget_mib=DEFAULT_BUDGET_MIB,
            max_replicas=DEFAULT_MAX_REPLICAS,
        )

        assert memory_safe_stream_cap() == CAP_AT_SHIPPED_PREFETCH


class TestPrefetchSource:
    """
    WHICH prefetch setting the formula reads.

    Three same-named or similarly-named values are in play: the non-durable
    prefetch on the library singleton (correct), the durable prefetch on the
    same singleton, and the inherited twin field on the service ``Settings``
    singleton -- a different object holding its own copy of the same field.
    Reading either of the wrong two computes the cap from a number the
    non-durable pool never applies.
    """

    def test_cap_tracks_the_library_non_durable_prefetch(self, cap_inputs):
        cap_inputs(prefetch=SHIPPED_PREFETCH)

        assert memory_safe_stream_cap() == CAP_AT_SHIPPED_PREFETCH

    def test_cap_is_invariant_to_the_durable_prefetch(self, cap_inputs, mocker):
        cap_inputs(prefetch=LEGACY_PREFETCH)
        mocker.patch.object(
            common_settings,
            "rabbitmq_durable_channel_prefetch_count",
            SENTINEL_DURABLE_PREFETCH,
        )

        assert memory_safe_stream_cap() == CAP_AT_LEGACY_PREFETCH

    def test_cap_is_invariant_to_the_service_settings_twin_field(
        self, cap_inputs, mocker
    ):
        # ``Settings`` inherits ``rabbitmq_channel_prefetch_count`` from
        # ``CommonSettings``, so ``settings.settings`` carries its own copy of
        # the field on a DIFFERENT object from the library singleton the
        # formula reads. Moving the twin must not move the cap.
        cap_inputs(prefetch=LEGACY_PREFETCH)
        mocker.patch.object(
            settings.settings,
            "rabbitmq_channel_prefetch_count",
            SENTINEL_SERVICE_TWIN_PREFETCH,
        )

        assert memory_safe_stream_cap() == CAP_AT_LEGACY_PREFETCH


class TestReplicaApportioning:
    """
    ``scaler_max_replicas`` is a DIVISOR, not a fudge factor.

    The budget is fleet-wide, so each pod may only admit its share of it.
    """

    @pytest.mark.parametrize(
        "max_replicas, expected_cap",
        [
            # Fleet cap at prefetch 500 / 1024 MiB is 1,186 streams:
            (1, 1_186),  # 1,186 / 1  = 1,186.0 -> 1,186 (undivided)
            (5, 237),  # 1,186 / 5  =   237.2 ->   237
            (10, 118),  # 1,186 / 10 =   118.6 ->   118
        ],
    )
    def test_cap_shrinks_as_the_fleet_grows(
        self, cap_inputs, max_replicas, expected_cap
    ):
        cap_inputs(prefetch=LEGACY_PREFETCH, max_replicas=max_replicas)

        assert memory_safe_stream_cap() == expected_cap

    def test_single_replica_gets_the_whole_fleet_budget(self, cap_inputs):
        """A one-pod fleet is capped at the undivided fleet cap."""
        cap_inputs(prefetch=LEGACY_PREFETCH, max_replicas=1)

        assert memory_safe_stream_cap() == FLEET_CAP_AT_LEGACY_PREFETCH


class TestBudgetScaling:
    """The budget is the lever an operator reaches for after raising RAM."""

    @pytest.mark.parametrize(
        "budget_mib, expected_cap",
        [
            # 1024 MiB = 1,073,741,824 / 905,120 = 1,186.3 -> 1,186 / 10 = 118
            (1024, 118),
            # 2048 MiB = 2,147,483,648 / 905,120 = 2,372.6 -> 2,372 / 10 = 237
            (2048, 237),
        ],
    )
    def test_cap_at_budget(self, cap_inputs, budget_mib, expected_cap):
        cap_inputs(prefetch=LEGACY_PREFETCH, budget_mib=budget_mib)

        assert memory_safe_stream_cap() == expected_cap


class TestAbsoluteOverride:
    """
    The operator override wins unconditionally.

    An operator who has measured their own deployment gets exactly the number
    they set -- not the smaller of theirs and the computed one.
    """

    def test_override_below_the_computed_default_wins(self, cap_inputs, mocker):
        cap_inputs(prefetch=SHIPPED_PREFETCH)
        mocker.patch.object(
            settings.settings,
            "max_non_durable_streams_per_pod",
            OVERRIDE_BELOW_COMPUTED,
        )

        assert memory_safe_stream_cap() == OVERRIDE_BELOW_COMPUTED

    def test_override_above_the_computed_default_wins(self, cap_inputs, mocker):
        cap_inputs(prefetch=SHIPPED_PREFETCH)
        mocker.patch.object(
            settings.settings,
            "max_non_durable_streams_per_pod",
            OVERRIDE_ABOVE_COMPUTED,
        )

        assert memory_safe_stream_cap() == OVERRIDE_ABOVE_COMPUTED

    def test_override_short_circuits_an_unbounded_prefetch(self, cap_inputs, mocker):
        cap_inputs(prefetch=0)
        mocker.patch.object(
            settings.settings,
            "max_non_durable_streams_per_pod",
            OVERRIDE_WITH_UNBOUNDED_PREFETCH,
        )

        assert memory_safe_stream_cap() == OVERRIDE_WITH_UNBOUNDED_PREFETCH

    def test_override_emits_no_unbounded_prefetch_warning(
        self, cap_inputs, mocker, caplog
    ):
        """An operator who set the cap explicitly is not nagged about prefetch."""
        cap_inputs(prefetch=0)
        mocker.patch.object(
            settings.settings,
            "max_non_durable_streams_per_pod",
            OVERRIDE_WITH_UNBOUNDED_PREFETCH,
        )

        with caplog.at_level(logging.WARNING, logger=stream_memory_cap.logger.name):
            memory_safe_stream_cap()

        assert caplog.records == []


class TestUnboundedPrefetch:
    """
    A prefetch of 0 or less means "unlimited" to ``set_qos``.

    The per-stream footprint is then undefined rather than small, so no
    memory-safe cap can be computed and the operator has to choose. The
    WARNING is the operator-facing contract of this branch, which is why it is
    asserted here despite the usual "don't test logging" rule.
    """

    @pytest.mark.parametrize("prefetch", [0, -1])
    def test_no_cap_is_computed(self, cap_inputs, prefetch):
        cap_inputs(prefetch=prefetch)

        assert memory_safe_stream_cap() is None

    def test_warning_is_emitted_once_across_repeated_calls(self, cap_inputs, caplog):
        cap_inputs(prefetch=0)

        with caplog.at_level(logging.WARNING, logger=stream_memory_cap.logger.name):
            memory_safe_stream_cap()
            memory_safe_stream_cap()
            memory_safe_stream_cap()

        assert len(caplog.records) == 1

    def test_warning_names_the_setting_that_fixes_it(self, cap_inputs, caplog):
        cap_inputs(prefetch=0)

        with caplog.at_level(logging.WARNING, logger=stream_memory_cap.logger.name):
            memory_safe_stream_cap()

        assert "max_non_durable_streams_per_pod" in caplog.text

    def test_a_newly_misconfigured_prefetch_warns_again(self, cap_inputs, caplog):
        """
        The warn-once cache is keyed on the prefetch VALUE, not on the process.

        An operator who edits the setting and lands on a second bad value has
        to hear about it, so suppression is per distinct value.
        """
        with caplog.at_level(logging.WARNING, logger=stream_memory_cap.logger.name):
            cap_inputs(prefetch=0)
            memory_safe_stream_cap()
            cap_inputs(prefetch=-1)
            memory_safe_stream_cap()

        assert len(caplog.records) == 2


class TestLateBinding:
    """
    The cap is recomputed on every call, deliberately.

    Memoizing it would freeze the result against a later settings change --
    the import-time-freeze problem this module exists to avoid.
    """

    def test_cap_reflects_a_setting_changed_between_calls(self, cap_inputs):
        cap_inputs(prefetch=LEGACY_PREFETCH)
        assert memory_safe_stream_cap() == CAP_AT_LEGACY_PREFETCH

        cap_inputs(prefetch=SHIPPED_PREFETCH)

        assert memory_safe_stream_cap() == CAP_AT_SHIPPED_PREFETCH


class TestFleetBudgetInvariant:
    """
    PRD AC-7's "publishers remain unblocked" clause, verified by ARITHMETIC.

    RabbitMQ blocks publishers when a node crosses its relative memory high
    watermark (0.6 on the measured 4Gi nodes). The service's only lever on
    that is admission: if the whole fleet, at its cap, cannot hold more
    worst-case stream memory than the configured budget, the budget -- not the
    service -- is what has to be sized under the watermark.

    That clause is deliberately NOT verified by provoking a live watermark
    alarm. Doing so needs a prod-sized broker filled to ~2.4 GB, while CI runs
    a plain ``rabbitmq:4.1.3`` container; such a test would be slow, hostile to
    every other job on the runner, and flaky, and it would still exercise no
    service code path. The service-side contract IS this arithmetic, so this
    is where it is pinned.
    """

    def test_fleet_worst_case_fits_the_budget_at_the_legacy_prefetch(self, cap_inputs):
        # Round-trip check, NOT a re-implementation of the divide-and-floor:
        # the cap comes from the code, and is multiplied back up against the
        # literal budget to prove the whole fleet still fits.
        cap_inputs(prefetch=LEGACY_PREFETCH)

        cap = memory_safe_stream_cap()
        fleet_worst_case_bytes = (
            cap * DEFAULT_MAX_REPLICAS * PER_STREAM_BYTES_AT_LEGACY_PREFETCH
        )

        assert fleet_worst_case_bytes <= DEFAULT_BUDGET_MIB * _BYTES_PER_MIB

    def test_fleet_worst_case_fits_the_budget_at_the_shipped_prefetch(self, cap_inputs):
        # The same round trip at the shipped prefetch. A CONSTANT default of
        # 3,342 sized at the shipped prefetch would satisfy this case and breach
        # the legacy/raised-prefetch one above by ~28x, which is exactly why the
        # default is computed.
        cap_inputs(prefetch=SHIPPED_PREFETCH)

        cap = memory_safe_stream_cap()
        fleet_worst_case_bytes = (
            cap * DEFAULT_MAX_REPLICAS * PER_STREAM_BYTES_AT_SHIPPED_PREFETCH
        )

        assert fleet_worst_case_bytes <= DEFAULT_BUDGET_MIB * _BYTES_PER_MIB


class TestRoundingDirection:
    """
    Floor division is the only safe direction.

    Rounding UP would admit a stream the budget cannot hold, breaching the
    fleet-budget invariant above by construction.
    """

    def test_budget_division_floors(self, cap_inputs):
        # 1 MiB = 1,048,576 / 32,120 = 32.6 streams. Rounding would give 33.
        cap_inputs(
            prefetch=SHIPPED_PREFETCH, budget_mib=UNEVEN_BUDGET_MIB, max_replicas=1
        )

        assert memory_safe_stream_cap() == 32

    def test_replica_division_floors(self, cap_inputs):
        # Fleet cap 1,186 / 10 replicas = 118.6. Rounding would give 119.
        cap_inputs(prefetch=LEGACY_PREFETCH, max_replicas=DEFAULT_MAX_REPLICAS)

        assert memory_safe_stream_cap() == 118


class TestDegenerateConfiguration:
    """
    A budget too small to hold even one stream.

    The floor at ``max(1, ...)`` is a deliberate, documented EXCEPTION to the
    fleet-budget invariant: in a configuration that is already pathologically
    wrong, availability beats budget conformance, and the capacity guard's
    shed logging keeps the misconfiguration visible rather than silent. Both
    halves are pinned so the tension is explicit -- a later change must not
    "fix" the floor to satisfy the invariant, nor generalize the invariant to
    cover this case.
    """

    @pytest.fixture
    def budget_below_one_stream(self, cap_inputs):
        # 1 MiB = 1,048,576 B against a 1,805,120 B stream at prefetch 1000:
        # the fleet cap floors to 0, and so does the per-pod share.
        cap_inputs(
            prefetch=1000,
            budget_mib=UNEVEN_BUDGET_MIB,
            max_replicas=DEFAULT_MAX_REPLICAS,
        )

    def test_cap_floors_at_one_stream_rather_than_zero(self, budget_below_one_stream):
        assert memory_safe_stream_cap() == 1

    def test_fleet_budget_invariant_is_deliberately_violated(
        self, budget_below_one_stream
    ):
        cap = memory_safe_stream_cap()
        fleet_worst_case_bytes = (
            cap * DEFAULT_MAX_REPLICAS * PER_STREAM_BYTES_AT_PREFETCH_1000
        )

        assert fleet_worst_case_bytes > UNEVEN_BUDGET_MIB * _BYTES_PER_MIB


class TestNonDurablePoolPrefetchCoupling:
    """
    The coupling contract in the ``stream_memory_cap`` module docstring.

    The formula reads the process-global
    ``common_settings.rabbitmq_channel_prefetch_count``, which is only the
    number the non-durable pool actually applies while that pool carries NO
    per-pool ``ChannelPoolConfig.prefetch_count`` override. Epic 3's per-pool
    prefetch option would break that silently, so it is guarded here: give the
    pool its own prefetch without re-pointing the formula and these fail.
    """

    @pytest.fixture
    def small_non_durable_pool(self, mocker):
        """Keep ``connect()`` cheap: pre-open one channel, not 75."""
        mocker.patch.object(common_settings, "rabbitmq_channel_pool_min_size", 1)

    @pytest.mark.asyncio
    async def test_pool_applies_the_same_global_the_formula_reads(
        self, small_non_durable_pool, mocker
    ):
        mocker.patch.object(
            common_settings, "rabbitmq_channel_prefetch_count", SENTINEL_POOL_PREFETCH
        )

        await RabbitMQBase.connector.connect()
        pool = RabbitMQBase.connector._channel_pool

        assert pool.config.prefetch_count == SENTINEL_POOL_PREFETCH

    @pytest.mark.asyncio
    async def test_pool_re_reads_the_global_after_it_changes(
        self, small_non_durable_pool, mocker
    ):
        """
        The pool LATE-BINDS to the global rather than capturing a value.

        This is the observable form of "constructed with no explicit
        ``prefetch_count`` override": a config that had been given its own
        prefetch would keep reporting it here.
        """
        mocker.patch.object(
            common_settings, "rabbitmq_channel_prefetch_count", SENTINEL_POOL_PREFETCH
        )
        await RabbitMQBase.connector.connect()
        pool = RabbitMQBase.connector._channel_pool

        mocker.patch.object(
            common_settings,
            "rabbitmq_channel_prefetch_count",
            SENTINEL_POOL_PREFETCH_AFTER_RETUNE,
        )

        assert pool.config.prefetch_count == SENTINEL_POOL_PREFETCH_AFTER_RETUNE
