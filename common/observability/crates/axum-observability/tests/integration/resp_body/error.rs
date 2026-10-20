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
use testing::body::ErrorBody;

const ERROR: &str = "task failed successfully";

#[instrument(name = "echo (server)", level = "info", skip_all)]
async fn handler() -> impl IntoResponse {
    axum::body::Body::new(ErrorBody::new(ERROR))
}

http_test!();

fn test<Http: HttpVersion>() {
    let (request, req_size) = Request::get("/").build();

    let resource = resource();
    let router = Router::new().route("/", get(handler));

    let output = harness(&resource, router, async |make| {
        let client = Http::client(&make);
        client.send(request).await
    });

    let actual_spans = output.spans(&resource);

    // sometimes, the response is send, and sometimes not - the important part
    // of this test are the span and metric attributes
    // actual_spans.assert(expected_hierarchy! {});

    actual_spans.hierarchy.assert_contains_children();

    actual_spans.assert_server_attrs(expected_attrs! {
        // metadata
        semconv::CODE_FILE_PATH => SERVER_TRACE_PATH,
        semconv::CODE_MODULE_NAME => SERVER_TRACE_MODULE,
        semconv::OTEL_STATUS_CODE => "Error",
        semconv::OTEL_STATUS_DESCRIPTION => ERROR,
        // General
        semconv::SERVER_ADDRESS => server_address(&output.server_uri),
        semconv::USER_AGENT_ORIGINAL => USER_AGENT,
        // HTTP
        semconv::HTTP_REQUEST_METHOD => "GET",
        semconv::HTTP_RESPONSE_STATUS_CODE => sc(OK),
        semconv::NETWORK_PROTOCOL_VERSION => ver(Http::VERSION),
        semconv::HTTP_ROUTE => "/",
        semconv::URL_PATH => "/",
        // Error
        semconv::ERROR_TYPE => "axum_core::error::Error",
    });

    actual_spans.assert_server_events(expected_events! {
        "http.server.request.exception" {
            semconv::EXCEPTION_TYPE => "axum_core::error::Error",
            semconv::EXCEPTION_MESSAGE => ERROR,
            semconv::EXCEPTION_STACKTRACE => format!(
                "{ERROR}\n\nCaused by:\n   0: {ERROR}",
            ),
        },
    });

    let actual_metrics = output.metrics(&resource);

    expected_metrics!(
        actual_metrics => Http, "GET" => "/",
        {
            semconv::HTTP_SERVER_REQUEST_STARTED => "{request}": sum { 1 },
            semconv::HTTP_SERVER_REQUEST_DURATION => "s": histogram { 1 x > 0.0 } => {
                semconv::HTTP_RESPONSE_STATUS_CODE => si(OK),
                semconv::ERROR_TYPE => "axum_core::error::Error",
            },
            semconv::HTTP_SERVER_REQUEST_BODY_SIZE => "By": histogram { 1 x req_size },
            semconv::HTTP_SERVER_RESPONSE_BODY_SIZE => "By": histogram { 1 x 0.0 } => {
                semconv::HTTP_RESPONSE_STATUS_CODE => si(OK),
                semconv::ERROR_TYPE => "axum_core::error::Error",
            },
        }
    );
}
