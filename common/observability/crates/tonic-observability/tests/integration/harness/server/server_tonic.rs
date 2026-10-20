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

use super::{Guards, HttpRequest, HttpResponse, MakeServer};
use crate::harness::utils::rpc_map;
use std::convert::Infallible;
use testing::in_flight::{InFlightLayer, in_flight};
use tokio::sync::oneshot;
use tonic::async_trait;
use tonic::transport::server::TcpIncoming;
use tonic_observability::server::metric_a66::TonicMetricLayer as A66MetricLayer;
use tonic_observability::server::trace::TonicTraceLayer;
use tower_layer::{Identity, Stack};

/// Serves the router made by `make_router` from a server builder, which has
/// the [`InFlightLayer`] as the outermost layer.
pub(crate) async fn make_server<Svc, RespBody, L, MakeRouter>(
    listener: tokio::net::TcpListener,
    make_router: MakeRouter,
) -> futures::future::BoxFuture<'static, ()>
where
    MakeRouter: FnOnce(
        tonic::transport::Server<Stack<InFlightLayer, Identity>>,
    ) -> tonic::transport::server::Router<L>,
    Svc: tower_service::Service<HttpRequest, Response = HttpResponse<RespBody>, Error = Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    Svc::Future: Send + 'static,
    RespBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status> + Send + 'static,
    L: tower_layer::Layer<tonic::service::Routes, Service = Svc> + Clone + Send + 'static,
{
    let (in_flight_layer, in_flight) = in_flight();
    let router = make_router(tonic::transport::Server::builder().layer(in_flight_layer));

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let incoming = TcpIncoming::from(listener);
    let signal = async move {
        let _ = shutdown_rx.await;
    };

    let handle = tokio::spawn(router.serve_with_incoming_shutdown(incoming, signal));

    Box::pin(async move {
        let _res: Result<(), ()> = shutdown_tx.send(());
        let _: () = handle
            .await
            .expect("failed to join server")
            .expect("failed to run server");
        // so the server spans end before the exporters are shut down
        in_flight.wait().await;
    })
}

#[async_trait]
impl<Svc, RespBody> MakeServer for Svc
where
    Svc: tower_service::Service<HttpRequest, Response = HttpResponse<RespBody>, Error = Infallible>
        + tonic::server::NamedService
        + Clone
        + Send
        + Sync
        + 'static,
    Svc::Future: Send + 'static,
    RespBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status> + Send + 'static,
{
    type Fut = futures::future::BoxFuture<'static, ()>;

    async fn make(self, listener: tokio::net::TcpListener, guards: &Guards) -> Self::Fut {
        let meter = guards.metrics_guard.meter("test").unwrap();
        let rpc_map = rpc_map();

        let metric_a66_layer = A66MetricLayer::builder(meter)
            .with_rpc_map(rpc_map.clone())
            .build();
        let trace_layer = TonicTraceLayer::builder().with_rpc_map(rpc_map).build();
        make_server(listener, |server| {
            server
                .layer(metric_a66_layer)
                .layer(trace_layer)
                .add_service(self)
        })
        .await
    }
}
