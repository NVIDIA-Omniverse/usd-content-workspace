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

//! A probe surface for a process with no API.
//!
//! The chart's existing probes are `httpGet` against the REST port, and the
//! worker deliberately does not link Axum, so it cannot inherit them. This is a
//! hand-written HTTP/1.1 responder on a `TcpListener` — about as much HTTP as a
//! `kubectl` probe needs, and no framework.
//!
//! "Ready" means something specific here. A consumer that is up but consuming
//! nothing is the failure this exists to catch, so readiness tracks whether the
//! consume stream is currently established, not whether the process started.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::metrics::WorkerMetrics;

#[cfg(not(test))]
const SOCKET_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const SOCKET_OPERATION_TIMEOUT: Duration = Duration::from_millis(100);

/// Shared liveness and readiness state.
#[derive(Default)]
pub struct HealthState {
    /// The consume stream is established and being polled.
    stream_connected: AtomicBool,
    /// The process is shutting down; readiness drops so the platform stops
    /// counting this replica before it stops working.
    draining: AtomicBool,
}

impl HealthState {
    pub fn stream_established(&self) {
        self.stream_connected.store(true, Ordering::SeqCst);
    }

    pub fn stream_lost(&self) {
        self.stream_connected.store(false, Ordering::SeqCst);
    }

    pub fn draining(&self) {
        self.draining.store(true, Ordering::SeqCst);
    }

    /// Ready to be counted as a working consumer.
    pub fn is_ready(&self) -> bool {
        self.stream_connected.load(Ordering::SeqCst) && !self.draining.load(Ordering::SeqCst)
    }

    /// Alive as a process. Deliberately weaker than readiness: a worker
    /// reconnecting after a transient stream error should be taken out of
    /// service, not restarted, because a restart abandons its in-flight work and
    /// does not fix the dependency.
    pub fn is_live(&self) -> bool {
        !self.draining.load(Ordering::SeqCst)
    }
}

/// Ask a already-running worker on this host whether it is ready.
///
/// The worker ships in a distroless image with no shell and no `curl`, so a
/// container health check has nothing to call the probe endpoint with — the
/// usual workaround, running the binary with `--help`, only proves the process
/// can start, which is precisely the thing readiness is *not* supposed to mean
/// here. Letting the binary probe itself keeps the check honest without adding
/// a shell to the image.
///
/// Kubernetes uses the `httpGet` probes directly and does not need this.
pub async fn probe_ready(port: u16) -> Result<(), String> {
    let mut stream = tokio::time::timeout(
        SOCKET_OPERATION_TIMEOUT,
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .map_err(|_| format!("timed out reaching the probe surface on port {port}"))?
    .map_err(|e| format!("could not reach the probe surface on port {port}: {e}"))?;

    tokio::time::timeout(
        SOCKET_OPERATION_TIMEOUT,
        stream.write_all(b"GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"),
    )
    .await
    .map_err(|_| "timed out sending the probe request".to_string())?
    .map_err(|e| format!("could not send the probe request: {e}"))?;
    tokio::time::timeout(SOCKET_OPERATION_TIMEOUT, stream.flush())
        .await
        .map_err(|_| "timed out flushing the probe request".to_string())?
        .map_err(|e| format!("could not flush the probe request: {e}"))?;

    let mut response = String::new();
    tokio::time::timeout(
        SOCKET_OPERATION_TIMEOUT,
        tokio::io::AsyncReadExt::read_to_string(&mut stream, &mut response),
    )
    .await
    .map_err(|_| "timed out reading the probe response".to_string())?
    .map_err(|e| format!("could not read the probe response: {e}"))?;

    let status = response.lines().next().unwrap_or_default();
    if status.contains(" 200 ") {
        Ok(())
    } else {
        Err(format!("not ready: {status}"))
    }
}

/// Bind the probe port.
///
/// Separate from [`serve`] so startup can fail on a port already in use rather
/// than discovering it through a background task nobody awaits. A worker whose
/// probes never answer is one Kubernetes will restart on a timeout with no
/// explanation of why, which is a poor way to learn the port was taken.
pub async fn bind(port: u16) -> io::Result<TcpListener> {
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    info!(port, "Worker probe surface listening");
    Ok(listener)
}

/// Serve `/healthz`, `/readyz`, and `/metrics` on an already-bound listener,
/// until shutdown.
pub async fn serve(
    listener: TcpListener,
    state: Arc<HealthState>,
    metrics: Arc<WorkerMetrics>,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    debug!("Probe surface shutting down");
                    return Ok(());
                }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let state = state.clone();
                        let metrics = metrics.clone();
                        tokio::spawn(async move {
                            if let Err(e) = answer(stream, state, metrics).await {
                                debug!(error = %e, "Probe connection ended early");
                            }
                        });
                    }
                    Err(e) => warn!(error = %e, "Probe listener accept failed"),
                }
            }
        }
    }
}

