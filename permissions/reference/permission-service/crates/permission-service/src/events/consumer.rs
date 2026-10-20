// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

//! Cross-replica cache invalidation.
//!
//! Each replica keeps its own in-memory evaluation and metadata caches, so a
//! write handled by one replica leaves the others stale until their TTL. This
//! module subscribes to the Notification Service Consumer API and invalidates
//! the local caches whenever another replica publishes a change event.

use super::{EVENT_TYPE_META_CHANGED, EVENT_TYPE_PERMISSIONS_CHANGED};
use crate::cache::{EvaluationCache, InvalidationKey, MetadataCache};
use crate::state::{build_event_consumer_endpoint, AppState};
use clap::Args;
use opentelemetry::KeyValue;
use permission_core::{Action, Principal, Resource};
use permission_service_notification_proto::nvidia::omniverse::notifications::consumer::v1beta::event_consumer_service_client::EventConsumerServiceClient;
use permission_service_notification_proto::nvidia::omniverse::notifications::consumer::v1beta::{
    resource_filter::FilterType, ConsumeNonDurableEventsRequest, ConsumeNonDurableEventsResponse,
    Event, FilterGroup, ResourceFilter,
};
use prost_types::{Struct, Timestamp};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_stream::wrappers::ReceiverStream;

const BASE_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const TOKEN_CHECK_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Args, Clone, Debug)]
pub struct EventConsumerArgs {
    /// Defines if the service needs to consume permission- and metadata-change
    /// events from the Notification Service to invalidate its local caches when
    /// another replica performs a write.
    #[arg(
        id = "enable-notification-consumer",
        long,
        env = "ENABLE_NOTIFICATION_CONSUMER",
        requires_ifs = [("true", "event-consumer-grpc-endpoint")],
    )]
    pub enabled: bool,

    /// Defines an endpoint for the Consumer API of the Notification Service to
    /// subscribe to permission- and metadata-change events.
    #[arg(
        id = "event-consumer-grpc-endpoint",
        long,
        env = "EVENT_CONSUMER_GRPC_ENDPOINT",
        default_value = "http://event-consumer-service:50051"
    )]
    pub event_consumer_grpc_endpoint: String,

    /// Enables TLS for the outbound gRPC connection to the Consumer API.
    /// When set, the connection is verified against the platform's native trust
    /// store unless `event-consumer-tls-ca-certificate-file` provides an
    /// explicit CA bundle.
    #[arg(
        id = "event-consumer-tls-enabled",
        long,
        env = "EVENT_CONSUMER_TLS_ENABLED",
        action = clap::ArgAction::Set,
        default_value_t = false,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = false,
    )]
    pub tls_enabled: bool,

    /// Path to a PEM-encoded CA certificate bundle used to verify the Consumer
    /// API server certificate. When empty, the platform's native root
    /// certificate store is used.
    #[arg(
        id = "event-consumer-tls-ca-certificate-file",
        long,
        env = "EVENT_CONSUMER_TLS_CA_CERTIFICATE_FILE",
        default_value = ""
    )]
    pub tls_ca_certificate_file: String,

    /// Optional override for the TLS server name (SNI / peer name) presented
    /// when connecting to the Consumer API. Defaults to the hostname taken from
    /// `event-consumer-grpc-endpoint`.
    #[arg(
        id = "event-consumer-tls-server-name",
        long,
        env = "EVENT_CONSUMER_TLS_SERVER_NAME",
        default_value = ""
    )]
    pub tls_server_name: String,
}

/// Spawns the background consumer when consumption is enabled. Returns
/// immediately otherwise, leaving cache coherence to TTL expiry.
pub fn spawn_permission_change_consumer(state: AppState) {
    if !state.config.consumer.enabled {
        return;
    }

    tracing::info!(
        "Starting permission-change event consumer against {}",
        state.config.consumer.event_consumer_grpc_endpoint
    );
    tokio::spawn(async move {
        run_consumer(state).await;
    });
}

