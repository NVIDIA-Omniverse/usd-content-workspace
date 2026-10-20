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

use super::attributes::{Attributes, attrs};
use super::call::TonicMetricAttempt;
use super::metrics::{CallMetrics, RequestMetrics, ResponseMetrics};
use super::request_body::TonicMetricRequestBody;
use crate::client_verbose as verbose;
use crate::common::attributes::TARGET_OTHER;
use crate::common::target_fn::TargetFn;
use crate::rpc_map::RpcMap;
use opentelemetry::StringValue;
use std::task::{Context, Poll};
use std::time::Instant;
use tonic::codegen::http;
use tower_service::Service;

/// Client metric service, returned by the metric layer.
///
/// This is a wrapper around the original service.
///
/// This is exported for service type signatures and cannot be constructed
/// directly.
#[derive(Debug, Clone)]
pub struct TonicMetricService<Svc> {
    pub(super) inner: Svc,
    pub(super) rpc_map: RpcMap,
    pub(super) target_fn: TargetFn,
    pub(super) call_metrics: CallMetrics,
    pub(super) req_metrics: RequestMetrics,
    pub(super) resp_metrics: ResponseMetrics,
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
//
// however, for compatibility, this layer always boxes the instrumented request
// body in a tonic body. otherwise, another layer would be required to box the
// metric layer's request body.
impl<Svc, ReqBody, RespBody> Service<http::Request<ReqBody>> for TonicMetricService<Svc>
where
    Svc: Service<
            http::Request<tonic::body::Body>,
            Response = http::Response<RespBody>,
            Error = tonic::transport::Error,
        > + Clone
        + Send
        + 'static,
    ReqBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status> + Send + 'static,
    RespBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status> + Send + 'static,
{
    type Response = http::Response<super::response_body::TonicMetricResponseBody<RespBody>>;
    type Error = tonic::transport::Error;
    type Future = TonicMetricAttempt<Svc::Future>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<ReqBody>) -> Self::Future {
        verbose!("metrics service call");

        // for the `grpc.client.attempt.duration` metric: this is the earliest
        // we know of a request/new attempt.
        let call_start = Instant::now();

        let (parts, body) = request.into_parts();
        let target = self
            .target_fn
            .call(&parts)
            .unwrap_or_else(|| StringValue::from(TARGET_OTHER));
        let attrs = Attributes::new(&self.rpc_map, &parts, target);
        let request = http::Request::from_parts(parts, body);

        let attributes = attrs!(attrs);
        self.call_metrics.started.add(1, attributes);

        // wrap the request stream
        let request = request.map(|inner| {
            let body = TonicMetricRequestBody {
                inner,
                attrs: attrs.clone(),
                metrics: self.req_metrics.clone(),
                size: 0,
            };
            // see the note at the top of the implementation block on why this
            // is required.
            tonic::body::Body::new(body)
        });
        self.req_metrics.active_requests.add(1, attributes);

        // "call" the inner service/create the attempt future (since futures
        // aren't executed until they are polled)
        let inner = self.inner.call(request);

        // wrap the call attempt
        let attempt = TonicMetricAttempt {
            inner,
            attrs,
            metrics: self.resp_metrics.clone(),
            call_start,
        };
        self.resp_metrics.active_attempts.add(1, attributes);
        attempt
    }
}
