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

//! Shared runtime/bootstrap helpers for service binaries.
//!
//! These helpers keep the `server` entrypoint thin by centralizing
//! startup wiring that should stay consistent across runtime paths.

use crate::config::QueueProvisioning;
use crate::errors::NotificationError;
use crate::notifications::provisioning::{ensure_durable_queue, EnsureQueueOutcome};
use crate::notifications::{EventConsumer, EventConsumerConfig};
use crate::{EventConsumerServer, OutboundClients, RuntimeConfigArgs};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Copy)]
pub enum EventConsumerRetryPolicy {
    Continuous,
    BoundedExponential { max_retries: u32 },
}

#[derive(Debug, Clone, Copy)]
pub struct EventConsumerSupervisorConfig {
    pub runtime: &'static RuntimeConfigArgs,
    pub service: EventConsumerServer,
    pub retry_policy: EventConsumerRetryPolicy,
}

fn service_name_for_logs(service: EventConsumerServer) -> &'static str {
    match service {
        EventConsumerServer::Http => "HTTP",
        EventConsumerServer::Grpc => "gRPC",
        EventConsumerServer::None => "NONE",
    }
}

fn resolve_consumer_assignment(cfg: EventConsumerSupervisorConfig) -> Option<EventConsumerServer> {
    let this_service = cfg.service;
    let consumer_server = cfg.runtime.event_consumer.event_consumer_server;

    match consumer_server {
        EventConsumerServer::None => {
            info!("[LOG] Event consumption disabled (EVENT_CONSUMER_SERVER=NONE)");
            info!(
                "[LOG] This {} service will skip event consumption",
                service_name_for_logs(this_service)
            );
            None
        }
        selected if selected == this_service => {
            info!(
                "[TARGET] This {} service will handle event consumption (EVENT_CONSUMER_SERVER={})",
                service_name_for_logs(this_service),
                selected.as_str()
            );
            Some(selected)
        }
        EventConsumerServer::Http => {
            info!("[LOG] Event consumption assigned to HTTP service (EVENT_CONSUMER_SERVER=HTTP)");
            info!(
                "[LOG] This {} service will skip event consumption",
                service_name_for_logs(this_service)
            );
            None
        }
        EventConsumerServer::Grpc => {
            info!("[LOG] Event consumption assigned to gRPC service (EVENT_CONSUMER_SERVER=GRPC)");
            info!(
                "[LOG] This {} service will skip event consumption",
                service_name_for_logs(this_service)
            );
            None
        }
    }
}

/// Resolve the Notifications endpoint required to start consuming events once
/// this binary is confirmed as the assigned, enabled consumer. StorageEventQueue's
/// id is derived later from resource prefixes (`resolve_queue_provisioning`).
fn event_consumer_env_or_skip(runtime: &RuntimeConfigArgs) -> Result<String, String> {
    let Some(consumer_endpoint) = runtime
        .endpoints
        .notifications_event_consumer_endpoint
        .clone()
    else {
        error!("[ERROR] NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT environment variable not set");
        error!("   This is required for event consumption to work properly");
        error!("   Set this in your Helm values or .env configuration");
        return Err(
            "Missing required environment variable: NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT"
                .to_string(),
        );
    };

    Ok(consumer_endpoint)
}

/// Sleep for `duration`, but return early if `shutdown` flips to `true`
/// first. Used for the retry-backoff waits below so a shutdown signal that
/// arrives mid-backoff doesn't add the full backoff delay on top of
/// whatever grace period callers budget for joining the supervisor task
/// (see `spawn_event_consumer_supervisor`'s callers).
async fn sleep_or_shutdown(duration: Duration, shutdown: &mut watch::Receiver<bool>) {
    tokio::select! {
        _ = tokio::time::sleep(duration) => {}
        _ = shutdown.wait_for(|stop| *stop) => {
            info!("[SHUTDOWN] Shutdown signal received during retry backoff, stopping event consumption service");
        }
    }
}

