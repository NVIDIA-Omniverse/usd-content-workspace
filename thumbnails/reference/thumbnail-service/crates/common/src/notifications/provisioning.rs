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

//! Durable-queue provisioning helpers.
//!
//! The public entry point is [`ensure_durable_queue`], which issues the
//! `CreateDurableQueue` RPC (ensure-exists semantics) with auth refresh, a
//! per-request timeout, and bounded transient-retry backoff. Everything below
//! that function in this file — outcome types, the filter-group builder, the
//! status classifier, and the pure decision core — exists to support that call.

use std::sync::Arc;
use std::time::Duration;

use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::{
    event_consumer_service_client::EventConsumerServiceClient, resource_filter::FilterType,
    CreateDurableQueueRequest, FilterGroup, ResourceFilter,
};
use thumbnail_auth::{AuthInterceptor, TokenRefresher};
use tokio::sync::watch;
use tonic::transport::Channel;
use tonic::Code;
use tracing::{info, warn};

use super::helpers::sleep_with_shutdown_check;
use crate::config::StreamRuntimeArgs;
use crate::errors::NotificationError;
use crate::notifications::StorageEventType;

/// Ensures the durable queue `queue_id` exists by issuing a single one-shot
/// `CreateDurableQueue` call (ensure-exists semantics — the server creates the queue or returns
/// the existing one). Returns an [`EnsureQueueOutcome`]: [`EnsureQueueOutcome::Created`] when this
/// call newly created the queue (`response.created == true`), [`EnsureQueueOutcome::AlreadyExisted`]
/// when it already existed (`response.created == false`), or [`EnsureQueueOutcome::ShutdownAborted`]
/// when shutdown interrupted a transient-retry backoff.
///
/// This is the thin async gRPC wrapper that threads together the pure pieces below
/// ([`build_storage_filter_groups`] + [`classify_provisioning_status`]) with the actual RPC and
/// the shared service-account auth. It builds its own one-shot
/// [`EventConsumerServiceClient`] by cloning the pre-connected `channel` and the shared
/// `interceptor` (exactly like [`crate::notifications::event_publisher::publish_event`] and
/// [`crate::notifications::EventConsumer::connect_grpc`]), so it never disturbs the long-lived
/// streaming client.
///
/// Failure handling, in priority order:
/// - `Unauthenticated`: refresh the service-account token once via `refresher` and retry the call
///   once, mirroring the reactive-refresh contract in `event_publisher.rs` / `streaming.rs`. A
///   persistent auth failure surfaces as [`NotificationError::QueueProvisioningFailed`].
/// - `PermissionDenied`: short-circuit immediately with
///   [`NotificationError::QueueProvisioningFailed`] (no bounded retry). The `tonic::Status` code
///   is preserved so the runtime event-consumer supervisor can apply its warn-and-attach fallback
///   for the consume-only-identity backward-compat path.
/// - `InvalidArgument` (permanent misconfig, per [`classify_provisioning_status`]): no retry —
///   return [`NotificationError::QueueProvisioningFailed`] immediately.
/// - Transient (`Unavailable` / `DeadlineExceeded` / connection-style): bounded-retry, sleeping
///   between attempts via [`sleep_with_shutdown_check`] with the same exponential backoff shape as
///   the reconnect loop in `streaming.rs`. The retry budget is `notifications_provisioning_max_retries`
///   from the runtime config (independent of `reconnect_backoff_exponent_cap`, which governs only
///   the backoff-growth exponent); when it is exhausted, return
///   [`NotificationError::QueueProvisioningFailed`].
/// - Timeout: each attempt is bounded by a per-request deadline (`stream_timeout_secs`). A server
///   that accepts the request post-connect but never responds is treated as a transient failure —
///   so a hang converts into bounded retries instead of blocking the startup critical path forever.
///
/// If shutdown is signalled while sleeping between transient retries, the function returns early
/// and cleanly with [`EnsureQueueOutcome::ShutdownAborted`] rather than an error — the caller is
/// tearing down and no queue was provisioned by this call.
pub async fn ensure_durable_queue(
    channel: Channel,
    interceptor: AuthInterceptor,
    refresher: Arc<dyn TokenRefresher>,
    queue_id: &str,
    filter_groups: Vec<FilterGroup>,
    shutdown: &mut watch::Receiver<bool>,
    backoff: &StreamRuntimeArgs,
) -> Result<EnsureQueueOutcome, NotificationError> {
    let policy = ProvisioningRetryPolicy {
        max_retries: backoff.notifications_provisioning_max_retries,
        base_backoff_secs: backoff.reconnect_base_backoff_seconds,
        exponent_cap: backoff.reconnect_backoff_exponent_cap,
        max_backoff_secs: backoff.max_reconnect_backoff_seconds,
    };
    let request_timeout = Duration::from_secs(backoff.stream_timeout_secs);
    let mut retry: u32 = 0;

    info!(
        "[PROVISION] Ensuring durable queue '{}' exists ({} filter group(s): {})",
        queue_id,
        filter_groups.len(),
        filter_groups
            .iter()
            .map(|group| group.event_type.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    loop {
        let request = CreateDurableQueueRequest {
            queue_id: queue_id.to_string(),
            filter_groups: filter_groups.clone(),
        };
        let mut client =
            EventConsumerServiceClient::with_interceptor(channel.clone(), interceptor.clone());

        // One-shot call under a per-request deadline. The reactive refresh-once contract still
        // applies: on `Unauthenticated`, refresh the service-account token and reissue the
        // identical request exactly once. A call that does not return within `request_timeout`
        // (a server that accepts the request post-connect but never responds) collapses into a
        // `TimedOut` attempt, which the decision routes into the transient-retry path rather than
        // hanging the startup critical path forever.
        let attempt = match tokio::time::timeout(
            request_timeout,
            client.create_durable_queue(tonic::Request::new(request.clone())),
        )
        .await
        {
            Err(_elapsed) => ProvisioningAttempt::TimedOut,
            Ok(Ok(resp)) => {
                let inner = resp.into_inner();
                ProvisioningAttempt::Responded {
                    created: inner.created,
                    queue_id: inner.queue_id,
                }
            }
            Ok(Err(status)) if status.code() == Code::Unauthenticated => {
                info!(
                    "[PROVISION] CreateDurableQueue got Unauthenticated; refreshing token and retrying once"
                );
                refresher.refresh().await;
                match tokio::time::timeout(
                    request_timeout,
                    client.create_durable_queue(tonic::Request::new(request)),
                )
                .await
                {
                    Err(_elapsed) => ProvisioningAttempt::TimedOut,
                    Ok(Ok(resp)) => {
                        let inner = resp.into_inner();
                        ProvisioningAttempt::Responded {
                            created: inner.created,
                            queue_id: inner.queue_id,
                        }
                    }
                    Ok(Err(status)) => ProvisioningAttempt::Failed(status),
                }
            }
            Ok(Err(status)) => ProvisioningAttempt::Failed(status),
        };

        let reason = attempt.describe();
        match decide_provisioning_step(attempt, queue_id, retry, &policy) {
            ProvisioningDecision::Succeed(outcome) => return Ok(outcome),
            ProvisioningDecision::Fail(status) => {
                return Err(NotificationError::QueueProvisioningFailed {
                    queue_id: queue_id.to_string(),
                    status,
                });
            }
            ProvisioningDecision::Retry(delay) => {
                retry += 1;
                warn!(
                    "[PROVISION] Transient CreateDurableQueue failure for '{}' ({}); retry {}/{} in {:?}",
                    queue_id, reason, retry, policy.max_retries, delay
                );
                if sleep_with_shutdown_check(
                    delay,
                    shutdown,
                    "durable-queue provisioning retry backoff",
                )
                .await
                {
                    return Ok(EnsureQueueOutcome::ShutdownAborted);
                }
            }
        }
    }
}

// --- Supporting types and pure helpers (used by `ensure_durable_queue` above) ---

/// Three-state outcome of a successful [`ensure_durable_queue`] call.
///
/// This distinguishes the shutdown-aborted early return from the two genuine
/// success cases so the caller never mistakes an aborted provisioning attempt
/// for a queue that already existed (and never spawns a consumer under that
/// false premise). Error conditions are still reported via
/// [`NotificationError::QueueProvisioningFailed`], not through this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnsureQueueOutcome {
    /// This call newly created the durable queue (`response.created == true`).
    Created,
    /// The durable queue already existed (`response.created == false`).
    AlreadyExisted,
    /// Shutdown was signalled while sleeping between transient retries; the
    /// call returned early and no queue was provisioned by it.
    ShutdownAborted,
}

/// Builds the two storage `FilterGroup`s used to subscribe the durable queue to
/// `omni.storage.created` and `omni.storage.deleted` events under each of `resource_prefixes`.
///
/// Each group carries one greedy-prefix [`ResourceFilter`] per configured prefix, in the order of
/// the supplied slice (callers that resolve from env — `resolve_queue_provisioning` — pass a
/// de-duplicated, lexicographically sorted list). Every resource at or below any of
/// `resource_prefixes` is matched. No `omni.durablequeue.roundtrip` group is emitted here — that is
/// a test-only concern owned by the Test Writer, not production output.
///
/// # Invariant
///
/// Callers MUST pass a non-empty `resource_prefixes` slice. Per the consumer proto contract, a
/// [`FilterGroup`] with an empty `filters` list matches ALL events of that type — so an empty slice
/// here would silently produce catch-all subscriptions instead of the intended prefix-scoped ones.
/// This invariant is enforced upstream by `resolve_queue_provisioning` (which maps an empty prefix
/// set to `Err` before any provisioning call is made), and is guarded here by an `assert!` so the
/// invariant holds in release builds too — an empty slice fails fast at startup rather than silently
/// installing catch-all subscriptions.
pub fn build_storage_filter_groups(resource_prefixes: &[String]) -> Vec<FilterGroup> {
    assert!(
        !resource_prefixes.is_empty(),
        "build_storage_filter_groups requires at least one resource prefix; an empty slice would produce a FilterGroup with empty filters, which matches ALL events of that type"
    );
    [StorageEventType::Created, StorageEventType::Deleted]
        .into_iter()
        .map(|event_type| FilterGroup {
            event_type: event_type.as_str().to_string(),
            filters: greedy_prefix_filters(resource_prefixes),
        })
        .collect()
}

/// Builds the single `FilterGroup` subscribing the worker's durable queue
/// (WorkerEventQueue) to `omni.thumbnail.generate`.
///
/// Pass real `resource_prefixes`. An empty slice is accepted by the type and by
/// the platform's API, and it produces a queue that receives nothing.
///
/// This comment previously said the opposite, on the strength of the proto:
/// "if no resource filters are supplied then messages will only be filtered by
/// event_type", which is also how the `notification-minimal` double behaves. The
/// deployed service does not. A queue created with an empty filter list binds
/// with a routing key carrying no resource segments — `3aa53.951d4.15117` where
/// a queue that receives traffic has `3aa53.ddece.e2fa5.4f61...` — and matches
/// no published event.
///
/// Nothing reports it. `CreateDurableQueue` succeeds, `PublishEvent` succeeds,
/// the consume stream establishes, and the worker sits idle while every event is
/// written to RabbitMQ and routed nowhere. Dev lost a 4GB upload's worth of
/// generate events to this, and the only way it was found was comparing bindings
/// against a queue that worked.
///
/// The prefixes belong to the *source* address, matching what the server
/// watches, because that is what the generate envelope carries as `resource_id`.
pub fn build_generate_filter_groups(resource_prefixes: &[String]) -> Vec<FilterGroup> {
    vec![FilterGroup {
        event_type: crate::work_events::GENERATE_EVENT_TYPE.to_string(),
        filters: greedy_prefix_filters(resource_prefixes),
    }]
}

fn greedy_prefix_filters(resource_prefixes: &[String]) -> Vec<ResourceFilter> {
    resource_prefixes
        .iter()
        .map(|resource_prefix| ResourceFilter {
            filter_type: FilterType::StartsWithGreedy as i32,
            resource_id: resource_prefix.clone(),
        })
        .collect()
}

/// Classification of a durable-queue provisioning failure.
///
/// Deliberately narrow: it distinguishes only the two decisions the provisioning wrapper needs
/// to make from a transport-level `tonic::Status`. Auth failures (`Unauthenticated` /
/// `PermissionDenied`) are intentionally NOT modeled here — those are handled statefully by
/// `ensure_durable_queue` (token refresh + retry) and never routed through this classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisioningErrorClass {
    /// The request is malformed, the queue configuration is invalid, or the server cannot
    /// service the call at all; retrying the same request will keep failing. Maps from
    /// `tonic::Code::InvalidArgument` and `tonic::Code::Unimplemented`.
    PermanentMisconfig,
    /// The failure is environmental (service unavailable, deadline exceeded, connection-style
    /// transport error) and the request may succeed on a later attempt.
    Transient,
}

