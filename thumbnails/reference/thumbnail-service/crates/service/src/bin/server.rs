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

use axum::serve;
use clap::Parser;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thumbnail_common::{
    build_outbound_clients, init_tracing, initialize_thumbnail_runtime,
    install_and_get_runtime_config, shutdown_receiver_with_signals,
    spawn_event_consumer_supervisor, EventConsumerRetryPolicy, EventConsumerSupervisorConfig,
    OutboundAuthArgs, OutboundClients, RuntimeConfigArgs,
};
use discovery_proto::nvidia::omniverse::discovery::capabilities::v2alpha::capabilities_service_server::CapabilitiesServiceServer;
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::thumbnail_service_server::ThumbnailServiceServer;
use thumbnail_service::grpc::{reflection_builder, CapabilitiesServiceImpl, ThumbnailServiceImpl};
use thumbnail_service::http::{app, AppState};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;
use tonic_observability::{
    rpc_map::{RpcMap, RpcMapOptimize},
    server::{metric_a66::TonicMetricLayer, trace::TonicTraceLayer},
};
use tracing::{error, info};

/// How long to wait, once one of HTTP/gRPC has finished (successfully or
/// not) and shutdown has been signaled, for the other to finish its own
/// graceful drain before giving up on it. Also used for the event-consumer
/// supervisor join once both protocol servers are down.
const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(10);

/// Unified thumbnail-service binary that serves both HTTP and gRPC protocols.
#[derive(Parser, Debug)]
#[command(author, version, about = "thumbnail-service unified server")]
struct Args {
    #[command(flatten)]
    auth: OutboundAuthArgs,

    #[command(flatten)]
    runtime: RuntimeConfigArgs,
}

#[tokio::main]
async fn main() {
    // `run` owns `_otel_guards`; all failure paths return from it rather than
    // calling `process::exit` directly.  That guarantees the OTel exporters are
    // flushed and shut down before the process exits — `process::exit` bypasses
    // destructors, so a direct call from `main` would lose any buffered spans
    // or metrics points.
    std::process::exit(run().await);
}

async fn run() -> i32 {
    dotenvy::dotenv().ok();
    let Args { auth, runtime } = Args::parse();

    let _otel_guards = init_tracing("thumbnail-service");

    info!("[START] Starting unified thumbnail service (HTTP + gRPC)");

    let runtime = match install_and_get_runtime_config(runtime) {
        Ok(runtime) => runtime,
        Err(e) => {
            error!("[ERROR] Failed to initialize runtime config: {}", e);
            return 1;
        }
    };

    if let Err(e) = initialize_thumbnail_runtime(runtime) {
        error!("[ERROR] {}", e);
        return 1;
    }

    let (shutdown_tx, shutdown_rx) = shutdown_receiver_with_signals();

    let outbound = match build_outbound_clients(auth, runtime).await {
        Ok(outbound) => outbound,
        Err(e) => {
            error!("[ERROR] {}", e);
            return 1;
        }
    };

    // This single process always "is" whichever role EVENT_CONSUMER_SERVER
    // configures (there's no second process to hand the other role to), so
    // mirror the parsed value rather than hardcoding Http — otherwise
    // EVENT_CONSUMER_SERVER=GRPC would fall into resolve_consumer_assignment's
    // "assigned elsewhere, skip" branch and silently disable consumption.
    let supervisor_handle = match spawn_event_consumer_supervisor(
        outbound.clone(),
        shutdown_rx.clone(),
        EventConsumerSupervisorConfig {
            runtime,
            service: runtime.event_consumer.event_consumer_server,
            retry_policy: EventConsumerRetryPolicy::Continuous,
        },
    )
    .await
    {
        Ok(handle) => handle,
        Err(e) => {
            error!("[ERROR] {}", e);
            return 1;
        }
    };

    let http_addr = format!("0.0.0.0:{}", runtime.service.port);
    let grpc_addr = runtime.service.grpc_port.clone();
    let grpc_healthy = Arc::new(AtomicBool::new(false));

    let had_error = run_http_and_grpc_until_either_exits(
        http_addr,
        grpc_addr,
        outbound,
        runtime,
        grpc_healthy,
        shutdown_tx,
        shutdown_rx,
    )
    .await;

    if let Some(handle) = supervisor_handle {
        if tokio::time::timeout(SHUTDOWN_GRACE_PERIOD, handle)
            .await
            .is_err()
        {
            error!(
                "[ERROR] Event consumer supervisor did not finish shutting down within {:?}",
                SHUTDOWN_GRACE_PERIOD
            );
        }
    }

    if had_error {
        error!("[ERROR] Unified service failed");
        return 1;
    }

    0
}

