# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from unittest.mock import MagicMock

import pytest

from app.lib.observability import cache_writer_metrics as cache_writer_metrics_module
from app.lib.observability.cache_writer_metrics import (
    CacheWriterMetrics,
    cache_writer_metrics,
)

# What a CacheWriter actually passes: the BARE `sha256-<12 hex>` reference
# for a non-durable queue. Not the queue's id -- that is the stream's
# reconnect token -- and not the reference wrapped back into a broker-name
# shape, because this exact spelling is what the same stream's spans carry
# as `queue_ref`, and an exported series joins its trace by exact match.
QUEUE_REF = "sha256-0123456789ab"
OTHER_QUEUE_REF = "sha256-ba9876543210"

# What a CacheWriter passes as `baseline_key`: its cache-scope key, which is
# the broker queue name plus the presenting principal's reference. It is
# internal-only -- it never reaches an exported attribute -- so it is spelled
# here in its real, unredacted shape rather than as a reference.
BASELINE_KEY = "event-queue:token-one:nd:sha256-1111aaaa2222"
OTHER_BASELINE_KEY = "event-queue:token-one:nd:sha256-3333bbbb4444"


@pytest.fixture
def mock_queue_depth():
    return MagicMock()


@pytest.fixture
def mock_flush_latency():
    return MagicMock()


@pytest.fixture
def mock_flush_ops():
    return MagicMock()


@pytest.fixture
def mock_drops():
    return MagicMock()


@pytest.fixture(autouse=True)
def setup_mock_meter(
    mock_meter,
    mock_queue_depth,
    mock_flush_latency,
    mock_flush_ops,
    mock_drops,
):
    mock_meter.create_up_down_counter.return_value = mock_queue_depth
    mock_meter.create_histogram.return_value = mock_flush_latency
    mock_meter.create_counter.side_effect = [mock_flush_ops, mock_drops]


def _named_call(calls, name):
    """
    Find a call from a list of mock call_args_list entries whose first
    positional argument equals ``name``, or whose ``name`` kwarg equals
    ``name``. Used to look up an instrument by its OTel metric name.
    """
    return next(
        c
        for c in calls
        if (c.args and c.args[0] == name) or c.kwargs.get("name") == name
    )


@pytest.fixture
def metrics():
    return CacheWriterMetrics()


class TestInstrumentNames:

    def test_creates_queue_depth_up_down_counter_with_correct_name(self, mock_meter):
        CacheWriterMetrics()

        assert _named_call(
            mock_meter.create_up_down_counter.call_args_list,
            "cache_writer.queue_depth",
        )

    def test_creates_flush_latency_histogram_with_correct_name(self, mock_meter):
        CacheWriterMetrics()

        assert _named_call(
            mock_meter.create_histogram.call_args_list,
            "cache_writer.flush_latency",
        )

    def test_creates_flush_ops_counter_with_correct_name(self, mock_meter):
        CacheWriterMetrics()

        assert _named_call(
            mock_meter.create_counter.call_args_list, "cache_writer.flush_ops"
        )

    def test_creates_drops_counter_with_correct_name(self, mock_meter):
        CacheWriterMetrics()

        assert _named_call(
            mock_meter.create_counter.call_args_list, "cache_writer.drops"
        )

    def test_queue_depth_unit_is_op(self, mock_meter):
        CacheWriterMetrics()

        c = _named_call(
            mock_meter.create_up_down_counter.call_args_list,
            "cache_writer.queue_depth",
        )
        assert c.kwargs.get("unit") == "{op}"

    def test_flush_latency_unit_is_ms(self, mock_meter):
        CacheWriterMetrics()

        c = _named_call(
            mock_meter.create_histogram.call_args_list,
            "cache_writer.flush_latency",
        )
        assert c.kwargs.get("unit") == "ms"

    def test_flush_ops_unit_is_op(self, mock_meter):
        CacheWriterMetrics()

        c = _named_call(
            mock_meter.create_counter.call_args_list, "cache_writer.flush_ops"
        )
        assert c.kwargs.get("unit") == "{op}"

    def test_drops_unit_is_op(self, mock_meter):
        CacheWriterMetrics()

        c = _named_call(mock_meter.create_counter.call_args_list, "cache_writer.drops")
        assert c.kwargs.get("unit") == "{op}"


class TestRecordFlush:

    def test_increments_flush_ops_counter_by_batch_size(self, metrics, mock_flush_ops):
        metrics.record_flush(QUEUE_REF, ops_count=7, latency_ms=12.5)

        mock_flush_ops.add.assert_called_once_with(7, {"queue_ref": QUEUE_REF})

    def test_records_flush_latency_histogram(self, metrics, mock_flush_latency):
        metrics.record_flush(QUEUE_REF, ops_count=7, latency_ms=12.5)

        mock_flush_latency.record.assert_called_once_with(
            12.5, {"queue_ref": QUEUE_REF}
        )

    def test_does_nothing_when_telemetry_disabled(
        self,
        telemetry_disabled,
        mock_flush_ops,
        mock_flush_latency,
    ):
        metrics = CacheWriterMetrics()

        metrics.record_flush(QUEUE_REF, ops_count=7, latency_ms=12.5)

        mock_flush_ops.add.assert_not_called()
        mock_flush_latency.record.assert_not_called()