/// Classifies a transport-level `tonic::Status` from a provisioning call into the coarse
/// [`ProvisioningErrorClass`] the wrapper acts on.
///
/// Definitively-permanent codes map to [`ProvisioningErrorClass::PermanentMisconfig`] so the
/// wrapper fails fast instead of burning the whole retry budget on a condition that cannot
/// improve: `InvalidArgument` (malformed request / invalid queue config) and `Unimplemented`
/// (a notifications server predating the ensure-exists / `queue_id=2` capability). `Unavailable`,
/// `DeadlineExceeded`, and connection-style failures are transient. All other codes default to
/// [`ProvisioningErrorClass::Transient`], preferring a bounded retry over an immediate abort for
/// genuinely-unknown codes. Auth codes (`Unauthenticated` / `PermissionDenied`) are excluded here
/// on purpose (see the enum doc comment) — the wrapper handles them statefully before consulting
/// this classifier, so the default is never reached for auth decisions.
pub fn classify_provisioning_status(status: &tonic::Status) -> ProvisioningErrorClass {
    match status.code() {
        tonic::Code::InvalidArgument | tonic::Code::Unimplemented => {
            ProvisioningErrorClass::PermanentMisconfig
        }
        // Illustrative of the primary transient codes; the catch-all below handles the rest
        // (including connection-style transport errors) identically, so this arm documents intent
        // rather than changing any outcome.
        tonic::Code::Unavailable | tonic::Code::DeadlineExceeded => {
            ProvisioningErrorClass::Transient
        }
        _ => ProvisioningErrorClass::Transient,
    }
}

