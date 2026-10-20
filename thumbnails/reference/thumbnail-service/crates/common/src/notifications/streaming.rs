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

//! Event stream management with liveness defenses and redelivery via NACK.
//!
//! At any time while streaming events, an UNAUTHENTICATED status code could be
//! returned — for example if a token expires mid-stream. The client refreshes
//! the token and reconnects. PERMISSION_DENIED (missing queue consume ACL) is
//! not refreshable: it is logged at error with the queue id and retried after
//! reconnect backoff, not spun through immediate token-refresh reconnects.
//!
//! Clients implement automatic reconnection on UNAVAILABLE errors because cloud
//! load balancers (AWS NLB/ALB, Azure Load Balancer) may terminate idle connections
//! based on their idle timeout settings (typically 60s to 4min). These load
//! balancers operate at Layer 4 (TCP) and do not recognize HTTP/2 PING frames
//! (gRPC keep-alive) as activity, so they may close idle streams even when gRPC
//! keep-alive is configured.
//!
//! Defense layers:
//!   1. Reactive auth — handle UNAUTHENTICATED, refresh token, reconnect;
//!      PERMISSION_DENIED takes the error+backoff path instead. A second
//!      consecutive UNAUTHENTICATED after refresh backs off rather than
//!      hammering the token endpoint (same policy as WorkerEventQueue).
//!   2. Proactive token-expiry reopen — a periodic tick checks whether the
//!      service-identity bearer is close to expiry and refreshes + reopens
//!      before the stream fails with UNAUTHENTICATED. Matches the
//!      permission-service consumer already approved in RC. Deliberately NOT
//!      an idle-silence rebuild: a quiet healthy stream stays open.
//!   3. Transport keepalive on the consumer channel (see connection.rs /
//!      outbound.rs). Keepalive notices a dead peer; it does not count as
//!      activity to an L4 idle reaper.
//!   4. Stream end / non-auth errors — reconnect with backoff (not an immediate
//!      spin), matching WorkerEventQueue's Interrupted classification.
//!   5. NOT_FOUND — the durable queue was deleted underneath the consumer.
//!      Rather than waiting for a pod restart, the consumer re-provisions the
//!      queue with its configured filter groups and resumes consuming. Logged at
//!      warn with the queue id so a repeatedly deleted queue is visible.
//!   6. Negative acknowledgement (NACK) — a retryable failure returns the
//!      delivery to its queue via an explicit NACK on the ack stream, rather than
//!      leaving it unacknowledged and waiting for a reconnect. The broker
//!      redelivers immediately, so spacing is implemented consumer-side: the NACK
//!      is deferred for `nack_redelivery_delay_secs` before being sent. The
//!      delivery key (`resource_identity`, falling back to `resource_address`)
//!      tracks attempts across session boundaries. The recycle is retained in two
//!      narrow roles: as the fallback when the NACK cannot be sent (ack stream
//!      broken), and as a circuit breaker when early redeliveries exceed
//!      `nack_early_redelivery_limit`.

use super::helpers::{
    is_auth_error, is_shutdown_requested, sleep_with_shutdown_check, wait_for_tasks,
};
use super::provisioning::{ensure_durable_queue, EnsureQueueOutcome};
use super::redelivery::{DeferredNacks, NackReason, RedeliveryTracker};
use super::session::{open_ack_stream, reconnect_backoff, send_ack, send_nack, AckStream};
use super::EventConsumer;
use crate::auth::TOKEN_EXPIRY_CHECK_INTERVAL;
use crate::outbound::OutboundClients;
use crate::RuntimeConfigArgs;
use futures_util::stream::StreamExt;
use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::Event as NotificationEvent;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{watch, Mutex, Semaphore};
use tokio::task::JoinSet;
use tokio::time::MissedTickBehavior;
use tracing::{debug, error, info, warn};

/// How long a session teardown may hold the consume call open so NACKs queued by
/// the deferral flush are written before the call is cancelled.
const NACK_FLUSH_GRACE: Duration = Duration::from_secs(2);

/// How long past its next-attempt time a delivery key's spacing state is kept.
///
/// Generous relative to the redelivery delay, since the only cost of keeping a
/// key is a map entry, while dropping one too early loses the spacing for a
/// delivery still on its way back.
const REDELIVERY_KEY_RETENTION: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeliveryProcessingOutcome {
    should_ack: bool,
}

/// Why a durable consume session ended in a way that wants a reconnect.
///
/// Distinguishes first-auth recovery (immediate reopen) from consecutive auth
/// rejection and from proactive token-expiry reopen, so `start_event_service`
/// can apply the same policy WorkerEventQueue encodes in `SessionEnd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionExit {
    Shutdown,
    AuthRefreshed,
    TokenRefreshed,
    /// Re-provisioning needed before reconnecting: the durable queue was deleted.
    NotFound,
    /// Deliberate teardown in one of two narrow roles:
    ///   • Fallback: NACK could not be delivered (ack stream broken).
    ///   • Circuit breaker: early redeliveries of one key exceeded the limit.
    Recycle,
}

fn is_filtered_event_type(event_type: &str) -> bool {
    event_type != super::StorageEventType::Created.as_str()
        && event_type != super::StorageEventType::Deleted.as_str()
}

/// Extract the delivery key from a batch of proto events.
///
/// The key is the logical work identity — `resource_identity` from the first
/// non-filtered event that has one, falling back to `resource_address`. Delivery
/// tags do not survive redelivery; this key does.
fn extract_delivery_key(events: &[NotificationEvent]) -> Option<String> {
    use prost_types::value::Kind;
    for event in events {
        if is_filtered_event_type(&event.event_type) {
            continue;
        }
        let Some(msg) = &event.message else { continue };
        // Prefer resource_identity if present and non-empty.
        if let Some(identity) = msg.fields.get("resource_identity") {
            if let Some(Kind::StringValue(s)) = &identity.kind {
                if !s.is_empty() {
                    return Some(s.clone());
                }
            }
        }
        // Fall back to resource_address.
        if let Some(addr) = msg.fields.get("resource_address") {
            if let Some(Kind::StringValue(s)) = &addr.kind {
                if !s.is_empty() {
                    return Some(s.clone());
                }
            }
        }
    }
    None
}

async fn process_delivery_events_with_handler<F, Fut>(
    events: Vec<NotificationEvent>,
    first_event_num: u64,
    mut handle_event: F,
) -> DeliveryProcessingOutcome
where
    F: FnMut(super::StorageEvent) -> Fut,
    Fut: Future<Output = bool>,
{
    let mut should_ack_delivery = true;

    for (offset, event) in events.into_iter().enumerate() {
        let event_num = first_event_num + offset as u64;
        let timestamp = chrono::Utc::now().format("%H:%M:%S%.3f");
        debug!("RAW EVENT #{} at {}: {:#?}", event_num, timestamp, event);

        if is_filtered_event_type(&event.event_type) {
            debug!(
                "[FILTER] Skipping unwanted event type '{}' (event #{})",
                event.event_type, event_num
            );
            continue;
        }

        let should_ack_event =
            match super::event_processing::convert_grpc_event_to_storage_event(event) {
                Ok(storage_event) => handle_event(storage_event).await,
                Err(e) => {
                    warn!("Failed to convert event #{}: {}", event_num, e);
                    true
                }
            };

        should_ack_delivery &= should_ack_event;
    }

    DeliveryProcessingOutcome {
        should_ack: should_ack_delivery,
    }
}

