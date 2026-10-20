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

use super::make_client::{Identity, MakeClient};
use super::server::MakeServer;
use testing::guards::Guards;
use testing::output::Output;

pub(crate) fn harness<Server, TestFn, TestFut, Res>(
    resource: &otel_tools::resource::Resource,
    make_server: Server,
    f: TestFn,
) -> Output<Res>
where
    Server: MakeServer,
    TestFn: FnOnce(MakeClient<Identity>) -> TestFut,
    TestFut: Future<Output = Res>,
{
    harness_custom(|| Guards::new(resource), make_server, f)
}

pub(crate) fn harness_custom<GuardsFn, GuardsFut, Server, TestFn, TestFut, Res>(
    make_guards: GuardsFn,
    make_server: Server,
    test_fn: TestFn,
) -> Output<Res>
where
    GuardsFn: FnOnce() -> GuardsFut,
    GuardsFut: Future<Output = Guards>,
    Server: MakeServer,
    TestFn: FnOnce(MakeClient<Identity>) -> TestFut,
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
    let mut output = collector.block_on(harness_async(make_guards, make_server, test_fn));
    // shut down collector
    let (_logs, mut metrics, spans) = collector.finish();

    // assert and strip metrics timestamps
    metrics.assert_timestamps();

    output.metrics = metrics;
    output.spans = spans;

    output
}

async fn harness_async<GuardsFn, GuardsFut, Server, TestFn, TestFut, Res>(
    make_guards: GuardsFn,
    make_server: Server,
    test_fn: TestFn,
) -> Output<Res>
where
    GuardsFn: FnOnce() -> GuardsFut,
    GuardsFut: Future<Output = Guards>,
    Server: MakeServer,
    TestFn: FnOnce(MakeClient<Identity>) -> TestFut,
    TestFut: Future<Output = Res>,
{
    // initialize OTLP exporters
    let guards = make_guards().await;

    // random port for server
    let listener = otel_asserts::TcpListener::bind_random();
    let uri = tonic::transport::Uri::builder()
        .scheme("http")
        .authority(listener.to_string())
        .path_and_query("")
        .build()
        .unwrap();
    // we want the URI without a trailing slash as the `grpc.target` attribute
    // doesn't include it
    let server_uri = uri.to_string();
    let server_uri = server_uri
        .as_str()
        .strip_suffix('/')
        .unwrap_or(server_uri.as_str())
        .to_string();

    // start server
    let shutdown = make_server.make(listener.into_tokio(), &guards).await;

    // make client
    let meter = guards.metrics_guard.meter("test").unwrap();
    let make_channel = MakeClient::new(uri, meter).await;

    // make root/test span
    let span = tracing::info_span!("test");
    otel_tools::context::get_span_context!(span_context = span);
    let root = span_context.clone();

    // invoke test code
    use tracing::Instrument as _;
    let res = test_fn(make_channel).instrument(span).await;

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