/// Run the HTTP and gRPC servers concurrently until either one exits, then
/// signal shutdown to the other and give it `SHUTDOWN_GRACE_PERIOD` to drain
/// before giving up. Unlike `tokio::try_join!`, this guarantees the peer
/// gets a real chance at its own graceful-shutdown path even when the first
/// server to exit failed (e.g. its port was already taken) rather than
/// being abruptly dropped when the joined future is cancelled.
///
/// Returns `true` if either server (or the supervisor caller separately
/// tracks) reported an error, panicked, or failed to shut down in time.
async fn run_http_and_grpc_until_either_exits(
    http_addr: String,
    grpc_addr: String,
    outbound: Arc<OutboundClients>,
    runtime: &'static RuntimeConfigArgs,
    grpc_healthy: Arc<AtomicBool>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
) -> bool {
    let mut http_task: JoinHandle<Result<(), String>> = tokio::spawn(run_http(
        http_addr,
        outbound.clone(),
        runtime,
        grpc_healthy.clone(),
        shutdown_rx.clone(),
    ));
    let mut grpc_task: JoinHandle<Result<(), String>> =
        tokio::spawn(run_grpc(grpc_addr, outbound, grpc_healthy, shutdown_rx));

    let (first_label, first_had_error) = tokio::select! {
        res = &mut http_task => ("HTTP", log_task_outcome("HTTP", res)),
        res = &mut grpc_task => ("GRPC", log_task_outcome("GRPC", res)),
    };
    info!(
        "[SHUTDOWN] {} server exited first, signaling shutdown",
        first_label
    );

    // Whichever finished first — successfully or with an error — signal the
    // other to gracefully drain via the same channel SIGTERM/SIGINT use,
    // instead of letting it be silently cancelled when this function returns.
    let _ = shutdown_tx.send(true);

    let (remaining_label, remaining_task) = if first_label == "HTTP" {
        ("GRPC", grpc_task)
    } else {
        ("HTTP", http_task)
    };

    let remaining_had_error =
        match tokio::time::timeout(SHUTDOWN_GRACE_PERIOD, remaining_task).await {
            Ok(res) => log_task_outcome(remaining_label, res),
            Err(_) => {
                error!(
                    "[ERROR] {} server did not finish shutting down within {:?}",
                    remaining_label, SHUTDOWN_GRACE_PERIOD
                );
                true
            }
        };

    first_had_error || remaining_had_error
}

/// Log a spawned server task's outcome and report whether it counts as an
/// error: an `Err` result, or a panic (join error) both count; a clean
/// `Ok(())` does not.
fn log_task_outcome(label: &str, res: Result<Result<(), String>, tokio::task::JoinError>) -> bool {
    match res {
        Ok(Ok(())) => false,
        Ok(Err(e)) => {
            error!("[ERROR] {} server error: {}", label, e);
            true
        }
        Err(join_err) => {
            error!("[ERROR] {} server task panicked: {}", label, join_err);
            true
        }
    }
}

async fn run_http(
    addr: String,
    outbound: Arc<OutboundClients>,
    runtime: &'static RuntimeConfigArgs,
    grpc_healthy: Arc<AtomicBool>,
    shutdown_rx: watch::Receiver<bool>,
) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("failed to bind HTTP listener on {addr}: {e}"))?;
    serve_http(listener, outbound, runtime, grpc_healthy, shutdown_rx).await
}

