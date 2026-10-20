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
use crate::harness::server::{HttpRequest, HttpResult, HttpServer as Server};
use protos::unary_client::UnaryClient as Client;
use std::convert::Infallible;
use testing::body::ErrorBody;
use testing::error::TestError;
use tonic::codegen::http;

#[instrument(name = "echo (server)", level = "info", skip_all)]
async fn handler(request: HttpRequest) -> HttpResult<Infallible> {
    consume_request(request.into_body()).await;

    let status = tonic::Status::from_error(TestError::boxed("foo"));
    let response = hyper::Response::builder()
        .status(200)
        .header(Status::GRPC_STATUS, http::HeaderValue::from_static("0"))
        .body(tonic::body::Body::new(ErrorBody::new(status)))
        .unwrap();
    Ok(response)
}

/// Test the layers against a HTTP/2 server that returns a body with an error.
/// While this is fun/interesting, it's not something tonic does as it causes
/// an hyper/h2 error.
///
/// This test is for coverage in `server::trace::span::record_tonic_status`,
/// specifically that the server records an exception event.
#[test]
fn test() {
    let (request, _req_size) = make_req(echo_req("Echo"));

    // response is variable (h2 error message is non-deterministic)

    let resource = resource();
    let svc = Server::tonic(handler);

    let output = harness(&resource, svc, async move |make| {
        let mut client: Client<_> = make.default();

        client
            .echo(request)
            .instrument(info_span!("echo (client)"))
            .await
            .consume()
            .await
    });

    let response = output.response();
    match response {
        Err(status)
            if status.code() == Code::Internal
                && status.message().starts_with("h2 protocol error:") => {}
        _ => panic!("unexpected response: {response:?}"),
    };

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

    // client span asserted below

    actual_spans.assert_server_attrs(expected_attrs! {
        // metadata
        semconv::CODE_FILE_PATH => SERVER_TRACE_PATH,
        semconv::CODE_MODULE_NAME => SERVER_TRACE_MODULE,
        semconv::OTEL_STATUS_CODE => "Error",
        semconv::OTEL_STATUS_DESCRIPTION => "foo",
        semconv::ERROR_TYPE => sn(Code::Unknown),
        // HTTP
        semconv::SERVER_ADDRESS => "127.0.0.1",
        // gRPC
        semconv::RPC_SYSTEM_NAME => "grpc",
        semconv::RPC_METHOD => "example.Unary/Echo",
        semconv::RPC_RESPONSE_STATUS_CODE => sn(Code::Unknown),
    });

    actual_spans.assert_server_events(expected_events! {
        "rpc.server.call.exception" {
            semconv::EXCEPTION_TYPE => "tonic::Status",
            semconv::EXCEPTION_MESSAGE => "foo",
            semconv::EXCEPTION_STACKTRACE => r#"status: 'Unknown error', self: "foo"

Caused by:
   0: foo"#,
        },
    });

    // the client span attributes are quite variable, specifically
    // semconv::OTEL_STATUS_DESCRIPTION and semconv::RPC_RESPONSE_STATUS_CODE.
    // it's enough that we assert the client span events, and that's difficult
    // enough.
    match &actual_spans.client.events[..] {
        [one] => {
            // we expect at least one event, but the formatting is very
            // variable. most reliable for me has been to look for stream
            // resets.
            let stacktrace = one
                .attributes
                .iter()
                .find_map(|kv| {
                    if kv.key() == semconv::EXCEPTION_STACKTRACE {
                        kv.value.as_ref().and_then(|v| v.as_str())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            assert!(
                stacktrace.contains("http2 error") || stacktrace.contains("h2 protocol error:"),
                "client event contains h2 error: {stacktrace}\n{one:#?}"
            );
        }
        other => panic!("unexpected client events: {other:#?}"),
    }

    // don't assert any metrics; this test is to cover the server trace event
}
