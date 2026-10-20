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

//! The WorkerEventQueue consume loop: deliveries in, acknowledgements out.
//!
//! # Redelivery via NACK
//!
//! The notification broker now carries a negative acknowledgement field
//! (`nack_delivery_tags`) on the same request that holds positive ACKs.  A
//! retryable conversion failure sends an explicit NACK on the ack stream so the
//! broker can redeliver immediately on its own schedule rather than waiting for
//! a reconnect.
//!
//! The recycle (`SessionEnd::Recycle`) is retained in two narrow fallback roles:
//!
//!   1. **Ack-stream broken**: if the mpsc sender that carries NACKs back to the
//!      gRPC request stream is closed before the NACK can be sent, the delivery
//!      is neither acknowledged nor negatively acknowledged — recycling the
//!      session is the only way to trigger redelivery.
//!   2. **Conversion task panic**: a panicked task leaves no delivery_tag to
//!      NACK; recycling is treated the same as an undeliverable NACK.
//!
//! A healthy worker never recycles.
//!
//! # Why it does not rebuild a quiet stream
//!
//! An earlier idle-silence rebuild matched StorageEventQueue's defense against L4 load
//! balancers that close idle connections without an error. That cycle (~270s)
//! is not how the Notification consumer already approved in RC
//! (permission-service) behaves: a quiet healthy stream stays open, and
//! reconnects happen on stream end, transport error, `UNAUTHENTICATED`, or
//! token near expiry.  Transport keepalive remains configured on the channel.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::event_consumer_service_client::EventConsumerServiceClient;
use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::Event;
use thumbnail_common::auth::TokenGate;
use thumbnail_common::notifications::provisioning::{ensure_durable_queue, EnsureQueueOutcome};
use thumbnail_common::notifications::redelivery::NackReason;
use thumbnail_common::notifications::{
    is_auth_error, open_ack_stream, reconnect_backoff, send_ack, send_nack, AckStream,
    DeferredNacks, FilterGroup, RedeliveryTracker,
};
use thumbnail_common::work_events::GENERATE_EVENT_TYPE;
use tokio::sync::{watch, Mutex, Semaphore};
use tokio::task::JoinSet;
use tonic::transport::Channel;
use tracing::{error, info, warn};

/// How long a teardown may hold the consume call open so negative
/// acknowledgements queued by the flush are written before the call is cancelled.
const NACK_FLUSH_GRACE: Duration = Duration::from_secs(2);

/// How long past its next-attempt time a delivery key's spacing state is kept.
const REDELIVERY_KEY_RETENTION: Duration = Duration::from_secs(3600);

/// Buffered acknowledgements before a sender waits. Deep enough that a burst of
/// completions never blocks a conversion task on the ack channel.
const ACK_CHANNEL_BUFFER: usize = 64;

/// Matches StorageEventQueue's `RECONNECT_BACKOFF_EXPONENT_CAP` default. Fixed rather than
/// configurable: the worker's ceiling is `max_reconnect_backoff_seconds`, and a
/// second knob shaping the curve underneath it has never been worth setting.
///
/// The backoff bounds beside it stay whole seconds rather than becoming
/// `Duration`s like the rest of the worker's timing settings. `reconnect_backoff`
/// is shared with StorageEventQueue, whose `StreamRuntimeArgs` stores seconds, so
/// converting here would move the unit handling into `streaming.rs` rather than
/// remove it.
const RECONNECT_BACKOFF_EXPONENT_CAP: u32 = 4;

use crate::config::WorkerArgs;
use crate::health::HealthState;
use crate::metrics::WorkerMetrics;
use crate::pipeline::MessageProcessor;

/// Whether a `watch::Receiver::changed()` result means stop.
///
/// A closed channel counts. `changed()` returns `Err` once every sender is
/// dropped, and it returns it immediately and forever after — so discarding the
/// error leaves a `select!` arm permanently ready, spinning the loop while the
/// last observed value is still `false`. Every sender being gone can only mean
/// the process is tearing down, which is the same instruction the value carries.
fn is_shutdown(
    changed: Result<(), watch::error::RecvError>,
    shutdown: &watch::Receiver<bool>,
) -> bool {
    changed.is_err() || *shutdown.borrow()
}

