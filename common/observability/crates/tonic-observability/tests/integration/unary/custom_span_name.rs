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
use tonic_observability::client::trace::{
    TonicTraceLayer as ClientTraceLayer, make_span as client_span,
};
use tonic_observability::server::trace::{
    TonicTraceLayer as ServerTraceLayer, make_span as server_span,
};

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

    async fn make(self, listener: tokio::net::TcpListener, _guards: &Guards) -> Self::Fut {
        let trace_layer = ServerTraceLayer::builder()
            .with_span(server_span!(concat!(module_path!(), "::my_grpc_server")))
            .with_rpc_map(rpc_map())
            .build();

        make_server(listener, |server| {
            server.layer(trace_layer).add_service(Server::new(Svc))
        })
        .await
    }
}

#[test]
fn server_make_span_debug() {
    let make_span = server_span!();
    let actual = format!("{make_span:?}");
    assert_eq!(actual, "MakeSpan { .. }");
}

#[test]
fn client_make_span_debug() {
    let make_span = client_span!();
    let actual = format!("{make_span:?}");
    assert_eq!(actual, "MakeSpan { .. }");
}

#[test]
fn test() {
    let (request, _req_size) = make_req(echo_req("Echo"));

    let (response, _resp_size) = make_resp(echo_resp("Echo"));

    let resource = resource();
    let make_guards = || Guards::custom(&resource, "info", true);

    let trace_layer = ClientTraceLayer::builder()
        .with_span(client_span!(concat!(module_path!(), "::my_grpc_client")))
        .build();

    let output = harness_custom(make_guards, Svc, async |make| {
        let mut client: Client<_> = make.layer(trace_layer).client();
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
                    "example.Unary/Echo (server)" => {
                        "echo (server)"
                    }
                }
            }
        }
    });
    actual_spans.hierarchy.assert_contains_children();

    actual_spans.assert_client_attrs(expected_attrs! {
        // metadata
        semconv::CODE_FILE_PATH => "crates/tonic-observability/tests/integration/unary/custom_span_name.rs",
        semconv::CODE_MODULE_NAME => "integration::unary::custom_span_name",
        semconv::OTEL_STATUS_CODE => "Unset",
        semconv::TARGET => "integration::unary::custom_span_name::my_grpc_client",
        // General
        semconv::SERVER_ADDRESS => output.server_uri.as_str(),
        // gRPC
        semconv::RPC_SYSTEM_NAME => "grpc",
        semconv::RPC_METHOD => "example.Unary/Echo",
        semconv::RPC_RESPONSE_STATUS_CODE => sn(Code::Ok),
    });

    actual_spans.assert_server_attrs(expected_attrs! {
        // metadata
        semconv::CODE_FILE_PATH => "crates/tonic-observability/tests/integration/unary/custom_span_name.rs",
        semconv::CODE_MODULE_NAME => "integration::unary::custom_span_name",
        semconv::OTEL_STATUS_CODE => "Unset",
        semconv::TARGET => "integration::unary::custom_span_name::my_grpc_server",
        // HTTP
        semconv::SERVER_ADDRESS => "127.0.0.1",
        // gRPC
        semconv::RPC_SYSTEM_NAME => "grpc",
        semconv::RPC_METHOD => "example.Unary/Echo",
        semconv::RPC_RESPONSE_STATUS_CODE => sn(Code::Ok),
    });

    // as this test focusses on custom spans, the metrics aren't asserted
}
