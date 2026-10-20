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
use super::call::TonicMetricHandler;
use super::metrics::{CallMetrics, RequestMetrics, ResponseMetrics};
use super::request_body::TonicMetricRequestBody;
use crate::rpc_map::RpcMap;
use crate::server_verbose as verbose;
use std::convert::Infallible;
use std::task::{Context, Poll};
use std::time::Instant;
use tonic::body::Body;
use tonic::codegen::http;
use tower_service::Service;

/// Server metric service, returned by the metric layer.
///
/// This is a wrapper around the original service.
///
/// This is exported for service type signatures and cannot be constructed
/// directly.
#[derive(Debug, Clone)]
pub struct TonicMetricService<Svc> {
    pub(super) inner: Svc,
    pub(super) rpc_map: RpcMap,
    pub(super) call_metrics: CallMetrics,
    pub(super) req_metrics: RequestMetrics,
    pub(super) resp_metrics: ResponseMetrics,
}

impl<Svc, RespBody> Service<http::Request<Body>> for TonicMetricService<Svc>
where
    Svc: Service<http::Request<Body>, Response = http::Response<RespBody>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    RespBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status> + Send + 'static,
{
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = TonicMetricHandler<Svc::Future>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        verbose!("metrics service call");

        // for the `grpc.server.call.duration` metric: this is the earliest we
        // know of a request/new stream.
        let call_start = Instant::now();

        let (parts, body) = request.into_parts();
        let attrs = Attributes::new(&self.rpc_map, &parts);
        let request = http::Request::from_parts(parts, body);

        let attributes = attrs!(attrs);
        self.call_metrics.started.add(1, attributes);

        // wrap the request stream
        let request = request.map(|inner| {
            Body::new(TonicMetricRequestBody {
                inner,
                attrs: attrs.clone(),
                metrics: self.req_metrics.clone(),
                size: 0,
            })
        });
        self.req_metrics.active_requests.add(1, attributes);

        // "call" the handler/create the handler future (since futures aren't
        // executed until they are polled)
        let inner = self.inner.call(request);

        // wrap the call handler
        let handler = TonicMetricHandler {
            inner,
            attrs,
            metrics: self.resp_metrics.clone(),
            call_start,
        };
        self.resp_metrics.active_handlers.add(1, attributes);
        handler
    }
}