class TestRecordDrop:

    def test_increments_drops_counter_by_one_by_default(self, metrics, mock_drops):
        metrics.record_drop(QUEUE_REF)

        mock_drops.add.assert_called_once_with(1, {"queue_ref": QUEUE_REF})

    def test_increments_drops_counter_by_provided_count(self, metrics, mock_drops):
        metrics.record_drop(QUEUE_REF, count=4)

        mock_drops.add.assert_called_once_with(4, {"queue_ref": QUEUE_REF})

    def test_does_nothing_when_telemetry_disabled(self, telemetry_disabled, mock_drops):
        metrics = CacheWriterMetrics()

        metrics.record_drop(QUEUE_REF)

        mock_drops.add.assert_not_called()


class TestRecordQueueDepth:

    def test_records_initial_depth_as_positive_delta(self, metrics, mock_queue_depth):
        metrics.record_queue_depth(QUEUE_REF, 5, BASELINE_KEY)

        mock_queue_depth.add.assert_called_once_with(5, {"queue_ref": QUEUE_REF})

    def test_records_increase_as_positive_delta(self, metrics, mock_queue_depth):
        metrics.record_queue_depth(QUEUE_REF, 3, BASELINE_KEY)
        mock_queue_depth.add.reset_mock()

        metrics.record_queue_depth(QUEUE_REF, 8, BASELINE_KEY)

        mock_queue_depth.add.assert_called_once_with(5, {"queue_ref": QUEUE_REF})

    def test_records_decrease_as_negative_delta(self, metrics, mock_queue_depth):
        metrics.record_queue_depth(QUEUE_REF, 10, BASELINE_KEY)
        mock_queue_depth.add.reset_mock()

        metrics.record_queue_depth(QUEUE_REF, 2, BASELINE_KEY)

        mock_queue_depth.add.assert_called_once_with(-8, {"queue_ref": QUEUE_REF})

    def test_does_not_emit_when_depth_unchanged(self, metrics, mock_queue_depth):
        metrics.record_queue_depth(QUEUE_REF, 7, BASELINE_KEY)
        mock_queue_depth.add.reset_mock()

        metrics.record_queue_depth(QUEUE_REF, 7, BASELINE_KEY)

        mock_queue_depth.add.assert_not_called()

    def test_baseline_is_keyed_on_baseline_key_not_queue_ref(
        self, metrics, mock_queue_depth
    ):
        # The complement of TestBaselineKeyIsPerScope below: one scope's
        # bookkeeping follows its `baseline_key` alone, so a sample under a
        # different `queue_ref` is still a delta against that baseline --
        # not the full depth a per-`queue_ref` baseline would report.
        metrics.record_queue_depth(QUEUE_REF, 5, BASELINE_KEY)
        mock_queue_depth.add.reset_mock()

        metrics.record_queue_depth(OTHER_QUEUE_REF, 8, BASELINE_KEY)

        mock_queue_depth.add.assert_called_once_with(3, {"queue_ref": OTHER_QUEUE_REF})

    def test_does_nothing_when_telemetry_disabled(
        self, telemetry_disabled, mock_queue_depth
    ):
        metrics = CacheWriterMetrics()

        metrics.record_queue_depth(QUEUE_REF, 5, BASELINE_KEY)

        mock_queue_depth.add.assert_not_called()


class TestBaselineKeyIsPerScope:
    """
    Two principals attached to one non-durable queue run one writer each.
    They report under the SAME ``queue_ref`` on purpose, so the delta
    bookkeeping cannot be keyed on it: sharing a baseline would make each
    writer's sample cancel the other's.
    """

    def test_second_scope_first_sample_is_its_own_full_depth(
        self, metrics, mock_queue_depth
    ):
        metrics.record_queue_depth(QUEUE_REF, 5, BASELINE_KEY)
        mock_queue_depth.add.reset_mock()

        metrics.record_queue_depth(QUEUE_REF, 3, OTHER_BASELINE_KEY)

        mock_queue_depth.add.assert_called_once_with(3, {"queue_ref": QUEUE_REF})

    def test_first_scope_next_sample_is_delta_against_its_own_baseline(
        self, metrics, mock_queue_depth
    ):
        metrics.record_queue_depth(QUEUE_REF, 5, BASELINE_KEY)
        metrics.record_queue_depth(QUEUE_REF, 3, OTHER_BASELINE_KEY)
        mock_queue_depth.add.reset_mock()

        metrics.record_queue_depth(QUEUE_REF, 6, BASELINE_KEY)

        mock_queue_depth.add.assert_called_once_with(1, {"queue_ref": QUEUE_REF})

    def test_both_scopes_still_report_the_same_queue_ref(
        self, metrics, mock_queue_depth
    ):
        metrics.record_queue_depth(QUEUE_REF, 5, BASELINE_KEY)
        metrics.record_queue_depth(QUEUE_REF, 3, OTHER_BASELINE_KEY)

        reported = {c.args[1]["queue_ref"] for c in mock_queue_depth.add.call_args_list}
        assert reported == {QUEUE_REF}


