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

use super::req_err_layer::ReqErrLayer;
use super::req_reset::assert_server_after_reset;
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

/// This test case is for the client layers' request body `Err(Status)`
/// outcome, which is hard to trigger. So, a layer is used to insert an error
/// before the request is sent.
///
/// See [`super::request_body_err`].
#[test]
fn test() {
    let (request, _req_size) = make_req(echo_req("Echo"));

    let (response, _resp_size) = make_resp(Status::internal("h2 protocol error: http2 error"));

    let resource = resource();
    let svc = Server::new(Svc);

    let output = harness(&resource, svc, async |make| {
        let status = tonic::Status::ok("ok");
        let mut client: Client<_> = make
            // note: the error layer must be the first, so that it's the
            // outer-most layer and doesn't replace the trace/metric bodies.
            // i.e. the layer order must be ReqErrLayer -> Trace/Metric layers
            // -> Channel
            .layer(ReqErrLayer::new(status))
            .metric_layer()
            .trace_layer()
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
    assert_server_after_reset(&actual_spans, &output.metrics(&resource));

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
   1: stream error sent by user: unexpected internal error encountered"#,
        },
    });
}
