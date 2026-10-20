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

use super::resp_err_layer::RespErrLayer;
use crate::harness::prelude::*;
use crate::harness::server::{MakeServer, make_server};
use protos::unary_client::UnaryClient as Client;
use protos::unary_server::{Unary as Service, UnaryServer as Server};
use testing::guards::Guards;
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

    async fn make(self, listener: tokio::net::TcpListener, _guards: &Guards) -> Self::Fut {
        // the server doesn't need metrics, as the error is injected after the
        // response rendering them meaningless for the test
        let trace_layer = ServerTraceLayer::builder().with_rpc_map(rpc_map()).build();

        make_server(listener, |server| {
            server.layer(trace_layer).add_service(Server::new(Svc))
        })
        .await
    }
}

/// Like [`super::response_body_err`], but using a bare [`tonic::Status`]
/// without an error source (`status.source()` returns `None`).
///
/// This covers the `None` branch of the `if let Some(e) = status.source()`
/// check in `client::trace::span::record_tonic_status()`.
#[test]
fn test() {
    let (request, req_size) = make_req(echo_req("Echo"));

    let (response, _resp_size) = make_resp(Status::unknown("foo"));

    let resource = resource();

    let output = harness(&resource, Svc, async |make| {
        let status = tonic::Status::unknown("foo");
        let mut client: Client<_> = make
            .default_layers()
            .layer(RespErrLayer::new(status))
            .client();
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
        semconv::CODE_FILE_PATH => CLIENT_TRACE_PATH,
        semconv::CODE_MODULE_NAME => CLIENT_TRACE_MODULE,
        semconv::ERROR_TYPE => sn(Code::Unknown),
        semconv::OTEL_STATUS_CODE => "Error",
        semconv::OTEL_STATUS_DESCRIPTION => "foo",
        // General
        semconv::SERVER_ADDRESS => output.server_uri.as_str(),
        // gRPC
        semconv::RPC_SYSTEM_NAME => "grpc",
        semconv::RPC_METHOD => "example.Unary/Echo",
        semconv::RPC_RESPONSE_STATUS_CODE => sn(Code::Unknown),
    });

    // no exception event: the bare tonic::Status has no error source

    // the server span doesn't need to be asserted, as the error is injected
    // after the response rendering them meaningless for the test

    let actual_metrics = output.metrics(&resource);

    expected_metrics!(
        actual_metrics => "example.Unary/Echo",
        {
        },
        {
            semconv::GRPC_CLIENT_ATTEMPT_STARTED => "{attempt}": sum { 1 },
            semconv::GRPC_CLIENT_ATTEMPT_ACTIVE => "{attempt}": sum { 0 },
            semconv::GRPC_CLIENT_REQUEST_ACTIVE => "{request}": sum { 0 },
            semconv::GRPC_CLIENT_RESPONSE_ACTIVE => "{response}": sum { 0 },
            semconv::GRPC_CLIENT_ATTEMPT_DURATION => "s": histogram { 1 x > 0.0 } => {
                semconv::GRPC_STATUS => sn(Code::Unknown),
            },
            semconv::GRPC_CLIENT_ATTEMPT_TX_SIZE => "By": histogram { 1 x req_size },
            semconv::GRPC_CLIENT_ATTEMPT_RX_SIZE => "By": histogram { 1 x 0.0 } => {
                semconv::GRPC_STATUS => sn(Code::Unknown),
            },
        },
    );
}
