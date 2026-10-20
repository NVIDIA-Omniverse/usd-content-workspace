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

const MESSAGE: &str = "response";

#[instrument(name = "echo (server)", level = "info", skip_all)]
async fn handler() -> impl IntoResponse {
    MESSAGE
}

http_test!();

fn test<Http: HttpVersion>() {
    let method = http::Method::from_bytes(b"FOO").unwrap();
    let (request, req_size) = Request::builder(method, "/foo").build();
    let (response, resp_size) = Response::builder(OK, Http::VERSION)
        .header(header::CONTENT_TYPE, MIME_TEXT_PLAIN)
        .body(body![MESSAGE])
        .build();

    let resource = resource();
    let router = Router::new().route("/foo", axum::routing::any(handler));

    let output = harness(&resource, router, async |make| {
        let client = Http::client(&make);
        client.send(request).await.unwrap()
    });

    assert_resp!(output.response(), response);

    let actual_spans = output.spans(&resource);

    actual_spans.assert(expected_hierarchy! {
        "test" => {
            "echo (client)" => {
                "HTTP /foo (server)" => {
                    "echo (server)"
                }
            },
            "response (client)",
        }
    });
    actual_spans.hierarchy.assert_contains_children();

    actual_spans.assert_server_attrs(expected_attrs! {
        // metadata
        semconv::CODE_FILE_PATH => SERVER_TRACE_PATH,
        semconv::CODE_MODULE_NAME => SERVER_TRACE_MODULE,
        semconv::OTEL_STATUS_CODE => "Unset",
        // General
        semconv::SERVER_ADDRESS => server_address(&output.server_uri),
        semconv::USER_AGENT_ORIGINAL => USER_AGENT,
        // HTTP
        semconv::HTTP_REQUEST_METHOD => "_OTHER",
        semconv::HTTP_REQUEST_METHOD_ORIGINAL => "FOO",
        semconv::HTTP_RESPONSE_STATUS_CODE => "200",
        semconv::NETWORK_PROTOCOL_VERSION => ver(Http::VERSION),
        semconv::HTTP_ROUTE => "/foo",
        semconv::URL_PATH => "/foo",
    });

    let actual_metrics = output.metrics(&resource);

    expected_metrics!(
        actual_metrics => Http, "_OTHER" => "/foo",
        {
            semconv::HTTP_SERVER_REQUEST_STARTED => "{request}": sum { 1 },
            semconv::HTTP_SERVER_REQUEST_DURATION => "s": histogram { 1 x > 0.0 } => {
                semconv::HTTP_RESPONSE_STATUS_CODE => si(OK),
            },
            semconv::HTTP_SERVER_REQUEST_BODY_SIZE => "By": histogram { 1 x req_size },
            semconv::HTTP_SERVER_RESPONSE_BODY_SIZE => "By": histogram { 1 x resp_size } => {
                semconv::HTTP_RESPONSE_STATUS_CODE => si(OK),
            },
        }
    );
}
