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

use super::serve::serve_with_incoming_shutdown;
use super::{HttpRequest, HttpResponse, HttpResult, HttpServer, MakeServer};
use crate::harness::utils::rpc_map;
use std::convert::Infallible;
use std::future::Future;
use std::marker::PhantomData;
use std::task::{Context, Poll};
use testing::guards::Guards;
use testing::in_flight::in_flight;
use tokio::sync::oneshot;
use tonic::async_trait;
use tonic_observability::server::metric_a66::TonicMetricLayer as A66MetricLayer;
use tonic_observability::server::trace::TonicTraceLayer;
use tower::Service;

/// Implement this to make a HTTP2 servers that mimic tonic. The server will
/// receive the layers also.
///
/// Is also implemented automatically for any <code>async fn ([HttpRequest]) ->
/// [HttpResult]</code> function.
pub(crate) trait TonicService: Clone + Send + Sync + 'static {
    type Future: Future<Output = HttpResult<Infallible>> + Send + 'static;

    fn call(&mut self, req: HttpRequest) -> Self::Future;
}

impl<F, Fut> TonicService for F
where
    F: Fn(HttpRequest) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = HttpResult<Infallible>> + Send + 'static,
{
    type Future = Fut;

    fn call(&mut self, req: HttpRequest) -> Self::Future {
        self(req)
    }
}

/// Private wrapper to adapt a [`TonicService`] to a tower [`Service`].
#[derive(Debug, Clone)]
#[repr(transparent)]
struct TonicAdapter<T>(T);

impl<T: TonicService> Service<HttpRequest> for TonicAdapter<T> {
    type Error = Infallible;
    type Response = HttpResponse;
    type Future = T::Future;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: HttpRequest) -> Self::Future {
        <T as TonicService>::call(&mut self.0, req)
    }
}

pub(crate) struct Tonic;

impl<Svc: TonicService> HttpServer<Svc, Tonic> {
    pub(crate) fn tonic(handler: Svc) -> Self {
        Self {
            inner: handler,
            marker: PhantomData,
        }
    }
}

#[async_trait]
impl<Svc: TonicService> MakeServer for HttpServer<Svc, Tonic> {
    type Fut = futures::future::BoxFuture<'static, ()>;

    async fn make(self, listener: tokio::net::TcpListener, guards: &Guards) -> Self::Fut {
        let meter = guards.metrics_guard.meter("test").unwrap();

        let service = TonicAdapter(self.inner);

        let metric_a66_layer = A66MetricLayer::builder(meter)
            .with_rpc_map(rpc_map())
            .build();
        let trace_layer = TonicTraceLayer::builder().with_rpc_map(rpc_map()).build();
        let (in_flight_layer, in_flight) = in_flight();
        let svc = tower::ServiceBuilder::new()
            .map_response(|response: tonic::codegen::http::Response<_>| {
                response.map(tonic::body::Body::new)
            })
            .layer(in_flight_layer)
            .layer(metric_a66_layer)
            .layer(trace_layer)
            .service(service);

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let fut = serve_with_incoming_shutdown(svc, listener, shutdown_rx, true);
        let handle = tokio::spawn(fut);

        Box::pin(async move {
            let _res: Result<(), ()> = shutdown_tx.send(());
            let _: () = handle.await.expect("failed to join server");
            // so the server spans end before the exporters are shut down
            in_flight.wait().await;
        })
    }
}
