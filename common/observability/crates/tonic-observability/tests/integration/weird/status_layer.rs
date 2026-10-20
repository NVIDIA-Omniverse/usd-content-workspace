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

use crate::harness::prelude::rpc_map;
use crate::harness::server::{HttpRequest, HttpResponse, MakeServer, make_server};
use std::convert::Infallible;
use std::task::{Context, Poll};
use testing::body::ErrorBody;
use testing::guards::Guards;
use tonic::Status;
use tonic::async_trait;
use tonic_observability::server::metric_a66::TonicMetricLayer as A66MetricLayer;
use tonic_observability::server::trace::TonicTraceLayer;

#[derive(Debug, Clone)]
#[repr(transparent)]
pub(super) struct StatusServer<Svc> {
    inner: Svc,
}

impl<Svc> StatusServer<Svc> {
    pub(super) fn new(inner: Svc) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<Svc> MakeServer for StatusServer<Svc>
where
    Svc: tower_service::Service<HttpRequest, Response = HttpResponse, Error = Infallible>
        + tonic::server::NamedService
        + Clone
        + Send
        + Sync
        + 'static,
    Svc::Future: Send + 'static,
{
    type Fut = futures::future::BoxFuture<'static, ()>;

    async fn make(self, listener: tokio::net::TcpListener, guards: &Guards) -> Self::Fut {
        let meter = guards.metrics_guard.meter("test").unwrap();

        let metric_a66_layer = A66MetricLayer::builder(meter)
            .with_rpc_map(rpc_map())
            .build();
        let trace_layer = TonicTraceLayer::builder().with_rpc_map(rpc_map()).build();
        make_server(listener, |server| {
            server
                .layer(StatusLayer)
                .layer(metric_a66_layer)
                .layer(trace_layer)
                .add_service(self.inner)
        })
        .await
    }
}

#[derive(Debug, Clone)]
pub(super) struct StatusLayer;

impl<Inner> tower_layer::Layer<Inner> for StatusLayer {
    type Service = StatusService<Inner>;

    fn layer(&self, inner: Inner) -> Self::Service {
        StatusService { inner }
    }
}

#[derive(Debug, Clone)]
#[repr(transparent)]
pub(super) struct StatusService<Svc> {
    inner: Svc,
}

impl<Svc> tower::Service<HttpRequest> for StatusService<Svc>
where
    Svc: tonic::client::GrpcService<
            tonic::body::Body,
            Error = Infallible,
            ResponseBody = tonic::body::Body,
        >,
{
    type Future = Svc::Future;
    type Response = HttpResponse;
    type Error = Infallible;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: HttpRequest) -> Self::Future {
        let request = request.map(|_body| {
            tonic::body::Body::new(ErrorBody::new(Status::resource_exhausted("oops")))
        });
        self.inner.call(request)
    }
}