/// Owns the reconnect loop: re-establishes the subscription forever, applying
/// exponential backoff to connect failures and a short delay to disconnects of
/// an already-established stream.
async fn run_consumer(state: AppState) {
    let mut reason = "initial";
    let mut backoff = BASE_BACKOFF;
    let mut reconnect_token: Option<String> = None;

    loop {
        state
            .metrics
            .notification_consume_reconnect_count
            .add(1, &[KeyValue::new("reason", reason)]);

        let outcome = run_subscription(&state, reconnect_token.take()).await;
        let disconnect = outcome.disconnect;
        record_disconnect_metric(&state, disconnect);

        // Resume the same server-side queue only on quick, expected reconnects.
        // After an error we start fresh, since the queue (short TTL, ~20s) may be
        // gone and presenting a stale token would just fail again.
        reconnect_token = match disconnect {
            Disconnect::StreamEnd | Disconnect::TokenExpiry => outcome.reconnect_token,
            Disconnect::ConnectError | Disconnect::Transport | Disconnect::Unauthenticated => None,
        };

        if matches!(disconnect, Disconnect::Unauthenticated) {
            if let Some(identity) = &state.service_identity {
                identity.refresh().await;
            }
        }

        let delay = if matches!(disconnect, Disconnect::ConnectError) {
            let current = backoff;
            backoff = (backoff * 2).min(MAX_BACKOFF);
            current
        } else {
            backoff = BASE_BACKOFF;
            BASE_BACKOFF
        };

        reason = disconnect.reconnect_reason();
        tracing::warn!(
            reason,
            backoff_ms = delay.as_millis() as u64,
            resuming = reconnect_token.is_some(),
            "Consumer subscription ended; re-opening after backoff"
        );
        tokio::time::sleep(delay).await;
    }
}

/// Why a subscription attempt ended; drives the reconnect reason and backoff.
#[derive(Clone, Copy, Debug)]
enum Disconnect {
    ConnectError,
    Transport,
    Unauthenticated,
    StreamEnd,
    TokenExpiry,
}

impl Disconnect {
    fn reconnect_reason(self) -> &'static str {
        match self {
            Disconnect::ConnectError | Disconnect::Transport => "error",
            Disconnect::Unauthenticated => "unauthenticated",
            Disconnect::StreamEnd => "stream_end",
            Disconnect::TokenExpiry => "token_expiry",
        }
    }
}

/// Result of one subscription attempt: why it ended and the most recent
/// server-issued reconnect token (used to resume the same queue next time).
struct SubscriptionOutcome {
    disconnect: Disconnect,
    reconnect_token: Option<String>,
}