async fn run_continuous_consumer_loop(
    outbound: Arc<OutboundClients>,
    runtime: &'static RuntimeConfigArgs,
    consumer_endpoint: String,
    queue_id: &str,
    filter_groups: Vec<crate::notifications::FilterGroup>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        if *shutdown.borrow() {
            info!("[SHUTDOWN] Shutdown signal received, stopping event consumption service");
            return;
        }

        info!("[PLUGIN] Starting event consumer service...");

        match start_event_consumer_service(
            outbound.clone(),
            runtime,
            consumer_endpoint.clone(),
            queue_id,
            filter_groups.clone(),
            shutdown.clone(),
        )
        .await
        {
            Ok(_) => {
                warn!("[WARN] Event service returned unexpectedly");
                warn!("[REFRESH] Restarting event consumer...");
                sleep_or_shutdown(Duration::from_secs(5), &mut shutdown).await;
            }
            Err(e) => {
                error!("[ERROR] Event consumer initialization failed: {}", e);
                warn!("[REFRESH] Retrying in 10 seconds...");
                sleep_or_shutdown(Duration::from_secs(10), &mut shutdown).await;
            }
        }
    }
}

async fn run_bounded_consumer_loop(
    outbound: Arc<OutboundClients>,
    runtime: &'static RuntimeConfigArgs,
    consumer_endpoint: String,
    queue_id: String,
    filter_groups: Vec<crate::notifications::FilterGroup>,
    mut shutdown: watch::Receiver<bool>,
    max_retries: u32,
) {
    let max_retries = max_retries.max(1);
    let mut retry_count = 0;

    loop {
        if *shutdown.borrow() {
            info!("[SHUTDOWN] Shutdown signal received, stopping event consumption service");
            return;
        }

        info!(
            "[PLUGIN] Attempting to start event consumer (attempt {}/{})",
            retry_count + 1,
            max_retries
        );

        match start_event_consumer_service(
            outbound.clone(),
            runtime,
            consumer_endpoint.clone(),
            &queue_id,
            filter_groups.clone(),
            shutdown.clone(),
        )
        .await
        {
            Ok(_) => {
                info!("[OK] Event consumption completed normally");
                return;
            }
            Err(e) => {
                error!("[ERROR] Event consumption failed: {}", e);
                retry_count += 1;

                if retry_count >= max_retries {
                    error!("[STOP] Max retries exceeded, disabling event consumption");
                    return;
                }

                let retry_delay = 2u64.pow(retry_count.min(5));
                warn!("[REFRESH] Retrying in {} seconds...", retry_delay);
                sleep_or_shutdown(Duration::from_secs(retry_delay), &mut shutdown).await;
            }
        }
    }
}

