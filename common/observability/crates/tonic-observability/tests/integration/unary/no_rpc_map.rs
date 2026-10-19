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

use crate::harness::harness_custom;
use crate::harness::prelude::*;
use crate::harness::server::{MakeServer, make_server};
use protos::unary_client::UnaryClient as Client;
use protos::unary_server::{Unary as Service, UnaryServer as Server};
use testing::guards::Guards;
use tonic_observability::client::metric_a66::TonicMetricLayer as ClientMetricLayer;
use tonic_observability::client::trace::TonicTraceLayer as ClientTraceLayer;
use tonic_observability::server::metric_a66::TonicMetricLayer as ServerMetricLayer;
use tonic_observability::server::trace::TonicTraceLayer as ServerTraceLayer;

struct Svc;

#[async_trait]
impl Service for Svc {
    #[instrument(name = "echo (server)", level = "info", skip_all)]
    async fn echo(&self, request: Request<EchoRequest>) -> Result<Response<EchoResponse>> {
        let message = request.into_inner().message;
        Ok(Response::new(EchoResponse { message }))
    }
}

#[async_trait]
impl MakeServer for Svc {
    type Fut = futures::future::BoxFuture<'static, ()>;

    async fn make(self, listener: tokio::net::TcpListener, guards: &Guards) -> Self::Fut {
        let meter = guards.metrics_guard.meter("test").unwrap();

        let metric_a66_layer = ServerMetricLayer::builder(meter).build();
        let trace_layer = ServerTraceLayer::builder().build();
        make_server(listener, |server| {
            server
                .layer(metric_a66_layer)
                .layer(trace_layer)
                .add_service(Server::new(Svc))
        })
        .await
    }
}

#[test]
fn test() {
    let (request, req_size) = make_req(echo_req("Echo"));
    let (response, resp_size) = make_resp(echo_resp("Echo"));

    let resource = resource();
    let make_guards = || Guards::new(&resource);

    let output = harness_custom(make_guards, Svc, async |make| {
        let metric_a66_layer = ClientMetricLayer::builder(make.meter()).build();
        let trace_layer = ClientTraceLayer::builder().build();

        let mut client: Client<_> = make.layer(metric_a66_layer).layer(trace_layer).client();
        client
            .echo(request)
            .instrument(info_span!("echo (client)"))
            .await
            .consume()
            .await
    });

    assert_eq!(output.response(), &response);

    let actual_spans = output.spans(&resource);

    actual_spans.assert(expected_hierarchy! {
        "test" => {
            "echo (client)" => {
                "example.Unary/Echo (client)" => {
                    "grpc (server)" => {
                        "echo (server)"
                    }
                }
            }
        }
    });
    actual_spans.hierarchy.assert_contains_children();

    actual_spans.assert_client_attrs(expected_attrs! {
        // metadata
        semconv::CODE_FILE_PATH => CLIENT_TRACE_PATH,
        semconv::CODE_MODULE_NAME => CLIENT_TRACE_MODULE,
        semconv::OTEL_STATUS_CODE => "Unset",
        // General
        semconv::SERVER_ADDRESS => output.server_uri.as_str(),
        // gRPC
        semconv::RPC_SYSTEM_NAME => "grpc",
        semconv::RPC_METHOD => "example.Unary/Echo",
        semconv::RPC_RESPONSE_STATUS_CODE => sn(Code::Ok),
    });

    actual_spans.assert_server_attrs(expected_attrs! {
        // metadata
        semconv::CODE_FILE_PATH => SERVER_TRACE_PATH,
        semconv::CODE_MODULE_NAME => SERVER_TRACE_MODULE,
        semconv::OTEL_STATUS_CODE => "Unset",
        // HTTP
        semconv::SERVER_ADDRESS => "127.0.0.1",
        // gRPC
        semconv::RPC_SYSTEM_NAME => "grpc",
        semconv::RPC_METHOD => "_OTHER",
        semconv::RPC_METHOD_ORIGINAL => "example.Unary/Echo",
        semconv::RPC_RESPONSE_STATUS_CODE => sn(Code::Ok),
    });

    let actual_metrics = output.metrics(&resource);

    expected_metrics!(
        actual_metrics => "example.Unary/Echo",
        {
            semconv::GRPC_SERVER_CALL_DURATION => "s": histogram { 1 x > 0.0 } => {
                semconv::GRPC_STATUS => sn(Code::Ok),
            },
            semconv::GRPC_SERVER_CALL_RX_SIZE => "By": histogram { 1 x req_size },
            semconv::GRPC_SERVER_CALL_TX_SIZE => "By": histogram { 1 x resp_size } => {
                semconv::GRPC_STATUS => sn(Code::Ok),
            },
            semconv::GRPC_SERVER_CALL_STARTED => "{call}": sum { 1 },
            semconv::GRPC_SERVER_HANDLER_ACTIVE => "{handler}": sum { 0 },
            semconv::GRPC_SERVER_REQUEST_ACTIVE => "{request}": sum { 0 },
            semconv::GRPC_SERVER_RESPONSE_ACTIVE => "{response}": sum { 0 },
        },
        {
            semconv::GRPC_CLIENT_ATTEMPT_STARTED => "{attempt}": sum { 1 },
            semconv::GRPC_CLIENT_ATTEMPT_ACTIVE => "{attempt}": sum { 0 },
            semconv::GRPC_CLIENT_REQUEST_ACTIVE => "{request}": sum { 0 },
            semconv::GRPC_CLIENT_RESPONSE_ACTIVE => "{response}": sum { 0 },
            semconv::GRPC_CLIENT_ATTEMPT_DURATION => "s": histogram { 1 x > 0.0 } => {
                semconv::GRPC_STATUS => sn(Code::Ok),
            },
            semconv::GRPC_CLIENT_ATTEMPT_TX_SIZE => "By": histogram { 1 x req_size },
            semconv::GRPC_CLIENT_ATTEMPT_RX_SIZE => "By": histogram { 1 x resp_size } => {
                semconv::GRPC_STATUS => sn(Code::Ok),
            },
        },
    );
}
