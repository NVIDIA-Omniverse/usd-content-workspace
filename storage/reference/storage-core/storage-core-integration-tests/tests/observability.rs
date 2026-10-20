// SPDX-FileCopyrightText: Copyright (c) 2024-2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Observability integration tests.
//!
//! Each test function runs in its own process (via cargo-nextest), which gives it
//! a fresh global tracing subscriber. Tests validate that the OTEL SDK correctly
//! exports metrics, traces, and logs to an in-process mock OTLP collector.
//!
//! **Run with `cargo nextest`** for full coverage — each test gets its own process.
//! Under `cargo test`, only the first test runs; the rest are skipped with a warning.
//!
//! ```sh
//! cargo nextest run -p storage-core-integration-tests --test observability --all-features
//! ```

use std::time::Duration;

use hex_literal::hex;
use storage_core::traits::fileobject::v1beta as fo_v1beta;
use storage_core_integration_tests::observability_harness::{OtelTestConfig, OtelTestHarness};
use storage_core_test::mock_otlp_collector::{get_string_attribute, log_body_text};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Helper: make a gRPC Stat call to generate telemetry.
async fn grpc_stat(addr: &str) {
    let mut client = fo_v1beta::file_object_service_client::FileObjectServiceClient::connect(addr.to_string())
        .await
        .expect("gRPC connect");
    // The stub returns Unimplemented, but the middleware still records telemetry.
    let _ = client
        .stat(fo_v1beta::StatRequest {
            resource_address: "https://example.com/test-object".to_string(),
        })
        .await;
}

// =========================================================================
// Group 1: Metric Export
// =========================================================================

/// gRPC call → collector receives `grpc.server.call.duration` and
/// `grpc.server.call.started` with correct attributes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_grpc_metrics_exported() {
    let Some(harness) = OtelTestHarness::start(OtelTestConfig::default()).await else {
        return;
    };

    grpc_stat(&harness.grpc_addr).await;
    harness.flush();

    let duration_metrics = harness
        .telemetry
        .wait_for_metrics_named("grpc.server.call.duration", 1, TIMEOUT)
        .await
        .expect("expected grpc.server.call.duration metric");
    assert!(!duration_metrics.is_empty(), "should have at least one duration metric");

    let started_metrics = harness.telemetry.find_metrics("grpc.server.call.started");
    assert!(
        !started_metrics.is_empty(),
        "should have grpc.server.call.started metric"
    );
}

/// REST call → collector receives `http.server.request.duration` metric.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_rest_metrics_exported() {
    let Some(harness) = OtelTestHarness::start(OtelTestConfig::default()).await else {
        return;
    };

    // HEAD request to the v1beta stat endpoint (will get an error from StubBackend, but
    // the HTTP metrics middleware fires regardless of the response status).
    let url = format!(
        "{}/v1beta/fileobject/by-address/{}",
        harness.rest_addr,
        percent_encoding::utf8_percent_encode("https://example.com/test", percent_encoding::NON_ALPHANUMERIC)
    );
    let _ = reqwest::Client::new().head(&url).send().await;

    harness.flush();

    let duration_metrics = harness
        .telemetry
        .wait_for_metrics_named("http.server.request.duration", 1, TIMEOUT)
        .await
        .expect("expected http.server.request.duration metric");
    assert!(
        !duration_metrics.is_empty(),
        "should have at least one HTTP request duration metric"
    );
}

// =========================================================================
// Group 2: Trace Export
// =========================================================================

/// Exported spans carry the correct resource attributes:
/// `service.name` and `service.namespace`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trace_resource_attributes() {
    let config = OtelTestConfig {
        service_name: "test-svc-attrs".to_string(),
        ..Default::default()
    };
    let Some(harness) = OtelTestHarness::start(config).await else {
        return;
    };

    grpc_stat(&harness.grpc_addr).await;
    harness.flush();

    let _spans = harness
        .telemetry
        .wait_for_spans(1, TIMEOUT)
        .await
        .expect("expected at least one span");

    let attrs = harness.telemetry.trace_resource_attributes();
    assert_eq!(
        get_string_attribute(&attrs, "service.name"),
        Some("test-svc-attrs".to_string()),
        "resource should have correct service.name"
    );
    assert_eq!(
        get_string_attribute(&attrs, "service.namespace"),
        Some("omniverse".to_string()),
        "resource should have service.namespace = omniverse"
    );
}

