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

use axum_observability::server::metric::TowerMetricLayer;
use axum_observability::server::trace::TowerTraceLayer;
use testing::guards::Guards;
use testing::in_flight::in_flight;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tower::ServiceBuilder;

pub(crate) trait MakeService {
    fn make(self, guards: &Guards) -> axum::Router;
}

pub(crate) struct DefaultMakeService {
    pub(crate) router: axum::Router<()>,
}

impl MakeService for DefaultMakeService {
    fn make(self, guards: &Guards) -> axum::Router {
        let Self { router } = self;

        let meter = guards.metrics_guard.meter("test").unwrap();

        let metric_layer = TowerMetricLayer::builder(meter).build();
        let trace_layer = TowerTraceLayer::builder().build();

        router.layer(ServiceBuilder::new().layer(metric_layer).layer(trace_layer))
    }
}

pub(crate) async fn spawn(
    router: axum::Router,
    listener: TcpListener,
) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let (in_flight_layer, in_flight) = in_flight();

    let make_service = router.layer(in_flight_layer).into_make_service();
    let fut = axum::serve(listener, make_service)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        })
        .into_future();
    let handle = tokio::spawn(fut);

    Box::pin(async move {
        let _res: Result<(), ()> = shutdown_tx.send(());
        let _: () = handle
            .await
            .expect("failed to join server")
            .expect("failed to spawn server");
        // so the server spans end before the exporters are shut down
        in_flight.wait().await;
    })
}