/// Retry/backoff knobs the provisioning decision needs, projected out of
/// [`StreamRuntimeArgs`] so the decision logic is a pure, unit-testable unit
/// independent of the full runtime-config surface.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProvisioningRetryPolicy {
    /// Maximum number of transient retries before failing fast.
    pub max_retries: u32,
    /// Base backoff (seconds) applied before the exponent.
    pub base_backoff_secs: u64,
    /// Cap on the backoff-growth exponent (`2^min(retry, cap)`).
    pub exponent_cap: u32,
    /// Ceiling (seconds) applied to every backoff sleep.
    pub max_backoff_secs: u64,
}

/// Terminal condition of a single `CreateDurableQueue` attempt, after the
/// stateful auth refresh-once has already been applied. This is the pure input
/// to [`decide_provisioning_step`] — the I/O (client build, RPC, per-request
/// timeout, token refresh) happens in [`ensure_durable_queue`] and collapses
/// into one of these variants.
#[derive(Debug)]
pub(crate) enum ProvisioningAttempt {
    /// The call returned a response. `queue_id` is `response.queue_id` (the id
    /// the server actually provisioned) and `created` is `response.created`.
    Responded { created: bool, queue_id: String },
    /// The call returned a gRPC error. Auth is resolved upstream, so an
    /// `Unauthenticated` here means the refresh-and-retry-once already failed.
    Failed(tonic::Status),
    /// The call did not return within the per-request deadline.
    TimedOut,
}

