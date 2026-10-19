// SPDX-FileCopyrightText: Copyright (c) 2024-2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! End-to-end test for the configurable metric prefix.
//!
//! Verifies that calling [`set_metric_prefix`] before the OTel SDK initializes results in registered metrics being
//! emitted with the configured prefix joined to the raw metric name. Run via `cargo nextest` so each test gets its own
//! process (the prefix is locked into a process-global `OnceLock`).
//!
//! ```sh
//! cargo nextest run -p storage-core-integration-tests --test metric_prefix --all-features
//! ```

use std::time::Duration;

use storage_core_integration_tests::observability_harness::{OtelTestConfig, OtelTestHarness};
use storage_core_util::observability::metrics::{metric_base, MetricBinder};
use storage_core_util::observability::set_metric_prefix;

const TIMEOUT: Duration = Duration::from_secs(10);

/// `set_metric_prefix("integration_test_prefix")` → an exported metric registered with raw name `prefix_test_counter`
/// arrives at the collector under the joined name `integration_test_prefix.prefix_test_counter`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_prefix_is_applied_to_emitted_metric_name() {
    // Configure the prefix BEFORE the harness initializes the OTel SDK. Metric instruments are lazily registered on
    // first use, so the prefix only needs to be set before the first deref of the metric_base! static below.
    set_metric_prefix("integration_test_prefix").expect("set_metric_prefix should succeed on first call");

    let Some(harness) = OtelTestHarness::start(OtelTestConfig::default()).await else {
        return;
    };

    metric_base!(
        static TEST_COUNTER: UIntCounter,
        name = "prefix_test_counter",
        description = "Counter used by the metric_prefix integration test",
        { kind }
    );

    TEST_COUNTER.bind([("kind", "test")]).add(1);

    harness.flush();

    let emitted = harness
        .telemetry
        .wait_for_metrics_named("integration_test_prefix.prefix_test_counter", 1, TIMEOUT)
        .await
        .expect("expected the test counter to arrive at the mock collector under the prefixed name");
    assert!(!emitted.is_empty(), "should have at least one exported metric");

    let unprefixed = harness.telemetry.find_metrics("prefix_test_counter");
    assert!(
        unprefixed.is_empty(),
        "metric should NOT be exported under its raw (unprefixed) name; found: {}",
        unprefixed.len()
    );
}

/// Calling `set_metric_prefix` a second time returns [`MetricPrefixError::AlreadySet`].
///
/// Under `cargo test` the prefix may already be set by a sibling test in the same process; in that case this test
/// silently no-ops, matching the pattern used by the harness-based tests.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_prefix_twice_returns_already_set() {
    use storage_core_util::observability::MetricPrefixError;

    if set_metric_prefix("first_prefix").is_err() {
        // Already set by a sibling test in this process — only meaningful under cargo nextest. Skip.
        return;
    }
    let result = set_metric_prefix("second_prefix");
    assert!(
        matches!(result, Err(MetricPrefixError::AlreadySet)),
        "second set_metric_prefix call should return AlreadySet, got {result:?}"
    );
}
