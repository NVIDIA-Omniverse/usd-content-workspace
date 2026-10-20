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

async fn handler(_request: HttpRequest) -> HttpResult<BoxError> {
    unreachable!("won't be called due to HTTP/1.1");
}

/// Test the client layers against a HTTP/1.1 server.
///
/// This covers `client::trace::span::record_transport_err`.
#[test]
fn test() {
    let (request, _req_size) = make_req(echo_req("Echo"));

    // response is variable

    let resource = resource();
    let svc = Server::http1(handler);

    let output = harness(&resource, svc, async move |make| {
        let mut client: Client<_> = make.default();

        client
            .echo(request)
            .instrument(info_span!("echo (client)"))
            .await
            .consume()
            .await
    });

    // the error is variable
    let response = output.response();

    let code = match response {
        Err(status) if status.code() == Code::Cancelled => Code::Cancelled,
        Err(status) if status.code() == Code::Unknown => Code::Unknown,
        _ => panic!("unexpected response: {response:?}"),
    };

    let actual_spans = output.spans(&resource);

    actual_spans.assert(expected_hierarchy! {
        "test" => {
            "echo (client)" => {
                "example.Unary/Echo (client)"
            }
        }
    });
    actual_spans.hierarchy.assert_contains_children();
    actual_spans.assert_no_server_span();

    actual_spans.assert_client_attrs(expected_attrs! {
        // metadata
        semconv::CODE_FILE_PATH => CLIENT_TRACE_PATH,
        semconv::CODE_MODULE_NAME => CLIENT_TRACE_MODULE,
        semconv::ERROR_TYPE => sn(code),
        semconv::OTEL_STATUS_CODE => "Error",
        semconv::OTEL_STATUS_DESCRIPTION => "transport error",
        // General
        semconv::SERVER_ADDRESS => output.server_uri.as_str(),
        // gRPC
        semconv::RPC_SYSTEM_NAME => "grpc",
        semconv::RPC_METHOD => "example.Unary/Echo",
        semconv::RPC_RESPONSE_STATUS_CODE => sn(code),
    });

    // the exception event contents are variable due to the HTTP/1.1 error
    let mut client_events = actual_spans.client.events.clone();
    for event in &mut client_events {
        for attr in &mut event.attributes {
            match attr.key() {
                semconv::EXCEPTION_MESSAGE => attr.value = Some("<erased>".into()),
                semconv::EXCEPTION_STACKTRACE => attr.value = Some("<erased>".into()),
                _ => {}
            }
        }
    }
    testing::output::OutputSpans::assert_events(
        &client_events,
        expected_events! {
            "rpc.client.call.exception" {
                semconv::EXCEPTION_TYPE => "tonic::transport::Error",
                semconv::EXCEPTION_MESSAGE => "<erased>",
                semconv::EXCEPTION_STACKTRACE => "<erased>",
            },
        },
        "client",
    );

    // because it's a HTTP/1.1 server, it has no trace or metric layers

    let actual_metrics = output.metrics(&resource);

    // the transmitted data is variable based on the hyper error
    let tx_size = get_histogram_sum(&actual_metrics, semconv::GRPC_CLIENT_ATTEMPT_TX_SIZE);

    expected_metrics!(
        actual_metrics => "example.Unary/Echo",
        {},
        {
            semconv::GRPC_CLIENT_ATTEMPT_STARTED => "{attempt}": sum { 1 },
            semconv::GRPC_CLIENT_ATTEMPT_ACTIVE => "{attempt}": sum { 0 },
            semconv::GRPC_CLIENT_REQUEST_ACTIVE => "{request}": sum { 0 },
            semconv::GRPC_CLIENT_ATTEMPT_DURATION => "s": histogram { 1 x > 0.0 } => {
                semconv::GRPC_STATUS => sn(code),
            },
            semconv::GRPC_CLIENT_ATTEMPT_TX_SIZE => "By": histogram { 1 x tx_size },
            // because no connection can be established, no response body is
            // created, so these metrics aren't emitted:
            // semconv::GRPC_CLIENT_ATTEMPT_RX_SIZE
            // semconv::GRPC_CLIENT_RESPONSE_ACTIVE
        },
    );
}