/// Injecting a `traceparent` gRPC metadata header → the exported span's
/// trace_id matches the injected value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trace_propagation_grpc() {
    let Some(harness) = OtelTestHarness::start(OtelTestConfig::default()).await else {
        return;
    };

    // Craft a known trace_id and inject via W3C traceparent header.
    let trace_id = "0af7651916cd43dd8448eb211c80319c";
    let span_id = "00f067aa0ba902b7";
    let traceparent = format!("00-{trace_id}-{span_id}-01");

    let mut client = fo_v1beta::file_object_service_client::FileObjectServiceClient::connect(harness.grpc_addr.clone())
        .await
        .expect("gRPC connect");

    let mut request = tonic::Request::new(fo_v1beta::StatRequest {
        resource_address: "https://example.com/trace-test".to_string(),
    });
    request
        .metadata_mut()
        .insert("traceparent", traceparent.parse().unwrap());

    let _ = client.stat(request).await;
    harness.flush();

    let spans = harness
        .telemetry
        .wait_for_spans(1, TIMEOUT)
        .await
        .expect("expected at least one span");

    // The OTLP span's trace_id is a 16-byte vec.
    let expected_trace_id = hex!("0af7651916cd43dd8448eb211c80319c");
    assert_eq!(
        spans[0].trace_id, expected_trace_id,
        "exported span trace_id should match injected traceparent"
    );
}

// =========================================================================
// Group 3: Log Export
// =========================================================================

/// Log records exported via OTLP have non-empty SeverityText and Body,
/// plus correct resource attributes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_log_records_exported() {
    let config = OtelTestConfig {
        service_name: "test-svc-logs".to_string(),
        ..Default::default()
    };
    let Some(harness) = OtelTestHarness::start(config).await else {
        return;
    };

    // The observability init itself emits INFO logs ("Initializing observability...").
    // Make a gRPC call to generate additional log activity.
    grpc_stat(&harness.grpc_addr).await;
    harness.flush();

    let logs = harness
        .telemetry
        .wait_for_logs(1, TIMEOUT)
        .await
        .expect("expected at least one log record");

    // Check first log record has severity and body.
    let record = &logs[0];
    assert!(!record.severity_text.is_empty(), "log record should have SeverityText");
    // SRD 3.11.1: "Don't set SeverityNumber as it is redundant."
    assert_eq!(
        record.severity_number, 0,
        "SeverityNumber should NOT be set (SRD: redundant)"
    );
    assert!(record.body.is_some(), "log record should have a non-empty Body");

    // Resource attributes should include service.name and service.namespace.
    let attrs = harness.telemetry.log_resource_attributes();
    assert_eq!(
        get_string_attribute(&attrs, "service.name"),
        Some("test-svc-logs".to_string()),
    );
    assert_eq!(
        get_string_attribute(&attrs, "service.namespace"),
        Some("omniverse".to_string()),
    );
}

// =========================================================================
// Group 4: Config Variants
// =========================================================================

/// With all three signals enabled, the collector receives traces, metrics, and logs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_config_all_enabled() {
    let Some(harness) = OtelTestHarness::start(OtelTestConfig::default()).await else {
        return;
    };

    grpc_stat(&harness.grpc_addr).await;
    harness.flush();

    // All three signal types should arrive.
    harness
        .telemetry
        .wait_for_spans(1, TIMEOUT)
        .await
        .expect("traces should be exported");
    harness
        .telemetry
        .wait_for_any_metrics(1, TIMEOUT)
        .await
        .expect("metrics should be exported");
    harness
        .telemetry
        .wait_for_logs(1, TIMEOUT)
        .await
        .expect("logs should be exported");
}

