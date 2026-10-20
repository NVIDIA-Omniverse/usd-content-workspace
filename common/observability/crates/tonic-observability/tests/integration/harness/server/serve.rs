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

use super::{HttpRequest, HttpResponse};
use futures::{FutureExt as _, StreamExt as _};
use hyper_util::service::TowerToHyperService;
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use tokio::sync::{oneshot, watch};
use tonic::codegen::http;
use tonic::transport::server::TcpIncoming;
use tower::Service;
use tower::util::ServiceExt as _;
use tracing::trace;

pub(super) async fn serve_with_incoming_shutdown<Svc>(
    svc: Svc,
    listener: tokio::net::TcpListener,
    shutdown_rx: oneshot::Receiver<()>,
    http2: bool,
) where
    Svc: Service<HttpRequest, Response = HttpResponse> + Clone + Send + Sync + 'static,
    Svc::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    Svc::Future: Future<Output = Result<HttpResponse, Svc::Error>> + Send + 'static,
{
    let executor = hyper_util::rt::TokioExecutor::new();
    let timer = hyper_util::rt::TokioTimer::new();
    let builder = hyper_util::server::conn::auto::Builder::new(executor);

    let server = if http2 {
        let mut builder = builder.http2_only();
        builder.http2().timer(timer);
        builder
    } else {
        let mut builder = builder.http1_only();
        builder.http1().timer(timer);
        builder
    };

    let (signal_tx, signal_rx) = watch::channel(());
    let signal_tx = Arc::new(signal_tx);

    let signal = async move {
        let _ = shutdown_rx.await;
    }
    .fuse();
    let mut sig = pin!(signal);

    let incoming = TcpIncoming::from(listener);
    let mut incoming = pin!(incoming);

    loop {
        tokio::select! {
            _ = &mut sig => {
                trace!("signal received, shutting down");
                break;
            },
            io = incoming.next() => {
                let io = match io {
                    Some(Ok(io)) => io,
                    Some(Err(e)) => {
                        trace!("error accepting connection: {:#}", e);
                        continue;
                    },
                    None => {
                        break
                    },
                };

                trace!("connection accepted");

                let req_svc = svc.clone();

                let hyper_io = hyper_util::rt::TokioIo::new(io);
                let hyper_svc = TowerToHyperService::new(req_svc.map_request(|req: http::Request<hyper::body::Incoming>| req.map(tonic::body::Body::new)));

                let builder = server.clone();
                let mut watcher = signal_rx.clone();

                tokio::spawn(async move {
                    {
                        let mut sig = pin!(watcher.changed().fuse());
                        let mut conn = pin!(builder.serve_connection(hyper_io, hyper_svc));

                        loop {
                            tokio::select! {
                                rv = &mut conn => {
                                    if let Err(err) = rv {
                                        println!("failed serving connection: {err:#}");
                                    }
                                    break;
                                },
                                _ = &mut sig => {
                                    conn.as_mut().graceful_shutdown();
                                }
                            }
                        }
                    }

                    drop(watcher);
                    trace!("connection closed");
                });
            }
        }
    }

    let _ = signal_tx.send(());
    drop(signal_rx);
    trace!(
        "waiting for {} connections to close",
        signal_tx.receiver_count()
    );

    // Wait for all connections to close
    signal_tx.closed().await;
}
