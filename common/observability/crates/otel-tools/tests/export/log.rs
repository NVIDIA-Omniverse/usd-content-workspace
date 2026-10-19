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

use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use otel_asserts::{TcpListener, grpc, http, types, unsafe_env};
use otel_tools::exporters::ShutdownError as _;
use otel_tools::exporters::logs::{LogsConfig, LogsGuard};
use otel_tools::resource::ResourceBuilder;
use pretty_assertions::assert_eq;
use std::mem::ManuallyDrop;
use std::time::{Duration, SystemTime};

const TRACE_ID: TraceId = TraceId::from_bytes(*b"awesome-trace-id");
const SPAN_ID: SpanId = SpanId::from_bytes(*b"span-id!");

#[test]
fn export_grpc_success() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
    }

    let collector = grpc::Collector::logs(listener);
    collector.block_on(async {
        let guard = setup_logs();
        do_log(&guard);
    });

    let actual = collector.finish();
    let expected = expected_logs();
    assert_eq!(actual.resource_logs, expected);
}

#[test]
fn export_http_success() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}/v1/logs");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "http/protobuf",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
    }

    let collector = http::Collector::logs(listener);
    collector.block_on(async {
        let guard = setup_logs();
        do_log(&guard);
    });

    let actual = collector.finish();
    let expected = expected_logs();
    assert_eq!(actual.resource_logs, expected);
}

#[test]
fn export_shutdown_success() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
    }

    let collector = grpc::Collector::logs(listener);
    let res = collector.block_on(async {
        let guard = setup_logs();

        // prevent the guard from being dropped, so shutdown won't be called
        let guard = ManuallyDrop::new(guard);
        // create a shutdown handle
        let handle = guard.shutdown_handle();

        // emit a log, so we can test that the shutdown handle ships data
        do_log(&guard);

        handle.shutdown()
    });

    res.expect("shutdown should succeed");

    let actual = collector.finish();
    let expected = expected_logs();
    assert_eq!(actual.resource_logs, expected);
}

#[test]
fn export_shutdown_double() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
    }

    let collector = grpc::Collector::logs(listener);
    let res = collector.block_on(async {
        let guard = setup_logs();
        let handle = guard.shutdown_handle();
        do_log(&guard);
        handle.shutdown()
    });

    res.expect("shutdown should succeed");

    let actual = collector.finish();
    let expected = expected_logs();
    assert_eq!(actual.resource_logs, expected);
}

#[test]
fn export_guard_fail() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
    }

    // don't spin up a collector
    let rt = make_rt();
    rt.block_on(async {
        let guard = setup_logs();
        // the batch can't be empty, otherwise the shutdown "succeeds"
        do_log(&guard);
    });
}

#[test]
fn export_shutdown_fail() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
    }

    // don't spin up a collector
    let rt = make_rt();
    let res = rt.block_on(async {
        let guard = setup_logs();

        // prevent the guard from being dropped, so shutdown won't be called
        let guard = ManuallyDrop::new(guard);
        // create a shutdown handle
        let handle = guard.shutdown_handle();

        // the batch can't be empty, otherwise the shutdown "succeeds"
        do_log(&guard);

        handle.shutdown()
    });

    let err = res.expect_err("shutdown should fail");
    assert!(
        !err.already_shutdown(),
        "should not be already shut down: {err}"
    );
}

fn make_rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(3)
        .enable_all()
        .build()
        .expect("failed to build the runtime")
}

fn setup_logs() -> LogsGuard {
    let resource = ResourceBuilder::without_default("nv.test").build();
    let config = LogsConfig::from_env();
    config.guard().with_resource(&resource).build()
}

fn do_log(guard: &LogsGuard) {
    let logger = guard.logger("test").unwrap();

    let ts = SystemTime::UNIX_EPOCH + Duration::from_nanos(100);
    let ots = SystemTime::UNIX_EPOCH + Duration::from_nanos(101);

    use opentelemetry::logs::{LogRecord as _, Logger as _, Severity};
    let mut record = logger.create_log_record();
    record.set_body("my event".into());
    record.set_event_name("test-event");
    record.set_observed_timestamp(ots);
    record.set_severity_text("info");
    record.set_severity_number(Severity::Info);
    // record.set_target("my_target"); // overrides instrumentation scope
    record.set_timestamp(ts);
    record.set_trace_context(TRACE_ID, SPAN_ID, Some(TraceFlags::SAMPLED));
    logger.emit(record);
}

fn expected_logs() -> Vec<types::logs::ResourceLogs> {
    use types::common::{InstrumentationScope, KeyValue};
    use types::logs::{LogRecord, ResourceLogs, ScopeLogs, SeverityNumber};
    use types::resource::Resource;

    let context = SpanContext::new(
        TRACE_ID,
        SPAN_ID,
        TraceFlags::SAMPLED,
        false,
        TraceState::NONE,
    );

    vec![ResourceLogs {
        resource: Some(Resource::new(vec![KeyValue::new(
            "service.name",
            "nv.test",
        )])),
        schema_url: String::new(),
        scope_logs: vec![ScopeLogs {
            scope: Some(InstrumentationScope::new("test", "", vec![])),
            schema_url: String::new(),
            log_records: vec![LogRecord {
                severity_number: Ok(SeverityNumber::Info),
                severity_text: "info".to_string(),
                event_name: "test-event".to_string(),
                body: Some("my event".into()),
                attributes: vec![],
                dropped_attributes_count: 0,
                context,
                time_unix_nano: 100,
                observed_time_unix_nano: 101,
            }],
        }],
    }]
}
