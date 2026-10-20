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

use super::handler::TowerTraceResponse;
use super::span::MakeSpan;
use crate::server_verbose as verbose;
use bytes::Bytes;
use otel_tools::context::propagate;
use std::convert::Infallible;
use std::task::{Context, Poll};
use tower_service::Service;

/// Tower server trace service.
///
/// This is exported for server type signatures constructed with a
/// [`TowerTraceLayer`](super::layer::TowerTraceLayer). A service cannot be
/// constructed directly.
#[derive(Debug, Clone)]
pub struct TowerTraceService<Svc> {
    pub(super) make_span: MakeSpan,
    pub(super) inner: Svc,
}

impl<Svc, ReqBody, RespBody> Service<http::Request<ReqBody>> for TowerTraceService<Svc>
where
    Svc: Service<http::Request<ReqBody>, Response = http::Response<RespBody>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    ReqBody: http_body::Body<Data = Bytes>,
    ReqBody::Error: std::error::Error + 'static,
    RespBody: http_body::Body<Data = Bytes>,
    RespBody::Error: std::error::Error + 'static,
{
    type Response = http::Response<super::response_body::TowerTraceResponseBody<RespBody>>;
    type Error = Svc::Error;
    type Future = TowerTraceResponse<Svc::Future>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<ReqBody>) -> Self::Future {
        verbose!("trace service call");

        let (parts, body) = request.into_parts();
        // create a span for the HTTP call
        let span = (self.make_span.0)(&parts);
        // extract trace context
        propagate::extract(&span, &parts.headers);
        let request = http::Request::from_parts(parts, body);

        // "create" the future (futures aren't executed until they are polled)
        let inner = {
            let _guard = span.enter();
            self.inner.call(request)
        };
        // wrap the future
        TowerTraceResponse { inner, span }
    }
}