/// Why a consume session ended.
#[derive(Debug, PartialEq, Eq)]
enum SessionEnd {
    /// Shutdown was requested; do not reconnect.
    Shutdown,
    /// Deliberate teardown to force redelivery of withheld acknowledgements.
    Recycle,
    /// Bearer is close to expiry; refresh and reopen without backoff.
    TokenExpiring,
    /// The token was rejected and has since been refreshed.
    ///
    /// Distinct from `Interrupted` because the cause has already been addressed:
    /// an expired token is the ordinary case, and sleeping before retrying with
    /// the new one only delays recovery. It is *not* unconditionally immediate,
    /// though — a refresh that keeps not helping means a revoked credential or a
    /// policy change, and reconnecting flat out would hammer both the token
    /// endpoint and Notification for as long as that lasts. See `run`.
    AuthRefreshed(String),
    /// WorkerEventQueue no longer exists; re-provision it before reconnecting.
    ///
    /// Distinct from `Interrupted` because reconnecting alone cannot fix it: a
    /// queue that was deleted underneath the consumer stays deleted, so the
    /// generic backoff path reconnects forever against a queue that is gone and
    /// only a pod restart recovers — the 2026-08-25 incident, which deleted five
    /// queues in one namespace. StorageEventQueue classifies the same status the
    /// same way.
    NotFound,
    /// The stream ended or failed; reconnect after a backoff.
    Interrupted(String),
}

/// Outcome of re-provisioning WorkerEventQueue after a `NOT_FOUND`.
enum Reprovision {
    /// The queue exists again; reconnect without backoff.
    Recovered,
    ShutdownRequested,
    /// Provisioning itself failed; back off before trying again.
    Failed(String),
}

pub struct Consumer {
    pub args: WorkerArgs,
    /// WorkerEventQueue's derived id ([`WorkerArgs::resolve_queue_provisioning`]),
    /// resolved once at startup rather than recomputed here.
    pub queue_id: String,
    /// The filter groups this queue was provisioned with, kept so the consume
    /// loop can re-provision it after a `NOT_FOUND` without reaching back into
    /// startup wiring. Re-creating the queue with anything else would silently
    /// change what the worker receives.
    pub filter_groups: Vec<FilterGroup>,
    pub consumer_channel: Channel,
    pub interceptor: thumbnail_auth::AuthInterceptor,
    pub refresher: Arc<dyn thumbnail_auth::TokenRefresher>,
    pub token_gate: TokenGate,
    pub pipeline: Arc<dyn MessageProcessor>,
    pub metrics: Arc<WorkerMetrics>,
    pub health: Arc<HealthState>,
}

