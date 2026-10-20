// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use opentelemetry::SpanId;
use opentelemetry::trace::SpanContext;
use otel_asserts::tracing::{Logs, TraceCapture};
use otel_asserts::{TcpListener, grpc, types, unsafe_env};
use pretty_assertions::assert_eq;
use serde_json::json;

async fn emit(default_dirs: &str) -> (SpanContext, String) {
    let resource = otel_tools::resource::ResourceBuilder::without_default("nv.test").build();

    let config = otel_tools::exporters::traces::TracesConfig::from_env();
    let traces_guard = config.guard().with_resource(&resource).build();

    let config = otel_tools::exporters::logs::LogsConfig::from_env();
    let logs_guard = config.guard().with_resource(&resource).build();

    let config = otel_tools::tracing::TracingConfig::from_env();
    let capture = TraceCapture::new();
    config
        .subscriber()
        .with_default_env_filter(default_dirs)
        .with_logs(&logs_guard)
        .with_resource(&resource)
        .with_traces(&traces_guard)
        .with_make_writer(capture.clone())
        .build("test");

    // span must be dropped before guards
    let context = {
        let span = tracing::info_span!("my_span");

        otel_tools::context::get_span_context!(span_context = span);

        span.in_scope(|| {
            tracing::info!(name: "test-event", "my event");
        });

        span_context.clone()
    };

    drop(logs_guard);
    drop(traces_guard);

    let lines = capture.take();

    (context, lines)
}

fn expected_spans(context: &SpanContext) -> Vec<types::trace::ResourceSpans> {
    use otel_asserts::semconv;
    use types::common::{InstrumentationScope, KeyValue};
    use types::resource::Resource;
    use types::trace::{Event, ResourceSpans, ScopeSpans, Span, SpanKind, Status, StatusCode};

    let context = context.clone();

    vec![ResourceSpans {
        resource: Some(Resource::new(vec![KeyValue::new(
            "service.name",
            "nv.test",
        )])),
        schema_url: String::new(),
        scope_spans: vec![ScopeSpans {
            scope: Some(InstrumentationScope::new("test", "", vec![])),
            schema_url: String::new(),
            spans: vec![Span::new(
                "my_span",
                SpanKind::Internal,
                vec![
                    KeyValue::new(semconv::CODE_FILE_PATH, file!()),
                    KeyValue::new(semconv::CODE_MODULE_NAME, module_path!()),
                    KeyValue::new(semconv::CODE_LINE_NUMBER, types::common::Value::Wildcard),
                    KeyValue::new(semconv::THREAD_ID, types::common::Value::Wildcard),
                    KeyValue::new(semconv::THREAD_NAME, types::common::Value::Wildcard),
                    KeyValue::new(semconv::BUSY_NS, types::common::Value::Wildcard),
                    KeyValue::new(semconv::IDLE_NS, types::common::Value::Wildcard),
                ],
                vec![Event::new(
                    "my event",
                    vec![
                        KeyValue::new(semconv::LEVEL, "INFO"),
                        KeyValue::new(semconv::TARGET, module_path!()),
                        KeyValue::new(semconv::CODE_FILE_PATH, file!()),
                        KeyValue::new(semconv::CODE_MODULE_NAME, module_path!()),
                        KeyValue::new(semconv::CODE_LINE_NUMBER, types::common::Value::Wildcard),
                    ],
                )],
                Status::new("", StatusCode::Unset),
                context,
                SpanId::INVALID,
            )],
        }],
    }]
}