/// With traces disabled, only metrics and logs arrive.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_config_traces_disabled() {
    let config = OtelTestConfig {
        traces_enabled: false,
        ..Default::default()
    };
    let Some(harness) = OtelTestHarness::start(config).await else {
        return;
    };

    grpc_stat(&harness.grpc_addr).await;
    harness.flush();

    // Metrics and logs should still arrive.
    harness
        .telemetry
        .wait_for_any_metrics(1, TIMEOUT)
        .await
        .expect("metrics should be exported");
    harness
        .telemetry
        .wait_for_logs(1, TIMEOUT)
        .await
        .expect("logs should be exported");

    // Traces should NOT arrive (short timeout to confirm absence).
    let result = harness.telemetry.wait_for_spans(1, Duration::from_secs(2)).await;
    assert!(
        result.is_err(),
        "no trace exports should arrive when traces are disabled"
    );
}

/// With `OTEL_SDK_DISABLED=true`, no telemetry of any kind should be exported.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_config_all_disabled() {
    let config = OtelTestConfig {
        traces_enabled: false,
        metrics_enabled: false,
        logs_enabled: false,
        ..Default::default()
    };
    let Some(harness) = OtelTestHarness::start(config).await else {
        return;
    };

    grpc_stat(&harness.grpc_addr).await;
    harness.flush();

    // None of the signals should arrive.
    let short = Duration::from_secs(2);
    assert!(
        harness.telemetry.wait_for_spans(1, short).await.is_err(),
        "no traces when SDK disabled"
    );
    assert!(
        harness.telemetry.wait_for_any_metrics(1, short).await.is_err(),
        "no metrics when SDK disabled"
    );
    assert!(
        harness.telemetry.wait_for_logs(1, short).await.is_err(),
        "no logs when SDK disabled"
    );
}

// =========================================================================
// Group 5: Log Suppression and Summary
// =========================================================================

/// Rate-limited error logging emits a SILENCED notice when the rate limit is
/// exceeded, then an UN-SILENCED notice with a suppression summary after the
/// silence period expires.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_log_suppression_and_summary() {
    // Configure aggressive rate limiting so we can trigger it quickly.
    // Safety: nextest = one test per process, single-threaded at this point.
    unsafe {
        // Allow only 1 error per interval before throttling.
        std::env::set_var("LOGS_RATE_LIMIT_COUNT", "1");
        std::env::set_var("LOGS_RATE_LIMIT_INTERVAL", "1s");
        // Silence for 2 seconds after exceeding the limit.
        std::env::set_var("LOGS_RATE_LIMIT_SILENCE", "2s");
        // Disable bypass so ERROR-level messages are also rate-limited
        // (Unimplemented maps to ERROR by default, which would normally bypass).
        std::env::set_var("LOGS_RATE_LIMIT_BYPASS_LEVEL", "NONE");
    }

    let Some(harness) = OtelTestHarness::start(OtelTestConfig::default()).await else {
        return;
    };

    // 1st call: the error log is allowed (within the rate limit count).
    grpc_stat(&harness.grpc_addr).await;
    // 2nd call: exceeds the rate limit → SILENCED notice emitted.
    grpc_stat(&harness.grpc_addr).await;
    // 3rd-4th calls: suppressed (added to the blocked count).
    grpc_stat(&harness.grpc_addr).await;
    grpc_stat(&harness.grpc_addr).await;

    harness.flush();

    // Wait for the SILENCED log to arrive at the collector.
    let logs = harness
        .telemetry
        .wait_for_logs(1, TIMEOUT)
        .await
        .expect("expected log records");

    let silenced: Vec<_> = logs.iter().filter(|l| log_body_text(l).contains("SILENCED")).collect();
    assert!(
        !silenced.is_empty(),
        "should have a SILENCED notice after exceeding the rate limit"
    );

    // Wait for the silence period to expire (2s configured + margin).
    tokio::time::sleep(Duration::from_millis(2500)).await;

    // Next call triggers UN-SILENCED with a summary of suppressed messages.
    grpc_stat(&harness.grpc_addr).await;
    harness.flush();

    let all_logs = harness
        .telemetry
        .wait_for_logs(1, TIMEOUT)
        .await
        .expect("expected log records after un-silence");

    let unsilenced: Vec<_> = all_logs
        .iter()
        .filter(|l| log_body_text(l).contains("UN-SILENCED"))
        .collect();
    assert!(
        !unsilenced.is_empty(),
        "should have an UN-SILENCED notice after the silence period expires"
    );

    // The UN-SILENCED notice should contain a suppression summary with a message count.
    let body = log_body_text(unsilenced[0]);
    assert!(
        body.contains("message"),
        "UN-SILENCED notice should contain suppression summary (got: {body:?})"
    );
    assert!(
        body.contains("seconds"),
        "UN-SILENCED notice should contain the silence duration (got: {body:?})"
    );
}