impl Consumer {
    /// Consume WorkerEventQueue until shutdown, reconnecting as needed.
    pub async fn run(&self, mut shutdown: watch::Receiver<bool>) {
        let mut attempts: u32 = 0;
        // Auth failures in a row. One is an expired token; several in a row mean
        // refreshing is not the fix.
        let mut auth_refreshes: u32 = 0;

        loop {
            if *shutdown.borrow() {
                return;
            }

            // Each arm decides whether to wait before the next session. The one
            // sleep below is shared so a new reason to reconnect cannot forget
            // to honor shutdown while it waits.
            let backoff = match self.session(shutdown.clone()).await {
                SessionEnd::Shutdown => return,
                SessionEnd::Recycle => {
                    self.metrics
                        .session_ended(crate::metrics::SessionEndCause::Recycle);
                    // Not a failure, so no backoff: the reconnect *is* the
                    // retry, and delaying it just delays the redelivery.
                    attempts = 0;
                    auth_refreshes = 0;
                    self.metrics.recycled();
                    info!("Recycled the consume stream to redeliver unacknowledged work");
                    None
                }
                SessionEnd::TokenExpiring => {
                    self.metrics
                        .session_ended(crate::metrics::SessionEndCause::TokenExpiring);
                    self.refresher.refresh().await;
                    if self.token_gate.is_expiring_soon() {
                        // Refresh did not help. Back off like any other failure
                        // rather than tearing the stream down once a second
                        // against a sick token endpoint or a sidecar file still
                        // inside its skew window.
                        attempts = attempts.saturating_add(1);
                        let backoff = self.reconnect_backoff(attempts);
                        error!(
                            attempt = attempts,
                            ?backoff,
                            "Token still near expiry after refresh; backing off"
                        );
                        Some(backoff)
                    } else {
                        attempts = 0;
                        auth_refreshes = 0;
                        self.metrics.token_reconnected();
                        info!("Token near expiry; refreshed and reconnecting");
                        None
                    }
                }
                SessionEnd::AuthRefreshed(reason) => {
                    self.metrics
                        .session_ended(crate::metrics::SessionEndCause::AuthRefreshed);
                    auth_refreshes = auth_refreshes.saturating_add(1);
                    if auth_refreshes == 1 {
                        // The ordinary case: the token aged out mid-stream and
                        // has been replaced. Waiting would only extend an outage
                        // whose cause is already fixed.
                        attempts = 0;
                        info!(reason, "Token refreshed; reconnecting immediately");
                        None
                    } else {
                        // Refreshing is not helping. Back off like any other
                        // failure rather than hammering the token endpoint and
                        // Notification while a credential stays rejected.
                        attempts = attempts.saturating_add(1);
                        let backoff = self.reconnect_backoff(attempts);
                        error!(
                            attempt = attempts,
                            consecutive_auth_failures = auth_refreshes,
                            reason,
                            ?backoff,
                            "Token was refreshed and rejected again; backing off"
                        );
                        Some(backoff)
                    }
                }
                SessionEnd::NotFound => {
                    self.metrics
                        .session_ended(crate::metrics::SessionEndCause::QueueMissing);
                    auth_refreshes = 0;
                    warn!(
                        queue_id = %self.queue_id,
                        "WorkerEventQueue is missing; re-provisioning before reconnect"
                    );
                    match self.reprovision_queue(&mut shutdown).await {
                        Reprovision::Recovered => {
                            // The queue is back, so the reconnect is the recovery
                            // rather than a retry after a failure.
                            attempts = 0;
                            None
                        }
                        Reprovision::ShutdownRequested => return,
                        Reprovision::Failed(reason) => {
                            // Escalate: a queue that cannot be re-created keeps
                            // returning NOT_FOUND, and resetting the counter here
                            // would recompute the same first-attempt backoff
                            // forever instead of easing off.
                            attempts = attempts.saturating_add(1);
                            let backoff = self.reconnect_backoff(attempts);
                            error!(
                                queue_id = %self.queue_id,
                                attempt = attempts,
                                reason,
                                ?backoff,
                                "Re-provisioning WorkerEventQueue failed; backing off"
                            );
                            Some(backoff)
                        }
                    }
                }
                SessionEnd::Interrupted(reason) => {
                    self.metrics
                        .session_ended(crate::metrics::SessionEndCause::Interrupted);
                    attempts = attempts.saturating_add(1);
                    auth_refreshes = 0;
                    let backoff = self.reconnect_backoff(attempts);
                    error!(
                        attempt = attempts,
                        reason,
                        ?backoff,
                        "Consume stream interrupted; reconnecting"
                    );
                    Some(backoff)
                }
            };

            if let Some(backoff) = backoff {
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    changed = shutdown.changed() => {
                        if is_shutdown(changed, &shutdown) { return; }
                    }
                }
            }
        }
    }

    /// Re-run the durable-queue provisioning call after a `NOT_FOUND`.
    ///
    /// Reuses the channel the consume loop already holds, so this adds no new
    /// connection handling, and passes the stored filter groups so the recreated
    /// queue subscribes to exactly what the deleted one did.
    async fn reprovision_queue(&self, shutdown: &mut watch::Receiver<bool>) -> Reprovision {
        match ensure_durable_queue(
            self.consumer_channel.clone(),
            self.interceptor.clone(),
            self.refresher.clone(),
            &self.queue_id,
            self.filter_groups.clone(),
            shutdown,
            &self.args.stream_runtime_args(),
        )
        .await
        {
            Ok(EnsureQueueOutcome::Created) => {
                info!(
                    queue_id = %self.queue_id,
                    "Re-provisioned WorkerEventQueue; reconnecting"
                );
                Reprovision::Recovered
            }
            Ok(EnsureQueueOutcome::AlreadyExisted) => {
                // Someone else recreated it first, which is just as recovered.
                info!(
                    queue_id = %self.queue_id,
                    "WorkerEventQueue now exists; reconnecting"
                );
                Reprovision::Recovered
            }
            Ok(EnsureQueueOutcome::ShutdownAborted) => Reprovision::ShutdownRequested,
            Err(e) => Reprovision::Failed(e.to_string()),
        }
    }

    fn reconnect_backoff(&self, attempt: u32) -> Duration {
        reconnect_backoff(
            attempt,
            self.args.reconnect_base_backoff_seconds,
            RECONNECT_BACKOFF_EXPONENT_CAP,
            self.args.max_reconnect_backoff_seconds,
        )
    }

    async fn session(&self, mut shutdown: watch::Receiver<bool>) -> SessionEnd {
        let mut client = EventConsumerServiceClient::with_interceptor(
            self.consumer_channel.clone(),
            self.interceptor.clone(),
        );

        // Opened through the shared session helper: same establishment bound,
        // same initial-request shape, same auth classification as StorageEventQueue.
        let opened = open_ack_stream(&mut client, &self.queue_id, ACK_CHANNEL_BUFFER).await;

        let AckStream {
            acks: ack_tx,
            deliveries: mut stream,
        } = match opened {
            Ok(session) => session,
            Err(other) => return SessionEnd::Interrupted(other.to_string()),
        };

        self.metrics.reset_unacknowledged();
        self.health.stream_established();
        info!(
            queue_id = %self.queue_id,
            "Consuming generate work from WorkerEventQueue"
        );

        // Admission control: reading pauses at the bound rather than letting the
        // unacknowledged set grow until the queue hits its maximum length and
        // the platform dead-letters the overflow.
        let permits = Arc::new(Semaphore::new(self.args.max_concurrent_conversions));
        let mut conversions: JoinSet<()> = JoinSet::new();

        // Conversion tasks signal the main loop to recycle when their NACK
        // fails (ack-stream broken — fallback role, or task panic).
        // Capacity 1: any extra signals from concurrent tasks are dropped.
        let (recycle_tx, mut recycle_rx) = tokio::sync::mpsc::channel::<()>(1);

        // Redelivery spacing, shared with StorageEventQueue. The broker requeues a
        // NACKed delivery immediately, so without this a conversion that keeps
        // failing — a corrupt asset, or storage being down — is retried as fast as
        // the network allows. The recycle timer this replaced supplied ~60s of
        // spacing as a side effect; that spacing is now explicit.
        let mut deferred_nacks = DeferredNacks::new(
            self.args.redelivery_recycle_interval,
            self.args.nack_deferral_concurrency_limit,
        );
        let tracker = Arc::new(Mutex::new(RedeliveryTracker::new()));
        // Unbounded, and it has to be. A bounded channel deadlocks this loop: the
        // conversion task hands off while still holding its semaphore permit, and
        // the main loop is the only thing that drains the channel — so once the
        // channel is full, the permit holders block on the send, the loop blocks
        // acquiring a permit for the next delivery, and nothing moves again.
        // Observed in dev on 2026-08-31: one worker wedged with 12 deliveries
        // in flight and no log output for 11 minutes.
        //
        // Unbounded is not unbounded in practice. Only spawned conversions send,
        // each sends at most one tag, and the semaphore caps them at
        // `max_concurrent_conversions`, so the channel can never hold more than
        // that before the loop drains it. The bound that matters for held NACKs
        // is `nack_deferral_concurrency_limit` on `deferred_nacks`, which is
        // enforced by the loop that owns them.
        let (defer_tx, mut defer_rx) = tokio::sync::mpsc::unbounded_channel::<(i64, NackReason)>();
        let nack_delay = self.args.redelivery_recycle_interval;
        let early_limit = self.args.nack_early_redelivery_limit;

        // Token-expiry check. Persistent timer — not a `tokio::time::timeout`
        // wrapped around `stream.next()` inside the `select!`: that future is
        // rebuilt from scratch on every iteration, so it never survives long
        // enough to elapse and the defense silently does nothing.
        let mut housekeeping = tokio::time::interval(Duration::from_secs(1));
        housekeeping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Conversion capacity is reserved *before* a delivery is read, not acquired
        // after one arrives.
        //
        // Acquiring inside the `stream.next()` arm looks equivalent and is not:
        // `select!` runs the winning arm to completion, so an await there stops
        // every other arm from being polled. At the concurrency bound that means
        // shutdown is not observed until a conversion finishes (before the drain
        // budget has even started counting), the token-expiry check does not run
        // precisely under the sustained load that makes refresh necessary, and a
        // deferred NACK cannot come due.
        //
        // Holding a spare permit costs nothing in throughput: it is held only
        // between capacity freeing and the next delivery being read, which under
        // load is immediate, and when the stream is idle the capacity was not
        // wanted anyway. In-flight conversions still never exceed
        // `max_concurrent_conversions`, because the reservation is drawn from the
        // same semaphore.
        let mut reserved: Option<tokio::sync::OwnedSemaphorePermit> = None;

        let ending = loop {
            let next_nack_deadline = deferred_nacks.next_deadline();

            tokio::select! {
                changed = shutdown.changed() => {
                    if is_shutdown(changed, &shutdown) {
                        break SessionEnd::Shutdown;
                    }
                }

                // Fallback role: NACK could not be delivered (ack-stream broken)
                // or a conversion task panicked. Recycle to force redelivery.
                _ = recycle_rx.recv() => {
                    warn!("NACK could not be delivered; recycling the stream (fallback role)");
                    break SessionEnd::Recycle;
                }

                // A conversion reported a retryable failure and wants its NACK
                // deferred. Held here rather than slept on in the task, which the
                // drain budget would abort with the NACK unsent.
                Some((tag, reason)) = defer_rx.recv() => {
                    if !deferred_nacks.try_defer(tag, reason) {
                        warn!(
                            delivery_tag = tag,
                            limit = self.args.nack_deferral_concurrency_limit,
                            "Deferral bound reached; negatively acknowledging immediately"
                        );
                        if send_nack(&ack_tx, tag).await.is_err() {
                            break SessionEnd::Recycle;
                        }
                        self.metrics.delivery_nacked(reason);
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
                        if send_nack(&ack_tx, tag).await.is_err() {
                            ack_stream_broken = true;
                            break;
                        }
                        self.metrics.delivery_nacked(reason);
                    }
                    if ack_stream_broken {
                        break SessionEnd::Recycle;
                    }
                }

                _ = housekeeping.tick() => {
                    if self.token_gate.is_expiring_soon() {
                        break SessionEnd::TokenExpiring;
                    }
                    // Spacing state is removed on success or reschedule, so a key
                    // that fails once and never returns would otherwise sit in the
                    // map for the life of the session.
                    tracker.lock().await.evict_expired(REDELIVERY_KEY_RETENTION);
                }

                // Reserve capacity for the next delivery. Guarded so it only runs
                // when nothing is held, and disabled once one is — otherwise this
                // arm would hoard permits.
                permit = permits.clone().acquire_owned(), if reserved.is_none() => {
                    match permit {
                        Ok(permit) => reserved = Some(permit),
                        Err(_) => {
                            break SessionEnd::Interrupted("conversion semaphore closed".into())
                        }
                    }
                }

                // Only read a delivery once there is capacity in hand, so a read
                // never has to wait for one.
                delivery = stream.next(), if reserved.is_some() => {
                    match delivery {
                        Some(Ok(response)) => {
                            let delivery_tag = response.delivery_tag;
                            let events = response.events;
                            self.metrics.delivery_received();

                            // The worker's delivery key is the generate message's
                            // `destination_resource_address`: already a deterministic
                            // function of source identity and specification, and unlike
                            // the delivery tag it survives a redelivery.
                            let delivery_key = extract_delivery_key(&events);

                            // Anti-spin gate: a redelivery that arrived before its
                            // next-attempt time is returned without redoing the
                            // conversion, and repeated early arrivals are bounded.
                            if let Some(ref key) = delivery_key {
                                let mut t = tracker.lock().await;
                                if t.is_early(key) {
                                    let count = t.record_early(key);
                                    drop(t);
                                    if count >= early_limit {
                                        error!(
                                            delivery_key = %key,
                                            consecutive_early_redeliveries = count,
                                            limit = early_limit,
                                            "Delivery key redelivered early past the limit; \
                                             recycling the stream (circuit-breaker role)"
                                        );
                                        if send_nack(&ack_tx, delivery_tag).await.is_ok() {
                                            self.metrics
                                                .delivery_nacked(NackReason::EarlyRedelivery);
                                        }
                                        break SessionEnd::Recycle;
                                    }
                                    if deferred_nacks
                                        .try_defer(delivery_tag, NackReason::EarlyRedelivery)
                                    {
                                        continue;
                                    }
                                    if send_nack(&ack_tx, delivery_tag).await.is_err() {
                                        break SessionEnd::Recycle;
                                    }
                                    self.metrics.delivery_nacked(NackReason::EarlyRedelivery);
                                    continue;
                                }
                                t.clear_early_count(key);
                            }

                            let permit = reserved
                                .take()
                                .expect("the arm guard admits a delivery only with a reservation");

                            let pipeline = self.pipeline.clone();
                            let metrics = self.metrics.clone();
                            let ack_tx = ack_tx.clone();
                            let recycle_tx = recycle_tx.clone();
                            let defer_tx = defer_tx.clone();
                            let tracker_task = tracker.clone();

                            conversions.spawn(async move {
                                let ack = process_delivery(pipeline.as_ref(), &metrics, events).await;

                                if ack {
                                    if let Some(ref key) = delivery_key {
                                        tracker_task.lock().await.clear(key);
                                    }
                                    if send_ack(&ack_tx, delivery_tag).await {
                                        metrics.delivery_acknowledged();
                                    }
                                } else {
                                    // Task 6.1: NACK retryable failures instead of
                                    // withholding and arming the recycle timer.
                                    warn!(
                                        delivery_tag,
                                        delay_secs = nack_delay.as_secs(),
                                        "Retryable failure; scheduling a spaced redelivery"
                                    );
                                    if let Some(ref key) = delivery_key {
                                        tracker_task
                                            .lock()
                                            .await
                                            .schedule_next_attempt(key, nack_delay);
                                    }
                                    // The delay is applied by the main loop; sleeping
                                    // here would put the NACK behind the drain budget,
                                    // which aborts this task long before it elapses.
                                    // The handoff must not block: this task still holds
                                    // its semaphore permit, and the loop that drains
                                    // this channel is the same one waiting for permits.
                                    if defer_tx
                                        .send((delivery_tag, NackReason::RetryableFailure))
                                        .is_err()
                                    {
                                        if send_nack(&ack_tx, delivery_tag).await.is_err() {
                                            // Task 6.2: Fallback role — ack-stream
                                            // broken; signal the main loop to recycle.
                                            warn!(
                                                delivery_tag,
                                                "NACK channel closed; signalling recycle (fallback role)"
                                            );
                                            let _ = recycle_tx.try_send(());
                                        } else {
                                            metrics.delivery_nacked(
                                                NackReason::RetryableFailure,
                                            );
                                        }
                                    }
                                }

                                drop(permit);
                            });
                        }
                        Some(Err(status)) if is_auth_error(&status) => {
                            self.refresher.refresh().await;
                            break SessionEnd::AuthRefreshed(format!("auth error on the stream: {status}"));
                        }
                        Some(Err(status)) if status.code() == tonic::Code::NotFound => {
                            // The queue was deleted underneath this consumer.
                            // Reconnecting alone cannot recover it, so `run`
                            // re-provisions before reopening.
                            warn!(
                                queue_id = %self.queue_id,
                                "WorkerEventQueue not found; will re-provision before reconnect: {status}"
                            );
                            break SessionEnd::NotFound;
                        }
                        Some(Err(status)) => {
                            // PermissionDenied is not refreshable (`is_auth_error`
                            // is Unauthenticated-only). Interrupted → error log +
                            // reconnect backoff in `run`, with the queue named so
                            // operators see ACL misses instead of a token spin.
                            let reason = if status.code() == tonic::Code::PermissionDenied {
                                format!(
                                    "Durable queue subscription failed for queue {}: {status}",
                                    self.queue_id
                                )
                            } else {
                                format!("stream error: {status}")
                            };
                            break SessionEnd::Interrupted(reason);
                        }
                        None => break SessionEnd::Interrupted("stream ended".into()),
                    }
                }

                Some(joined) = conversions.join_next(), if !conversions.is_empty() => {
                    if let Err(e) = joined {
                        // A panicked task leaves no delivery_tag to NACK.
                        // Signal the main loop to recycle (fallback role): the
                        // reconnect is what produces redelivery of the unresolved
                        // delivery, same as when the ack-stream is broken.
                        error!(error = %e, "Conversion task panicked; signalling recycle (fallback role)");
                        let _ = recycle_tx.try_send(());
                    }
                }
            }
        };

        self.health.stream_lost();

        // Drain before the stream drops, so acknowledgements for work that
        // finished actually reach the server. Anything still running when the
        // budget expires stays unacknowledged and comes back — which is correct,
        // if wasteful, and strictly better than acknowledging work that did not
        // complete.
        // Whether a conversion *completed* during the drain, not merely whether one
        // was running. A conversion that finishes here sends its own ACK on
        // `ack_tx`, and that ACK needs the same grace before teardown as a flushed
        // NACK does. One that is aborted when the budget expires queues nothing, so
        // paying the grace for it would add two seconds to every shutdown that
        // timed out its drain — which is the shutdown already running late.
        let mut completed_during_drain = false;

        let drained = tokio::time::timeout(self.args.shutdown_drain_timeout, async {
            while conversions.join_next().await.is_some() {
                completed_during_drain = true;
            }
        })
        .await;
        if drained.is_err() {
            warn!(
                "In-flight conversions did not finish within the drain budget; \
                 their deliveries stay unacknowledged"
            );
            conversions.abort_all();
        }

        // Flush every held NACK before the sender goes, including those requested
        // by conversions that finished during the drain. The remaining delay is
        // abandoned deliberately: the session is ending, so the reconnect supplies
        // the spacing, while an unsent NACK would leave the delivery in-flight
        // until the broker's acknowledgement timeout cancels the consumer.
        defer_rx.close();
        let mut to_flush = deferred_nacks.drain_all();
        while let Ok((tag, reason)) = defer_rx.try_recv() {
            to_flush.push((tag, reason));
        }
        let flushed = !to_flush.is_empty();
        if flushed {
            info!(
                deferred = to_flush.len(),
                "Flushing deferred negative acknowledgements before teardown"
            );
            for (tag, reason) in to_flush {
                if send_nack(&ack_tx, tag).await.is_err() {
                    break;
                }
                self.metrics.delivery_nacked(reason);
            }
        }

        drop(ack_tx);

        // Dropping the response stream cancels the call, which would discard the
        // negative acknowledgements just queued on the request stream — and an
        // unsent NACK is precisely the in-flight-forever state the flush exists to
        // avoid. Holding the call open briefly lets them reach the wire. Bounded,
        // and only on a teardown that actually had deferrals to flush.
        // Sleep rather than awaiting the stream: `stream.next()` returns as soon as
        // the next delivery arrives, so on a busy queue the grace collapsed to
        // nearly nothing and the NACKs were cancelled anyway. What matters is that
        // `stream` is not dropped yet, which holding here achieves regardless of
        // what the peer sends.
        //
        // Also applied when a conversion completed during the drain, not only when
        // NACKs were flushed. Such a conversion queues a positive ACK on `ack_tx`,
        // and gating the grace on `flushed` alone dropped the stream out from under
        // it — so the drain waited for the work to finish and then discarded the
        // acknowledgement it existed to deliver, and the delivery came back to be
        // redone. Found by `a_conversion_finishing_during_drain_is_acknowledged`.
        if flushed || completed_during_drain {
            tokio::time::sleep(NACK_FLUSH_GRACE).await;
        }

        ending
    }
}