fn expected_logs(context: &SpanContext) -> Vec<types::logs::ResourceLogs> {
    use types::common::{InstrumentationScope, KeyValue};
    use types::logs::{LogRecord, ResourceLogs, ScopeLogs, SeverityNumber};
    use types::resource::Resource;

    // The `OpenTelemetryTracingBridge` from `opentelemetry-appender-tracing`
    // doesn't have access to the trace flags, so they will always be empty.
    //
    // https://github.com/open-telemetry/opentelemetry-rust/blob/opentelemetry-appender-tracing-0.29.1/opentelemetry-appender-tracing/src/layer.rs#L263
    let trace_flags = opentelemetry::trace::TraceFlags::NOT_SAMPLED;
    let trace_state = context.trace_state().clone();
    let context = SpanContext::new(
        context.trace_id(),
        context.span_id(),
        trace_flags,
        context.is_remote(),
        trace_state,
    );

    vec![ResourceLogs {
        resource: Some(Resource::new(vec![KeyValue::new(
            "service.name",
            "nv.test",
        )])),
        schema_url: String::new(),
        scope_logs: vec![ScopeLogs {
            scope: Some(InstrumentationScope::new(module_path!(), "", vec![])),
            schema_url: String::new(),
            log_records: vec![LogRecord::new(
                SeverityNumber::Info,
                "INFO",
                "test-event",
                "my event",
                vec![],
                context,
            )],
        }],
    }]
}

fn expected_lines(context: &SpanContext) -> Vec<otel_asserts::tracing::LogLine<'static>> {
    use opentelemetry::{SpanId, TraceId};
    use otel_asserts::tracing::{LogLevel, LogLine};

    let trace_id = context.trace_id();
    let trace_id = if trace_id == TraceId::INVALID {
        None
    } else {
        Some(trace_id)
    };

    let span_id = context.span_id();
    let span_id = if span_id == SpanId::INVALID {
        None
    } else {
        Some(span_id)
    };

    let attrs = json!({
        "service.name": "nv.test",
    });

    vec![LogLine {
        message: "[my_span] my event",
        level: LogLevel::Info,
        timestamp: 0,
        target: module_path!(),
        line: None,
        file: Some(file!()),
        trace_id,
        span_id,
        attrs,
    }]
}

#[test]
fn filter_default_info_no_override() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp,console",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": endpoint,
        "OTEL_METRICS_EXPORTER": "none",
    }

    let collector = grpc::Collector::all(listener);
    let (context, stdout) = collector.block_on(emit("info"));

    let (mut logs, metrics, mut spans) = collector.finish();

    assert_ne!(context, SpanContext::NONE);

    assert_eq!(&metrics.resource_metrics, &[], "metrics");

    logs.assert_timestamps();
    let expected_logs = expected_logs(&context);
    assert_eq!(logs.resource_logs, expected_logs, "logs");

    spans.assert_timestamps();
    let expected_spans = expected_spans(&context);
    assert_eq!(spans.resource_spans, expected_spans, "spans");

    let mut lines = Logs::parse(&stdout).unwrap();
    lines.assert_timestamps();
    lines.assert_line_numbers();
    let expected_lines = expected_lines(&context);
    assert_eq!(lines.lines, expected_lines, "stdout");
}

#[test]
fn filter_default_info_logs_warn() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp,console",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": endpoint,
        "OTEL_METRICS_EXPORTER": "none",
        "OTEL_RUST_OTLP_LOGS_FILTER": "warn",
        "RUST_LOG": "warn",
    }

    let collector = grpc::Collector::all(listener);
    let (context, stdout) = collector.block_on(emit("info"));

    let (logs, metrics, mut spans) = collector.finish();

    assert_ne!(context, SpanContext::NONE);

    assert_eq!(&metrics.resource_metrics, &[], "metrics");
    assert_eq!(&logs.resource_logs, &[], "logs");
    assert_eq!(&stdout, "", "stdout");

    spans.assert_timestamps();
    let expected_spans = expected_spans(&context);
    assert_eq!(spans.resource_spans, expected_spans, "spans");
}

