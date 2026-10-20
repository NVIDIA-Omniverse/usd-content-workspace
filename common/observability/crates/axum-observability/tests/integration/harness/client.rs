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

use super::request::Request;
use super::response::{Consumed, Response};
use std::pin::{Pin, pin};
use tracing::{Instrument, info_span, instrument};

pub(crate) const USER_AGENT: &str = "reqwest-test";

/// This should not exist. However, the reqwest and url crates are really
/// awkward to use.
#[derive(Debug, Clone)]
pub(crate) struct MakeClient {
    url: reqwest::Url,
}

impl MakeClient {
    pub(super) fn new(url: reqwest::Url) -> Self {
        Self { url }
    }

    fn builder(&self) -> reqwest::ClientBuilder {
        reqwest::Client::builder().user_agent(USER_AGENT)
    }

    pub(crate) fn http1(&self) -> Client {
        let client = self.builder().http1_only().build().unwrap();
        let url = self.url.clone();
        Client { client, url }
    }

    pub(crate) fn http2(&self) -> Client {
        let client = self.builder().http2_prior_knowledge().build().unwrap();
        let url = self.url.clone();
        Client { client, url }
    }
}

#[derive(Debug)]
pub(crate) struct Client {
    client: reqwest::Client,
    url: reqwest::Url,
}

impl Client {
    pub(crate) async fn send(&self, request: Request) -> Result<Response, reqwest::Error> {
        self.send_with(request, std::convert::identity).await
    }

    pub(crate) async fn send_with<F>(
        &self,
        request: Request,
        f: F,
    ) -> Result<Response, reqwest::Error>
    where
        F: FnOnce(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    {
        let Request {
            method,
            path,
            fragment,
            mut headers,
            body,
        } = request;

        let mut url = self.url.clone();
        url.set_path(&path);
        url.set_fragment(fragment.as_deref());

        let mut builder = self.client.request(method, url);

        if let Some(body) = body {
            builder = builder.body(body);
        }

        let span = info_span!("echo", otel.kind = "Client");
        otel_tools::context::propagate::inject(&span, &mut headers);

        builder = builder.headers(headers);
        builder = f(builder);

        let response = builder.send().instrument(span).await?;
        Ok(consume(response).await)
    }
}

pub(crate) async fn poll_body<B: http_body::Body>(
    mut body: Pin<&mut B>,
) -> Option<Result<http_body::Frame<B::Data>, B::Error>> {
    std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await
}

#[instrument(name = "response (client)", level = "info", skip_all)]
async fn consume(response: reqwest::Response) -> Response {
    let resp: http::Response<reqwest::Body> = response.into();
    let (parts, body) = resp.into_parts();
    let http::response::Parts {
        status,
        version,
        mut headers,
        ..
    } = parts;

    // remove DATE header for deterministic test assertions
    headers.remove(http::header::DATE);

    let mut body = pin!(body);

    let mut consumed = Vec::new();
    while let Some(res) = poll_body(body.as_mut()).await {
        match res {
            Err(e) => {
                // there isn't much we can do with the error w.r.t.
                // asserting on it, as it's opaque.
                let e = e.without_url();
                let err = format!("{e}");
                consumed.push(Consumed::Error(err));
                break;
            }
            Ok(frame) => match frame.into_data() {
                Ok(data) => {
                    if data.is_empty() {
                        // ignore end of stream for HTTP2
                        continue;
                    }
                    match str::from_utf8(&data) {
                        Ok(s) => consumed.push(s.into()),
                        Err(_e) => consumed.push(Consumed::Bytes(data.to_vec())),
                    }
                }
                Err(frame) => match frame.into_trailers() {
                    Ok(trailers) => {
                        consumed.push(trailers.into());
                    }
                    Err(frame) => panic!("unknown frame type {frame:?}"),
                },
            },
        }
    }

    Response {
        status,
        version,
        headers,
        body: consumed,
    }
}
