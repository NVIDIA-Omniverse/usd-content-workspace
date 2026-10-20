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
use bytes::{BufMut as _, Bytes, BytesMut};
use prost::Message;
use protos::unary_client::UnaryClient as Client;
use std::convert::Infallible;
use testing::body::EndOfStreamBody;
use tonic::codegen::http;

#[instrument(name = "echo (server)", level = "info", skip_all)]
async fn handler(request: HttpRequest) -> HttpResult<Infallible> {
    consume_request(request.into_body()).await;

    let resp = echo_resp("Echo");
    let data = encode_grpc(resp);

    let response = hyper::Response::builder()
        .status(200)
        .header(Status::GRPC_STATUS, http::HeaderName::from_static("0"))
        .body(tonic::body::Body::new(EndOfStreamBody::new(data)))
        .unwrap();
    Ok(response)
}

fn encode_grpc<T: Message>(v: T) -> Bytes {
    // first, encode the payload, since we must know the length
    let mut buf = BytesMut::with_capacity(128);
    v.encode(&mut buf).unwrap();

    // next, construct the gRPC framing (compression + length)
    let len = buf.len() as u32;
    let mut b = BytesMut::with_capacity(128);
    b.put_u8(0); // compression
    b.put_u32(len); // length
    b.put(buf); // encoded

    b.freeze()
}

/// Test the layers against a HTTP/2 server that returns a body which streams
/// some data, and then returns end-of-stream (EOS).
///
/// Interestingly, this works, although it's forbidden per gRPC without
/// trailers. <https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md>
#[test]
fn test() {
    let (request, req_size) = make_req(echo_req("Echo"));

    let (response, resp_size) = make_resp(echo_resp("Echo"));

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
        semconv::RPC_METHOD => "example.Unary/Echo",
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
