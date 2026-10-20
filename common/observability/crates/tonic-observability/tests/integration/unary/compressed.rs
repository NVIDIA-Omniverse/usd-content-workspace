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
use protos::unary_client::UnaryClient as Client;
use protos::unary_server::{Unary as Service, UnaryServer as Server};
use tonic::codec::CompressionEncoding;

struct Svc;

#[async_trait]
impl Service for Svc {
    #[instrument(name = "echo (server)", level = "info", skip_all)]
    async fn echo(&self, request: Request<EchoRequest>) -> Result<Response<EchoResponse>> {
        let message = request.into_inner().message;
        Ok(Response::new(EchoResponse { message }))
    }
}

#[test]
fn test() {
    // use a large, highly compressible payload
    let payload = "A".repeat(1024);

    let (request, req_size) = make_req(echo_req(&payload));
    let (response, resp_size) = make_resp(echo_resp(&payload));

    let resource = resource();
    let svc = Server::new(Svc)
        .send_compressed(CompressionEncoding::Gzip)
        .accept_compressed(CompressionEncoding::Gzip);

    let output = harness(&resource, svc, async |make| {
        let client: Client<_> = make.default();
        let mut client = client
            .send_compressed(CompressionEncoding::Gzip)
            .accept_compressed(CompressionEncoding::Gzip);
        client
            .echo(request)
            .instrument(info_span!("echo (client)"))
            .await
            .consume()
            .await
    });

    assert_eq!(output.response(), &response);

    let actual_metrics = output.metrics(&resource);

    // extract actual compressed sizes from the metrics
    let a66_server_rx = get_histogram_sum(&actual_metrics, semconv::GRPC_SERVER_CALL_RX_SIZE);
    let a66_server_tx = get_histogram_sum(&actual_metrics, semconv::GRPC_SERVER_CALL_TX_SIZE);
    let a66_client_tx = get_histogram_sum(&actual_metrics, semconv::GRPC_CLIENT_ATTEMPT_TX_SIZE);
    let a66_client_rx = get_histogram_sum(&actual_metrics, semconv::GRPC_CLIENT_ATTEMPT_RX_SIZE);

    expected_metrics!(
        actual_metrics => "example.Unary/Echo",
        {
            semconv::GRPC_SERVER_CALL_DURATION => "s": histogram { 1 x > 0.0 } => {
                semconv::GRPC_STATUS => sn(Code::Ok),
            },
            semconv::GRPC_SERVER_CALL_RX_SIZE => "By": histogram { 1 x a66_server_rx },
            semconv::GRPC_SERVER_CALL_TX_SIZE => "By": histogram { 1 x a66_server_tx } => {
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
            semconv::GRPC_CLIENT_ATTEMPT_TX_SIZE => "By": histogram { 1 x a66_client_tx },
            semconv::GRPC_CLIENT_ATTEMPT_RX_SIZE => "By": histogram { 1 x a66_client_rx } => {
                semconv::GRPC_STATUS => sn(Code::Ok),
            },
        },
    );

    // with gzip on a highly compressible payload, compressed sizes must be
    // smaller than uncompressed sizes
    assert!(
        a66_client_tx < req_size,
        "client tx {a66_client_tx} < uncompressed {req_size}"
    );
    assert!(
        a66_client_rx < resp_size,
        "client rx {a66_client_rx} < uncompressed {resp_size}"
    );
    assert!(
        a66_server_rx < req_size,
        "server rx {a66_server_rx} < uncompressed {req_size}"
    );
    assert!(
        a66_server_tx < resp_size,
        "server tx {a66_server_tx} < uncompressed {resp_size}"
    );
}
