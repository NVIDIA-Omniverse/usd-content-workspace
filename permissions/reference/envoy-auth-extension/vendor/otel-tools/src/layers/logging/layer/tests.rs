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

use super::{Level, LoggingLayer, level};
use crate::testing::FixedTracer;
use opentelemetry::KeyValue;
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState, Tracer, noop};
use opentelemetry_sdk::Resource;
use otel_asserts::tracing::TraceCapture;
use serde_json::Value;
use tracing_subscriber::fmt::MakeWriter;

const TEST_TRACE_ID: TraceId = TraceId::from_bytes([
    0x0a, 0xf7, 0x65, 0x19, 0x16, 0xcd, 0x43, 0xdd, 0x84, 0x48, 0xeb, 0x21, 0x1c, 0x80, 0x31, 0x9c,
]);
const TEST_SPAN_ID: SpanId = SpanId::from_bytes([0xb7, 0xad, 0x6b, 0x71, 0x69, 0x20, 0x33, 0x31]);

fn capture(resource: &Resource, f: impl FnOnce()) -> String {
    let buf = TraceCapture::new();
    let layer = LoggingLayer::builder()
        .with_make_writer(buf.clone())
        .with_resource(resource)
        .build();
    use tracing_subscriber::layer::SubscriberExt as _;
    let subscriber = tracing_subscriber::registry().with(layer);
    let dispatcher = tracing::Dispatch::new(subscriber);
    let guard = tracing::dispatcher::set_default(&dispatcher);
    f();
    drop(guard);
    drop(dispatcher);
    buf.take()
}

fn deserialize(s: &str) -> serde_json::Map<String, Value> {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("invalid json:{e}\n{s}"))
}

macro_rules! map {
    ({ $($k:literal: $v:expr),+ $(,)? }) => {{
        let mut map = serde_json::Map::<String, Value>::new();
        $(if let Some(_v) = map.insert($k.to_string(), Value::from($v)) {
            panic!("duplicate key `{}`", $k);
        })+
        map
    }};
}

/// This function removes the `timestamp` and `line` keys, and asserts them.
#[track_caller]
fn assert_variable(actual: &mut serde_json::Map<String, Value>) {
    let timestamp = actual.remove("timestamp").expect("`timestamp` missing");
    let timestamp = timestamp
        .as_str()
        .unwrap_or_else(|| panic!("`timestamp` is not a string: {timestamp:?}"));
    assert!(
        timestamp.len() >= 20,
        "timestamp: len(`{}`) == {} >= 20",
        timestamp,
        timestamp.len()
    );
    assert!(
        timestamp.ends_with('Z'),
        "timestamp: `{timestamp}` ends with 'Z'"
    );

    let line = actual.remove("line").expect("`line` is missing");
    let line = line
        .as_i64()
        .unwrap_or_else(|| panic!("`line` is not a i64: {line:?}"));
    assert!(line > 0, "line: {line} > 0");
}

#[track_caller]
fn assert_order(
    actual: &serde_json::Map<String, Value>,
    expected: &serde_json::Map<String, Value>,
) {
    for ((k1, v1), (k2, v2)) in expected.iter().zip(actual.iter()) {
        assert_eq!(k1, k2, "key");
        assert_eq!(v1, v2, "val");
    }
}

#[expect(dead_code)]
fn test_type_signature_compiles() -> LoggingLayer<impl for<'a> MakeWriter<'a> + 'static> {
    LoggingLayer::builder().build()
}

#[test]
fn stdout_make_writer_is_std_io_stdout() {
    let stdout = super::Stdout;
    let _writer: std::io::Stdout = stdout.make_writer();
}

#[test]
fn event_basic() {
    let resource = Resource::builder_empty().build();
    let json = capture(&resource, || {
        tracing::info!("foo");
    });

    let mut actual = deserialize(&json);
    assert_variable(&mut actual);

    let expected = map!({
        "message": "foo",
        "level": "info",
        "target": module_path!(),
        "file": file!(),
    });
    assert_eq!(actual, expected);
    assert_order(&actual, &expected);
}

#[test]
fn event_reserved_field_is_skipped() {
    let resource = Resource::builder_empty().build();
    let json = capture(&resource, || {
        let level = "error";
        tracing::info!(level, "foo");
    });

    let actual = deserialize(&json);
    assert_eq!(actual.get("level").cloned(), Some(Value::from("info")));
}

#[test]
fn event_no_message() {
    let resource = Resource::builder_empty().build();
    let json = capture(&resource, || {
        tracing::info!(target = "pass");
    });

    let mut actual = deserialize(&json);
    assert_variable(&mut actual);

    let expected = map!({
        "message": "<empty>",
        "level": "info",
        "target": module_path!(),
        "file": file!(),
    });
    assert_eq!(actual, expected);
    assert_order(&actual, &expected);
}

#[test]
fn event_with_resource() {
    let resource = Resource::builder_empty()
        .with_attribute(KeyValue::new("service.name", "awesome-service"))
        .build();
    let json = capture(&resource, || {
        tracing::info!("foo");
    });

    let mut actual = deserialize(&json);
    assert_variable(&mut actual);

    let expected = map!({
        "message": "foo",
        "level": "info",
        "target": module_path!(),
        "file": file!(),
        "service.name": "awesome-service",
    });
    assert_eq!(actual, expected);
    assert_order(&actual, &expected);
}