// =========================================================================
// Group 6: Log Trace Context
// =========================================================================

/// Log records emitted during a traced gRPC request carry the trace's
/// TraceId and SpanId (both non-zero).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_log_trace_context() {
    let Some(harness) = OtelTestHarness::start(OtelTestConfig::default()).await else {
        return;
    };

    // Inject a known trace context so we can match it in the log records.
    let trace_id_hex = "abcdef1234567890abcdef1234567890";
    let span_id_hex = "1234567890abcdef";
    let traceparent = format!("00-{trace_id_hex}-{span_id_hex}-01");

    let mut client = fo_v1beta::file_object_service_client::FileObjectServiceClient::connect(harness.grpc_addr.clone())
        .await
        .expect("gRPC connect");

    let mut request = tonic::Request::new(fo_v1beta::StatRequest {
        resource_address: "https://example.com/trace-ctx".to_string(),
    });
    request
        .metadata_mut()
        .insert("traceparent", traceparent.parse().unwrap());

    let _ = client.stat(request).await;
    harness.flush();

    let logs = harness
        .telemetry
        .wait_for_logs(1, TIMEOUT)
        .await
        .expect("expected log records");

    // Find a log record that was emitted during the traced request
    // (it should carry the injected trace context).
    let expected_trace_id = hex!("abcdef1234567890abcdef1234567890");
    let with_trace_ctx: Vec<_> = logs
        .iter()
        .filter(|l| l.trace_id == expected_trace_id.as_slice())
        .collect();
    assert!(
        !with_trace_ctx.is_empty(),
        "at least one log record should carry the injected trace_id"
    );

    // The span_id should also be non-zero (the server creates child spans).
    let zero_span = [0u8; 8];
    assert!(
        with_trace_ctx.iter().any(|l| l.span_id != zero_span),
        "log records in a traced context should have a non-zero span_id"
    );
}

// =========================================================================
// Group 7: LOGS_LEVEL Filtering
// =========================================================================

/// `LOGS_LEVEL=ERROR` suppresses INFO-level logs from OTLP export.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_logs_level_filtering() {
    let config = OtelTestConfig {
        logs_level: Some("ERROR".to_string()),
        ..Default::default()
    };
    let Some(harness) = OtelTestHarness::start(config).await else {
        return;
    };

    // The observability init emits INFO logs ("Initializing observability..."),
    // and the gRPC error handler emits an ERROR log for UnimplementedError.
    grpc_stat(&harness.grpc_addr).await;
    harness.flush();

    let logs = harness
        .telemetry
        .wait_for_logs(1, TIMEOUT)
        .await
        .expect("expected at least one log record (ERROR level)");

    // All exported logs should be at ERROR severity — no INFO logs should leak.
    for record in &logs {
        assert_ne!(
            record.severity_text,
            "INFO",
            "INFO logs should be suppressed when LOGS_LEVEL=ERROR (got body: {:?})",
            log_body_text(record)
        );
    }

    // Confirm we got at least one ERROR-level record.
    assert!(
        logs.iter().any(|l| l.severity_text == "ERROR"),
        "should have at least one ERROR log record"
    );
}

// =========================================================================
// Group 8: Rate-Limit Bypass Level (SRD 3.8)
// =========================================================================