impl ProvisioningAttempt {
    /// Short human-readable reason for a retry log line, computed before the
    /// value is consumed by the decision function.
    fn describe(&self) -> String {
        match self {
            ProvisioningAttempt::Responded { created, queue_id } => {
                format!("unexpected response (created={created}, queue_id='{queue_id}')")
            }
            ProvisioningAttempt::Failed(status) => {
                format!("{} ({})", status.message(), status.code())
            }
            ProvisioningAttempt::TimedOut => "request timed out".to_string(),
        }
    }
}

/// The decision [`decide_provisioning_step`] hands back to the wrapper loop.
#[derive(Debug)]
pub(crate) enum ProvisioningDecision {
    /// Terminal success — return this outcome.
    Succeed(EnsureQueueOutcome),
    /// Terminal failure — surface `QueueProvisioningFailed` carrying this status.
    Fail(tonic::Status),
    /// Sleep `delay` (honoring shutdown) and issue another attempt.
    Retry(Duration),
}

/// Pure decision core of [`ensure_durable_queue`]: given the terminal condition
/// of one attempt plus the current retry count and policy, decide whether to
/// succeed, fail fast, or retry. Kept free of I/O so the branching that is most
/// likely to regress silently — success/already-existed, the response-id
/// echo-check, permission/auth short-circuit vs. classifier routing, the
/// permanent-vs-transient split, timeout handling, and the retry-budget
/// boundary — is unit-testable without a live server.
pub(crate) fn decide_provisioning_step(
    attempt: ProvisioningAttempt,
    requested_queue_id: &str,
    retry: u32,
    policy: &ProvisioningRetryPolicy,
) -> ProvisioningDecision {
    match attempt {
        ProvisioningAttempt::Responded { created, queue_id } => {
            if queue_id != requested_queue_id {
                // Defensive echo-check: a conformant server returns the bare
                // queue_id we supplied. A different id means the queue was not
                // provisioned under the id the consumer will attach to, so fail
                // loudly rather than consume from a phantom queue.
                ProvisioningDecision::Fail(tonic::Status::internal(format!(
                    "CreateDurableQueue returned queue_id '{queue_id}' but '{requested_queue_id}' \
                     was requested; the queue was not provisioned under the requested id, so the \
                     consumer will not attach"
                )))
            } else if created {
                ProvisioningDecision::Succeed(EnsureQueueOutcome::Created)
            } else {
                ProvisioningDecision::Succeed(EnsureQueueOutcome::AlreadyExisted)
            }
        }
        ProvisioningAttempt::Failed(status) => {
            // Auth codes are resolved statefully upstream (refresh-once) and are
            // excluded from the pure classifier, so decide them here: a
            // persistent `Unauthenticated` or any `PermissionDenied` is terminal.
            if matches!(
                status.code(),
                Code::Unauthenticated | Code::PermissionDenied
            ) {
                return ProvisioningDecision::Fail(status);
            }
            match classify_provisioning_status(&status) {
                ProvisioningErrorClass::PermanentMisconfig => ProvisioningDecision::Fail(status),
                ProvisioningErrorClass::Transient => decide_transient(retry, policy, status),
            }
        }
        // A server that accepts the request but never responds is treated as a
        // transient failure so a hang converts into bounded retries instead of
        // blocking the startup critical path forever.
        ProvisioningAttempt::TimedOut => decide_transient(
            retry,
            policy,
            tonic::Status::deadline_exceeded(
                "CreateDurableQueue did not respond within the provisioning request timeout",
            ),
        ),
    }
}

/// Retry-or-fail decision for a transient condition: retry with exponential
/// backoff until the budget is exhausted, then fail fast carrying `status`.
fn decide_transient(
    retry: u32,
    policy: &ProvisioningRetryPolicy,
    status: tonic::Status,
) -> ProvisioningDecision {
    if retry >= policy.max_retries {
        return ProvisioningDecision::Fail(status);
    }
    let next = retry + 1;
    // Saturating arithmetic so a large `exponent_cap`/`base_backoff_secs` can
    // neither panic (`pow` overflow in debug) nor wrap (release); the
    // `max_backoff_secs` ceiling below then clamps the saturated value.
    let factor = 2u64
        .checked_pow(next.min(policy.exponent_cap))
        .unwrap_or(u64::MAX);
    let secs = policy
        .base_backoff_secs
        .saturating_mul(factor)
        .min(policy.max_backoff_secs);
    // Jittered for the same reason the reconnect backoff is: replicas provisioning
    // against the same Notification deployment would otherwise retry in lock-step.
    ProvisioningDecision::Retry(super::session::jittered(Duration::from_secs(secs)))
}