async fn answer(
    mut stream: TcpStream,
    state: Arc<HealthState>,
    metrics: Arc<WorkerMetrics>,
) -> io::Result<()> {
    let mut buffer = [0u8; 1024];
    let read = socket_operation(stream.read(&mut buffer)).await?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    let (status, body) = match path {
        "/readyz" => {
            if state.is_ready() {
                ("200 OK", "ready\n".to_string())
            } else {
                ("503 Service Unavailable", "not ready\n".to_string())
            }
        }
        "/healthz" => {
            if state.is_live() {
                ("200 OK", "ok\n".to_string())
            } else {
                ("503 Service Unavailable", "draining\n".to_string())
            }
        }
        "/metrics" => ("200 OK", metrics.render()),
        _ => ("404 Not Found", "not found\n".to_string()),
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket_operation(stream.write_all(response.as_bytes())).await?;
    socket_operation(stream.flush()).await
}

async fn socket_operation<T>(
    operation: impl std::future::Future<Output = io::Result<T>>,
) -> io::Result<T> {
    tokio::time::timeout(SOCKET_OPERATION_TIMEOUT, operation)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "probe socket operation timed out"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A worker that has not established its stream must not be Ready, or the
    /// platform counts a replica that is consuming nothing.
    #[test]
    fn readiness_requires_an_established_stream() {
        let state = HealthState::default();
        assert!(
            !state.is_ready(),
            "a process that just started is not ready"
        );

        state.stream_established();
        assert!(state.is_ready());

        state.stream_lost();
        assert!(
            !state.is_ready(),
            "a broken stream must take the replica out of service"
        );
    }

    /// The property that lets `main` serve probes before it provisions its
    /// queue, which is the whole reason startup can survive a slow dependency.
    ///
    /// Provisioning waits on the credential sidecar and retries at 10s, 20s,
    /// 40s. While the probe surface came up after it, liveness spent that minute
    /// probing a closed port, failed three times, and the kubelet killed a worker
    /// that was merely still starting. Serving first is only safe because a
    /// just-started worker is live without being ready: the kubelet leaves it
    /// alone, and nothing routes work to it until the consume stream exists.
    #[test]
    fn a_worker_that_has_not_provisioned_yet_is_live_but_not_ready() {
        let state = HealthState::default();

        assert!(
            state.is_live(),
            "a process still starting up must answer liveness, or it is killed \
             before it can finish starting"
        );
        assert!(
            !state.is_ready(),
            "and must not be ready, or work is routed to a worker with no stream"
        );
    }

    /// Liveness is deliberately weaker: restarting a worker whose dependency is
    /// down abandons its in-flight work and fixes nothing.
    #[test]
    fn a_lost_stream_does_not_fail_liveness() {
        let state = HealthState::default();
        state.stream_established();
        state.stream_lost();

        assert!(state.is_live());
        assert!(!state.is_ready());
    }

    #[test]
    fn draining_fails_both_probes() {
        let state = HealthState::default();
        state.stream_established();

        state.draining();

        assert!(!state.is_ready());
        assert!(!state.is_live());
    }

    /// The container health check path: the binary asks its own probe surface,
    /// because the distroless image has nothing else that can.
    #[tokio::test]
    async fn the_binary_can_probe_its_own_readiness() {
        let state = Arc::new(HealthState::default());
        let metrics = Arc::new(WorkerMetrics::new_for_tests());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let scratch = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let port = scratch.local_addr().expect("addr").port();
        drop(scratch);

        // Nothing listening yet: the probe must fail rather than hang or panic.
        assert!(
            probe_ready(port).await.is_err(),
            "a probe against a dead port should fail"
        );

        // Bound before the task is spawned, exactly as `main` does it, so the
        // port is already accepting when the first probe arrives.
        let listener = bind(port).await.expect("the scratch port is free again");
        let server = tokio::spawn(serve(listener, state.clone(), metrics, shutdown_rx));

        // Serving but not consuming — the failure mode readiness exists to catch.
        assert!(
            probe_ready(port).await.is_err(),
            "a worker with no consume stream is not ready"
        );

        state.stream_established();
        assert!(
            probe_ready(port).await.is_ok(),
            "a consuming worker should probe ready"
        );

        let _ = shutdown_tx.send(true);
        let _ = server.await;
    }

    #[tokio::test]
    async fn a_probe_times_out_when_the_peer_never_answers() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let peer = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.expect("accept");
            tokio::time::sleep(Duration::from_secs(1)).await;
        });

        let result = tokio::time::timeout(Duration::from_millis(250), probe_ready(port)).await;
        assert!(
            matches!(result, Ok(Err(_))),
            "probe_ready itself must bound a peer that accepts but never responds"
        );

        peer.abort();
    }

    #[tokio::test]
    async fn the_probe_surface_answers_without_an_http_framework() {
        let state = Arc::new(HealthState::default());
        let metrics = Arc::new(WorkerMetrics::new_for_tests());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = tokio::spawn(serve(listener, state.clone(), metrics, shutdown_rx));

        let probe = |path: &'static str| async move {
            let mut stream = TcpStream::connect(("127.0.0.1", port))
                .await
                .expect("connect");
            stream
                .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .await
                .expect("write");
            let mut response = String::new();
            tokio::io::AsyncReadExt::read_to_string(&mut stream, &mut response)
                .await
                .expect("read");
            response
        };

        assert!(
            probe("/readyz").await.starts_with("HTTP/1.1 503"),
            "not ready before the stream is established"
        );
        assert!(probe("/healthz").await.starts_with("HTTP/1.1 200"));

        state.stream_established();
        assert!(probe("/readyz").await.starts_with("HTTP/1.1 200"));
        assert!(probe("/metrics")
            .await
            .contains("thumbnail_worker_outcomes_total"));

        let _ = shutdown_tx.send(true);
        let _ = server.await;
    }
}