/// Shared event-consumer supervisor wiring used by HTTP and gRPC binaries.
///
/// Returns `Err` if this binary is the assigned, enabled event consumer but
/// required configuration (`NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT` /
/// `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`) is missing — callers should log
/// and exit, the same way they already handle `build_outbound_clients`
/// errors, rather than let the process continue in a half-configured state.
///
/// On success, returns the supervisor's `JoinHandle` so callers can await it
/// during shutdown to confirm the background task actually drained rather
/// than being silently dropped when the process exits. Returns `Ok(None)`
/// for the three cases where no task was spawned at all (event consumption
/// disabled, or this binary/service isn't the assigned consumer).
pub async fn spawn_event_consumer_supervisor(
    outbound: Arc<OutboundClients>,
    shutdown: watch::Receiver<bool>,
    cfg: EventConsumerSupervisorConfig,
) -> Result<Option<tokio::task::JoinHandle<()>>, String> {
    if cfg.runtime.event_consumer.disable_event_consumption {
        info!("[LOG] Event consumption DISABLED via DISABLE_EVENT_CONSUMPTION=true");
        info!(
            "[LOG] This {} service will only serve API requests",
            service_name_for_logs(cfg.service)
        );
        return Ok(None);
    }

    if resolve_consumer_assignment(cfg).is_none() {
        return Ok(None);
    }

    let consumer_endpoint = event_consumer_env_or_skip(cfg.runtime)?;

    // For the assigned consumer, provision the durable queue synchronously
    // before spawning the consume loop. A permanent failure returns
    // `Err(String)` here so the binary's `main` aborts startup — the same
    // fail-fast contract as `event_consumer_env_or_skip` above. This is
    // deliberately NOT inside either spawned loop nor the per-reconnect
    // `connect_grpc` path. The config helper (`resolve_queue_provisioning`) is
    // the single resolution point for prefix validation and StorageEventQueue's
    // derived id: it takes `notifications_queue_resource_prefixes: Option<Vec<String>>`
    // into an owned `Vec<String>` or an `Err` naming the missing env var, so
    // there is no `Option` unwrap/panic here.
    let QueueProvisioning {
        queue_id,
        resource_prefixes,
    } = cfg
        .runtime
        .endpoints
        .resolve_queue_provisioning()
        .map_err(|err| err.to_string())?;

    // Build filter_groups once here; both the provisioning call and the
    // EventConsumerConfig (for NOT_FOUND re-provisioning) use the same set.
    let filter_groups =
        crate::notifications::provisioning::build_storage_filter_groups(&resource_prefixes);

    info!("[LISTEN] Starting background event consumption service");
    info!("[TARGET] Queue ID: {}", queue_id);

    let Some(channel) = outbound.event_consumer_channel.clone() else {
        error!("[ERROR] Event-consumer channel unavailable for durable-queue provisioning");
        return Err(
            "event-consumer channel is not initialized (NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT \
             is set but the outbound channel was not built); cannot provision the durable queue"
                .to_string(),
        );
    };

    // Provisioning must take its own shutdown clone: `shutdown` is moved
    // into the spawned consume loop(s) below.
    let mut provision_shutdown = shutdown.clone();

    let outcome = ensure_durable_queue(
        channel,
        outbound.interceptor.clone(),
        outbound.refresher.clone(),
        &queue_id,
        filter_groups.clone(),
        &mut provision_shutdown,
        &cfg.runtime.stream,
    )
    .await;

    match outcome {
        Ok(EnsureQueueOutcome::Created) => {
            info!(created = true, queue_id = %queue_id, "[PROVISION] Durable queue created");
        }
        Ok(EnsureQueueOutcome::AlreadyExisted) => {
            info!(created = false, queue_id = %queue_id, "[PROVISION] Durable queue already existed");
        }
        Ok(EnsureQueueOutcome::ShutdownAborted) => {
            info!(
                queue_id = %queue_id,
                "[PROVISION] Durable-queue provisioning aborted due to shutdown; not starting consumer"
            );
            return Ok(None);
        }
        Err(err) => {
            let code = match &err {
                NotificationError::QueueProvisioningFailed { status, .. } => Some(status.code()),
                _ => None,
            };
            match code {
                Some(tonic::Code::PermissionDenied) => {
                    // Backward-compat: a consume-only identity may lack
                    // create permission. Attach to the presumed
                    // pre-existing queue instead of aborting startup.
                    warn!(
                        queue_id = %queue_id,
                        "[PROVISION] Durable-queue provisioning skipped: permission denied. \
                         Attaching to presumed pre-existing queue and continuing."
                    );
                }
                _ => {
                    error!(
                        queue_id = %queue_id,
                        "[PROVISION] Durable-queue provisioning failed; aborting startup: {}",
                        err
                    );
                    return Err(err.to_string());
                }
            }
        }
    }

    let handle = match cfg.retry_policy {
        EventConsumerRetryPolicy::Continuous => tokio::spawn(async move {
            let mut shutdown = shutdown;
            loop {
                if *shutdown.borrow() {
                    info!(
                        "[SHUTDOWN] Shutdown signal received, stopping event consumption service"
                    );
                    return;
                }

                let shutdown_inner = shutdown.clone();
                let queue_id_inner = queue_id.clone();
                let consumer_endpoint_inner = consumer_endpoint.clone();
                let outbound_inner = outbound.clone();
                let runtime_inner = cfg.runtime;
                let filter_groups_inner = filter_groups.clone();

                let handle = tokio::spawn(async move {
                    run_continuous_consumer_loop(
                        outbound_inner,
                        runtime_inner,
                        consumer_endpoint_inner,
                        &queue_id_inner,
                        filter_groups_inner,
                        shutdown_inner,
                    )
                    .await;
                });

                match handle.await {
                    Ok(()) => return,
                    Err(e) => {
                        error!("[PANIC] Event consumer task panicked: {:?}", e);
                        warn!("[RECOVERY] Restarting event consumer in 10s...");
                        sleep_or_shutdown(Duration::from_secs(10), &mut shutdown).await;
                    }
                }
            }
        }),
        EventConsumerRetryPolicy::BoundedExponential { max_retries } => tokio::spawn(async move {
            run_bounded_consumer_loop(
                outbound,
                cfg.runtime,
                consumer_endpoint,
                queue_id,
                filter_groups,
                shutdown,
                max_retries,
            )
            .await;
        }),
    };

    Ok(Some(handle))
}