impl EventConsumer {
    /// Reconnection loop: stream events → on auth error refresh token → on other errors backoff.
    pub async fn start_event_service(
        &mut self,
        queue_id: &str,
        mut shutdown: watch::Receiver<bool>,
        runtime: &'static RuntimeConfigArgs,
        outbound: Arc<OutboundClients>,
    ) -> Result<(), String> {
        let mut reconnect_attempts = 0u32;
        // Consecutive UNAUTHENTICATED recoveries. One is an expired token; several
        // in a row mean refreshing is not the fix (wrong audience/scope) — same
        // counter WorkerEventQueue keeps on `SessionEnd::AuthRefreshed`.
        let mut auth_refreshes: u32 = 0;

        info!("[START] Event service with reconnection loop");

        loop {
            if is_shutdown_requested(&shutdown) {
                info!("[SHUTDOWN] Shutdown requested, stopping event service");
                return Ok(());
            }

            match self
                .consume_durable_events_continuously(
                    queue_id,
                    shutdown.clone(),
                    runtime,
                    outbound.clone(),
                )
                .await
            {
                Ok(SessionExit::Shutdown) => {
                    info!("[SHUTDOWN] Shutdown observed after stream returned, exiting");
                    return Ok(());
                }

                Ok(SessionExit::TokenRefreshed) => {
                    auth_refreshes = 0;
                    reconnect_attempts = 0;
                    info!("[REFRESH] Token near expiry; refreshed and reconnecting immediately...");
                    if self
                        .reconnect_after_refresh(&mut shutdown, runtime, &mut reconnect_attempts)
                        .await?
                    {
                        return Ok(());
                    }
                }

                Ok(SessionExit::AuthRefreshed) => {
                    auth_refreshes = auth_refreshes.saturating_add(1);
                    if auth_refreshes == 1 {
                        reconnect_attempts = 0;
                        info!("[REFRESH] Token refreshed after UNAUTHENTICATED; reconnecting immediately...");
                        if self
                            .reconnect_after_refresh(
                                &mut shutdown,
                                runtime,
                                &mut reconnect_attempts,
                            )
                            .await?
                        {
                            return Ok(());
                        }
                    } else {
                        reconnect_attempts = reconnect_attempts.saturating_add(1);
                        let stream = &runtime.stream;
                        let backoff = reconnect_backoff(
                            reconnect_attempts,
                            stream.reconnect_base_backoff_seconds,
                            stream.reconnect_backoff_exponent_cap,
                            stream.max_reconnect_backoff_seconds,
                        );
                        error!(
                            attempt = reconnect_attempts,
                            consecutive_auth_failures = auth_refreshes,
                            ?backoff,
                            "[RECONNECT] Token was refreshed and rejected again; backing off"
                        );
                        if sleep_with_shutdown_check(backoff, &mut shutdown, "auth backoff").await {
                            return Ok(());
                        }
                        if self
                            .reconnect_after_refresh(
                                &mut shutdown,
                                runtime,
                                &mut reconnect_attempts,
                            )
                            .await?
                        {
                            return Ok(());
                        }
                    }
                }

                Ok(SessionExit::Recycle) => {
                    // Not a failure in the conventional sense: either the ack stream
                    // broke (fallback role) or the circuit breaker fired. Either way,
                    // reconnect without backoff so the redelivery is not additionally
                    // delayed. Matches the worker's SessionEnd::Recycle behavior.
                    auth_refreshes = 0;
                    reconnect_attempts = 0;
                    info!("[RECYCLE] Recycling the consume stream (fallback or circuit breaker)");
                    match self.connect_grpc().await {
                        Ok(_) => {
                            info!("[OK] ✓ Reconnected after recycle");
                        }
                        Err(e) => {
                            error!("[ERROR] ✗ Reconnection after recycle failed: {}", e);
                            // Fall through; the outer loop will retry.
                        }
                    }
                }

                Ok(SessionExit::NotFound) => {
                    // The durable queue was deleted. Re-provision it with the stored
                    // filter groups, then reconnect. Apply backoff on re-provisioning
                    // failure so a deleted-and-gone queue does not spin.
                    //
                    // `reconnect_attempts` is deliberately *not* reset here: a queue
                    // that cannot be re-provisioned returns NOT_FOUND on every
                    // reopen, and resetting would recompute the same first-attempt
                    // backoff forever instead of escalating.
                    auth_refreshes = 0;
                    warn!(
                        queue_id = %queue_id,
                        "[NOT_FOUND] Durable queue missing; re-provisioning before reconnect"
                    );

                    let filter_groups = self.config.filter_groups.clone();
                    if !filter_groups.is_empty() {
                        // Build a fresh one-shot channel for the provisioning call.
                        // `self.config.endpoint` is always available; keepalive isn't
                        // needed for a single short-lived RPC.
                        let endpoint_str = if self.config.endpoint.starts_with("http://")
                            || self.config.endpoint.starts_with("https://")
                        {
                            self.config.endpoint.clone()
                        } else {
                            format!("http://{}", self.config.endpoint)
                        };

                        match tonic::transport::Endpoint::new(endpoint_str)
                            .map_err(|e| e.to_string())
                        {
                            Ok(ep) => match ep.connect().await {
                                Ok(channel) => {
                                    let mut provision_shutdown = shutdown.clone();
                                    match ensure_durable_queue(
                                        channel,
                                        self.config.interceptor.clone(),
                                        self.config.refresher.clone(),
                                        queue_id,
                                        filter_groups,
                                        &mut provision_shutdown,
                                        &runtime.stream,
                                    )
                                    .await
                                    {
                                        Ok(EnsureQueueOutcome::Created) => {
                                            // Recovered: the next NOT_FOUND is a new
                                            // fault, not a continuing one.
                                            reconnect_attempts = 0;
                                            info!(
                                                queue_id = %queue_id,
                                                "[NOT_FOUND] Re-provisioned durable queue; reconnecting"
                                            );
                                        }
                                        Ok(EnsureQueueOutcome::AlreadyExisted) => {
                                            reconnect_attempts = 0;
                                            info!(
                                                queue_id = %queue_id,
                                                "[NOT_FOUND] Durable queue now exists; reconnecting"
                                            );
                                        }
                                        Ok(EnsureQueueOutcome::ShutdownAborted) => {
                                            info!("[SHUTDOWN] Shutdown during re-provisioning; stopping");
                                            return Ok(());
                                        }
                                        Err(e) => {
                                            reconnect_attempts =
                                                reconnect_attempts.saturating_add(1);
                                            let stream = &runtime.stream;
                                            let backoff = reconnect_backoff(
                                                reconnect_attempts,
                                                stream.reconnect_base_backoff_seconds,
                                                stream.reconnect_backoff_exponent_cap,
                                                stream.max_reconnect_backoff_seconds,
                                            );
                                            error!(
                                                queue_id = %queue_id,
                                                attempt = reconnect_attempts,
                                                ?backoff,
                                                "[NOT_FOUND] Re-provisioning failed: {}; backing off",
                                                e
                                            );
                                            if sleep_with_shutdown_check(
                                                backoff,
                                                &mut shutdown,
                                                "not_found re-provisioning backoff",
                                            )
                                            .await
                                            {
                                                return Ok(());
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!(
                                        queue_id = %queue_id,
                                        "[NOT_FOUND] Could not connect for re-provisioning: {}",
                                        e
                                    );
                                }
                            },
                            Err(e) => {
                                error!(
                                    queue_id = %queue_id,
                                    "[NOT_FOUND] Invalid endpoint for re-provisioning: {}",
                                    e
                                );
                            }
                        }
                    } else {
                        // Provisioning with no filter groups is not the fallback it
                        // looks like: empty filters match *every* event of a type, so
                        // `build_storage_filter_groups` asserts against it and
                        // `resolve_queue_provisioning` rejects an empty prefix set at
                        // startup. A consumer here therefore cannot re-provision, and
                        // reconnecting at full speed against a queue that is gone just
                        // spins — so this escalates like any other unrecoverable
                        // reconnect and waits for the queue to reappear externally.
                        reconnect_attempts = reconnect_attempts.saturating_add(1);
                        let stream = &runtime.stream;
                        let backoff = reconnect_backoff(
                            reconnect_attempts,
                            stream.reconnect_base_backoff_seconds,
                            stream.reconnect_backoff_exponent_cap,
                            stream.max_reconnect_backoff_seconds,
                        );
                        error!(
                            queue_id = %queue_id,
                            attempt = reconnect_attempts,
                            ?backoff,
                            "[NOT_FOUND] No filter groups configured, so the queue cannot be \
                             re-provisioned; backing off until it is recreated externally"
                        );
                        if sleep_with_shutdown_check(
                            backoff,
                            &mut shutdown,
                            "not_found without filter groups",
                        )
                        .await
                        {
                            return Ok(());
                        }
                    }

                    if self
                        .reconnect_after_refresh(&mut shutdown, runtime, &mut reconnect_attempts)
                        .await?
                    {
                        return Ok(());
                    }
                }

                Err(e) => {
                    auth_refreshes = 0;
                    reconnect_attempts += 1;
                    // The interruption was already logged at ERROR where the
                    // stream failed (queue id + reason). Keep attempt tracking
                    // at info so one operator-facing ERROR is not doubled here.
                    info!(
                        "[RECONNECT] Stream error already reported (attempt {}): {}",
                        reconnect_attempts, e
                    );

                    let stream = &runtime.stream;
                    let backoff = reconnect_backoff(
                        reconnect_attempts,
                        stream.reconnect_base_backoff_seconds,
                        stream.reconnect_backoff_exponent_cap,
                        stream.max_reconnect_backoff_seconds,
                    );

                    info!(
                        "[RECONNECT] Retrying in {:?} (attempt #{})",
                        backoff, reconnect_attempts
                    );

                    if reconnect_attempts.is_multiple_of(stream.health_warning_interval) {
                        warn!(
                            "[WARN] Event service reconnected {} times. Check network/service health.",
                            reconnect_attempts
                        );
                    }

                    if sleep_with_shutdown_check(backoff, &mut shutdown, "error backoff").await {
                        return Ok(()); // Shutdown requested
                    }

                    match self.connect_grpc().await {
                        Ok(_) => {
                            info!("[OK] ✓ Reconnected after error");
                            reconnect_attempts = 0;
                        }
                        Err(e) => {
                            error!("[ERROR] ✗ Reconnection failed: {}, will retry", e);
                            continue;
                        }
                    }
                }
            }
        }
    }

    /// Immediate reconnect after a successful token refresh. Returns `true` when
    /// shutdown was requested during the optional backoff after a failed connect.
    async fn reconnect_after_refresh(
        &mut self,
        shutdown: &mut watch::Receiver<bool>,
        runtime: &'static RuntimeConfigArgs,
        reconnect_attempts: &mut u32,
    ) -> Result<bool, String> {
        match self.connect_grpc().await {
            Ok(_) => {
                info!("[OK] ✓ Reconnected with refreshed token");
                Ok(false)
            }
            Err(e) => {
                error!("[ERROR] ✗ Reconnection failed: {}, will retry", e);
                *reconnect_attempts = 1;

                if sleep_with_shutdown_check(
                    Duration::from_secs(runtime.stream.reconnect_base_backoff_seconds),
                    shutdown,
                    "backoff after reconnect failure",
                )
                .await
                {
                    return Ok(true);
                }
                Ok(false)
            }
        }
    }

    pub(crate) async fn consume_durable_events_continuously(
        &mut self,
        queue_id: &str,
        mut shutdown: watch::Receiver<bool>,
        runtime: &'static RuntimeConfigArgs,
        outbound: Arc<OutboundClients>,
    ) -> Result<SessionExit, String> {
        info!("[START] Event consumption from queue: {}", queue_id);
        let client = self
            .grpc_client
            .as_mut()
            .ok_or("gRPC client not connected")?;

        // Opened through the shared session helper so the establishment bound, the
        // initial-request shape, and the auth-error classification are the same
        // ones WorkerEventQueue uses. Opening no longer waits for the server to accept the
        // session — the platform withholds response headers until it has an event
        // to write, so waiting means waiting for traffic — and an auth rejection
        // now arrives as the stream's first item, where the loop below already
        // classifies it.
        let session = open_ack_stream(client, queue_id, 32).await;

        let AckStream {
            acks: ack_tx,
            deliveries: mut stream,
        } = match session {
            Ok(session) => session,
            Err(other) => return Err(format!("Failed to start consuming events: {}", other)),
        };

        info!("[OK] Event stream opened. Listening for events...");

        let stream_runtime = &runtime.stream;
        let semaphore = Arc::new(Semaphore::new(
            stream_runtime.event_processing_max_concurrent,
        ));
        let mut events_processed = 0u64;
        let mut events_filtered = 0u64;
        let mut task_set: JoinSet<()> = JoinSet::new();

        // Persistent interval, not a `tokio::time::sleep` recreated each loop
        // iteration: `interval.tick()` tracks its own fixed schedule and stays
        // reliable even when repeatedly raced against other `select!` branches
        // (see module doc comment / heartbeat_tests for why). Drives STATS
        // logging and the token-expiry reopen check.
        let mut periodic_timer = tokio::time::interval(TOKEN_EXPIRY_CHECK_INTERVAL);
        periodic_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
        periodic_timer.tick().await; // Consume the immediate first tick.

        // The STATS line rides the same timer as the token-expiry check above (so a
        // second `tokio::time::interval` isn't needed), but logs on its own, much
        // coarser cadence: every tick would be once per `TOKEN_EXPIRY_CHECK_INTERVAL`
        // (30s), which is far noisier than an operator watching this log needs.
        // `None` logs on the first eligible tick so STATS is still visible shortly
        // after the stream starts, not only after the first full interval.
        const STATS_LOG_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);
        let mut last_stats_logged_at: Option<Instant> = None;

        let stream_started_at = Instant::now();
        // Task 1.1: track time since last delivery and unacknowledged count.
        let mut last_delivery_at: Option<Instant> = None;
        let unacknowledged_count = Arc::new(AtomicUsize::new(0));

        // Redelivery state shared between the main loop (anti-spin gate, circuit
        // breaker) and spawned tasks (clearing on success).
        let tracker = Arc::new(Mutex::new(RedeliveryTracker::new()));

        // Deliveries whose NACK is waiting out the redelivery delay, owned by the
        // main loop rather than slept on inside a spawned task — see
        // `DeferredNacks` for why that distinction is what closes the incident.
        let mut deferred_nacks = DeferredNacks::new(
            Duration::from_secs(stream_runtime.nack_redelivery_delay_secs),
            stream_runtime.nack_deferral_concurrency_limit,
        );
        // Unbounded, and it has to be. A bounded channel deadlocks this loop: the
        // spawned task hands off while still holding its semaphore permit, and the
        // main loop is the only thing that drains the channel — so once the channel
        // is full, the permit holders block on the send, the loop blocks acquiring
        // a permit for the next delivery, and nothing moves again. WorkerEventQueue
        // wedged exactly that way in dev on 2026-08-31, with its capacity tied to
        // its concurrency bound as this was.
        //
        // Unbounded is not unbounded in practice. Only spawned tasks send, each
        // sends at most one tag, and the semaphore caps them at
        // `event_processing_max_concurrent`, so the channel cannot hold more than
        // that before the loop drains it. The bound that matters for held NACKs is
        // `nack_deferral_concurrency_limit` on `deferred_nacks`.
        let (defer_tx, mut defer_rx) = tokio::sync::mpsc::unbounded_channel::<(i64, NackReason)>();

        // Spawned tasks signal the main loop to recycle when their NACK fails
        // (ack stream broken — fallback role). A capacity-1 channel; any extra
        // signals from concurrent tasks are simply dropped (one recycle suffices).
        let (recycle_tx, mut recycle_rx) = tokio::sync::mpsc::channel::<()>(1);

        let nack_delay = Duration::from_secs(stream_runtime.nack_redelivery_delay_secs);
        let deferral_limit = stream_runtime.nack_deferral_concurrency_limit;
        let early_limit = stream_runtime.nack_early_redelivery_limit;

        info!(
            "[DEFENSE] Token-expiry check every {}s; no idle-silence reconnect",
            TOKEN_EXPIRY_CHECK_INTERVAL.as_secs(),
        );
        info!(
            "[NACK] Redelivery delay {}s, deferral limit {}, early-redelivery circuit-breaker {}",
            nack_delay.as_secs(),
            deferral_limit,
            early_limit,
        );

        #[derive(Clone, Copy)]
        enum LoopEnd {
            Shutdown,
            AuthError,
            TokenExpiring,
            StreamEnded,
            /// NOT_FOUND: queue was deleted; needs re-provisioning.
            NotFound,
            /// Recycle: either the NACK failed (fallback) or circuit breaker fired.
            Recycle,
        }

        // Processing capacity is reserved before a delivery is read rather than
        // acquired after one arrives. Awaiting inside the `stream.next()` arm
        // stops every other arm being polled while it waits, because `select!`
        // runs the winning arm to completion — so at the bound, shutdown and the
        // token-expiry reopen check stop happening. Same fix, same reasoning, as
        // `task-worker`'s consume loop.
        let mut reserved: Option<tokio::sync::OwnedSemaphorePermit> = None;

        let ending = loop {
            let next_nack_deadline = deferred_nacks.next_deadline();

            tokio::select! {
                _ = shutdown.changed() => {
                    if is_shutdown_requested(&shutdown) {
                        info!("[SHUTDOWN] Shutdown signal received, stopping event consumption");
                        break LoopEnd::Shutdown;
                    }
                }

                // Spawned task signals: NACK failed → recycle (fallback role).
                _ = recycle_rx.recv() => {
                    warn!("[RECYCLE] NACK could not be delivered; recycling to force redelivery (fallback role)");
                    break LoopEnd::Recycle;
                }

                // A task reported a retryable failure and wants its NACK deferred.
                Some((tag, reason)) = defer_rx.recv() => {
                    if !deferred_nacks.try_defer(tag, reason) {
                        // Task 5.4: at the bound, NACK now rather than holding more.
                        // The anti-spin gate spaces the resulting early redelivery.
                        warn!(
                            delivery_tag = tag,
                            deferred = deferred_nacks.len(),
                            limit = deferral_limit,
                            "[NACK] Deferral concurrency limit reached; NACKing immediately",
                        );
                        unacknowledged_count.fetch_sub(1, Ordering::SeqCst);
                        if send_nack(&ack_tx, tag).await.is_err() {
                            break LoopEnd::Recycle;
                        }
                    }
                }

                // The earliest deferred NACK came due.
                _ = async move {
                    match next_nack_deadline {
                        Some(at) => tokio::time::sleep_until(at.into()).await,
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    let mut ack_stream_broken = false;
                    for (tag, reason) in deferred_nacks.take_due() {
                        unacknowledged_count.fetch_sub(1, Ordering::SeqCst);
                        debug!(
                            delivery_tag = tag,
                            reason = reason.as_label(),
                            "[NACK] Redelivery delay elapsed; negatively acknowledging",
                        );
                        if send_nack(&ack_tx, tag).await.is_err() {
                            ack_stream_broken = true;
                            break;
                        }
                    }
                    if ack_stream_broken {
                        break LoopEnd::Recycle;
                    }
                }

                // Reserve capacity for the next delivery; disabled once held so
                // this arm cannot hoard permits.
                permit = semaphore.clone().acquire_owned(), if reserved.is_none() => {
                    match permit {
                        Ok(permit) => reserved = Some(permit),
                        Err(error) => {
                            error!("[ERROR] Failed to acquire semaphore permit: {}", error);
                            return Err(format!("Semaphore error: {}", error));
                        }
                    }
                }

                stream_result = stream.next(), if reserved.is_some() => {
                    match stream_result {
                        Some(Ok(events_response)) => {
                            let delivery_tag = events_response.delivery_tag;
                            let events = events_response.events;

                            // Task 1.1: record delivery arrival time.
                            last_delivery_at = Some(Instant::now());

                            let first_event_num = events_processed + 1;
                            events_processed += events.len() as u64;
                            let has_handled_event = events
                                .iter()
                                .any(|event| !is_filtered_event_type(&event.event_type));
                            events_filtered += events
                                .iter()
                                .filter(|event| is_filtered_event_type(&event.event_type))
                                .count() as u64;

                            if !has_handled_event {
                                debug!(
                                    "[FILTER] Delivery tag {} contained no handled event types; acking",
                                    delivery_tag
                                );
                                if send_ack(&ack_tx, delivery_tag).await {
                                    debug!("[ACK] Acked filtered delivery_tag {}", delivery_tag);
                                }
                                continue;
                            }

                            // Extract the delivery key before moving `events` into the task.
                            // Task 5.2: key is resource_identity, falling back to resource_address.
                            let delivery_key = extract_delivery_key(&events);

                            // Task 5.5: Anti-spin gate — reject an early redelivery without
                            // re-entering processing, and advance the circuit breaker counter.
                            if let Some(ref key) = delivery_key {
                                let mut t = tracker.lock().await;
                                if t.is_early(key) {
                                    let count = t.record_early(key);
                                    if count >= early_limit {
                                        // Task 5.6: Report the bounded condition.
                                        error!(
                                            delivery_key = %key,
                                            consecutive_early_redeliveries = count,
                                            limit = early_limit,
                                            "[CIRCUIT_BREAKER] Delivery key redelivered early {} times \
                                             (limit {}); recycling session to force spacing",
                                            count, early_limit,
                                        );
                                        // NACK immediately then recycle (circuit-breaker role).
                                        drop(t);
                                        let _ = send_nack(&ack_tx, delivery_tag).await;
                                        break LoopEnd::Recycle;
                                    }
                                    // Task 5.5: reject without reprocessing, and back
                                    // off per key. NACKing an early redelivery
                                    // straight back is itself the spin the gate
                                    // exists to prevent — the broker requeues
                                    // immediately, so an undeferred NACK returns the
                                    // same key at network speed. Deferring it holds
                                    // the round-trip open for the remaining delay.
                                    debug!(
                                        delivery_key = %key,
                                        consecutive_early = count,
                                        "[NACK] Early redelivery of {}; deferring its \
                                         negative acknowledgement without reprocessing",
                                        key,
                                    );
                                    drop(t);
                                    if deferred_nacks.try_defer(delivery_tag, NackReason::EarlyRedelivery) {
                                        unacknowledged_count.fetch_add(1, Ordering::SeqCst);
                                    } else if send_nack(&ack_tx, delivery_tag).await.is_err() {
                                        // At the bound there is nowhere to hold it, so an
                                        // immediate NACK is the only option left.
                                        break LoopEnd::Recycle;
                                    }
                                    continue;
                                }
                                // Not an early redelivery: reset the early count and proceed.
                                t.clear_early_count(key.as_str());
                            }

                            let permit = reserved.take().expect(
                                "the arm guard admits a delivery only with a reservation",
                            );

                            // Backpressure is now visible as capacity, not as time
                            // spent waiting: the read does not happen until a permit
                            // is in hand, so there is no per-delivery wait to
                            // measure. A saturated worker shows up as available
                            // permits sitting at zero.
                            if semaphore.available_permits() == 0 {
                                warn!(
                                    "[BACKPRESSURE] Delivery tag {} consumed the last permit (tasks={}/{})",
                                    delivery_tag,
                                    stream_runtime.event_processing_max_concurrent,
                                    stream_runtime.event_processing_max_concurrent
                                );
                            }

                            let ack_tx = ack_tx.clone();
                            let outbound = outbound.clone();
                            let unacknowledged_count = unacknowledged_count.clone();
                            let tracker_task = tracker.clone();
                            let defer_tx_task = defer_tx.clone();
                            let recycle_tx_task = recycle_tx.clone();

                            task_set.spawn(async move {
                                let task_start = Instant::now();

                                // A delivery tag ACKs the whole response, not an individual
                                // event, so defer the ACK until every event in this delivery has
                                // either been skipped or processed successfully.
                                let outcome = process_delivery_events_with_handler(
                                    events,
                                    first_event_num,
                                    |storage_event| {
                                        super::event_processing::handle_storage_event(
                                            storage_event,
                                            runtime,
                                            outbound.clone(),
                                        )
                                    },
                                )
                                .await;

                                if outcome.should_ack {
                                    // Task 5.2: clear delivery-key state on success.
                                    if let Some(ref key) = delivery_key {
                                        tracker_task.lock().await.clear(key);
                                    }
                                    if send_ack(&ack_tx, delivery_tag).await {
                                        debug!(
                                            "[ACK] Acked delivery_tag {} after successful processing",
                                            delivery_tag
                                        );
                                    }
                                } else {
                                    // Task 5.1: Replace the unacknowledged-and-hope path with NACK.
                                    // Task 5.3: consumer-side delay (no broker-side delay on the operation).
                                    unacknowledged_count.fetch_add(1, Ordering::SeqCst);
                                    warn!(
                                        delivery_tag,
                                        "[NACK] Delivery processing failed; \
                                         scheduling redelivery with {}s delay",
                                        nack_delay.as_secs(),
                                    );

                                    // Task 5.2: set the next-attempt time so early redeliveries
                                    // are caught by the anti-spin gate.
                                    if let Some(ref key) = delivery_key {
                                        tracker_task.lock().await.schedule_next_attempt(key, nack_delay);
                                    }

                                    // Task 5.3/5.4: hand the delay to the main loop. Sleeping here
                                    // would put the NACK behind `task_cleanup_timeout_secs`, which
                                    // aborts this task long before a 60s delay elapses.
                                    // The handoff must not block: this task still holds
                                    // its semaphore permit, and the loop that drains
                                    // this channel is the same one waiting for permits.
                                    if defer_tx_task
                                        .send((delivery_tag, NackReason::RetryableFailure))
                                        .is_err()
                                    {
                                        // Main loop is gone, so nothing will flush the deferral:
                                        // NACK directly and fall back to the recycle if even that
                                        // cannot be delivered (task 5.7, fallback role).
                                        unacknowledged_count.fetch_sub(1, Ordering::SeqCst);
                                        if send_nack(&ack_tx, delivery_tag).await.is_err() {
                                            warn!(
                                                delivery_tag,
                                                "[RECYCLE] NACK for delivery {} could not be sent; \
                                                 signalling recycle (fallback role)",
                                                delivery_tag,
                                            );
                                            let _ = recycle_tx_task.try_send(());
                                        }
                                    }
                                }

                                let task_duration = task_start.elapsed();

                                if task_duration.as_secs() > 2 {
                                    warn!(
                                        "[SLOW] Delivery tag {} took {}ms to process",
                                        delivery_tag,
                                        task_duration.as_millis()
                                    );
                                }

                                drop(permit);
                            });
                        }

                        Some(Err(status)) if is_auth_error(&status) => {
                            info!("[AUTH] Received {} error from event-consumer", status.code());
                            info!("[AUTH] Processed {} events before auth error", events_processed);
                            break LoopEnd::AuthError;
                        }

                        Some(Err(status)) if status.code() == tonic::Code::NotFound => {
                            // Task 1.4: classify NOT_FOUND as its own reason.
                            // Task 1.5: handled in start_event_service (re-provisioning path).
                            warn!(
                                queue_id = %queue_id,
                                "[NOT_FOUND] Durable queue not found; will re-provision before reconnect: {}",
                                status,
                            );
                            info!("[STATS] Processed {} events before queue deletion", events_processed);
                            wait_for_tasks(
                                &mut task_set,
                                "queue not-found condition",
                                stream_runtime.task_cleanup_timeout_secs,
                            )
                            .await;
                            break LoopEnd::NotFound;
                        }

                        Some(Err(status)) => {
                            // PermissionDenied (missing queue ACL) shares this
                            // Err+backoff path with other stream failures — not
                            // the UNAUTHENTICATED refresh+immediate-reconnect path.
                            let message = if status.code() == tonic::Code::PermissionDenied {
                                format!(
                                    "Durable queue subscription failed for queue {queue_id}: {status}"
                                )
                            } else {
                                format!("gRPC stream error: {status}")
                            };
                            // Single ERROR for this interruption; start_event_service
                            // propagates Err(message) and backs off without a second ERROR.
                            error!("[ERROR] {}", message);
                            info!("[STATS] Processed {} events before error", events_processed);
                            wait_for_tasks(
                                &mut task_set,
                                "error condition",
                                stream_runtime.task_cleanup_timeout_secs,
                            )
                            .await;
                            return Err(message);
                        }

                        None => {
                            // Match WorkerEventQueue: a server that closes the stream is a
                            // reconnect-with-backoff case, not an immediate spin
                            // through refresh + Ok(()).
                            info!("[MSG] Stream ended gracefully");
                            break LoopEnd::StreamEnded;
                        }
                    }
                }

                Some(task_result) = task_set.join_next(), if !task_set.is_empty() => {
                    if let Err(e) = task_result {
                        error!("[ERROR] Event processing task panicked: {}", e);
                    }
                }

                // Peer-aligned token-expiry reopen (not idle silence).
                _ = periodic_timer.tick() => {
                    if self.config.token_gate.is_expiring_soon() {
                        info!(
                            "[AUTH] Service-identity token expiring soon; refreshing and reopening the stream"
                        );
                        wait_for_tasks(
                            &mut task_set,
                            "token expiry reopen",
                            stream_runtime.task_cleanup_timeout_secs,
                        )
                        .await;
                        break LoopEnd::TokenExpiring;
                    }

                    let due = last_stats_logged_at
                        .is_none_or(|at| at.elapsed() >= STATS_LOG_INTERVAL);
                    if due {
                        let active_tasks = stream_runtime.event_processing_max_concurrent
                            - semaphore.available_permits();
                        let capacity_pct = (semaphore.available_permits() as f64
                            / stream_runtime.event_processing_max_concurrent as f64)
                            * 100.0;
                        let stream_age_secs = stream_started_at.elapsed().as_secs();
                        // Task 1.2: report time-since-last-delivery and unacknowledged count.
                        let since_last_delivery = last_delivery_at
                            .map(|at| at.elapsed().as_secs())
                            .map(|s| format!("{s}s"))
                            .unwrap_or_else(|| "never".to_string());
                        let unacked = unacknowledged_count.load(Ordering::SeqCst);
                        let deferred = deferred_nacks.len();

                        info!(
                            "[STATS] Total: {} | Filtered: {} | Active: {}/{} ({:.0}% free) | \
                             Pending: {} | Age: {}s | Unacked: {} | Deferred: {} | Since-last-delivery: {}",
                            events_processed,
                            events_filtered,
                            active_tasks,
                            stream_runtime.event_processing_max_concurrent,
                            capacity_pct,
                            task_set.len(),
                            stream_age_secs,
                            unacked,
                            deferred,
                            since_last_delivery,
                        );
                        last_stats_logged_at = Some(Instant::now());

                        // Keys are only removed on success or reschedule, so one
                        // that fails once and never returns would sit in the map
                        // for the life of a session that runs for days.
                        tracker.lock().await.evict_expired(REDELIVERY_KEY_RETENTION);
                    }
                }
            }
        };

        // Drain before dropping the sender, not after. Dropping first ends the
        // request stream, so a task that finished during cleanup could not send
        // its acknowledgement and its delivery came back despite having been
        // processed successfully. WorkerEventQueue's loop had this ordering right; this
        // one did not, which is the kind of divergence two hand-written loops
        // produce and why the pieces underneath them are now shared.
        wait_for_tasks(
            &mut task_set,
            "stream end",
            stream_runtime.task_cleanup_timeout_secs,
        )
        .await;

        // Flush every deferred NACK before the sender goes, including deferrals
        // requested by tasks that finished during cleanup. The remaining delay is
        // abandoned deliberately: the session is ending, so the reconnect already
        // supplies the spacing, and an unsent NACK would leave the delivery
        // in-flight until the broker's acknowledgement timeout.
        defer_rx.close();
        let mut to_flush = deferred_nacks.drain_all();
        while let Ok((tag, reason)) = defer_rx.try_recv() {
            to_flush.push((tag, reason));
        }
        let flushed = !to_flush.is_empty();
        if flushed {
            info!(
                deferred = to_flush.len(),
                "[NACK] Flushing deferred negative acknowledgements before session teardown"
            );
            for (tag, _reason) in to_flush {
                unacknowledged_count.fetch_sub(1, Ordering::SeqCst);
                if send_nack(&ack_tx, tag).await.is_err() {
                    warn!(
                        delivery_tag = tag,
                        "[NACK] Ack stream already closed; delivery returns on reconnect"
                    );
                    break;
                }
            }
        }
        drop(ack_tx);

        // Dropping the response stream cancels the call, which would discard the
        // NACKs just queued on the request stream — leaving those deliveries
        // in-flight until the broker's acknowledgement timeout, the state the
        // flush exists to avoid. Bounded, and only when something was flushed.
        // Sleep rather than awaiting the stream: `stream.next()` returns as soon as
        // the next delivery arrives, so on a busy queue the grace collapsed to
        // nearly nothing and the NACKs were cancelled anyway. Holding here keeps
        // `stream` undropped, which is the part that matters.
        if flushed {
            tokio::time::sleep(NACK_FLUSH_GRACE).await;
        }
        info!(
            "[STATS] Stream session complete: {} events in {:.0}s",
            events_processed,
            stream_started_at.elapsed().as_secs_f64()
        );

        match ending {
            LoopEnd::Shutdown => Ok(SessionExit::Shutdown),
            LoopEnd::StreamEnded => Err("stream ended".to_string()),
            LoopEnd::NotFound => Ok(SessionExit::NotFound),
            LoopEnd::Recycle => Ok(SessionExit::Recycle),
            LoopEnd::AuthError => {
                info!("[AUTH] Refreshing token before reconnection...");
                self.refresher().refresh().await;
                info!("[AUTH] ✓ Refresh attempted after UNAUTHENTICATED");
                Ok(SessionExit::AuthRefreshed)
            }
            LoopEnd::TokenExpiring => {
                info!("[AUTH] Refreshing token before reconnection...");
                self.refresher().refresh().await;
                if self.config.token_gate.is_expiring_soon() {
                    // Same failure guard as WorkerEventQueue: refresh() swallows errors, so a
                    // sick IdP or an unchanged sidecar file must not reset reconnect
                    // attempts and reopen immediately every tick.
                    return Err(
                        "token still near expiry after refresh; backing off before reconnect"
                            .to_string(),
                    );
                }
                info!("[AUTH] ✓ Refresh attempted, ready to reconnect");
                Ok(SessionExit::TokenRefreshed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost_types::{value::Kind, Struct, Value};
    use std::collections::BTreeMap;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn notification_event(event_type: &str, resource_address: &str) -> NotificationEvent {
        let mut fields = BTreeMap::new();
        fields.insert(
            "resource_address".to_string(),
            Value {
                kind: Some(Kind::StringValue(resource_address.to_string())),
            },
        );
        fields.insert(
            "resource_identity".to_string(),
            Value {
                kind: Some(Kind::StringValue("identity-123".to_string())),
            },
        );

        NotificationEvent {
            event_type: event_type.to_string(),
            principal_identity: "test-user".to_string(),
            occurred_at: None,
            published_at: None,
            message: Some(Struct { fields }),
        }
    }

    async fn assert_mixed_delivery_nacks(events: Vec<NotificationEvent>) {
        let handled_created = Arc::new(AtomicUsize::new(0));
        let handled_created_for_handler = Arc::clone(&handled_created);
        let handled_deleted = Arc::new(AtomicUsize::new(0));
        let handled_deleted_for_handler = Arc::clone(&handled_deleted);

        let outcome = process_delivery_events_with_handler(events, 1, move |storage_event| {
            let handled_created = Arc::clone(&handled_created_for_handler);
            let handled_deleted = Arc::clone(&handled_deleted_for_handler);
            async move {
                match super::super::StorageEventType::from_str(&storage_event.event_type) {
                    Some(super::super::StorageEventType::Created) => {
                        handled_created.fetch_add(1, Ordering::SeqCst);
                        false
                    }
                    Some(super::super::StorageEventType::Deleted) => {
                        handled_deleted.fetch_add(1, Ordering::SeqCst);
                        true
                    }
                    event_type => panic!("unexpected handled event type: {event_type:?}"),
                }
            }
        })
        .await;

        assert!(
            !outcome.should_ack,
            "a retryable CREATE failure must keep the whole delivery unacked"
        );
        assert_eq!(handled_created.load(Ordering::SeqCst), 1);
        assert_eq!(handled_deleted.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn delivery_processing_does_not_ack_mixed_delivery_when_created_event_nacks() {
        assert_mixed_delivery_nacks(vec![
            notification_event(
                super::super::StorageEventType::Deleted.as_str(),
                "omniverse://storage/projects/deleted.png",
            ),
            notification_event(
                super::super::StorageEventType::Created.as_str(),
                "omniverse://storage/projects/image.png",
            ),
        ])
        .await;

        assert_mixed_delivery_nacks(vec![
            notification_event(
                super::super::StorageEventType::Created.as_str(),
                "omniverse://storage/projects/image.png",
            ),
            notification_event(
                super::super::StorageEventType::Deleted.as_str(),
                "omniverse://storage/projects/deleted.png",
            ),
        ])
        .await;
    }

    #[tokio::test]
    async fn delivery_processing_acks_delivery_with_only_filtered_events() {
        let outcome = process_delivery_events_with_handler(
            vec![notification_event(
                super::super::StorageEventType::DirDeleted.as_str(),
                "omniverse://storage/projects/deleted/",
            )],
            1,
            |_storage_event| async { panic!("filtered event should not be handled") },
        )
        .await;

        assert!(outcome.should_ack);
    }

    #[tokio::test]
    async fn delivery_processing_routes_storage_deleted_events_to_the_handler() {
        let handled = Arc::new(AtomicUsize::new(0));
        let handled_for_handler = Arc::clone(&handled);
        let mut deleted_event = notification_event(
            super::super::StorageEventType::Deleted.as_str(),
            "https://examplestorage.blob.core.windows.net/example/test-folder/JfwQGlj2.jpg",
        );
        deleted_event
            .message
            .as_mut()
            .expect("test event has a message")
            .fields
            .remove("resource_identity");

        let outcome = process_delivery_events_with_handler(
            vec![deleted_event],
            1,
            move |storage_event| {
                let handled = Arc::clone(&handled_for_handler);
                async move {
                    assert_eq!(
                        storage_event.event_type,
                        super::super::StorageEventType::Deleted.as_str()
                    );
                    assert_eq!(
                        storage_event.message.resource_address,
                        "https://examplestorage.blob.core.windows.net/example/test-folder/JfwQGlj2.jpg"
                    );
                    assert!(storage_event.message.resource_identity.is_none());
                    handled.fetch_add(1, Ordering::SeqCst);
                    true
                }
            },
        )
        .await;

        assert!(outcome.should_ack);
        assert_eq!(handled.load(Ordering::SeqCst), 1);
    }

    // `RedeliveryTracker` and `DeferredNacks` are covered in
    // `notifications::redelivery`, where both queues' copies of this logic now live.

    #[tokio::test]
    async fn extract_delivery_key_prefers_resource_identity() {
        use prost_types::{value::Kind, Struct, Value};
        use std::collections::BTreeMap;
        let mut fields = BTreeMap::new();
        fields.insert(
            "resource_identity".to_string(),
            Value {
                kind: Some(Kind::StringValue("identity-xyz".to_string())),
            },
        );
        fields.insert(
            "resource_address".to_string(),
            Value {
                kind: Some(Kind::StringValue("omniverse://bucket/file.usd".to_string())),
            },
        );
        let event = NotificationEvent {
            event_type: super::super::StorageEventType::Created.as_str().to_string(),
            principal_identity: "test".to_string(),
            occurred_at: None,
            published_at: None,
            message: Some(Struct { fields }),
        };
        assert_eq!(
            extract_delivery_key(&[event]),
            Some("identity-xyz".to_string())
        );
    }

    #[tokio::test]
    async fn extract_delivery_key_falls_back_to_resource_address() {
        use prost_types::{value::Kind, Struct, Value};
        use std::collections::BTreeMap;
        let mut fields = BTreeMap::new();
        fields.insert(
            "resource_address".to_string(),
            Value {
                kind: Some(Kind::StringValue("omniverse://bucket/file.usd".to_string())),
            },
        );
        let event = NotificationEvent {
            event_type: super::super::StorageEventType::Created.as_str().to_string(),
            principal_identity: "test".to_string(),
            occurred_at: None,
            published_at: None,
            message: Some(Struct { fields }),
        };
        assert_eq!(
            extract_delivery_key(&[event]),
            Some("omniverse://bucket/file.usd".to_string())
        );
    }

    #[tokio::test]
    async fn extract_delivery_key_skips_filtered_events() {
        use prost_types::{value::Kind, Struct, Value};
        use std::collections::BTreeMap;
        let mut fields = BTreeMap::new();
        fields.insert(
            "resource_identity".to_string(),
            Value {
                kind: Some(Kind::StringValue("filtered-identity".to_string())),
            },
        );
        let filtered_event = NotificationEvent {
            event_type: super::super::StorageEventType::DirDeleted
                .as_str()
                .to_string(),
            principal_identity: "test".to_string(),
            occurred_at: None,
            published_at: None,
            message: Some(Struct { fields }),
        };
        assert_eq!(
            extract_delivery_key(&[filtered_event]),
            None,
            "filtered events must not contribute a delivery key"
        );
    }

    // -------------------------------------------------------------------------
    // Task 1.7: NOT_FOUND path integration tests.
    //
    // These exercise `consume_durable_events_continuously` against a live (loopback)
    // gRPC mock so the NOT_FOUND classification travels through the real tonic
    // call stack, not just a unit-test branch predicate.
    // -------------------------------------------------------------------------

    use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::{
        event_consumer_service_server::{EventConsumerService, EventConsumerServiceServer},
        ConsumeDurableEventsRequest, ConsumeDurableEventsResponse,
        ConsumeDurableEventsWithAcksRequest, ConsumeDurableEventsWithAcksResponse,
        ConsumeNonDurableEventsRequest, ConsumeNonDurableEventsResponse, CreateDurableQueueRequest,
        CreateDurableQueueResponse, DeleteDurableQueueRequest, DeleteDurableQueueResponse,
        ManageDeadLetteredEventsRequest, ManageDeadLetteredEventsResponse,
        UpdateDurableQueueRequest, UpdateDurableQueueResponse,
    };
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::Server;
    use tonic::{Request, Response, Status, Streaming};

    struct NotFoundOnStreamConsumer;

    #[tonic::async_trait]
    impl EventConsumerService for NotFoundOnStreamConsumer {
        type ConsumeDurableEventsStream =
            futures_util::stream::Empty<Result<ConsumeDurableEventsResponse, Status>>;
        async fn consume_durable_events(
            &self,
            _: Request<ConsumeDurableEventsRequest>,
        ) -> Result<Response<Self::ConsumeDurableEventsStream>, Status> {
            Err(Status::unimplemented("not used"))
        }

        type ConsumeDurableEventsWithAcksStream = std::pin::Pin<
            Box<
                dyn tokio_stream::Stream<
                        Item = Result<ConsumeDurableEventsWithAcksResponse, Status>,
                    > + Send,
            >,
        >;
        async fn consume_durable_events_with_acks(
            &self,
            _: Request<Streaming<ConsumeDurableEventsWithAcksRequest>>,
        ) -> Result<Response<Self::ConsumeDurableEventsWithAcksStream>, Status> {
            // Open the stream successfully, then immediately terminate it with
            // NOT_FOUND — this is what the notification service does when the
            // durable queue is deleted while a consumer is mid-stream.
            let stream = futures_util::stream::once(async {
                Err(Status::not_found(
                    "durable queue deleted while consumer was attached",
                ))
            });
            Ok(Response::new(Box::pin(stream)))
        }

        type ConsumeNonDurableEventsStream =
            futures_util::stream::Empty<Result<ConsumeNonDurableEventsResponse, Status>>;
        async fn consume_non_durable_events(
            &self,
            _: Request<Streaming<ConsumeNonDurableEventsRequest>>,
        ) -> Result<Response<Self::ConsumeNonDurableEventsStream>, Status> {
            Err(Status::unimplemented("not used"))
        }
        async fn create_durable_queue(
            &self,
            request: Request<CreateDurableQueueRequest>,
        ) -> Result<Response<CreateDurableQueueResponse>, Status> {
            Ok(Response::new(CreateDurableQueueResponse {
                queue_id: request.into_inner().queue_id,
                created: true,
            }))
        }
        async fn delete_durable_queue(
            &self,
            _: Request<DeleteDurableQueueRequest>,
        ) -> Result<Response<DeleteDurableQueueResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }
        async fn update_durable_queue(
            &self,
            _: Request<UpdateDurableQueueRequest>,
        ) -> Result<Response<UpdateDurableQueueResponse>, Status> {
            Err(Status::unimplemented("not used"))
        }
        type ManageDeadLetteredEventsStream =
            futures_util::stream::Empty<Result<ManageDeadLetteredEventsResponse, Status>>;
        async fn manage_dead_lettered_events(
            &self,
            _: Request<Streaming<ManageDeadLetteredEventsRequest>>,
        ) -> Result<Response<Self::ManageDeadLetteredEventsStream>, Status> {
            Err(Status::unimplemented("not used"))
        }
    }

    /// Start a loopback gRPC server using `NotFoundOnStreamConsumer` and
    /// return a connected channel.
    async fn start_not_found_server() -> tonic::transport::Channel {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test: must bind ephemeral port");
        let addr = listener.local_addr().expect("bound addr");

        tokio::spawn(async move {
            let _ = Server::builder()
                .add_service(EventConsumerServiceServer::new(NotFoundOnStreamConsumer))
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await;
        });

        tonic::transport::Endpoint::from_shared(format!("http://{}", addr))
            .expect("test: endpoint url must parse")
            .connect()
            .await
            .expect("test: must connect to loopback server")
    }

    /// Task 1.7 (path A): a NOT_FOUND status on an open consume stream is
    /// classified as `SessionExit::NotFound` rather than as a generic transport
    /// failure.  Downstream (`start_event_service`) uses this to re-provision the
    /// durable queue; here we verify only the classification because re-provisioning
    /// requires a full `start_event_service` harness.
    #[tokio::test]
    async fn not_found_on_stream_is_classified_as_session_exit_not_found() {
        let channel = start_not_found_server().await;
        // Sets all THUMBNAIL_* and STORAGE_GRPC_ENDPOINT vars, installs the
        // global runtime config exactly once, and returns the static reference.
        let runtime = crate::test_fixtures::init_test_runtime_config("streaming-test");
        let outbound = crate::outbound::OutboundClients::for_tests();

        let config = crate::notifications::EventConsumerConfig {
            endpoint: "http://127.0.0.1:1".to_string(), // unused — channel is pre-built
            interceptor: outbound.interceptor.clone(),
            refresher: outbound.refresher.clone(),
            token_gate: outbound.token_gate.clone(),
            filter_groups: vec![],
        };

        let mut consumer = crate::notifications::EventConsumer {
            config,
            grpc_client: Some(
                notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::event_consumer_service_client::EventConsumerServiceClient::with_interceptor(
                    channel,
                    outbound.interceptor.clone(),
                ),
            ),
        };

        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let result = consumer
            .consume_durable_events_continuously(
                "test-queue-id",
                shutdown_rx,
                runtime,
                Arc::new(outbound),
            )
            .await;

        assert_eq!(
            result,
            Ok(SessionExit::NotFound),
            "a NOT_FOUND error on an open stream must surface as SessionExit::NotFound"
        );
    }
}
