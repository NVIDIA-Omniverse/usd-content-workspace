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

use otel_asserts::{TcpListener, grpc, http, types, unsafe_env};
use otel_tools::exporters::ShutdownError as _;
use otel_tools::exporters::metrics::MetricsGuard;
use pretty_assertions::assert_eq;
use std::mem::ManuallyDrop;

#[test]
fn export_grpc_success() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_METRICS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT": endpoint,
    }

    let collector = grpc::Collector::metrics(listener);
    collector.block_on(async {
        let guard = setup_metrics();
        do_metric(&guard);
    });

    let mut actual = collector.finish();
    actual.assert_timestamps();
    let expected = expected_metrics();
    assert_eq!(actual.resource_metrics, expected);
}

#[test]
fn export_http_success() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}/v1/metrics");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_METRICS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL": "http/protobuf",
        "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT": endpoint,
    }

    let collector = http::Collector::metrics(listener);
    collector.block_on(async {
        let guard = setup_metrics();
        do_metric(&guard);
    });

    let mut actual = collector.finish();
    actual.assert_timestamps();
    let expected = expected_metrics();
    assert_eq!(actual.resource_metrics, expected);
}

#[test]
fn export_shutdown_success() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_METRICS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT": endpoint,
    }

    let collector = grpc::Collector::metrics(listener);
    let res = collector.block_on(async {
        let guard = setup_metrics();

        // prevent the guard from being dropped, so shutdown won't be called
        let guard = ManuallyDrop::new(guard);
        // create a shutdown handle
        let handle = guard.shutdown_handle();

        // emit a metric, so we can test that the shutdown handle ships data
        do_metric(&guard);

        handle.shutdown()
    });

    res.expect("shutdown should succeed");

    let mut actual = collector.finish();
    actual.assert_timestamps();
    let expected = expected_metrics();
    assert_eq!(actual.resource_metrics, expected);
}

#[test]
fn export_shutdown_double() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_METRICS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT": endpoint,
    }

    let collector = grpc::Collector::metrics(listener);
    let res = collector.block_on(async {
        let guard = setup_metrics();
        let handle = guard.shutdown_handle();
        do_metric(&guard);
        handle.shutdown()
    });

    res.expect("shutdown should succeed");

    let mut actual = collector.finish();
    actual.assert_timestamps();
    let expected = expected_metrics();
    assert_eq!(actual.resource_metrics, expected);
}

#[test]
fn export_guard_fail() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_METRICS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT": endpoint,
    }

    // don't spin up a collector
    let rt = make_rt();
    rt.block_on(async {
        let guard = setup_metrics();
        do_metric(&guard);
    });
}

#[test]
fn export_shutdown_fail() {
    let listener = TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // SAFETY: there is only 1 thread, and nothing is reading env vars
    unsafe_env! {
        "OTEL_METRICS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT": endpoint,
    }

    // don't spin up a collector
    let rt = make_rt();
    let res = rt.block_on(async {
        let guard = setup_metrics();

        // prevent the guard from being dropped, so shutdown won't be called
        let guard = ManuallyDrop::new(guard);
        // create a shutdown handle
        let handle = guard.shutdown_handle();

        // emit a metric, so we can test that the shutdown handle ships data
        do_metric(&guard);

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

fn setup_metrics() -> MetricsGuard {
    let resource = otel_tools::resource::ResourceBuilder::without_default("nv.test").build();
    let config = otel_tools::exporters::metrics::MetricsConfig::from_env();
    config.guard().with_resource(&resource).build()
}

fn do_metric(guard: &MetricsGuard) {
    use opentelemetry::KeyValue;

    let meter = guard.meter("test").unwrap();
    let metric = meter
        .i64_gauge("foo")
        .with_description("bar")
        .with_unit("{baz}")
        .build();
    metric.record(42, &[KeyValue::new("spam", "eggs")]);
}

fn expected_metrics() -> Vec<types::metrics::ResourceMetrics> {
    use types::common::{InstrumentationScope, KeyValue};
    use types::metrics::{Data, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics};
    use types::resource::Resource;

    vec![ResourceMetrics {
        resource: Some(Resource::new(vec![KeyValue::new(
            "service.name",
            "nv.test",
        )])),
        schema_url: String::new(),
        scope_metrics: vec![ScopeMetrics {
            scope: Some(InstrumentationScope::new("test", "", vec![])),
            schema_url: String::new(),
            metrics: vec![Metric::new(
                "foo",
                "bar",
                "{baz}",
                Data::gauge(NumberDataPoint::new(
                    42,
                    vec![KeyValue::new("spam", "eggs")],
                )),
            )],
        }],
    }]
}