class TestForgetQueue:

    def test_drops_baseline_so_next_record_is_full_delta(
        self, metrics, mock_queue_depth
    ):
        metrics.record_queue_depth(QUEUE_REF, 6, BASELINE_KEY)
        metrics.forget_queue(BASELINE_KEY)
        mock_queue_depth.add.reset_mock()

        metrics.record_queue_depth(QUEUE_REF, 4, BASELINE_KEY)

        mock_queue_depth.add.assert_called_once_with(4, {"queue_ref": QUEUE_REF})

    def test_leaves_another_scopes_baseline_on_the_same_queue_intact(
        self, metrics, mock_queue_depth
    ):
        metrics.record_queue_depth(QUEUE_REF, 6, BASELINE_KEY)
        metrics.record_queue_depth(QUEUE_REF, 4, OTHER_BASELINE_KEY)
        metrics.forget_queue(BASELINE_KEY)
        mock_queue_depth.add.reset_mock()

        metrics.record_queue_depth(QUEUE_REF, 9, OTHER_BASELINE_KEY)

        mock_queue_depth.add.assert_called_once_with(5, {"queue_ref": QUEUE_REF})

    def test_is_no_op_for_unknown_queue(self, metrics):
        metrics.forget_queue("never-recorded")  # should not raise

    def test_does_nothing_when_telemetry_disabled(self, telemetry_disabled):
        metrics = CacheWriterMetrics()

        metrics.forget_queue(BASELINE_KEY)  # should not raise


class TestExportedAttributeKey:
    """
    Every instrument identifies its writer by ``queue_ref``, never
    ``queue_id``. The distinction is not cosmetic: these writers exist only
    for non-durable queues, whose id is the stream's reconnect token, so an
    attribute named ``queue_id`` would invite a caller to pass the real one.
    """

    @pytest.mark.parametrize(
        "record, instrument_fixture",
        [
            (
                lambda m: m.record_flush(QUEUE_REF, ops_count=1, latency_ms=1.0),
                "mock_flush_ops",
            ),
            (lambda m: m.record_drop(QUEUE_REF), "mock_drops"),
            (
                lambda m: m.record_queue_depth(QUEUE_REF, 1, BASELINE_KEY),
                "mock_queue_depth",
            ),
        ],
        ids=["record_flush", "record_drop", "record_queue_depth"],
    )
    def test_attributes_are_keyed_on_queue_ref(
        self, metrics, record, instrument_fixture, request
    ):
        instrument = request.getfixturevalue(instrument_fixture)

        record(metrics)

        assert set(instrument.add.call_args.args[1]) == {"queue_ref"}

    def test_flush_latency_attributes_are_keyed_on_queue_ref(
        self, metrics, mock_flush_latency
    ):
        metrics.record_flush(QUEUE_REF, ops_count=1, latency_ms=1.0)

        assert set(mock_flush_latency.record.call_args.args[1]) == {"queue_ref"}

    def test_attribute_value_is_passed_through_unchanged(self, metrics, mock_drops):
        # The redaction is the CALLER's job (see CacheWriter), so this layer
        # must not transform the reference it is handed -- a series recorded
        # here has to join the span attribute carrying the same value.
        metrics.record_drop(QUEUE_REF)

        assert mock_drops.add.call_args.args[1]["queue_ref"] == QUEUE_REF

    def test_queue_depth_exports_the_reference_not_the_baseline_key(
        self, metrics, mock_queue_depth
    ):
        # The baseline key carries the raw broker queue name, and a
        # non-durable name embeds the stream's reconnect token, so it must
        # stay internal to the delta bookkeeping.
        metrics.record_queue_depth(QUEUE_REF, 1, BASELINE_KEY)

        assert mock_queue_depth.add.call_args.args[1]["queue_ref"] == QUEUE_REF


class TestTelemetryDisabled:

    def test_does_not_create_meter_when_telemetry_disabled(
        self, telemetry_disabled, mock_meter
    ):
        CacheWriterMetrics()

        mock_meter.create_up_down_counter.assert_not_called()
        mock_meter.create_histogram.assert_not_called()
        mock_meter.create_counter.assert_not_called()

    def test_does_not_create_meter_when_only_tracing_enabled(
        self, tracing_only, mock_meter
    ):
        # Metrics must gate on metrics_enabled, not tracing_enabled.
        CacheWriterMetrics()

        mock_meter.create_up_down_counter.assert_not_called()
        mock_meter.create_histogram.assert_not_called()
        mock_meter.create_counter.assert_not_called()


class TestSingleton:

    def test_module_level_singleton_is_cache_writer_metrics_instance(self):
        assert isinstance(cache_writer_metrics, CacheWriterMetrics)

    def test_singleton_is_exported_from_module(self):
        assert cache_writer_metrics_module.cache_writer_metrics is cache_writer_metrics
