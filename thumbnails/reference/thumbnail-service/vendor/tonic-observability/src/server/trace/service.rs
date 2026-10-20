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

use super::call::TonicTraceResponse;
use super::response_body::TonicTraceResponseBody;
use super::span::MakeSpan;
use crate::rpc_map::RpcMap;
use crate::server_verbose as verbose;
use otel_tools::context::propagate;
use std::convert::Infallible;
use std::task::{Context, Poll};
use tonic::codegen::http;
use tower_service::Service;

/// Tonic server trace service.
///
/// This is exported for server type signatures constructed with a
/// [`TonicTraceLayer`](super::layer::TonicTraceLayer). A service cannot be constructed directly.
#[derive(Debug, Clone)]
pub struct TonicTraceService<Svc> {
    pub(super) make_span: MakeSpan,
    pub(super) rpc_map: RpcMap,
    pub(super) inner: Svc,
}

impl<Svc, ReqBody, RespBody> Service<http::Request<ReqBody>> for TonicTraceService<Svc>
where
    Svc: Service<http::Request<ReqBody>, Response = http::Response<RespBody>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    ReqBody: http_body::Body,
    RespBody: http_body::Body<Error = tonic::Status>,
{
    type Response = http::Response<TonicTraceResponseBody<RespBody>>;
    type Error = Infallible;
    type Future = TonicTraceResponse<Svc::Future>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<ReqBody>) -> Self::Future {
        verbose!("trace service call");

        let (parts, body) = request.into_parts();
        // create a span for the RPC call
        let span = (self.make_span.0)(&parts, &self.rpc_map);
        // extract trace context
        propagate::extract(&span, &parts.headers);
        let request = http::Request::from_parts(parts, body);

        // "create" the future (futures aren't executed until they are polled)
        let inner = {
            let _guard = span.enter();
            self.inner.call(request)
        };
        // wrap the future
        TonicTraceResponse { inner, span }
    }
}