/// Connects, opens one non-durable subscription for both change event types,
/// and reads it until it ends. Re-opens (rather than mutates) the stream when
/// the attached service-identity token is close to expiry, so every stream
/// carries a currently-valid bearer token. When `reconnect_token` is supplied it
/// resumes the existing server-side queue instead of opening a fresh one.
async fn run_subscription(
    state: &AppState,
    reconnect_token: Option<String>,
) -> SubscriptionOutcome {
    let mut latest_token = reconnect_token;

    let endpoint = match build_event_consumer_endpoint(&state.config.consumer) {
        Ok(endpoint) => endpoint,
        Err(err) => {
            tracing::error!("Failed to build Event Consumer Service endpoint: {}", err);
            return SubscriptionOutcome {
                disconnect: Disconnect::ConnectError,
                reconnect_token: latest_token,
            };
        }
    };

    let channel = match endpoint.connect().await {
        Ok(channel) => channel,
        Err(err) => {
            tracing::warn!(
                "Failed to connect to Event Consumer Service at {}: {}",
                state.config.consumer.event_consumer_grpc_endpoint,
                err
            );
            return SubscriptionOutcome {
                disconnect: Disconnect::ConnectError,
                reconnect_token: latest_token,
            };
        }
    };

    let mut client = EventConsumerServiceClient::new(channel);

    let (tx, rx) = tokio::sync::mpsc::channel::<ConsumeNonDurableEventsRequest>(1);
    if tx
        .send(subscribe_request(latest_token.as_deref()))
        .await
        .is_err()
    {
        return SubscriptionOutcome {
            disconnect: Disconnect::ConnectError,
            reconnect_token: latest_token,
        };
    }

    let mut request = tonic::Request::new(ReceiverStream::new(rx));
    if let Some(identity) = &state.service_identity {
        if identity.is_authenticated() {
            match identity.bearer_token() {
                Some(token) => match format!("Bearer {token}").parse() {
                    Ok(value) => {
                        request.metadata_mut().insert("authorization", value);
                    }
                    Err(_) => {
                        tracing::error!(
                            "Consumer service-identity token is not a valid HTTP header value"
                        );
                        return SubscriptionOutcome {
                            disconnect: Disconnect::Unauthenticated,
                            reconnect_token: latest_token,
                        };
                    }
                },
                None => {
                    tracing::warn!(
                        "No valid service-identity token available for the consumer subscription"
                    );
                    return SubscriptionOutcome {
                        disconnect: Disconnect::Unauthenticated,
                        reconnect_token: latest_token,
                    };
                }
            }
        }
    }

    let response = match client.consume_non_durable_events(request).await {
        Ok(response) => response,
        Err(status) => {
            return SubscriptionOutcome {
                disconnect: classify_stream_error(status),
                reconnect_token: latest_token,
            };
        }
    };
    let mut stream = response.into_inner();

    tracing::info!(
        resuming = latest_token.is_some(),
        "Subscribed to permission and metadata change events at {}",
        state.config.consumer.event_consumer_grpc_endpoint
    );
    state.metrics.notification_consume_connected.record(1, &[]);

    // Keep the outbound half open so the server does not treat the subscription
    // as finished; dropping it at scope end tears the stream down cleanly.
    let _tx = tx;

    // Start the first tick one interval out so it only governs token renewal;
    // incoming events are still handled immediately by the `stream.message()`
    // branch of the `select!`.
    let mut token_check = tokio::time::interval_at(
        tokio::time::Instant::now() + TOKEN_CHECK_INTERVAL,
        TOKEN_CHECK_INTERVAL,
    );

    let disconnect = loop {
        tokio::select! {
            message = stream.message() => match message {
                Ok(Some(response)) => {
                    if !response.reconnect_token.is_empty() {
                        latest_token = Some(response.reconnect_token.clone());
                    }
                    handle_response(state, response);
                }
                Ok(None) => break Disconnect::StreamEnd,
                Err(status) => break classify_stream_error(status),
            },
            _ = token_check.tick() => {
                if let Some(identity) = &state.service_identity {
                    if identity.is_authenticated() && identity.is_expiring_soon() {
                        tracing::debug!(
                            "Consumer service-identity token expiring soon; refreshing and \
                             re-opening the subscription"
                        );
                        identity.refresh().await;
                        break Disconnect::TokenExpiry;
                    }
                }
            }
        }
    };

    state.metrics.notification_consume_connected.record(0, &[]);
    SubscriptionOutcome {
        disconnect,
        reconnect_token: latest_token,
    }
}

/// Builds the outbound subscription request. On a fresh subscription it declares
/// one `FilterGroup` per event type (OR-combined); on a resume it presents the
/// reconnect token and omits `filter_groups`, since the filters are unchanged.
fn subscribe_request(reconnect_token: Option<&str>) -> ConsumeNonDurableEventsRequest {
    match reconnect_token {
        Some(token) => ConsumeNonDurableEventsRequest {
            filter_groups: vec![],
            reconnect_token: Some(token.to_string()),
            previous_filter_groups: vec![],
        },
        None => ConsumeNonDurableEventsRequest {
            filter_groups: vec![
                match_all_filter_group(EVENT_TYPE_PERMISSIONS_CHANGED),
                match_all_filter_group(EVENT_TYPE_META_CHANGED),
            ],
            reconnect_token: None,
            previous_filter_groups: vec![],
        },
    }
}

