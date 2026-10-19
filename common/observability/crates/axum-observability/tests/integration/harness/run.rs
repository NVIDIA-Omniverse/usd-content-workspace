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

use super::client::MakeClient;
use super::server::MakeService;
use testing::guards::Guards;
use testing::output::Output;

pub(crate) fn harness<TestFn, TestFut, Res>(
    resource: &otel_tools::resource::Resource,
    router: axum::Router<()>,
    f: TestFn,
) -> Output<Res>
where
    TestFn: FnOnce(MakeClient) -> TestFut,
    TestFut: Future<Output = Res>,
{
    let service = super::server::DefaultMakeService { router };
    harness_custom(|| Guards::new(resource), service, f)
}

pub(crate) fn harness_custom<GuardsFn, GuardsFut, Service, TestFn, TestFut, Res>(
    make_guards: GuardsFn,
    service: Service,
    test_fn: TestFn,
) -> Output<Res>
where
    GuardsFn: FnOnce() -> GuardsFut,
    GuardsFut: Future<Output = Guards>,
    Service: MakeService,
    TestFn: FnOnce(MakeClient) -> TestFut,
    TestFut: Future<Output = Res>,
{
    // random port for collector
    let listener = otel_asserts::TcpListener::bind_random();
    let endpoint = format!("http://{listener}");

    // set otel env vars
    // SAFETY: there is only 1 thread, and nothing is reading env vars
    otel_asserts::unsafe_env! {
        "OTEL_LOGS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT": endpoint,
        "OTEL_METRICS_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT": endpoint,
        "OTEL_TRACES_EXPORTER": "otlp",
        "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL": "grpc",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": endpoint,
    }

    // start collector
    let collector = otel_asserts::grpc::Collector::all(listener);
    // run server + client
    let mut output = collector.block_on(harness_async(make_guards, service, test_fn));
    // shut down collector
    let (_logs, mut metrics, spans) = collector.finish();

    // assert and strip metrics timestamps
    metrics.assert_timestamps();

    output.metrics = metrics;
    output.spans = spans;

    output
}

async fn harness_async<GuardsFn, GuardsFut, Service, TestFn, TestFut, Res>(
    make_guards: GuardsFn,
    service: Service,
    test_fn: TestFn,
) -> Output<Res>
where
    GuardsFn: FnOnce() -> GuardsFut,
    GuardsFut: Future<Output = Guards>,
    Service: MakeService,
    TestFn: FnOnce(MakeClient) -> TestFut,
    TestFut: Future<Output = Res>,
{
    // initialize OTLP exporters
    let guards = make_guards().await;

    // random port for server
    let listener = otel_asserts::TcpListener::bind_random();
    let server_uri = format!("http://{listener}");

    // print the server port; this is useful for filtering packet captures
    println!("server port: {}", listener.local_addr().port());

    // client
    let url = reqwest::Url::parse(&server_uri).unwrap();
    let client = MakeClient::new(url);

    // spawn server
    let router = service.make(&guards);
    let shutdown = super::server::spawn(router, listener.into_tokio()).await;

    // make root/test span
    let span = tracing::info_span!("test");
    otel_tools::context::get_span_context!(span_context = span);
    let root = span_context.clone();

    // invoke test code
    use tracing::Instrument as _;
    let res = test_fn(client).instrument(span).await;

    // shut down server
    shutdown.await;

    // shut down OTLP exporters
    drop(guards);

    Output {
        root,
        server_uri,
        metrics: otel_asserts::types::metrics::OutputMetrics {
            resource_metrics: Vec::new(),
        },
        spans: otel_asserts::types::trace::OutputSpans {
            resource_spans: Vec::new(),
        },
        res,
    }
}