#[test]
fn filter_default_info_traces_warn() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp,console",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": endpoint,
        "OTEL_METRICS_EXPORTER": "none",
        "OTEL_RUST_OTLP_TRACES_FILTER": "warn",
    }

    let collector = grpc::Collector::all(listener);
    let (context, stdout) = collector.block_on(emit("info"));

    let (mut logs, metrics, spans) = collector.finish();

    assert_eq!(
        context,
        SpanContext::NONE,
        "span is filtered, so no context expected"
    );

    assert_eq!(&metrics.resource_metrics, &[], "metrics");
    assert_eq!(&spans.resource_spans, &[], "spans");

    logs.assert_timestamps();
    let expected_logs = expected_logs(&context);
    assert_eq!(logs.resource_logs, expected_logs, "logs");

    let mut lines = Logs::parse(&stdout).unwrap();
    lines.assert_timestamps();
    lines.assert_line_numbers();
    let expected_lines = expected_lines(&context);
    assert_eq!(lines.lines, expected_lines, "stdout");
}

#[test]
fn filter_default_warn_logs_info() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp,console",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": endpoint,
        "OTEL_METRICS_EXPORTER": "none",
        "OTEL_RUST_OTLP_LOGS_FILTER": "info",
        "RUST_LOG": "info",
    }

    let collector = grpc::Collector::all(listener);
    let (context, stdout) = collector.block_on(emit("warn"));

    let (mut logs, metrics, spans) = collector.finish();

    assert_eq!(
        context,
        SpanContext::NONE,
        "span is filtered, so no context expected"
    );

    assert_eq!(&metrics.resource_metrics, &[], "metrics");
    assert_eq!(&spans.resource_spans, &[], "spans");

    logs.assert_timestamps();
    let expected_logs = expected_logs(&context);
    assert_eq!(logs.resource_logs, expected_logs, "logs");

    let mut lines = Logs::parse(&stdout).unwrap();
    lines.assert_timestamps();
    lines.assert_line_numbers();
    let expected_lines = expected_lines(&context);
    assert_eq!(lines.lines, expected_lines, "stdout");
}

#[test]
fn filter_default_warn_traces_info() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp,console",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": endpoint,
        "OTEL_METRICS_EXPORTER": "none",
        "OTEL_RUST_OTLP_TRACES_FILTER": "info",
    }

    let collector = grpc::Collector::all(listener);
    let (context, stdout) = collector.block_on(emit("warn"));

    let (logs, metrics, mut spans) = collector.finish();

    assert_ne!(context, SpanContext::NONE);

    assert_eq!(&metrics.resource_metrics, &[], "metrics");
    assert_eq!(&logs.resource_logs, &[], "logs");
    assert_eq!(&stdout, "", "stdout");

    spans.assert_timestamps();
    let expected_spans = expected_spans(&context);
    assert_eq!(spans.resource_spans, expected_spans, "spans");
}

#[test]
fn filter_logs_console_independent_logs_info() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp,console",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
        "OTEL_TRACES_EXPORTER": "none",
        "OTEL_METRICS_EXPORTER": "none",
        "OTEL_RUST_OTLP_LOGS_FILTER": "info",
    }

    let collector = grpc::Collector::all(listener);
    let (context, stdout) = collector.block_on(emit("warn"));

    let (logs, metrics, spans) = collector.finish();

    assert_eq!(
        context,
        SpanContext::NONE,
        "span is filtered, so no context expected"
    );

    assert_eq!(&metrics.resource_metrics, &[], "metrics");
    assert_eq!(&spans.resource_spans, &[], "spans");

    assert_eq!(&stdout, "", "stdout");
    assert_ne!(&logs.resource_logs, &[], "logs");
}

#[test]
fn filter_logs_console_independent_console_info() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp,console",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
        "OTEL_TRACES_EXPORTER": "none",
        "OTEL_METRICS_EXPORTER": "none",
        "RUST_LOG": "info",
    }

    let collector = grpc::Collector::all(listener);
    let (context, stdout) = collector.block_on(emit("warn"));

    let (logs, metrics, spans) = collector.finish();

    assert_eq!(
        context,
        SpanContext::NONE,
        "span is filtered, so no context expected"
    );

    assert_eq!(&metrics.resource_metrics, &[], "metrics");
    assert_eq!(&spans.resource_spans, &[], "spans");

    assert_ne!(&stdout, "", "stdout");
    assert_eq!(&logs.resource_logs, &[], "logs");
}