/// Log the already-parsed/validated `THUMBNAIL_SIZES` once at startup.
///
/// `THUMBNAIL_SIZES` and the other `THUMBNAIL_*` specification defaults are
/// parsed and validated as part of `RuntimeConfigArgs` itself (see
/// `config.rs`); this only logs the result for startup visibility.
pub fn initialize_thumbnail_runtime(runtime: &RuntimeConfigArgs) -> Result<(), String> {
    let sizes = &runtime.thumbnail_defaults.thumbnail_sizes;
    info!(
        "[LOG] THUMBNAIL_SIZES parsed ({} entries): {:?}",
        sizes.len(),
        sizes.as_slice()
    );

    Ok(())
}

/// Build a shutdown sender/receiver pair and install signal listeners that
/// flip it to `true` for graceful shutdown paths: Ctrl+C (SIGINT) everywhere,
/// plus SIGTERM on Unix. Kubernetes sends SIGTERM (not SIGINT) when
/// terminating a pod, so both must be handled for graceful shutdown to
/// actually run there rather than the process being force-killed after the
/// grace period.
///
/// Returns the `Sender` (not just clones of the `Receiver`) so callers can
/// also trigger the same graceful-shutdown path themselves — e.g. when one
/// of two concurrently-running servers fails, so the other gets a real
/// chance to drain instead of being abruptly cancelled.
pub fn shutdown_receiver_with_signals() -> (watch::Sender<bool>, watch::Receiver<bool>) {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let tx_for_signal_task = shutdown_tx.clone();

    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};

            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(sigterm) => sigterm,
                Err(err) => {
                    error!("[ERROR] Failed to install SIGTERM listener: {}", err);
                    if tokio::signal::ctrl_c().await.is_ok() {
                        info!("[SHUTDOWN] Received SIGINT, stopping gracefully...");
                        let _ = tx_for_signal_task.send(true);
                    }
                    return;
                }
            };

            tokio::select! {
                result = tokio::signal::ctrl_c() => {
                    if let Err(err) = result {
                        error!("[ERROR] Failed to listen for SIGINT: {}", err);
                        return;
                    }
                    info!("[SHUTDOWN] Received SIGINT, stopping gracefully...");
                }
                _ = sigterm.recv() => {
                    info!("[SHUTDOWN] Received SIGTERM, stopping gracefully...");
                }
            }
            let _ = tx_for_signal_task.send(true);
        }

        #[cfg(not(unix))]
        {
            match tokio::signal::ctrl_c().await {
                Ok(()) => {
                    info!("[SHUTDOWN] Received SIGINT, stopping gracefully...");
                    let _ = tx_for_signal_task.send(true);
                }
                Err(err) => {
                    error!("[ERROR] Failed to listen for shutdown signal: {}", err);
                }
            }
        }
    });

    (shutdown_tx, shutdown_rx)
}

fn log_stream_runtime_config(runtime: &RuntimeConfigArgs) {
    let stream = &runtime.stream;
    info!(
        "[CONFIG] stream.max_concurrent = {}",
        stream.event_processing_max_concurrent
    );
    info!(
        "[CONFIG] stream.max_reconnect_backoff_seconds = {}",
        stream.max_reconnect_backoff_seconds
    );
    info!(
        "[CONFIG] stream.reconnect_base_backoff_seconds = {}",
        stream.reconnect_base_backoff_seconds
    );
    info!(
        "[CONFIG] stream.reconnect_backoff_exponent_cap = {}",
        stream.reconnect_backoff_exponent_cap
    );
    info!(
        "[CONFIG] stream.health_warning_interval = {}",
        stream.health_warning_interval
    );
    info!(
        "[CONFIG] stream.task_cleanup_timeout_secs = {}",
        stream.task_cleanup_timeout_secs
    );
    info!(
        "[CONFIG] stream.stream_timeout_secs = {}",
        stream.stream_timeout_secs
    );
    info!(
        "[CONFIG] stream.event_task_submission_max_retry_age = {}s",
        stream.event_task_submission_max_retry_age.as_secs()
    );
}

