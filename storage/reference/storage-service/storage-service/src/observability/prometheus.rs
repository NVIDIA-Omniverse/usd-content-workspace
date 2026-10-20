// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use std::{
    net::{SocketAddr, SocketAddrV4},
    sync::Arc,
};

use axum::{
    extract::State,
    http::{header::CONTENT_TYPE, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use opentelemetry_prometheus_text_exporter::PrometheusExporter;
use tokio::{net::TcpListener, sync::oneshot::channel};
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::{error, info};

use crate::observability::error::Error;

/// A prometheus service drop guard.
#[derive(Debug)]
pub struct Handle {
    _guard: DropGuard,
}

/// Start a prometheus service.
pub async fn start_prometheus_service(exporter: PrometheusExporter, addr: SocketAddrV4) -> Result<Handle, Error> {
    let cancel = CancellationToken::new();
    let (status_tx, status_rx) = channel();
    tokio::spawn({
        let cancel = cancel.clone();
        async move {
            let (listener, socket_addr) = match create_listener(addr).await {
                Ok(result) => {
                    let _ = status_tx.send(Ok(()));
                    result
                },
                Err(error) => {
                    let _ = status_tx.send(Err(error));
                    return;
                },
            };

            info!("prometheus server is listening on {}", socket_addr);

            if let Err(error) = axum::serve(listener, create_router(exporter).into_make_service())
                .with_graceful_shutdown(cancel.cancelled_owned())
                .await
            {
                error!(?error, "prometheus service has unexpectedly closed");
            } else {
                info!("prometheus service shut down");
            }
        }
    });

    status_rx.await.expect("send half has unexpectedly closed")?;

    Ok(Handle {
        _guard: cancel.drop_guard(),
    })
}

/// Create a metrics router.
fn create_router(exporter: PrometheusExporter) -> Router {
    Router::new()
        .route("/metrics", get(get_metrics))
        .with_state(Arc::new(exporter))
}

/// Create a TCP listener.
async fn create_listener(addr: SocketAddrV4) -> Result<(TcpListener, SocketAddr), Error> {
    let listener = TcpListener::bind(addr).await.map_err(Error::prometheus_exporter_init)?;
    let listen_addr = listener.local_addr().map_err(Error::prometheus_exporter_init)?;

    Ok((listener, listen_addr))
}

/// Collect and return metrics.
async fn get_metrics(State(exporter): State<Arc<PrometheusExporter>>) -> Result<impl IntoResponse, StatusCode> {
    let mut content = Vec::new();
    if let Err(error) = exporter.export(&mut content) {
        error!(?error, "prometheus metrics write failed");
        Err(StatusCode::INTERNAL_SERVER_ERROR)
    } else {
        Ok((StatusCode::OK, [(CONTENT_TYPE, "text/plain")], content))
    }
}