/// A `FilterGroup` that matches every event of `event_type`, whether or not it
/// carries a resource id. An empty `filters` list only delivers events without a
/// resource id, so publisher events that set `Event.resource` (policies scoped to
/// a resource) would be silently dropped; a greedy filter with an empty
/// `resource_id` is the documented way to match all of them.
fn match_all_filter_group(event_type: &str) -> FilterGroup {
    FilterGroup {
        event_type: event_type.to_string(),
        filters: vec![ResourceFilter {
            filter_type: FilterType::StartsWithGreedy as i32,
            resource_id: String::new(),
        }],
    }
}

fn classify_stream_error(status: tonic::Status) -> Disconnect {
    if status.code() == tonic::Code::Unauthenticated {
        tracing::warn!(
            "Consumer subscription rejected as Unauthenticated: {}",
            status.message()
        );
        Disconnect::Unauthenticated
    } else {
        tracing::warn!("Consumer subscription stream error: {}", status);
        Disconnect::Transport
    }
}

fn record_disconnect_metric(state: &AppState, disconnect: Disconnect) {
    let reason = match disconnect {
        Disconnect::Unauthenticated => "unauthenticated",
        Disconnect::Transport => "transport",
        Disconnect::StreamEnd => "stream_closed",
        Disconnect::ConnectError | Disconnect::TokenExpiry => return,
    };
    state
        .metrics
        .notification_consume_errors
        .add(1, &[KeyValue::new("reason", reason)]);
}

fn handle_response(state: &AppState, response: ConsumeNonDurableEventsResponse) {
    for event in response.events {
        handle_event(state, event);
    }
}

fn handle_event(state: &AppState, event: Event) {
    if let Some(published_at) = &event.published_at {
        state
            .metrics
            .notification_consume_delivery_lag
            .record(delivery_lag_ms(published_at), &[]);
    }

    let message = event.message.as_ref();
    match event.event_type.as_str() {
        EVENT_TYPE_PERMISSIONS_CHANGED => invalidate_from_permission_event(
            state,
            &event.event_type,
            message,
            &event.principal_identity,
        ),
        EVENT_TYPE_META_CHANGED => invalidate_from_metadata_event(
            state,
            &event.event_type,
            message,
            &event.principal_identity,
        ),
        other => tracing::debug!("Ignoring change event of unexpected type {}", other),
    }
}

/// Invalidates the evaluation cache from an `omni.permissions.changed` event,
/// reconstructing the same `(principal, action, resource)` key the write path
/// uses and falling back to a full flush on a global or unparseable scope.
#[tracing::instrument(level = "debug", skip(state, message))]
fn invalidate_from_permission_event(
    state: &AppState,
    event_type: &str,
    message: Option<&Struct>,
    principal_identity: &str,
) {
    tracing::debug!(%event_type, %principal_identity, "Received permission change event");
    let outcome = apply_permission_invalidation(&state.evaluation_cache, message);
    state.metrics.notification_consume_count.add(
        1,
        &[
            KeyValue::new("event_type", event_type.to_string()),
            KeyValue::new("outcome", outcome),
        ],
    );
}

fn apply_permission_invalidation(
    evaluation_cache: &EvaluationCache,
    message: Option<&Struct>,
) -> &'static str {
    let principal = string_field(message, "principal");
    let action = string_field(message, "action");
    let resource = string_field(message, "resource");

    match parse_permission_scope(&principal, &action, &resource) {
        Ok(Some((principal, action, resource))) => {
            tracing::debug!(
                ?principal,
                ?action,
                ?resource,
                "Invalidating evaluation cache from received permission change event"
            );
            evaluation_cache.invalidate(&(principal, action, resource));
            "invalidated_targeted"
        }
        Ok(None) => {
            tracing::debug!(
                "Permission change event has global scope; flushing the evaluation cache"
            );
            evaluation_cache.invalidate_all();
            "invalidated_all"
        }
        Err(()) => {
            tracing::debug!(
                "Permission change event scope could not be parsed; flushing the evaluation cache"
            );
            evaluation_cache.invalidate_all();
            "invalidated_all_parse_error"
        }
    }
}

