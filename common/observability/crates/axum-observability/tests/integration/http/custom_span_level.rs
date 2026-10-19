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
use crate::harness::{MakeService, harness_custom};
use axum_observability::server::trace::{TowerTraceLayer, make_span as server_span};
use testing::guards::Guards;
use tracing::Level;

const MESSAGE: &str = "response";

#[instrument(name = "echo (server)", level = "info", skip_all)]
async fn handler() -> impl IntoResponse {
    MESSAGE
}

struct Service;

impl MakeService for Service {
    fn make(self, _guards: &testing::guards::Guards) -> Router {
        let router = Router::new().route("/foo", get(handler));

        let trace_layer = TowerTraceLayer::builder()
            .with_span(server_span!(Level::TRACE, "my_http_server"))
            .build();

        router.layer(tower::ServiceBuilder::new().layer(trace_layer))
    }
}

http_test!();

fn test<Http: HttpVersion>() {
    let (request, _req_size) = Request::get("/foo").build();
    let (response, _resp_size) = Response::builder(OK, Http::VERSION)
        .header(header::CONTENT_TYPE, MIME_TEXT_PLAIN)
        .body(body![MESSAGE])
        .build();

    let resource = resource();
    let make_guards = || Guards::custom(&resource, "info,my_http_server=trace", true);

    let output = harness_custom(make_guards, Service, async |make| {
        let client = Http::client(&make);
        client.send(request).await.unwrap()
    });

    assert_resp!(output.response(), response);

    let actual_spans = output.spans(&resource);

    actual_spans.assert(expected_hierarchy! {
        "test" => {
            "echo (client)" => {
                "GET /foo (server)" => {
                    "echo (server)"
                }
            },
            "response (client)",
        }
    });
    actual_spans.hierarchy.assert_contains_children();

    actual_spans.assert_server_attrs(expected_attrs! {
        // metadata
        semconv::CODE_FILE_PATH => "crates/axum-observability/tests/integration/http/custom_span_level.rs",
        semconv::CODE_MODULE_NAME => "integration::http::custom_span_level",
        semconv::OTEL_STATUS_CODE => "Unset",
        semconv::TARGET => "my_http_server",
        // General
        semconv::SERVER_ADDRESS => server_address(&output.server_uri),
        semconv::USER_AGENT_ORIGINAL => USER_AGENT,
        // HTTP
        semconv::HTTP_REQUEST_METHOD => "GET",
        semconv::HTTP_RESPONSE_STATUS_CODE => "200",
        semconv::NETWORK_PROTOCOL_VERSION => ver(Http::VERSION),
        semconv::HTTP_ROUTE => "/foo",
        semconv::URL_PATH => "/foo",
    });
}