/// Serve HTTP on an already-bound listener.
///
/// Split from the bind above so a caller that has a listener can hand it over
/// rather than surrendering the port and asking for it back by address. Tests
/// need exactly that: picking an ephemeral port, dropping it, and re-binding the
/// number leaves a window in which anything else on the machine can take it,
/// which is the `EADDRINUSE` flake this suite already hit once with hardcoded
/// ports.
async fn serve_http(
    listener: tokio::net::TcpListener,
    outbound: Arc<OutboundClients>,
    runtime: &'static RuntimeConfigArgs,
    grpc_healthy: Arc<AtomicBool>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), String> {
    match listener.local_addr() {
        Ok(addr) => info!("[HTTP] Server listening on {}", addr),
        Err(e) => info!("[HTTP] Server listening (address unavailable: {})", e),
    }

    let state = AppState {
        outbound,
        runtime,
        grpc_healthy,
    };
    serve(listener, app(state).into_make_service())
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.wait_for(|stop| *stop).await;
            info!("[SHUTDOWN] HTTP server stopping");
        })
        .await
        .map_err(|e| format!("HTTP server error: {e}"))
}

/// Marks `grpc_healthy` back to `false` on drop, so every `run_grpc` exit
/// path — clean shutdown, serve error, or an early `?` return — reliably
/// clears the shared liveness flag without needing to remember to do so at
/// each `return` site.
struct GrpcLivenessGuard(Arc<AtomicBool>);

impl Drop for GrpcLivenessGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

async fn run_grpc(
    addr: String,
    outbound: Arc<OutboundClients>,
    grpc_healthy: Arc<AtomicBool>,
    shutdown_rx: watch::Receiver<bool>,
) -> Result<(), String> {
    let parsed_addr: std::net::SocketAddr = addr
        .parse()
        .map_err(|e| format!("Invalid gRPC bind address {addr}: {e}"))?;
    let listener = tokio::net::TcpListener::bind(parsed_addr)
        .await
        .map_err(|e| format!("failed to bind gRPC listener on {addr}: {e}"))?;
    serve_grpc(listener, outbound, grpc_healthy, shutdown_rx).await
}