/// Shared event-consumer startup path used by service binaries.
pub async fn start_event_consumer_service(
    outbound: Arc<OutboundClients>,
    runtime: &'static RuntimeConfigArgs,
    consumer_endpoint: String,
    queue_id: &str,
    filter_groups: Vec<crate::notifications::FilterGroup>,
    shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    info!("[INIT] Initializing Event Consumer Configuration");

    let config = EventConsumerConfig {
        endpoint: consumer_endpoint,
        interceptor: outbound.interceptor.clone(),
        refresher: outbound.refresher.clone(),
        token_gate: outbound.token_gate.clone(),
        filter_groups,
    };
    let mut consumer = EventConsumer::new(config);

    info!("[PLUGIN] Connecting to notification service...");
    consumer.connect_grpc().await.map_err(|e| {
        error!("[ERROR] Failed to connect to notification service: {}", e);
        format!("Connection failed: {}", e)
    })?;

    info!("[OK] Successfully connected to notification service");

    log_stream_runtime_config(runtime);

    info!("[LISTEN] Starting event service with graceful shutdown support...");
    info!("[SIGNAL] Will log all received events. Press Ctrl+C for graceful shutdown.");
    info!("[DEBUG] Set RUST_LOG=debug for raw protobuf event structures");

    consumer
        .start_event_service(queue_id, shutdown, runtime, outbound)
        .await
        .map_err(|e| {
            error!("[ERROR] Event service failed: {}", e);
            format!("Event service error: {}", e)
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT` is required once a binary is
    /// confirmed as the assigned, enabled event consumer. StorageEventQueue's id
    /// is derived from prefixes.
    #[test]
    #[serial_test::serial]
    fn missing_consumer_endpoint_fails() {
        let saved_consumer_endpoint =
            crate::config::raw_env_value("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT");

        std::env::remove_var("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT");
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let err = event_consumer_env_or_skip(&runtime)
            .expect_err("missing NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT must be an Err");
        assert!(err.contains("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT"));

        std::env::set_var("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT", "localhost:50002");
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let consumer_endpoint =
            event_consumer_env_or_skip(&runtime).expect("endpoint present must succeed");
        assert_eq!(consumer_endpoint, "localhost:50002");

        match saved_consumer_endpoint {
            Some(v) => std::env::set_var("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT", v),
            None => std::env::remove_var("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT"),
        }
    }

    /// The unified `server` binary has only one process, so it always
    /// mirrors whichever role `EVENT_CONSUMER_SERVER` parses to (see
    /// `server.rs`) rather than hardcoding `Http`. Before that fix,
    /// `EVENT_CONSUMER_SERVER=GRPC` fell into the `EventConsumerServer::Grpc`
    /// arm below (this binary "isn't" Grpc if it hardcodes Http) and silently
    /// skipped consumption. Locks in that HTTP/GRPC/NONE all resolve the way
    /// a single self-assigning process expects.
    #[test]
    #[serial_test::serial]
    fn self_assigned_role_resolves_consumption_correctly_for_http_grpc_and_none() {
        let saved = crate::config::raw_env_value("EVENT_CONSUMER_SERVER");

        for (env_value, expected) in [
            ("HTTP", Some(EventConsumerServer::Http)),
            ("GRPC", Some(EventConsumerServer::Grpc)),
            ("NONE", None),
        ] {
            std::env::set_var("EVENT_CONSUMER_SERVER", env_value);
            let runtime: &'static RuntimeConfigArgs = Box::leak(Box::new(
                crate::tests::parse_runtime_config_from_env_for_tests(),
            ));

            let assignment = resolve_consumer_assignment(EventConsumerSupervisorConfig {
                runtime,
                service: runtime.event_consumer.event_consumer_server,
                retry_policy: EventConsumerRetryPolicy::Continuous,
            });

            assert_eq!(
                assignment, expected,
                "EVENT_CONSUMER_SERVER={env_value} should resolve to {expected:?}"
            );
        }

        match saved {
            Some(v) => std::env::set_var("EVENT_CONSUMER_SERVER", v),
            None => std::env::remove_var("EVENT_CONSUMER_SERVER"),
        }
    }

    /// `spawn_event_consumer_supervisor` must return `Ok(None)` for every
    /// early-return path (disabled outright, or not this binary's assigned
    /// role) so callers can tell "no task was spawned" apart from "a task is
    /// running and should be joined at shutdown" — and `Ok(Some(_))` for the
    /// two paths that actually spawn one, regardless of retry policy.
    #[tokio::test]
    #[serial_test::serial]
    async fn spawn_event_consumer_supervisor_reports_whether_a_task_was_spawned() {
        use crate::test_fixtures::mock_consumer_server::{CreateBehavior, MockConsumerServer};

        let saved_disable = crate::config::raw_env_value("DISABLE_EVENT_CONSUMPTION");
        let saved_server = crate::config::raw_env_value("EVENT_CONSUMER_SERVER");
        let saved_consumer_endpoint =
            crate::config::raw_env_value("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT");
        let saved_resource_prefix =
            crate::config::raw_env_value("NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES");

        // A live consumer server that always reports the queue created, so the
        // (now unconditional) durable-queue provisioning step succeeds and the
        // `Ok(Some(_))` outcome is decided solely by the retry policy.
        let server = MockConsumerServer::start(vec![CreateBehavior::Created(true)]).await;
        let endpoint = format!("http://{}", server.addr);

        let mut outbound = crate::OutboundClients::for_tests();
        outbound.event_consumer_channel = Some(server.channel().await);
        let outbound = std::sync::Arc::new(outbound);
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);

        // Early return: event consumption disabled outright.
        std::env::set_var("DISABLE_EVENT_CONSUMPTION", "true");
        std::env::remove_var("EVENT_CONSUMER_SERVER");
        let runtime: &'static RuntimeConfigArgs = Box::leak(Box::new(
            crate::tests::parse_runtime_config_from_env_for_tests(),
        ));
        let result = spawn_event_consumer_supervisor(
            outbound.clone(),
            shutdown_rx.clone(),
            EventConsumerSupervisorConfig {
                runtime,
                service: EventConsumerServer::Http,
                retry_policy: EventConsumerRetryPolicy::Continuous,
            },
        )
        .await;
        assert!(
            matches!(result, Ok(None)),
            "disabled event consumption must spawn nothing: {result:?}"
        );

        // Early return: enabled, but this service isn't the assigned consumer.
        std::env::set_var("DISABLE_EVENT_CONSUMPTION", "false");
        std::env::set_var("EVENT_CONSUMER_SERVER", "GRPC");
        let runtime: &'static RuntimeConfigArgs = Box::leak(Box::new(
            crate::tests::parse_runtime_config_from_env_for_tests(),
        ));
        let result = spawn_event_consumer_supervisor(
            outbound.clone(),
            shutdown_rx.clone(),
            EventConsumerSupervisorConfig {
                runtime,
                service: EventConsumerServer::Http,
                retry_policy: EventConsumerRetryPolicy::Continuous,
            },
        )
        .await;
        assert!(
            matches!(result, Ok(None)),
            "unassigned service must spawn nothing: {result:?}"
        );

        // Spawn path: assigned, enabled, valid config, Continuous policy.
        // Durable-queue provisioning is always performed now, so point the
        // outbound channel at the mock server (above) and supply the required
        // resource prefixes; provisioning succeeds and the
        // `Ok(Some(_))` outcome is decided solely by the retry policy.
        std::env::set_var("EVENT_CONSUMER_SERVER", "HTTP");
        std::env::set_var("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT", &endpoint);
        std::env::set_var(
            "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
            "omniverse://storage/projects/my-project/",
        );
        let runtime: &'static RuntimeConfigArgs = Box::leak(Box::new(
            crate::tests::parse_runtime_config_from_env_for_tests(),
        ));
        let result = spawn_event_consumer_supervisor(
            outbound.clone(),
            shutdown_rx.clone(),
            EventConsumerSupervisorConfig {
                runtime,
                service: EventConsumerServer::Http,
                retry_policy: EventConsumerRetryPolicy::Continuous,
            },
        )
        .await;
        assert!(
            matches!(result, Ok(Some(_))),
            "assigned+enabled Continuous policy must spawn a task: {result:?}"
        );
        let _ = _shutdown_tx.send(true);
        let handle = result
            .expect("Continuous policy must spawn")
            .expect("Continuous policy must return a JoinHandle");
        let joined = tokio::time::timeout(Duration::from_secs(5), handle).await;
        assert!(
            joined.is_ok(),
            "Continuous supervisor must join within the timeout after shutdown, not leak: {joined:?}"
        );

        // Spawn path: assigned, enabled, valid config, BoundedExponential policy.
        let (shutdown_tx2, shutdown_rx2) = watch::channel(false);
        let result = spawn_event_consumer_supervisor(
            outbound,
            shutdown_rx2,
            EventConsumerSupervisorConfig {
                runtime,
                service: EventConsumerServer::Http,
                retry_policy: EventConsumerRetryPolicy::BoundedExponential { max_retries: 1 },
            },
        )
        .await;
        assert!(
            matches!(result, Ok(Some(_))),
            "assigned+enabled BoundedExponential policy must spawn a task: {result:?}"
        );
        let _ = shutdown_tx2.send(true);
        let handle = result
            .expect("BoundedExponential policy must spawn")
            .expect("BoundedExponential policy must return a JoinHandle");
        let joined = tokio::time::timeout(Duration::from_secs(5), handle).await;
        assert!(
            joined.is_ok(),
            "BoundedExponential supervisor must join within the timeout after shutdown, not leak: {joined:?}"
        );

        server.shutdown().await;

        match saved_disable {
            Some(v) => std::env::set_var("DISABLE_EVENT_CONSUMPTION", v),
            None => std::env::remove_var("DISABLE_EVENT_CONSUMPTION"),
        }
        match saved_server {
            Some(v) => std::env::set_var("EVENT_CONSUMER_SERVER", v),
            None => std::env::remove_var("EVENT_CONSUMER_SERVER"),
        }
        match saved_consumer_endpoint {
            Some(v) => std::env::set_var("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT", v),
            None => std::env::remove_var("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT"),
        }
        match saved_resource_prefix {
            Some(v) => std::env::set_var("NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES", v),
            None => std::env::remove_var("NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES"),
        }
    }

    /// Provisioning that fails with `PermissionDenied` is the backward-compat
    /// safety valve for a consume-only identity that lacks queue-create
    /// permission: the supervisor must warn-and-continue (attach to the
    /// presumed pre-existing queue) rather than abort startup. A regression here
    /// would silently start a consumer against a non-existent queue, so this
    /// exercises the whole glue — a live server returning `PermissionDenied`,
    /// the real `ensure_durable_queue` wrapper, and the supervisor's fallback —
    /// and asserts a task is still spawned.
    #[tokio::test]
    #[serial_test::serial]
    async fn supervisor_continues_and_spawns_when_provisioning_is_permission_denied() {
        use crate::test_fixtures::mock_consumer_server::{CreateBehavior, MockConsumerServer};

        let saved_disable = crate::config::raw_env_value("DISABLE_EVENT_CONSUMPTION");
        let saved_server = crate::config::raw_env_value("EVENT_CONSUMER_SERVER");
        let saved_consumer_endpoint =
            crate::config::raw_env_value("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT");
        let saved_resource_prefix =
            crate::config::raw_env_value("NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES");

        // A live consumer server that denies queue creation. `PermissionDenied`
        // is terminal (no bounded retry), so it is hit exactly once.
        let server =
            MockConsumerServer::start(vec![CreateBehavior::Fail(tonic::Code::PermissionDenied)])
                .await;
        let endpoint = format!("http://{}", server.addr);

        // `for_tests()` wires a static-token interceptor + no-op refresher; only
        // the event-consumer channel must point at the mock so provisioning
        // actually reaches it.
        let mut outbound = crate::OutboundClients::for_tests();
        outbound.event_consumer_channel = Some(server.channel().await);
        let outbound = std::sync::Arc::new(outbound);

        std::env::set_var("DISABLE_EVENT_CONSUMPTION", "false");
        std::env::set_var("EVENT_CONSUMER_SERVER", "HTTP");
        std::env::set_var("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT", &endpoint);
        std::env::set_var(
            "NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES",
            "omniverse://storage/projects/my-project/",
        );
        let runtime: &'static RuntimeConfigArgs = Box::leak(Box::new(
            crate::tests::parse_runtime_config_from_env_for_tests(),
        ));

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let result = spawn_event_consumer_supervisor(
            outbound,
            shutdown_rx,
            EventConsumerSupervisorConfig {
                runtime,
                service: EventConsumerServer::Http,
                retry_policy: EventConsumerRetryPolicy::Continuous,
            },
        )
        .await;

        assert!(
            matches!(result, Ok(Some(_))),
            "PermissionDenied provisioning must warn-and-continue, still spawning the consumer: {result:?}"
        );
        assert_eq!(
            server.create_call_count(),
            1,
            "PermissionDenied is terminal — CreateDurableQueue must be attempted exactly once"
        );

        let _ = shutdown_tx.send(true);
        let handle = result
            .expect("supervisor must return Ok")
            .expect("supervisor must have spawned a task");
        let joined = tokio::time::timeout(Duration::from_secs(5), handle).await;
        assert!(
            joined.is_ok(),
            "supervisor task must join within the timeout after shutdown, not leak: {joined:?}"
        );

        server.shutdown().await;

        match saved_disable {
            Some(v) => std::env::set_var("DISABLE_EVENT_CONSUMPTION", v),
            None => std::env::remove_var("DISABLE_EVENT_CONSUMPTION"),
        }
        match saved_server {
            Some(v) => std::env::set_var("EVENT_CONSUMER_SERVER", v),
            None => std::env::remove_var("EVENT_CONSUMER_SERVER"),
        }
        match saved_consumer_endpoint {
            Some(v) => std::env::set_var("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT", v),
            None => std::env::remove_var("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT"),
        }
        match saved_resource_prefix {
            Some(v) => std::env::set_var("NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES", v),
            None => std::env::remove_var("NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES"),
        }
    }
}

#[cfg(test)]
mod sleep_or_shutdown_tests {
    use super::sleep_or_shutdown;
    use std::time::{Duration, Instant};
    use tokio::sync::watch;

    /// A shutdown arriving mid-backoff returns immediately instead of serving out
    /// the delay.
    ///
    /// The delay matters: the reconnect backoff reaches a 60s ceiling, and callers
    /// budget a much shorter grace for joining the supervisor. Sleeping through it
    /// means the join times out and the process is killed rather than stopping,
    /// which reads as a broken shutdown rather than a slow one.
    #[tokio::test]
    async fn a_shutdown_mid_backoff_returns_early() {
        let (tx, mut rx) = watch::channel(false);

        let waiter = tokio::spawn(async move {
            let started = Instant::now();
            sleep_or_shutdown(Duration::from_secs(60), &mut rx).await;
            started.elapsed()
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(true).expect("the waiter still holds the receiver");

        let waited = waiter.await.expect("the waiter must not panic");
        assert!(
            waited < Duration::from_secs(5),
            "a shutdown mid-backoff must not serve out the 60s delay; waited {waited:?}"
        );
    }

    /// A shutdown already signalled before the wait starts returns at once, rather
    /// than requiring an edge while the wait is in progress.
    #[tokio::test]
    async fn a_shutdown_already_set_returns_without_waiting() {
        let (_tx, mut rx) = watch::channel(true);

        let started = Instant::now();
        sleep_or_shutdown(Duration::from_secs(60), &mut rx).await;

        assert!(
            started.elapsed() < Duration::from_secs(5),
            "an already-set shutdown must not wait; waited {:?}",
            started.elapsed()
        );
    }

    /// Without a shutdown it still sleeps, so the early return cannot be an
    /// unconditional one that removes the backoff entirely.
    #[tokio::test]
    async fn without_a_shutdown_the_full_duration_elapses() {
        let (_tx, mut rx) = watch::channel(false);

        let started = Instant::now();
        sleep_or_shutdown(Duration::from_millis(300), &mut rx).await;

        assert!(
            started.elapsed() >= Duration::from_millis(250),
            "the backoff must actually back off; waited {:?}",
            started.elapsed()
        );
    }
}
