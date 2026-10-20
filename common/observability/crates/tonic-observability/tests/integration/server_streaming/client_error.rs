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
use protos::server_streaming_client::ServerStreamingClient as Client;
use protos::server_streaming_server::{
    ServerStreaming as Service, ServerStreamingServer as Server,
};

struct Svc;

#[async_trait]
impl Service for Svc {
    type EchoStream = stream::Many<Result<EchoResponse>>;

    #[instrument(name = "echo (server)", level = "info", skip_all)]
    async fn echo(&self, request: Request<EchoRequest>) -> Result<Response<Self::EchoStream>> {
        let message = request.into_inner().message;
        let stream = (0..3)
            .map(|i| Ok(echo_resp(format!("{message} {i}"))))
            .chain(Some(Err(Status::invalid_argument("Cya!"))))
            .collect();
        Ok(Response::new(stream))
    }
}

#[test]
fn test() {
    let (request, req_size) = make_req(echo_req("Echo"));

    let message = vec![
        Ok(echo_resp("Echo 0")),
        Ok(echo_resp("Echo 1")),
        Ok(echo_resp("Echo 2")),
        Err(Status::invalid_argument("Cya!")),
    ];
    let (response, resp_size) = make_resp(message);

    let resource = resource();
    let svc = Server::new(Svc);

    let output = harness(&resource, svc, async |make| {
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
                "example.ServerStreaming/Echo (client)" => {
                    "example.ServerStreaming/Echo (server)" => {
                        "echo (server)"
                    }
                }
            },
            "streaming (client)"
        },
    });
    actual_spans.hierarchy.assert_server_streaming();

    actual_spans.assert_client_attrs(expected_attrs! {
        // metadata
        semconv::CODE_FILE_PATH => CLIENT_TRACE_PATH,
        semconv::CODE_MODULE_NAME => CLIENT_TRACE_MODULE,
        semconv::ERROR_TYPE => sn(Code::InvalidArgument),
        semconv::OTEL_STATUS_CODE => "Error",
        semconv::OTEL_STATUS_DESCRIPTION => "Cya!",
        // General
        semconv::SERVER_ADDRESS => output.server_uri.as_str(),
        // gRPC
        semconv::RPC_SYSTEM_NAME => "grpc",
        semconv::RPC_METHOD => "example.ServerStreaming/Echo",
        semconv::RPC_RESPONSE_STATUS_CODE => sn(Code::InvalidArgument),
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
        semconv::RPC_METHOD => "example.ServerStreaming/Echo",
        semconv::RPC_RESPONSE_STATUS_CODE => sn(Code::InvalidArgument),
    });

    let actual_metrics = output.metrics(&resource);

    expected_metrics!(
        actual_metrics => "example.ServerStreaming/Echo",
        {
            semconv::GRPC_SERVER_CALL_DURATION => "s": histogram { 1 x > 0.0 } => {
                semconv::GRPC_STATUS => sn(Code::InvalidArgument),
            },
            semconv::GRPC_SERVER_CALL_RX_SIZE => "By": histogram { 1 x req_size },
            semconv::GRPC_SERVER_CALL_TX_SIZE => "By": histogram { 1 x resp_size - 4.0 } => {
                semconv::GRPC_STATUS => sn(Code::InvalidArgument),
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
                semconv::GRPC_STATUS => sn(Code::InvalidArgument),
            },
            semconv::GRPC_CLIENT_ATTEMPT_TX_SIZE => "By": histogram { 1 x req_size },
            semconv::GRPC_CLIENT_ATTEMPT_RX_SIZE => "By": histogram { 1 x resp_size - 4.0 } => {
                semconv::GRPC_STATUS => sn(Code::InvalidArgument),
            },
        },
    );
}
