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

use opentelemetry::KeyValue;
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use otel_asserts::{TcpListener, grpc, http, types, unsafe_env};
use otel_tools::exporters::ShutdownError as _;
use otel_tools::exporters::traces::TracesGuard;
use pretty_assertions::assert_eq;
use std::mem::ManuallyDrop;

#[test]
fn export_grpc_success() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": endpoint,
    }

    let collector = grpc::Collector::spans(listener);
    let context = collector.block_on(async {
        let guard = setup_traces();
        do_span(&guard)
    });

    let mut actual = collector.finish();
    actual.assert_timestamps();
    let expected = expected_spans(context);
    assert_eq!(actual.resource_spans, expected);
}

#[test]
fn export_http_success() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}/v1/traces");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "http/protobuf",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": endpoint,
    }

    let collector = http::Collector::spans(listener);
    let context = collector.block_on(async {
        let guard = setup_traces();
        do_span(&guard)
    });

    let mut actual = collector.finish();
    actual.assert_timestamps();
    let expected = expected_spans(context);
    assert_eq!(actual.resource_spans, expected);
}

#[test]
fn export_shutdown_success() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": endpoint,
    }

    let collector = grpc::Collector::spans(listener);
    let (res, context) = collector.block_on(async {
        let guard = setup_traces();

        // prevent the guard from being dropped, so shutdown won't be called
        let guard = ManuallyDrop::new(guard);
        // create a shutdown handle
        let handle = guard.shutdown_handle();

        // emit a span, so we can test that the shutdown handle ships data
        let context = do_span(&guard);

        let res = handle.shutdown();
        (res, context)
    });

    res.expect("shutdown should succeed");

    let mut actual = collector.finish();
    actual.assert_timestamps();
    let expected = expected_spans(context);
    assert_eq!(actual.resource_spans, expected);
}

#[test]
fn export_shutdown_double() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": endpoint,
    }

    let collector = grpc::Collector::spans(listener);
    let (res, context) = collector.block_on(async {
        let guard = setup_traces();
        let handle = guard.shutdown_handle();
        let context = do_span(&guard);
        let res = handle.shutdown();
        (res, context)
    });

    res.expect("shutdown should succeed");

    let mut actual = collector.finish();
    actual.assert_timestamps();
    let expected = expected_spans(context);
    assert_eq!(actual.resource_spans, expected);
}

#[test]
fn export_guard_fail() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": endpoint,
    }

    // don't spin up a collector
    let rt = make_rt();
    rt.block_on(async {
        let guard = setup_traces();
        do_span(&guard);
    });
}

#[test]
fn export_shutdown_fail() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": endpoint,
    }

    // don't spin up a collector
    let rt = make_rt();
    let res = rt.block_on(async {
        let guard = setup_traces();

        // prevent the guard from being dropped, so shutdown won't be called
        let guard = ManuallyDrop::new(guard);
        // create a shutdown handle
        let handle = guard.shutdown_handle();

        do_span(&guard);

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

fn setup_traces() -> TracesGuard {
    let resource = otel_tools::resource::ResourceBuilder::without_default("nv.test").build();
    let config = otel_tools::exporters::traces::TracesConfig::from_env();
    config.guard().with_resource(&resource).build()
}

fn do_span(guard: &TracesGuard) -> SpanContext {
    // emit a span (not using tracing!)
    let tracer = guard.tracer("test").unwrap();
    let mut context = SpanContext::NONE;

    use opentelemetry::trace::{Tracer as _, get_active_span};
    tracer.in_span("foo", |_context| {
        get_active_span(|span| {
            context = span.span_context().clone();
            span.add_event("bar", vec![KeyValue::new("spam", "eggs")]);
        });
    });

    assert_ne!(context, SpanContext::NONE);
    context
}

fn expected_spans(context: SpanContext) -> Vec<types::trace::ResourceSpans> {
    use types::common::{InstrumentationScope, KeyValue};
    use types::resource::Resource;
    use types::trace::{Event, ResourceSpans, ScopeSpans, Span, SpanKind, Status, StatusCode};

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
                "foo",
                SpanKind::Internal,
                vec![],
                vec![Event::new("bar", vec![KeyValue::new("spam", "eggs")])],
                Status::new("", StatusCode::Unset),
                context,
                SpanId::INVALID,
            )],
        }],
    }]
}