/// Process every event in one delivery and decide the delivery's acknowledgement.
///
/// The acknowledgement unit is the `delivery_tag`, not the event, and one tag can
/// cover several events. The rule is the same one StorageEventQueue already uses: every
/// event is processed, and the delivery is acknowledged only if none of them
/// withheld. A retryable failure on one event therefore redelivers its
/// siblings too — accepted, because the worker is idempotent on the destination,
/// and the alternative is acknowledging work that failed.
/// The delivery's logical work identity, for spacing successive attempts.
///
/// `destination_resource_address` is already a deterministic function of the
/// source identity and the specification, so the same work carries the same key
/// across redeliveries — which the delivery tag does not, being per-session.
fn extract_delivery_key(events: &[Event]) -> Option<String> {
    use prost_types::value::Kind;
    for event in events {
        if event.event_type != GENERATE_EVENT_TYPE {
            continue;
        }
        let Some(message) = &event.message else {
            continue;
        };
        if let Some(value) = message.fields.get("destination_resource_address") {
            if let Some(Kind::StringValue(address)) = &value.kind {
                if !address.is_empty() {
                    return Some(address.clone());
                }
            }
        }
    }
    None
}

async fn process_delivery(
    pipeline: &dyn MessageProcessor,
    metrics: &WorkerMetrics,
    events: Vec<Event>,
) -> bool {
    let mut ack = true;

    for event in events {
        // WorkerEventQueue filters on the generate type, so anything else here is a
        // provisioning mistake rather than work. Acknowledge it: redelivering an
        // event this worker will never handle only fills the queue.
        if event.event_type != GENERATE_EVENT_TYPE {
            warn!(
                event_type = %event.event_type,
                "Ignoring an event type WorkerEventQueue should not be delivering"
            );
            continue;
        }

        let Some(message) = event.message else {
            warn!("Generate event carried no message; acknowledging as malformed");
            continue;
        };

        let started = Instant::now();
        let outcome = pipeline.process(&message).await;
        metrics.record(&outcome, started.elapsed().as_millis() as u64);

        match &outcome {
            crate::outcome::WorkOutcome::Succeeded { .. } => {}
            crate::outcome::WorkOutcome::Permanent {
                class,
                destination,
                detail,
            } => {
                // Logged before the acknowledgement, and at error level: a
                // permanent rejection never reaches the dead-letter queue, so
                // this line is the only durable trace it happened.
                error!(
                    destination = %destination,
                    failure_class = %class,
                    detail = %detail,
                    "Permanently rejecting a generate message"
                );
            }
            crate::outcome::WorkOutcome::Retryable {
                class,
                destination,
                detail,
            } => {
                warn!(
                    destination = %destination,
                    failure_class = %class,
                    detail = %detail,
                    "Retryable failure; withholding the acknowledgement"
                );
            }
        }

        ack &= outcome.should_ack();
    }

    ack
}
