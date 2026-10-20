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

use crate::harness::prelude::*;
use crate::harness::server::{BoxError, HttpRequest, HttpResult, HttpServer as Server};
use protos::unary_client::UnaryClient as Client;

async fn handler(request: HttpRequest) -> HttpResult<BoxError> {
    let span = info_span!("hyper", otel.kind = "Server");
    otel_tools::context::propagate::extract(&span, request.headers());
    let _guard = span.enter();

    Err(std::io::Error::new(std::io::ErrorKind::CrossesDevices, "bar").into())
}

/// Test the client layers against a HTTP/2 server that returns an error
/// immediately.
///
/// The server layers don't work in this case, as they can only be applied to
/// servers returning [`std::convert::Infallible`].
///
/// This covers `client::trace::span::record_transport_err`.
#[test]
fn test() {
    let (request, req_size) = make_req(echo_req("Echo"));

    let (response, _resp_size) = make_resp(Status::internal("h2 protocol error: http2 error"));

    let resource = resource();
    let svc = Server::http2(handler);

    let output = harness(&resource, svc, async move |make| {
        let mut client: Client<_> = make.default();

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
                    "hyper (server)",
                }
            }
        }
    });
    actual_spans.hierarchy.assert_contains_children();

    actual_spans.assert_client_attrs(expected_attrs! {
        // metadata
        semconv::CODE_FILE_PATH => CLIENT_TRACE_PATH,
        semconv::CODE_MODULE_NAME => CLIENT_TRACE_MODULE,
        semconv::ERROR_TYPE => sn(Code::Internal),
        semconv::OTEL_STATUS_CODE => "Error",
        semconv::OTEL_STATUS_DESCRIPTION => "transport error",
        // General
        semconv::SERVER_ADDRESS => output.server_uri.as_str(),
        // gRPC
        semconv::RPC_SYSTEM_NAME => "grpc",
        semconv::RPC_METHOD => "example.Unary/Echo",
        semconv::RPC_RESPONSE_STATUS_CODE => sn(Code::Internal),
    });

    actual_spans.assert_client_events(expected_events! {
        "rpc.client.call.exception" {
            semconv::EXCEPTION_TYPE => "tonic::transport::Error",
            semconv::EXCEPTION_MESSAGE => "transport error",
            semconv::EXCEPTION_STACKTRACE => r#"transport error

Caused by:
   0: http2 error
   1: stream error received: unexpected internal error encountered"#,
        },
    });

    // because it's a HTTP/2 server, it has no trace or metric layers

    let actual_metrics = output.metrics(&resource);
    // The server resets the stream as soon as it errors. Whether the request
    // body is counted as sent before that reset lands is a race in the client,
    // not in the test: the attempt records either the whole request or nothing.
    // Everything else about the attempt is the same either way.
    let candidates = [req_size, 0.0]
        .into_iter()
        .map(|tx_size| {
            expected_metrics!(
                build actual_metrics => "example.Unary/Echo",
                {
                    semconv::GRPC_CLIENT_ATTEMPT_STARTED => "{attempt}": sum { 1 },
                    semconv::GRPC_CLIENT_ATTEMPT_ACTIVE => "{attempt}": sum { 0 },
                    semconv::GRPC_CLIENT_REQUEST_ACTIVE => "{request}": sum { 0 },
                    semconv::GRPC_CLIENT_ATTEMPT_DURATION => "s": histogram { 1 x > 0.0 } => {
                        semconv::GRPC_STATUS => sn(Code::Internal),
                    },
                    semconv::GRPC_CLIENT_ATTEMPT_TX_SIZE => "By": histogram { 1 x tx_size },
                    // no response body is created. when the service/channel future is
                    // polled, it's unable to construct a well-formed `Response<Body>`
                    // due to the immediate error, and so these metrics aren't emitted:
                    // semconv::GRPC_CLIENT_ATTEMPT_RX_SIZE
                    // semconv::GRPC_CLIENT_RESPONSE_ACTIVE
                },
            )
        })
        .collect();
    actual_metrics.assert_one_of(candidates);
}