/// Invalidates the metadata and evaluation caches from an
/// `omni.permissions.meta.changed` event, mirroring the service-wide
/// invalidation the metadata write path performs.
#[tracing::instrument(level = "debug", skip(state, message))]
fn invalidate_from_metadata_event(
    state: &AppState,
    event_type: &str,
    message: Option<&Struct>,
    principal_identity: &str,
) {
    tracing::debug!(
        %event_type,
        %principal_identity,
        entity = %string_field(message, "entity"),
        operation = %string_field(message, "operation"),
        name = %string_field(message, "name"),
        "Received metadata change event"
    );
    let outcome =
        apply_metadata_invalidation(&state.evaluation_cache, &state.metadata_cache, message);
    state.metrics.notification_consume_count.add(
        1,
        &[
            KeyValue::new("event_type", event_type.to_string()),
            KeyValue::new("outcome", outcome),
        ],
    );
}

fn apply_metadata_invalidation(
    evaluation_cache: &EvaluationCache,
    metadata_cache: &MetadataCache,
    message: Option<&Struct>,
) -> &'static str {
    let service = string_field(message, "service");

    if service.is_empty() {
        tracing::debug!(
            "Metadata change event has no service; flushing the metadata and evaluation caches"
        );
        metadata_cache.invalidate_all();
        evaluation_cache.invalidate_all();
        "invalidated_all"
    } else {
        tracing::debug!(
            %service,
            "Invalidating metadata and evaluation caches for service from received metadata \
             change event"
        );
        metadata_cache.invalidate_service(&service);
        evaluation_cache.invalidate_service(&service);
        "invalidated_service"
    }
}

/// Reconstructs an [`InvalidationKey`] from the string fields of a permission
/// event. `Ok(None)` means a global scope (full flush); `Err` means a present
/// field failed to parse and the caller should fall back to a full flush.
fn parse_permission_scope(
    principal: &str,
    action: &str,
    resource: &str,
) -> Result<Option<InvalidationKey>, ()> {
    let principal = if principal.is_empty() {
        None
    } else {
        Some(Principal {
            id: principal.to_string(),
            info: None,
        })
    };
    let action = if action.is_empty() {
        None
    } else {
        // The publisher emits the action entity id (`service:name`), which is
        // exactly what `Action::from_str` expects.
        Some(Action::from_str(action).map_err(|_| ())?)
    };
    let resource = if resource.is_empty() {
        None
    } else {
        Some(Resource::from_str(resource).map_err(|_| ())?)
    };

    if principal.is_none() && action.is_none() && resource.is_none() {
        Ok(None)
    } else {
        Ok(Some((principal, action, resource)))
    }
}