/// Serve gRPC on an already-bound listener. See [`serve_http`] for why the bind
/// is separable.
async fn serve_grpc(
    listener: tokio::net::TcpListener,
    outbound: Arc<OutboundClients>,
    grpc_healthy: Arc<AtomicBool>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), String> {
    match listener.local_addr() {
        Ok(addr) => info!("[GRPC] Server listening on {}", addr),
        Err(e) => info!("[GRPC] Server listening (address unavailable: {})", e),
    }

    // Only claim readiness — for the shared HTTP /health flag and the
    // standard gRPC health service below — once the listener above has
    // actually bound successfully. Kubernetes liveness uses /health/live
    // (dependency-free); /health gates readiness on this flag.
    grpc_healthy.store(true, Ordering::Relaxed);
    let _liveness_guard = GrpcLivenessGuard(grpc_healthy);

    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<ThumbnailServiceServer<ThumbnailServiceImpl>>()
        .await;
    health_reporter
        .set_serving::<CapabilitiesServiceServer<CapabilitiesServiceImpl>>()
        .await;

    let svc = ThumbnailServiceImpl::new(outbound);
    let reflection_service_v1 = reflection_builder()
        .build_v1()
        .map_err(|e| format!("Failed to build v1 reflection service: {e}"))?;
    let reflection_service_v1alpha = reflection_builder()
        .build_v1alpha()
        .map_err(|e| format!("Failed to build v1alpha reflection service: {e}"))?;

    // Build an RPC map from the thumbnail proto file descriptor set so the
    // observability layers know the full set of expected gRPC methods.
    let rpc_map = RpcMap::builder()
        .optimize(RpcMapOptimize::STATIC)
        .add_encoded_file_descriptor_set(
            thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::FILE_DESCRIPTOR_SET,
        )
        .map_err(|e| format!("Failed to build gRPC RPC map: {e}"))?
        .build();

    let meter = opentelemetry::global::meter("thumbnail-service");

    Server::builder()
        .layer(
            TonicTraceLayer::builder()
                .with_rpc_map(rpc_map.clone())
                .build(),
        )
        .layer(
            TonicMetricLayer::builder(meter)
                .with_rpc_map(rpc_map)
                .build(),
        )
        .add_service(ThumbnailServiceServer::new(svc))
        // Registered bare, with no auth: the discovery operator dials this with a
        // plaintext credential-less channel, so an interceptor here would make the
        // service undiscoverable rather than secure it.
        .add_service(CapabilitiesServiceServer::new(CapabilitiesServiceImpl))
        .add_service(reflection_service_v1)
        .add_service(reflection_service_v1alpha)
        .add_service(health_service)
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async move {
            let _ = shutdown_rx.wait_for(|stop| *stop).await;
            info!("[SHUTDOWN] gRPC server stopping");
        })
        .await
        .map_err(|e| format!("gRPC server error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Builds an independent, `env`-derived `&'static RuntimeConfigArgs` for
    /// a single test, without going through the process-wide install-once
    /// singleton (`thumbnail_common::install_runtime_config` /
    /// `current_runtime_config`). `run_http`/`run_grpc`/
    /// `spawn_event_consumer_supervisor` all take their config by parameter
    /// rather than reaching for the global, so each test can have its own
    /// independent values (e.g. differing `EVENT_CONSUMER_SERVER`) without
    /// fighting the singleton's "install once per process" semantics.
    fn leaked_runtime_config(bin_name: &str) -> &'static RuntimeConfigArgs {
        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            runtime: RuntimeConfigArgs,
        }
        let runtime = TestCli::parse_from([bin_name]).runtime;
        Box::leak(Box::new(runtime))
    }

    fn set_baseline_thumbnail_env() {
        std::env::set_var(
            "GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY",
            "thumbnail-task-queue",
        );
        std::env::set_var(
            "THUMBNAIL_PATH_TEMPLATE",
            "{resource_path}/.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
        );
        std::env::set_var("THUMBNAIL_SIZES", "256x256");
        std::env::set_var("THUMBNAIL_ENCODING_FORMAT", "PNG");
        std::env::set_var("THUMBNAIL_TRANSFORM_TYPE", "CROP");
        std::env::set_var("THUMBNAIL_CROP_STRATEGY", "SMART");
        std::env::set_var("THUMBNAIL_QUALITY", "85");
        std::env::set_var("THUMBNAIL_PRESERVE_ASPECT_RATIO", "true");
    }

    /// A bound loopback listener and the address it holds.
    ///
    /// The listener is handed to the server rather than closed and re-bound by
    /// number, so the port is never unowned. These tests used to hardcode
    /// 58080/58090/58100 and their gRPC peers, all of which sit inside Linux's
    /// default ephemeral range (32768-60999): any outbound connection from a
    /// test binary running in parallel could already hold one, and `bind` then
    /// failed with `EADDRINUSE` for reasons that had nothing to do with the code
    /// under test. Letting the kernel choose fixed the collisions but left a
    /// smaller version of the same window — between dropping the scratch
    /// listener and the server re-binding its number — which keeping the
    /// listener closes outright.
    async fn bound_local_listener() -> (tokio::net::TcpListener, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binding an ephemeral loopback port should succeed");
        let addr = listener
            .local_addr()
            .expect("a bound listener has a local address")
            .to_string();
        (listener, addr)
    }

    /// An address whose port was free a moment ago, for the one test whose
    /// subject binds by address and so cannot be handed a listener.
    ///
    /// This keeps the window [`bound_local_listener`] exists to close, and that
    /// is unavoidable here rather than an oversight: the code under test is
    /// `run_http_and_grpc_until_either_exits`, whose whole job is to bind two
    /// addresses and react when one of them fails. Handing it a listener would
    /// mean not testing it. If the port is stolen in the gap the test fails
    /// loudly with the supervisor's own bind error, which is diagnosable rather
    /// than mysterious.
    async fn free_local_addr() -> String {
        let (listener, addr) = bound_local_listener().await;
        drop(listener);
        addr
    }

    /// Waits for `addr` to accept, but gives up early — reporting the server's
    /// own error — if its task exits first.
    ///
    /// Polling the port alone cannot distinguish "still starting" from "already
    /// dead", so a failed bind used to burn the full timeout and then blame the
    /// port instead of printing the `failed to bind ...` that `run_http` had
    /// returned immediately.
    async fn wait_until_serving(
        addr: &str,
        task: &mut JoinHandle<Result<(), String>>,
        timeout: Duration,
    ) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if tokio::net::TcpStream::connect(addr).await.is_ok() {
                return;
            }
            if task.is_finished() {
                match task.await {
                    Ok(Err(e)) => panic!("server for {addr} exited before serving: {e}"),
                    Ok(Ok(())) => panic!("server for {addr} returned Ok before serving"),
                    Err(join_err) => panic!("server for {addr} panicked: {join_err}"),
                }
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("timed out waiting for {addr} to accept connections");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    // 4.1/4.2/4.3: run_http and run_grpc, called directly, serve concurrently
    // on independent ports (a real gRPC channel connect succeeds, not just a
    // raw TCP one), and flipping the shared shutdown sender resolves both
    // futures to `Ok(())` within a bounded time.
    #[tokio::test]
    #[serial_test::serial]
    async fn http_and_grpc_serve_concurrently_then_shut_down_cleanly() {
        set_baseline_thumbnail_env();
        std::env::remove_var("EVENT_CONSUMER_SERVER");
        std::env::set_var("DISABLE_EVENT_CONSUMPTION", "true");
        let runtime = leaked_runtime_config("services-server-test-1");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let grpc_healthy = Arc::new(AtomicBool::new(false));

        let (http_listener, http_addr) = bound_local_listener().await;
        let (grpc_listener, grpc_addr) = bound_local_listener().await;

        let mut http_task = tokio::spawn(serve_http(
            http_listener,
            Arc::new(OutboundClients::for_tests()),
            runtime,
            grpc_healthy.clone(),
            shutdown_rx.clone(),
        ));
        let mut grpc_task = tokio::spawn(serve_grpc(
            grpc_listener,
            Arc::new(OutboundClients::for_tests()),
            grpc_healthy.clone(),
            shutdown_rx.clone(),
        ));

        wait_until_serving(&http_addr, &mut http_task, Duration::from_secs(5)).await;
        wait_until_serving(&grpc_addr, &mut grpc_task, Duration::from_secs(5)).await;

        tonic::transport::Endpoint::from_shared(format!("http://{grpc_addr}"))
            .expect("valid endpoint")
            .connect()
            .await
            .expect("gRPC channel should connect while the server is running");

        assert!(
            grpc_healthy.load(Ordering::Relaxed),
            "gRPC should report healthy once bound"
        );

        let _ = shutdown_tx.send(true);

        let http_res = tokio::time::timeout(Duration::from_secs(5), http_task)
            .await
            .expect("HTTP task should finish within the timeout");
        let grpc_res = tokio::time::timeout(Duration::from_secs(5), grpc_task)
            .await
            .expect("gRPC task should finish within the timeout");

        assert!(
            matches!(http_res, Ok(Ok(()))),
            "HTTP should shut down cleanly: {http_res:?}"
        );
        assert!(
            matches!(grpc_res, Ok(Ok(()))),
            "gRPC should shut down cleanly: {grpc_res:?}"
        );
        assert!(
            !grpc_healthy.load(Ordering::Relaxed),
            "gRPC liveness flag should clear once the server exits"
        );
    }

    // 4.4: an event-consumer supervisor spawned alongside HTTP/gRPC, sharing
    // the same shutdown receiver, must also resolve within a bounded time
    // once shutdown fires.
    #[tokio::test]
    #[serial_test::serial]
    async fn event_consumer_supervisor_joins_after_shutdown_alongside_http_and_grpc() {
        use thumbnail_common::test_fixtures::mock_consumer_server::{
            CreateBehavior, MockConsumerServer,
        };

        set_baseline_thumbnail_env();
        std::env::set_var("DISABLE_EVENT_CONSUMPTION", "false");
        std::env::set_var("EVENT_CONSUMER_SERVER", "HTTP");
        std::env::set_var(
            "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
            "omniverse://storage/projects/my-project/",
        );
        // Durable-queue provisioning is always performed, so stand up a live
        // mock event-consumer server that reports the queue created; the
        // supervisor provisions successfully and spawns. This test only cares
        // that it started and joins promptly on shutdown.
        let mock = MockConsumerServer::start(vec![CreateBehavior::Created(true)]).await;
        std::env::set_var(
            "NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT",
            format!("http://{}", mock.addr),
        );
        let runtime = leaked_runtime_config("services-server-test-2");

        let mut outbound = OutboundClients::for_tests();
        outbound.event_consumer_channel = Some(mock.channel().await);
        let outbound = Arc::new(outbound);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let grpc_healthy = Arc::new(AtomicBool::new(false));

        let (http_listener, http_addr) = bound_local_listener().await;
        let (grpc_listener, _grpc_addr) = bound_local_listener().await;

        let mut http_task = tokio::spawn(serve_http(
            http_listener,
            outbound.clone(),
            runtime,
            grpc_healthy.clone(),
            shutdown_rx.clone(),
        ));
        let grpc_task = tokio::spawn(serve_grpc(
            grpc_listener,
            outbound.clone(),
            grpc_healthy,
            shutdown_rx.clone(),
        ));

        let supervisor_handle = spawn_event_consumer_supervisor(
            outbound,
            shutdown_rx.clone(),
            EventConsumerSupervisorConfig {
                runtime,
                service: runtime.event_consumer.event_consumer_server,
                retry_policy: EventConsumerRetryPolicy::Continuous,
            },
        )
        .await
        .expect("supervisor config is valid, should spawn")
        .expect("assigned+enabled consumer should spawn a task");

        wait_until_serving(&http_addr, &mut http_task, Duration::from_secs(5)).await;

        let _ = shutdown_tx.send(true);

        let http_res = tokio::time::timeout(Duration::from_secs(5), http_task)
            .await
            .expect("HTTP task should finish within the timeout");
        let grpc_res = tokio::time::timeout(Duration::from_secs(5), grpc_task)
            .await
            .expect("gRPC task should finish within the timeout");
        let supervisor_res = tokio::time::timeout(Duration::from_secs(5), supervisor_handle).await;

        assert!(matches!(http_res, Ok(Ok(()))), "{http_res:?}");
        assert!(matches!(grpc_res, Ok(Ok(()))), "{grpc_res:?}");
        assert!(
            supervisor_res.is_ok(),
            "event consumer supervisor should join within the timeout: {supervisor_res:?}"
        );

        mock.shutdown().await;
        std::env::remove_var("NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES");
        std::env::remove_var("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT");
    }

    // 1.1/1.3: env_filter behavior — task followups service-log-level-configuration.
    // These must NOT call `.init()` (which is process-global); they just build the
    // filter and inspect its directive string, which is stable and human-readable.

    #[test]
    #[serial_test::serial]
    fn env_filter_honors_rust_log_debug_directive() {
        use thumbnail_common::env_log_filter;
        std::env::set_var("RUST_LOG", "debug");
        let directives = env_log_filter().to_string();
        std::env::remove_var("RUST_LOG");
        // The filter must carry the user-supplied directive, not the hardcoded
        // "info" fallback — the old code appended "info" after RUST_LOG=debug,
        // clamping the effective level back to INFO.
        assert!(
            directives.contains("debug"),
            "filter should reflect RUST_LOG=debug; got directives: {directives}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn env_filter_falls_back_to_info_when_rust_log_is_unset() {
        use thumbnail_common::env_log_filter;
        std::env::remove_var("RUST_LOG");
        let directives = env_log_filter().to_string();
        assert!(
            directives == "info" || directives.contains("info"),
            "filter should default to 'info' when RUST_LOG is unset; got: {directives}"
        );
    }

    // 4.5: reproduces the original audit's partial-bind-failure scenario --
    // one port is already taken when the process starts -- and asserts the
    // surviving peer now drains gracefully (via
    // `run_http_and_grpc_until_either_exits`, task group 2's fix) instead of
    // being abruptly dropped.
    #[tokio::test]
    #[serial_test::serial]
    async fn partial_bind_failure_lets_surviving_server_drain_gracefully() {
        set_baseline_thumbnail_env();
        std::env::remove_var("EVENT_CONSUMER_SERVER");
        std::env::set_var("DISABLE_EVENT_CONSUMPTION", "true");
        let runtime = leaked_runtime_config("services-server-test-3");

        // Held for the duration, so the gRPC bind below is guaranteed to lose:
        // the occupied port is whichever one the kernel just handed us, rather
        // than a fixed number we hope nothing else took.
        let blocker = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to occupy the test gRPC port");
        let blocked_grpc_addr = blocker
            .local_addr()
            .expect("a bound listener has a local address")
            .to_string();
        let http_addr = free_local_addr().await;

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let grpc_healthy = Arc::new(AtomicBool::new(false));

        let had_error = tokio::time::timeout(
            Duration::from_secs(15),
            run_http_and_grpc_until_either_exits(
                http_addr,
                blocked_grpc_addr,
                Arc::new(OutboundClients::for_tests()),
                runtime,
                grpc_healthy,
                shutdown_tx,
                shutdown_rx,
            ),
        )
        .await
        .expect(
            "run_http_and_grpc_until_either_exits should finish once the peer's bind fails, \
             instead of hanging or abruptly dropping the surviving HTTP server",
        );

        assert!(
            had_error,
            "a bind failure on one side must be reported as an error"
        );
        drop(blocker);
    }

    /// `RUST_LOG` is the only way an operator can raise this service's log
    /// level — the chart writes it straight from `observability.logging.level`
    /// — so a filter that silently ignores it makes every `debug!` in the
    /// codebase unreachable in a deployed environment.
    mod log_filter {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use thumbnail_common::env_log_filter;
        use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
        use tracing_subscriber::Registry;

        /// Restores `RUST_LOG` when the test ends, including on panic, so a
        /// failing case cannot leak a level into the tests that follow.
        struct ScopedRustLog(Option<std::ffi::OsString>);

        impl ScopedRustLog {
            fn set(value: Option<&str>) -> Self {
                let previous = std::env::var_os("RUST_LOG");
                match value {
                    Some(value) => std::env::set_var("RUST_LOG", value),
                    None => std::env::remove_var("RUST_LOG"),
                }
                Self(previous)
            }
        }

        impl Drop for ScopedRustLog {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(previous) => std::env::set_var("RUST_LOG", previous),
                    None => std::env::remove_var("RUST_LOG"),
                }
            }
        }

        #[derive(Clone, Default)]
        struct CountEvents(Arc<AtomicUsize>);

        impl<S: tracing::Subscriber> Layer<S> for CountEvents {
            fn on_event(&self, _event: &tracing::Event<'_>, _: Context<'_, S>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        /// Counts how many events at `level` survive the filter. Asserting on
        /// the filter's own level hint is not enough: the pre-fix filter
        /// reports a `DEBUG` hint while still dropping every debug event,
        /// because `add_directive("info")` replaces the global directive
        /// `RUST_LOG=debug` just installed rather than narrowing it.
        fn events_reaching_the_subscriber(level: tracing::Level) -> usize {
            let counter = CountEvents::default();
            let observed = counter.0.clone();
            let subscriber = Registry::default().with(env_log_filter()).with(counter);

            tracing::subscriber::with_default(subscriber, || match level {
                tracing::Level::DEBUG => tracing::debug!("probe"),
                tracing::Level::INFO => tracing::info!("probe"),
                other => panic!("unhandled probe level {other}"),
            });

            observed.load(Ordering::SeqCst)
        }

        #[test]
        #[serial_test::serial]
        fn rust_log_debug_lets_debug_events_through() {
            let _restore = ScopedRustLog::set(Some("debug"));

            assert_eq!(
                events_reaching_the_subscriber(tracing::Level::DEBUG),
                1,
                "RUST_LOG=debug must reach the subscriber; dev sets it and \
                 logged no debug lines at all"
            );
        }

        #[test]
        #[serial_test::serial]
        fn an_unset_rust_log_still_defaults_to_info() {
            let _restore = ScopedRustLog::set(None);

            assert_eq!(
                events_reaching_the_subscriber(tracing::Level::INFO),
                1,
                "with no RUST_LOG the service must keep logging at INFO"
            );
            assert_eq!(
                events_reaching_the_subscriber(tracing::Level::DEBUG),
                0,
                "with no RUST_LOG debug must stay off"
            );
        }

        #[test]
        #[serial_test::serial]
        fn a_malformed_rust_log_falls_back_to_info_rather_than_panicking() {
            let _restore = ScopedRustLog::set(Some("=$%not a filter"));

            assert_eq!(
                events_reaching_the_subscriber(tracing::Level::INFO),
                1,
                "a typo in RUST_LOG must not take the service down at startup"
            );
        }
    }
}
