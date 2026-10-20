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
use std::time::Duration;
use testing::stream::Slow;

const MESSAGE: &str = "Hello, world!";
const ERROR: &str = "task failed successfully";

#[instrument(name = "echo (server)", level = "info", skip_all)]
async fn handler(req: axum::extract::Request) -> impl IntoResponse {
    let body = req.into_body();
    let mut body = std::pin::pin!(body);

    while let Some(res) = poll_body(body.as_mut()).await {
        println!("handler: {res:?}");
    }

    OK
}

#[ignore = "doesn't work"]
#[test]
fn http2() {
    // the HTTP2 test doesn't work, as the stream is buffered. in theory, the
    // underlying hyper client's buffer size can be configured; however reqwest
    // doesn't expose this on the builder. it also isn't possible to set a
    // custom hyper client for reqwest.
    // https://docs.rs/hyper/1.8.1/hyper/client/conn/http2/struct.Builder.html#method.max_send_buf_size
    // https://github.com/seanmonstar/reqwest/issues/1535 "Support for custom hyper client"
}

#[test]
fn http11() {
    test::<Http11>();
}

fn test<Http: HttpVersion>() {
    // kind of yank also, but we must delay each request body so that the client
    // cannot consume the stream too quickly and get to the error too quickly.
    // that causes the request to be aborted, and in some cases the handler is
    // never even run. also, this only works for HTTP/1.1 for some reason?
    let stream = Slow::try_stream(
        Duration::from_millis(20),
        vec![
            Ok(MESSAGE),
            Ok(MESSAGE),
            Ok(MESSAGE),
            Ok(MESSAGE),
            Err(ERROR),
        ],
    );
    let (request, req_size) = Request::post("/").stream(stream).build();

    let resource = resource();
    let router = Router::new().route("/", post(handler));

    let output = harness(&resource, router, async |make| {
        let client = Http::client(&make);
        client.send(request).await.unwrap_err()
    });

    let actual_spans = output.spans(&resource);

    actual_spans.assert(expected_hierarchy! {
        "test" => {
            "echo (client)" => {
                "POST / (server)" => {
                    "echo (server)"
                }
            },
        }
    });

    actual_spans.assert_server_attrs(expected_attrs! {
        // metadata
        semconv::CODE_FILE_PATH => SERVER_TRACE_PATH,
        semconv::CODE_MODULE_NAME => SERVER_TRACE_MODULE,
        semconv::OTEL_STATUS_CODE => "Unset",
        // General
        semconv::SERVER_ADDRESS => server_address(&output.server_uri),
        semconv::USER_AGENT_ORIGINAL => USER_AGENT,
        // HTTP
        semconv::HTTP_REQUEST_METHOD => "POST",
        semconv::HTTP_RESPONSE_STATUS_CODE => sc(OK),
        semconv::NETWORK_PROTOCOL_VERSION => ver(Http::VERSION),
        semconv::HTTP_ROUTE => "/",
        semconv::URL_PATH => "/",
    });

    let actual_metrics = output.metrics(&resource);

    expected_metrics!(
        actual_metrics => Http, "POST" => "/",
        {
            semconv::HTTP_SERVER_REQUEST_STARTED => "{request}": sum { 1 },
            semconv::HTTP_SERVER_REQUEST_DURATION => "s": histogram { 1 x > 0.0 } => {
                semconv::HTTP_RESPONSE_STATUS_CODE => si(OK),
            },
            semconv::HTTP_SERVER_REQUEST_BODY_SIZE => "By": histogram { 1 x req_size },
            semconv::HTTP_SERVER_RESPONSE_BODY_SIZE => "By": histogram { 1 x 0.0 } => {
                semconv::HTTP_RESPONSE_STATUS_CODE => si(OK),
            },
        }
    );
}
