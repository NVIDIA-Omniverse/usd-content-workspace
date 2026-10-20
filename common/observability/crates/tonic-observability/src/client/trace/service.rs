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
use crate::client_verbose as verbose;
use crate::common::target_fn::TargetFn;
use otel_tools::context::propagate;
use std::task::{Context, Poll};
use tonic::codegen::http;
use tower_service::Service;

type ServiceError = tonic::transport::Error;

/// Tonic client trace service.
///
/// This is exported for client type signatures constructed with a
/// [`TonicTraceLayer`](super::layer::TonicTraceLayer). A service cannot be constructed directly.
#[derive(Debug, Clone)]
pub struct TonicTraceService<Svc> {
    pub(super) make_span: MakeSpan,
    pub(super) target_fn: TargetFn,
    pub(super) inner: Svc,
}

// there's a question as to whether the request body type needs to be generic,
// or should just be `tonic::body::Body`.
//
// `tonic::transport::Channel` only implements `tower_service::Service` for
// requests with a tonic body:
// https://github.com/hyperium/tonic/blob/v0.14.5/tonic/src/transport/channel/mod.rs#L208
//
// the gRPC client dispatcher `tonic::client::Grpc` only implements dispatch for
// requests with a tonic body:
// https://github.com/hyperium/tonic/blob/v0.14.5/tonic/src/client/grpc.rs#L214
//
// so the tower service passed to a tonic gRPC client must have a tonic body.
// for now, i've chosen to keep it generic as it potentially allows not boxing
// the request body. the trade-off is that if another layer causes an issue, the
// generic bounds can make it harder to debug the issue.
impl<Svc, ReqBody, RespBody> Service<http::Request<ReqBody>> for TonicTraceService<Svc>
where
    Svc: Service<http::Request<ReqBody>, Response = http::Response<RespBody>, Error = ServiceError>
        + Clone
        + Send
        + 'static,
    ReqBody: http_body::Body,
    RespBody: http_body::Body<Error = tonic::Status>,
{
    type Response = http::Response<TonicTraceResponseBody<RespBody>>;
    type Error = ServiceError;
    type Future = TonicTraceResponse<Svc::Future>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<ReqBody>) -> Self::Future {
        verbose!("trace service call");

        let (mut parts, body) = request.into_parts();
        // the target should be available at span creation for samplers
        let server_address = self.target_fn.call(&parts);
        // create a span for the RPC call
        let span = (self.make_span.0)(&parts, server_address.as_ref().map(|v| v.as_str()));
        // inject trace context
        propagate::inject(&span, &mut parts.headers);
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