#[test]
fn event_with_field() {
    let resource = Resource::builder_empty().build();
    let json = capture(&resource, || {
        let field = 42;
        tracing::info!(field, "foo");
    });

    let mut actual = deserialize(&json);
    assert_variable(&mut actual);

    let expected = map!({
        "message": "foo",
        "level": "info",
        "target": module_path!(),
        "file": file!(),
        "field": 42,
    });
    assert_eq!(actual, expected);
    assert_order(&actual, &expected);
}

#[test]
fn event_with_span() {
    let resource = Resource::builder_empty().build();
    let json = capture(&resource, || {
        let span = tracing::info_span!("span");
        let _guard = span.enter();
        tracing::info!("foo");
    });

    let mut actual = deserialize(&json);
    assert_variable(&mut actual);

    let expected = map!({
        "message": "[span] foo",
        "level": "info",
        "target": module_path!(),
        "file": file!(),
    });
    assert_eq!(actual, expected);
    assert_order(&actual, &expected);
}

fn capture_with_otel<T, F>(resource: &Resource, tracer: T, f: F) -> String
where
    T: Send + Sync + Tracer + 'static,
    T::Span: Send + Sync + 'static,
    F: FnOnce(),
{
    let buf = TraceCapture::new();
    let logging_layer = LoggingLayer::builder()
        .with_make_writer(buf.clone())
        .with_resource(resource)
        .build();
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    use tracing_subscriber::layer::SubscriberExt as _;
    let subscriber = tracing_subscriber::registry()
        .with(logging_layer)
        .with(otel_layer);
    let dispatcher = tracing::Dispatch::new(subscriber);
    let guard = tracing::dispatcher::set_default(&dispatcher);
    f();
    drop(guard);
    drop(dispatcher);
    buf.take()
}

#[test]
fn event_with_context_missing() {
    let resource = Resource::builder_empty().build();
    let tracer = noop::NoopTracer::new();
    let json = capture_with_otel(&resource, tracer, || {
        let span = tracing::info_span!("span");
        let _guard = span.enter();
        tracing::info!("foo");
    });

    let mut actual = deserialize(&json);
    assert_variable(&mut actual);

    let expected = map!({
        "message": "[span] foo",
        "level": "info",
        "target": module_path!(),
        "file": file!(),
    });
    assert_eq!(actual, expected);
    assert_order(&actual, &expected);
}

#[test]
fn event_with_context_valid() {
    let resource = Resource::builder_empty().build();
    // fake a context that might be provided if OpenTelemetry is set up with
    // a `RandomIdGenerator` (the default `IdGenerator`).
    let tracer = FixedTracer::new(SpanContext::new(
        TEST_TRACE_ID,
        TEST_SPAN_ID,
        TraceFlags::SAMPLED,
        false,
        TraceState::NONE,
    ));
    let json = capture_with_otel(&resource, tracer, || {
        let span = tracing::info_span!("span");
        let _guard = span.enter();
        tracing::info!("foo");
    });

    let mut actual = deserialize(&json);
    assert_variable(&mut actual);

    let expected = map!({
        "message": "[span] foo",
        "level": "info",
        "target": module_path!(),
        "file": file!(),
        "trace_id": "0af7651916cd43dd8448eb211c80319c",
        "span_id": "b7ad6b7169203331",
    });
    assert_eq!(actual, expected);
    assert_order(&actual, &expected);
}

#[test]
fn event_with_context_invalid_trace_id() {
    let resource = Resource::builder_empty().build();
    // fake a context that might be provided if OpenTelemetry is set up with
    // a `RandomIdGenerator` (the default `IdGenerator`).
    let tracer = FixedTracer::new(SpanContext::new(
        TraceId::INVALID,
        TEST_SPAN_ID,
        TraceFlags::SAMPLED,
        false,
        TraceState::NONE,
    ));

    let json = capture_with_otel(&resource, tracer, || {
        let span = tracing::info_span!("span");
        let _guard = span.enter();
        tracing::info!("foo");
    });

    let mut actual = deserialize(&json);
    assert_variable(&mut actual);

    let expected = map!({
        "message": "[span] foo",
        "level": "info",
        "target": module_path!(),
        "file": file!(),
    });
    assert_eq!(actual, expected);
    assert_order(&actual, &expected);
}

#[test]
fn event_with_context_invalid_span_id() {
    let resource = Resource::builder_empty().build();
    // fake a context that might be provided if OpenTelemetry is set up with
    // a `RandomIdGenerator` (the default `IdGenerator`).
    let tracer = FixedTracer::new(SpanContext::new(
        TEST_TRACE_ID,
        SpanId::INVALID,
        TraceFlags::SAMPLED,
        false,
        TraceState::NONE,
    ));
    let json = capture_with_otel(&resource, tracer, || {
        let span = tracing::info_span!("span");
        let _guard = span.enter();
        tracing::info!("foo");
    });

    let mut actual = deserialize(&json);
    assert_variable(&mut actual);

    let expected = map!({
        "message": "[span] foo",
        "level": "info",
        "target": module_path!(),
        "file": file!(),
    });
    assert_eq!(actual, expected);
    assert_order(&actual, &expected);
}

#[test]
fn log_levels() {
    // only for coverage, can be changed if wanted
    assert_eq!(level(&Level::TRACE), "trace");
    assert_eq!(level(&Level::DEBUG), "debug");
    assert_eq!(level(&Level::INFO), "info");
    assert_eq!(level(&Level::WARN), "warn");
    assert_eq!(level(&Level::ERROR), "error");
}
