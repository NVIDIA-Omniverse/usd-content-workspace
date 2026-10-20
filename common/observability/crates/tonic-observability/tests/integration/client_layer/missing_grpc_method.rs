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

use super::strip_grpc_method_layer::StripGrpcMethodLayer;
use crate::harness::prelude::*;
use protos::unary_client::UnaryClient as Client;
use protos::unary_server::{Unary as Service, UnaryServer as Server};

struct Svc;

#[async_trait]
impl Service for Svc {
    #[instrument(name = "echo (server)", level = "info", skip_all)]
    async fn echo(&self, request: Request<EchoRequest>) -> Result<Response<EchoResponse>> {
        let message = request.into_inner().message;
        Ok(Response::new(EchoResponse { message }))
    }
}

/// When the `GrpcMethod` tonic extension is absent, `rpc.method` should be
/// `_OTHER` and `rpc.method_original` should contain the URI path.
#[test]
fn test() {
    let (request, _req_size) = make_req(echo_req("Echo"));

    let (response, _resp_size) = make_resp(echo_resp("Echo"));

    let resource = resource();
    let svc = Server::new(Svc);

    let output = harness(&resource, svc, async |make| {
        let mut client: Client<_> = make.layer(StripGrpcMethodLayer).trace_layer().client();
        client
            .echo(request)
            .instrument(info_span!("echo (client)"))
            .await
            .consume()
            .await
    });

    assert_eq!(output.response(), &response);

    let actual_spans = output.spans(&resource);

    // the span name should be `grpc` (i.e. `{rpc.system.name}`) when
    // `rpc.method` is `_OTHER`.
    actual_spans.assert(expected_hierarchy! {
        "test" => {
            "echo (client)" => {
                "grpc (client)" => {
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
        semconv::RPC_METHOD => "_OTHER",
        semconv::RPC_METHOD_ORIGINAL => "example.Unary/Echo",
        semconv::RPC_RESPONSE_STATUS_CODE => sn(Code::Ok),
    });
}