fn string_field(message: Option<&Struct>, key: &str) -> String {
    message
        .and_then(|message| message.fields.get(key))
        .and_then(|value| match &value.kind {
            Some(prost_types::value::Kind::StringValue(value)) => Some(value.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn delivery_lag_ms(published_at: &Timestamp) -> u64 {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default();
    let published_ms = published_at.seconds * 1000 + i64::from(published_at.nanos) / 1_000_000;
    (now_ms - published_ms).max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{context_key, EvaluationCacheKey, EvaluationCacheValue};
    use crate::service::authorization::AuthorizationDecision;
    use permission_db::{Meta, ResourceTypeMeta, ServiceMeta};
    use prost_types::value::Kind;
    use prost_types::Value;
    use std::collections::BTreeMap;

    fn eval_cache() -> EvaluationCache {
        EvaluationCache::new(100, Duration::from_secs(60))
    }

    fn metadata_cache() -> MetadataCache {
        MetadataCache::new(Duration::from_secs(60), 100)
    }

    fn allow() -> EvaluationCacheValue {
        (AuthorizationDecision::Allow, None)
    }

    fn eval_key(
        principal: &str,
        service: &str,
        name: &str,
        kind: &str,
        id: &str,
    ) -> EvaluationCacheKey {
        EvaluationCacheKey::new(
            Some(Principal {
                id: principal.to_string(),
                info: None,
            }),
            Some(Action {
                service: service.to_string(),
                name: name.to_string(),
            }),
            Some(Resource {
                kind: kind.to_string(),
                id: id.to_string(),
                data: None,
            }),
            context_key(None),
        )
    }

    fn string_value(value: &str) -> Value {
        Value {
            kind: Some(Kind::StringValue(value.to_string())),
        }
    }

    /// Builds a permission payload exactly as `publisher::publish_permission_event`
    /// does: bare principal id, action entity id, and the Cedar `Display` form for
    /// the resource.
    fn permission_message(
        principal: Option<&Principal>,
        action: Option<&Action>,
        resource: Option<&Resource>,
    ) -> Struct {
        Struct {
            fields: BTreeMap::from([
                (
                    "principal".to_string(),
                    string_value(&principal.map(|p| p.id.clone()).unwrap_or_default()),
                ),
                (
                    "action".to_string(),
                    string_value(&action.map(|a| a.entity_id()).unwrap_or_default()),
                ),
                (
                    "resource".to_string(),
                    string_value(&resource.map(|r| r.to_string()).unwrap_or_default()),
                ),
            ]),
        }
    }

    fn metadata_message(service: &str) -> Struct {
        Struct {
            fields: BTreeMap::from([
                ("service".to_string(), string_value(service)),
                ("entity".to_string(), string_value("resource")),
                ("operation".to_string(), string_value("upsert")),
                ("name".to_string(), string_value("file")),
            ]),
        }
    }

    fn meta(service: &str, kind: &str) -> Meta {
        Meta {
            service: ServiceMeta::new(service),
            resource: ResourceTypeMeta::new(service, kind),
        }
    }

    #[tokio::test]
    async fn permission_event_targeted_invalidation_drops_only_matching_principal() {
        let cache = eval_cache();
        let alice = eval_key("alice", "storage", "read", "file", "secret.txt");
        let bob = eval_key("bob", "storage", "read", "file", "secret.txt");
        cache.set(alice.clone(), allow()).await;
        cache.set(bob.clone(), allow()).await;

        let principal = Principal {
            id: "alice".to_string(),
            info: None,
        };
        let action = Action {
            service: "storage".to_string(),
            name: "read".to_string(),
        };
        let resource = Resource {
            kind: "file".to_string(),
            id: "secret.txt".to_string(),
            data: None,
        };
        let message = permission_message(Some(&principal), Some(&action), Some(&resource));

        let outcome = apply_permission_invalidation(&cache, Some(&message));

        assert_eq!(outcome, "invalidated_targeted");
        assert_eq!(cache.get(&alice).await, None);
        assert_eq!(cache.get(&bob).await, Some(allow()));
    }

    #[tokio::test]
    async fn permission_event_action_only_scope_targets_action() {
        let cache = eval_cache();
        let read = eval_key("alice", "storage", "read", "file", "secret.txt");
        let write = eval_key("alice", "storage", "write", "file", "secret.txt");
        cache.set(read.clone(), allow()).await;
        cache.set(write.clone(), allow()).await;

        let action = Action {
            service: "storage".to_string(),
            name: "read".to_string(),
        };
        let message = permission_message(None, Some(&action), None);

        let outcome = apply_permission_invalidation(&cache, Some(&message));

        assert_eq!(outcome, "invalidated_targeted");
        assert_eq!(cache.get(&read).await, None);
        assert_eq!(cache.get(&write).await, Some(allow()));
    }

    #[tokio::test]
    async fn permission_event_global_scope_flushes_everything() {
        let cache = eval_cache();
        let alice = eval_key("alice", "storage", "read", "file", "secret.txt");
        cache.set(alice.clone(), allow()).await;

        let message = permission_message(None, None, None);
        let outcome = apply_permission_invalidation(&cache, Some(&message));

        assert_eq!(outcome, "invalidated_all");
        assert_eq!(cache.get(&alice).await, None);
    }

    #[tokio::test]
    async fn permission_event_unparseable_action_flushes_everything() {
        let cache = eval_cache();
        let alice = eval_key("alice", "storage", "read", "file", "secret.txt");
        cache.set(alice.clone(), allow()).await;

        let message = Struct {
            fields: BTreeMap::from([
                ("principal".to_string(), string_value("")),
                ("action".to_string(), string_value("not-a-cedar-action")),
                ("resource".to_string(), string_value("")),
            ]),
        };
        let outcome = apply_permission_invalidation(&cache, Some(&message));

        assert_eq!(outcome, "invalidated_all_parse_error");
        assert_eq!(cache.get(&alice).await, None);
    }

    #[tokio::test]
    async fn metadata_event_invalidates_named_service() {
        let eval = eval_cache();
        let meta_cache = metadata_cache();

        let storage_eval = eval_key("alice", "storage", "read", "file", "secret.txt");
        let other_eval = eval_key("alice", "other", "read", "file", "secret.txt");
        eval.set(storage_eval.clone(), allow()).await;
        eval.set(other_eval.clone(), allow()).await;

        let storage_meta = ("storage".to_string(), Some("file".to_string()));
        let other_meta = ("other".to_string(), Some("file".to_string()));
        meta_cache
            .set(storage_meta.clone(), meta("storage", "file"))
            .await;
        meta_cache
            .set(other_meta.clone(), meta("other", "file"))
            .await;

        let message = metadata_message("storage");
        let outcome = apply_metadata_invalidation(&eval, &meta_cache, Some(&message));

        assert_eq!(outcome, "invalidated_service");
        assert_eq!(meta_cache.get(&storage_meta).await, None);
        assert!(meta_cache.get(&other_meta).await.is_some());
        assert_eq!(eval.get(&storage_eval).await, None);
        assert_eq!(eval.get(&other_eval).await, Some(allow()));
    }

    #[tokio::test]
    async fn metadata_event_without_service_flushes_everything() {
        let eval = eval_cache();
        let meta_cache = metadata_cache();

        let key = eval_key("alice", "storage", "read", "file", "secret.txt");
        eval.set(key.clone(), allow()).await;
        let meta_key = ("storage".to_string(), None);
        meta_cache
            .set(meta_key.clone(), meta("storage", "file"))
            .await;

        let message = metadata_message("");
        let outcome = apply_metadata_invalidation(&eval, &meta_cache, Some(&message));

        assert_eq!(outcome, "invalidated_all");
        assert_eq!(eval.get(&key).await, None);
        assert_eq!(meta_cache.get(&meta_key).await, None);
    }

    #[test]
    fn permission_scope_parses_publisher_action_entity_id() {
        let action = Action {
            service: "storage".to_string(),
            name: "read".to_string(),
        };
        // The publisher encodes the action as its entity id (`service:name`).
        let scope = parse_permission_scope("", &action.entity_id(), "").unwrap();
        assert_eq!(scope, Some((None, Some(action), None)));
    }

    #[test]
    fn permission_scope_rejects_malformed_action() {
        // A value without the `service:name` separator cannot be parsed and must
        // signal a full-flush fallback.
        assert!(parse_permission_scope("", "not-a-cedar-action", "").is_err());
    }

    #[test]
    fn fresh_subscription_requests_both_event_types_with_greedy_resource_filter() {
        let request = subscribe_request(None);

        assert!(request.reconnect_token.is_none());
        let event_types: Vec<&str> = request
            .filter_groups
            .iter()
            .map(|group| group.event_type.as_str())
            .collect();
        assert_eq!(
            event_types,
            vec![EVENT_TYPE_PERMISSIONS_CHANGED, EVENT_TYPE_META_CHANGED]
        );

        // Each group must carry a single greedy, empty-resource filter so events
        // are delivered whether or not they carry a resource id.
        for group in &request.filter_groups {
            assert_eq!(group.filters.len(), 1);
            let filter = &group.filters[0];
            assert_eq!(filter.filter_type, FilterType::StartsWithGreedy as i32);
            assert!(filter.resource_id.is_empty());
        }
    }

    #[test]
    fn resumed_subscription_presents_token_and_omits_filters() {
        let request = subscribe_request(Some("queue-token"));

        assert_eq!(request.reconnect_token.as_deref(), Some("queue-token"));
        assert!(request.filter_groups.is_empty());
    }
}