/// With the default bypass level (ERROR), ERROR-level messages pass through
/// the rate limiter even when it is blocking lower-severity messages.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_rate_limit_bypass_level() {
    // Safety: nextest = one test per process, single-threaded at this point.
    unsafe {
        // Very aggressive rate limit: allow 1, then block.
        std::env::set_var("LOGS_RATE_LIMIT_COUNT", "1");
        std::env::set_var("LOGS_RATE_LIMIT_INTERVAL", "1s");
        std::env::set_var("LOGS_RATE_LIMIT_SILENCE", "30s");
        // Keep the default bypass: ERROR (do NOT set LOGS_RATE_LIMIT_BYPASS_LEVEL).
        // Unimplemented errors log at ERROR, so they should bypass.
    }

    let Some(harness) = OtelTestHarness::start(OtelTestConfig::default()).await else {
        return;
    };

    // Make many calls to saturate the rate limiter.
    for _ in 0..5 {
        grpc_stat(&harness.grpc_addr).await;
    }
    harness.flush();

    let logs = harness
        .telemetry
        .wait_for_logs(1, TIMEOUT)
        .await
        .expect("expected log records");

    // Count ERROR-level logs that contain the Unimplemented error message.
    // With bypass=ERROR, ALL of these should pass through (not just the first one).
    let error_logs: Vec<_> = logs
        .iter()
        .filter(|l| l.severity_text == "ERROR" && log_body_text(l).contains("Unimplemented"))
        .collect();

    // We made 5 calls, so at least 5 error logs should exist (all bypassed the limiter).
    assert!(
        error_logs.len() >= 5,
        "ERROR-level messages should bypass rate limiting (got {} error logs, expected >= 5)",
        error_logs.len()
    );
}

// =========================================================================
// Group 9: REST Trace Propagation
// =========================================================================

/// Injecting a `traceparent` HTTP header on a REST request → the exported
/// span's trace_id matches the injected value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trace_propagation_rest() {
    let Some(harness) = OtelTestHarness::start(OtelTestConfig::default()).await else {
        return;
    };

    let trace_id_hex = "fedcba0987654321fedcba0987654321";
    let span_id_hex = "abcdef0123456789";
    let traceparent = format!("00-{trace_id_hex}-{span_id_hex}-01");

    let url = format!(
        "{}/v1beta/fileobject/by-address/{}",
        harness.rest_addr,
        percent_encoding::utf8_percent_encode("https://example.com/rest-trace", percent_encoding::NON_ALPHANUMERIC)
    );
    let _ = reqwest::Client::new()
        .head(&url)
        .header("traceparent", &traceparent)
        .send()
        .await;

    harness.flush();

    let spans = harness
        .telemetry
        .wait_for_spans(1, TIMEOUT)
        .await
        .expect("expected at least one span from REST request");

    let expected_trace_id = hex!("fedcba0987654321fedcba0987654321");
    let matching: Vec<_> = spans.iter().filter(|s| s.trace_id == expected_trace_id).collect();
    assert!(
        !matching.is_empty(),
        "exported span should carry the injected traceparent trace_id"
    );
}

// =========================================================================
// Group 10: Error Severity Mapping (SRD 3.2)
// =========================================================================

/// The error severity mapping per SRD 3.2 is preserved through OTLP export:
/// - `Unimplemented` → ERROR
/// - `NotFound` → INFO
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_error_severity_mapping() {
    let Some(harness) = OtelTestHarness::start_with_not_found_backend(OtelTestConfig::default()).await else {
        return;
    };

    // A stat() call to the NotFoundBackend returns NotFound → logged at INFO per SRD.
    grpc_stat(&harness.grpc_addr).await;
    harness.flush();

    let logs = harness
        .telemetry
        .wait_for_logs(1, TIMEOUT)
        .await
        .expect("expected log records");

    // The error handler (log_grpc_error) should emit an INFO-level log for NotFound.
    let not_found_logs: Vec<_> = logs
        .iter()
        .filter(|l| l.severity_text == "INFO" && log_body_text(l).contains("NotFound"))
        .collect();
    assert!(
        !not_found_logs.is_empty(),
        "NotFound errors should be logged at INFO severity per SRD 3.2 (got logs: {:?})",
        logs.iter()
            .map(|l| format!("[{}] {}", l.severity_text, log_body_text(l)))
            .collect::<Vec<_>>()
    );
}