fn link1() -> SpanContext {
    SpanContext::new(
        TraceId::from_hex("A0112233445566778899aabbccddeeff").unwrap(),
        SpanId::from_hex("A011223344556677").unwrap(),
        TraceFlags::SAMPLED,
        false,
        TraceState::NONE,
    )
}

fn link2() -> SpanContext {
    SpanContext::new(
        TraceId::from_hex("B0112233445566778899aabbccddeeff").unwrap(),
        SpanId::from_hex("B011223344556677").unwrap(),
        TraceFlags::SAMPLED,
        false,
        TraceState::NONE,
    )
}

fn link3() -> SpanContext {
    SpanContext::new(
        TraceId::from_hex("C0112233445566778899aabbccddeeff").unwrap(),
        SpanId::from_hex("C011223344556677").unwrap(),
        TraceFlags::SAMPLED,
        false,
        TraceState::NONE,
    )
}

#[test]
fn export_limits() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": endpoint,
        "OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT": "2",
        "OTEL_SPAN_EVENT_COUNT_LIMIT": "2",
        "OTEL_SPAN_LINK_COUNT_LIMIT": "2",
        "OTEL_EVENT_ATTRIBUTE_COUNT_LIMIT": "2",
        "OTEL_LINK_ATTRIBUTE_COUNT_LIMIT": "2",
    }

    let collector = grpc::Collector::spans(listener);
    let context = collector.block_on(async {
        let guard = setup_traces();

        let attributes = vec![
            KeyValue::new("key 1", "val 1"),
            KeyValue::new("key 2", "val 2"),
            KeyValue::new("key 3", "val 3"),
        ];

        // emit a span (not using tracing!)
        let tracer = guard.tracer("test").unwrap();
        let mut context = SpanContext::NONE;

        use opentelemetry::trace::{Tracer as _, get_active_span};
        tracer.in_span("span 1", |_context| {
            get_active_span(|span| {
                span.set_attributes(attributes.clone());

                context = span.span_context().clone();

                span.add_event("event 1", attributes.clone());
                span.add_event("event 2", attributes.clone());
                span.add_event("event 3", attributes.clone());

                span.add_link(link1(), attributes.clone());
                span.add_link(link2(), attributes.clone());
                span.add_link(link3(), attributes.clone());
            });
        });

        assert_ne!(context, SpanContext::NONE);
        context
    });

    let mut actual = collector.finish();
    actual.assert_timestamps();
    let expected = export_limits_expected_spans(context);
    assert_eq!(actual.resource_spans, expected);
}

fn export_limits_expected_spans(context: SpanContext) -> Vec<types::trace::ResourceSpans> {
    use types::common::{InstrumentationScope, KeyValue};
    use types::resource::Resource;
    use types::trace::{
        Event, Link, ResourceSpans, ScopeSpans, Span, SpanKind, Status, StatusCode,
    };

    let attributes = vec![
        KeyValue::new("key 1", "val 1"),
        KeyValue::new("key 2", "val 2"),
    ];

    vec![ResourceSpans {
        resource: Some(Resource::new(vec![KeyValue::new(
            "service.name",
            "nv.test",
        )])),
        schema_url: String::new(),
        scope_spans: vec![ScopeSpans {
            scope: Some(InstrumentationScope::new("test", "", vec![])),
            schema_url: String::new(),
            spans: vec![Span {
                name: "span 1".to_string(),
                kind: Some(SpanKind::Internal),
                attributes: attributes.clone(),
                dropped_attributes_count: 1,
                events: vec![
                    Event {
                        name: "event 1".to_string(),
                        attributes: attributes.clone(),
                        dropped_attributes_count: 1,
                        time_unix_nano: 0,
                    },
                    Event {
                        name: "event 2".to_string(),
                        attributes: attributes.clone(),
                        dropped_attributes_count: 1,
                        time_unix_nano: 0,
                    },
                ],
                dropped_events_count: 1,
                links: vec![
                    Link {
                        context: link1(),
                        attributes: attributes.clone(),
                        dropped_attributes_count: 1,
                    },
                    Link {
                        context: link2(),
                        attributes,
                        dropped_attributes_count: 1,
                    },
                ],
                dropped_links_count: 1,
                status: Some(Status::new("", StatusCode::Unset)),
                context,
                parent: SpanId::INVALID,
                start_time_unix_nano: 0,
                end_time_unix_nano: 0,
            }],
        }],
    }]
}
