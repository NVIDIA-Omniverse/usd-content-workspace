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
use super::handler::TowerMetricHandler;
use super::metrics::{RequestMetrics, ResponseMetrics};
use super::request_body::TowerMetricRequestBody;
use super::response_body::TowerMetricResponseBody;
use crate::server_verbose as verbose;
use bytes::Bytes;
use std::convert::Infallible;
use std::task::{Context, Poll};
use std::time::Instant;
use tower_service::Service;

/// Server metric service, returned by the metric layer.
///
/// This wraps the original service to instrument requests and responses.
///
/// This is exported for service type signatures and cannot be constructed
/// directly.
#[derive(Debug, Clone)]
pub struct TowerMetricService<Svc> {
    pub(super) inner: Svc,
    pub(super) req_metrics: RequestMetrics,
    pub(super) resp_metrics: ResponseMetrics,
}

impl<Svc, ReqBody, RespBody> Service<http::Request<ReqBody>> for TowerMetricService<Svc>
where
    Svc: Service<
            http::Request<TowerMetricRequestBody<ReqBody>>,
            Response = http::Response<RespBody>,
            Error = Infallible,
        > + Clone
        + Send
        + 'static,
    ReqBody: http_body::Body<Data = Bytes>,
    ReqBody::Error: std::error::Error + 'static,
    RespBody: http_body::Body<Data = Bytes> + Send,
    RespBody::Error: std::error::Error + 'static,
{
    type Response = http::Response<TowerMetricResponseBody<RespBody>>;
    type Error = Infallible;
    type Future = TowerMetricHandler<Svc::Future>;

    #[inline]
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<ReqBody>) -> Self::Future {
        verbose!("metrics service call");

        let start = Instant::now();

        let (parts, body) = request.into_parts();
        let attrs = Attributes::new(&parts);
        let request = http::Request::from_parts(parts, body);

        let attributes = attrs!(attrs);
        self.req_metrics.request_started.add(1, attributes);

        // wrap the request stream
        let request = request.map(|inner| TowerMetricRequestBody {
            inner,
            attrs: attrs.clone(),
            metrics: self.req_metrics.clone(),
            size: 0,
        });

        // "call" the handler/create the handler future (since futures aren't
        // executed until they are polled)
        let inner = self.inner.call(request);

        // wrap the call handler
        TowerMetricHandler {
            inner,
            attrs,
            start,
            metrics: self.resp_metrics.clone(),
        }
    }
}
