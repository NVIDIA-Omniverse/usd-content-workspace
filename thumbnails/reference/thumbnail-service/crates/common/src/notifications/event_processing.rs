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

//! Event filtering, conversion, and routing logic

use super::{StorageEvent, StorageEventMessage, StorageEventType};
use crate::outbound::OutboundClients;
use crate::storage::association::disassociate_all_thumbnails;
use crate::storage::AssociationTargetKind;
use crate::work_events::{
    dispatch_generate_for_configured_sizes, set_remote_parent, GenerateSource,
};
use crate::RuntimeConfigArgs;
use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::Event as NotificationEvent;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn, Instrument};

/// A CREATE event field problem that no redelivery fixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CreateEventRejection {
    #[error("CREATE event carries no resource_address")]
    MissingResourceAddress,
    #[error("CREATE event carries no resource_identity")]
    MissingResourceIdentity,
}

/// Centralize the ACK/NACK policy for CREATE-event dispatch failures.
///
/// Input-shape problems from upstream events are permanent for this delivery and
/// ACK, because redelivering the same malformed event loops forever. Transport
/// and dependency failures reaching event aggregation withhold the ACK so the
/// broker redelivers once the dependency recovers.
///
/// Keeping the decision in one place is what stops validation failures and
/// publish failures from drifting into inconsistent redelivery behavior.
///
/// - retryable failure => NACK (`false`)
/// - non-retryable failure => ACK (`true`)
fn ack_for_dispatch_failure(
    resource_address: &str,
    retryable: bool,
    failure: &dyn std::fmt::Display,
) -> bool {
    if retryable {
        error!(
            "Retryable CREATE event failure for '{}': {}. NACKing for redelivery.",
            resource_address, failure
        );
        false
    } else {
        error!(
            "Non-retryable CREATE event failure for '{}': {}. ACKing to avoid infinite redelivery.",
            resource_address, failure
        );
        true
    }
}

pub(crate) fn convert_grpc_event_to_storage_event(
    event: NotificationEvent,
) -> Result<StorageEvent, String> {
    convert_grpc_event_to_storage_event_impl(event)
}

fn convert_grpc_event_to_storage_event_impl(
    event: NotificationEvent,
) -> Result<StorageEvent, String> {
    let message_struct = event.message.ok_or("Event message is missing")?;

    let resource_address = message_struct
        .fields
        .get("resource_address")
        .and_then(|v| v.kind.as_ref())
        .and_then(|k| match k {
            prost_types::value::Kind::StringValue(s) => Some(s.clone()),
            _ => None,
        })
        .ok_or("Missing resource_address in event message")?;

    let resource_identity = message_struct
        .fields
        .get("resource_identity")
        .and_then(|v| v.kind.as_ref())
        .and_then(|k| match k {
            prost_types::value::Kind::StringValue(s) => Some(s.clone()),
            _ => None,
        });

    let mime_type = message_struct
        .fields
        .get("mime_type")
        .and_then(|v| v.kind.as_ref())
        .and_then(|k| match k {
            prost_types::value::Kind::StringValue(s) => Some(s.clone()),
            _ => None,
        });

    let generated_by_thumbnail_pipeline = message_struct
        .fields
        .get("generated_by_thumbnail_pipeline")
        .and_then(|v| v.kind.as_ref())
        .and_then(|k| match k {
            prost_types::value::Kind::BoolValue(value) => Some(*value),
            _ => None,
        });

    // Used as the remote parent of this delivery's span, not forwarded onto the
    // generate events it publishes — those carry the context of the span it
    // parents, so this service appears in the trace instead of being skipped over.
    // Normally absent: the storage service does not stamp CREATE events with trace
    // context yet, and the delivery span roots its own trace until it does.
    //
    // Both fields, because the W3C context is two of them. Reading `traceparent`
    // alone is the asymmetry this hop had against every other one, and it would
    // have gone unnoticed until the day storage starts stamping events — during
    // the very check that was meant to confirm the hop works.
    let string_field = |name: &str| {
        message_struct
            .fields
            .get(name)
            .and_then(|v| v.kind.as_ref())
            .and_then(|k| match k {
                prost_types::value::Kind::StringValue(s) if !s.trim().is_empty() => Some(s.clone()),
                _ => None,
            })
    };
    let traceparent = string_field("traceparent");
    let tracestate = string_field("tracestate");

    Ok(StorageEvent {
        event_type: event.event_type,
        principal_identity: event.principal_identity,
        occurred_at: event
            .occurred_at
            .map(|ts| format!("{}.{}Z", ts.seconds, ts.nanos))
            .unwrap_or_else(|| "unknown".to_string()),
        published_at: event
            .published_at
            .map(|ts| format!("{}.{}Z", ts.seconds, ts.nanos))
            .unwrap_or_else(|| "unknown".to_string()),
        message: StorageEventMessage {
            resource_identity,
            resource_address,
            mime_type,
            generated_by_thumbnail_pipeline,
            traceparent,
            tracestate,
        },
    })
}

/// Outcome of evaluating all preconditions for a `omni.storage.created` event.
pub(crate) enum CreateEventPrecondition {
    /// All checks passed; proceed with task submission.
    Eligible {
        resource_address: String,
        resource_identity: String,
    },
    /// Event is intentionally skipped; ACK without dispatching work.
    Skip,
    /// Permanent field error; ACK to avoid infinite redelivery.
    Reject(CreateEventRejection),
}

fn skip_create_event(
    resource_address: &str,
    reason: impl std::fmt::Display,
) -> CreateEventPrecondition {
    debug!(
        resource_address = %resource_address,
        reason = %reason,
        "Skipping CREATE event"
    );
    CreateEventPrecondition::Skip
}

/// Evaluate every precondition that must hold before submitting a thumbnail task.
///
/// Checks in order:
/// 1. Non-blank `resource_address`
/// 2. Non-blank `resource_identity`
/// 3. Folder path (trailing `/`, `.folder` suffix, `/.folder/` segment)
/// 4. Event was not emitted for a write by the thumbnail pipeline
/// 5. Resource address does not match the configured template output shape
/// 6. File extension is in the allowed set and not in the rejected set
pub(crate) fn check_create_event_preconditions(
    storage_event: &StorageEvent,
    runtime: &RuntimeConfigArgs,
) -> CreateEventPrecondition {
    let raw_address = &storage_event.message.resource_address;

    let resource_address = raw_address.trim().to_string();
    if resource_address.is_empty() {
        return CreateEventPrecondition::Reject(CreateEventRejection::MissingResourceAddress);
    }

    let resource_identity = storage_event
        .message
        .resource_identity
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    if resource_identity.is_empty() {
        return CreateEventPrecondition::Reject(CreateEventRejection::MissingResourceIdentity);
    }

    if resource_address.ends_with('/')
        || resource_address.ends_with(".folder")
        || resource_address.contains("/.folder/")
    {
        return skip_create_event(&resource_address, "folder path");
    }

    if storage_event.message.generated_by_thumbnail_pipeline == Some(true) {
        return skip_create_event(&resource_address, "generated thumbnail pipeline marker");
    }

    // Deliberately a standalone config value, not derived from an existing
    // service_identity (thumbnail-service's own OAuth client ID, or
    // thumbnail-task-queue's): storage does not yet stamp CREATE events with
    // the actual writer's identity (see ADR 0001), so there is no existing
    // identity value guaranteed to equal what storage will eventually emit
    // here. Revisit once storage propagates a real writer principal.
    if runtime
        .event_consumer
        .generated_thumbnail_principal_identity
        .as_deref()
        .is_some_and(|principal| storage_event.principal_identity.trim() == principal.trim())
    {
        return skip_create_event(&resource_address, "generated thumbnail principal");
    }

    if runtime
        .thumbnail_path
        .thumbnail_path_template
        .matches_rendered_output(&resource_address)
    {
        return skip_create_event(&resource_address, "configured template output address");
    }

    let extension = match crate::source_formats::extract_extension(&resource_address) {
        Some(ext) => ext.to_lowercase(),
        None => {
            return skip_create_event(
                &resource_address,
                "resource address does not contain an extension",
            )
        }
    };

    if crate::source_formats::ALLOWED_EXTENSIONS.contains(extension.as_str()) {
        return CreateEventPrecondition::Eligible {
            resource_address,
            resource_identity,
        };
    }

    skip_create_event(
        &resource_address,
        format_args!("unsupported extension .{extension}"),
    )
}

/// Handle an `omni.storage.created` event by publishing one generate event per
/// configured size.
///
/// The returned acknowledgement means the work was durably *enqueued*, not that
/// any thumbnail exists yet. Conversion happens on WorkerEventQueue and never reopens
/// this delivery: a convert failure later is that message's problem, and no
/// per-size convert result is aggregated back onto this CREATE.
async fn handle_create_event(
    storage_event: StorageEvent,
    runtime: &RuntimeConfigArgs,
    outbound: Arc<OutboundClients>,
) -> bool {
    match check_create_event_preconditions(&storage_event, runtime) {
        CreateEventPrecondition::Skip => true,
        CreateEventPrecondition::Reject(rejection) => {
            ack_for_dispatch_failure(&storage_event.message.resource_address, false, &rejection)
        }
        CreateEventPrecondition::Eligible {
            resource_address,
            resource_identity,
        } => {
            let dispatch_start = Instant::now();
            let result = dispatch_generate_for_configured_sizes(
                GenerateSource {
                    resource_identity: &resource_identity,
                    resource_address: &resource_address,
                },
                runtime,
                &outbound,
            )
            .await;

            debug!(
                "[TIMING] Generate work published in {}ms",
                dispatch_start.elapsed().as_millis()
            );

            match result {
                Ok(destinations) => {
                    info!(
                        "Published {} generate event(s) for {}: {:?}",
                        destinations.len(),
                        resource_identity,
                        destinations
                    );
                    true
                }
                // One batch outcome, never "2 of 3 published": this delivery has
                // an acknowledgement to decide and no caller to report to, and a
                // redelivery re-publishes the whole size matrix.
                Err(failure) => {
                    ack_for_dispatch_failure(&resource_address, failure.retryable, &failure)
                }
            }
        }
    }
}

/// Handle an `omni.storage.deleted` event by sweeping every thumbnail associated
/// with the deleted source. The sweep removes association metadata, emits
/// disassociation events, and asks thumbnail-task-queue to delete each successfully
/// disassociated thumbnail blob.
async fn handle_delete_event(storage_event: StorageEvent, outbound: Arc<OutboundClients>) -> bool {
    let resource_address = storage_event.message.resource_address.trim();
    if resource_address.is_empty() {
        error!("DELETE event has no resource_address; ACKing to avoid a redelivery loop");
        return true;
    }

    let auth_token = match outbound.get_auth_token() {
        Ok(t) => t,
        Err(e) if e.is_retryable() => {
            warn!(
                resource_address,
                error = %e,
                "Service auth token unavailable for DELETE event cleanup (transient); NACKing for retry"
            );
            return false;
        }
        Err(e) => {
            error!(
                resource_address,
                error = %e,
                "Service auth token permanently unavailable for DELETE event cleanup (config error); ACKing to avoid redelivery loop"
            );
            return true;
        }
    };

    info!(
        resource_address,
        "Handling omni.storage.deleted notification"
    );

    let outcome = disassociate_all_thumbnails(
        resource_address,
        AssociationTargetKind::File,
        &auth_token,
        outbound,
    )
    .await;

    if let Some(abort) = outcome.aborted {
        warn!(
            resource_address,
            error = %abort,
            "DELETE event cleanup could not enumerate associations; NACKing for retry"
        );
        return false;
    }

    if !outcome.failed.is_empty() {
        let retryable = outcome.failed.iter().any(|f| f.error.is_retryable());
        // Named here rather than left for a reader to cross-reference against
        // `disassociate_all_thumbnails`'s own per-index WARN logs: this is the one
        // summary line an operator watching this event's outcome will see, so it
        // carries the actual reason(s) rather than a generic "transient"/"permanent"
        // label that can be wrong (a "duplicate batch entry" failure, for example,
        // is permanent, not transient, even though it comes back through the same
        // storage-removal-failure path as a genuine transient fault).
        let sample_reasons: Vec<String> = outcome
            .failed
            .iter()
            .take(3)
            .map(|f| f.error.to_string())
            .collect();
        if retryable {
            warn!(
                resource_address,
                succeeded = outcome.succeeded.len(),
                failed = outcome.failed.len(),
                sample_reasons = ?sample_reasons,
                "DELETE event cleanup was incomplete (at least one retryable storage failure); NACKing for retry"
            );
            return false;
        }
        // Every failure is permanent (malformed stored entries, or a storage
        // failure classified non-retryable) — replaying the event cannot fix any
        // of them, so ACK to avoid an infinite redelivery loop.
        error!(
            resource_address,
            succeeded = outcome.succeeded.len(),
            failed = outcome.failed.len(),
            sample_reasons = ?sample_reasons,
            "DELETE event cleanup was incomplete (all failures permanent); ACKing to avoid redelivery loop"
        );
    }

    info!(
        resource_address,
        cleaned = outcome.succeeded.len(),
        "DELETE event cleanup completed"
    );
    true
}

/// Entry point for storage events that have passed the stream-level type gate.
/// Returns `true` when the delivery should be ACKed and `false` for a retryable
/// processing failure that should be redelivered.
///
/// Every delivery gets a span, including ones that turn out to be ineligible.
/// "Why was no thumbnail generated for this upload" is asked far more often than
/// "how long did an eligible one take", and it is unanswerable if a skipped
/// delivery leaves no trace behind.
pub(super) async fn handle_storage_event(
    storage_event: StorageEvent,
    runtime: &RuntimeConfigArgs,
    outbound: Arc<OutboundClients>,
) -> bool {
    let span = delivery_span(&storage_event);

    handle_storage_event_inner(storage_event, runtime, outbound)
        .instrument(span)
        .await
}

/// The delivery span, with the event's trace context as its remote parent.
///
/// Split out of [`handle_storage_event`] so a test can read the resulting span's
/// context *synchronously*, which is what makes "both W3C fields were applied, not
/// just `traceparent`" assertable at all. Driving the same assertion through
/// `handle_storage_event` is flaky under parallel test execution — the probe's
/// thread-local subscriber does not reliably survive that function's awaits — and
/// this function has no awaits, so there is nothing for interleaving to disturb.
fn delivery_span(storage_event: &StorageEvent) -> tracing::Span {
    let span = tracing::info_span!(
        "storage.event",
        event_type = %storage_event.event_type,
        resource_address = %storage_event.message.resource_address,
    );
    set_remote_parent(
        &span,
        storage_event.message.traceparent.as_deref(),
        storage_event.message.tracestate.as_deref(),
    );
    span
}

async fn handle_storage_event_inner(
    storage_event: StorageEvent,
    runtime: &RuntimeConfigArgs,
    outbound: Arc<OutboundClients>,
) -> bool {
    debug!(
        "Processing event: {} from {}",
        storage_event.event_type, storage_event.principal_identity
    );

    match StorageEventType::from_str(&storage_event.event_type) {
        Some(StorageEventType::Created) => {
            handle_create_event(storage_event, runtime, outbound).await
        }
        Some(StorageEventType::Deleted) => handle_delete_event(storage_event, outbound).await,
        Some(StorageEventType::DirCreated | StorageEventType::DirDeleted) | None => {
            warn!(
                event_type = storage_event.event_type,
                "Ignoring an event type that passed the stream filter unexpectedly"
            );
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::StorageEventType;
    use crate::work_events::GenerateFanOutFailure;
    use prost_types::{value::Kind, Struct, Value};
    use std::collections::BTreeMap;

    fn make_event(resource_address: &str, resource_identity: Option<&str>) -> StorageEvent {
        StorageEvent {
            event_type: StorageEventType::Created.as_str().to_string(),
            principal_identity: "test-user".to_string(),
            occurred_at: "2024-01-01T00:00:00Z".to_string(),
            published_at: "2024-01-01T00:00:01Z".to_string(),
            message: StorageEventMessage {
                resource_identity: resource_identity.map(str::to_string),
                resource_address: resource_address.to_string(),
                mime_type: None,
                generated_by_thumbnail_pipeline: None,
                traceparent: None,
                tracestate: None,
            },
        }
    }

    fn make_grpc_event_with_fields(fields: BTreeMap<String, Value>) -> NotificationEvent {
        NotificationEvent {
            event_type: StorageEventType::Created.as_str().to_string(),
            principal_identity: "test-user".to_string(),
            occurred_at: None,
            published_at: None,
            message: Some(Struct { fields }),
        }
    }

    fn string_value(value: &str) -> Value {
        Value {
            kind: Some(Kind::StringValue(value.to_string())),
        }
    }

    fn event_fields_with_address(resource_address: &str) -> BTreeMap<String, Value> {
        BTreeMap::from([(
            "resource_address".to_string(),
            string_value(resource_address),
        )])
    }

    #[test]
    fn a_retryable_dispatch_failure_withholds_the_ack() {
        let failure = GenerateFanOutFailure {
            failed: 1,
            total: 3,
            reason: "256x256: event aggregation unavailable".to_string(),
            retryable: true,
        };

        assert!(!ack_for_dispatch_failure(
            "omniverse://storage/projects/test/image.png",
            failure.retryable,
            &failure
        ));
    }

    #[test]
    fn a_permanent_dispatch_failure_acks_to_stop_redelivery() {
        let failure = GenerateFanOutFailure {
            failed: 3,
            total: 3,
            reason: "256x256: specification width is required".to_string(),
            retryable: false,
        };

        assert!(ack_for_dispatch_failure(
            "omniverse://storage/projects/test/image.png",
            failure.retryable,
            &failure
        ));

        for rejection in [
            CreateEventRejection::MissingResourceAddress,
            CreateEventRejection::MissingResourceIdentity,
        ] {
            assert!(ack_for_dispatch_failure(
                "omniverse://storage/projects/test/image.png",
                false,
                &rejection
            ));
        }
    }

    /// An eligible CREATE publishes one generate event per configured size and
    /// acknowledges on the publish result — not on any conversion, which happens
    /// later on a different queue and never reopens this delivery.
    #[tokio::test]
    async fn eligible_create_publishes_every_size_then_acks() {
        // Installed because `publish_generate_event` reads the process-global
        // runtime config for its deadline. Without this the test passes only when
        // another test in the binary happened to install it first, which is the
        // ordering flake recorded as `thumbnail-service-review-remediation` §11.7.
        crate::test_fixtures::init_test_runtime_config("thumbnail-common-event-processing-tests");
        use crate::test_fixtures::mock_publisher_server::MockPublisherServer;

        let mut runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        runtime.thumbnail_defaults.thumbnail_sizes = "64x64,128x128,256x256"
            .parse()
            .expect("test sizes must parse");
        let publisher = MockPublisherServer::start().await;

        let event = make_event(
            "https://source.example/assets/image.png",
            Some("identity-1"),
        );
        let acked = handle_storage_event(event, &runtime, publisher.outbound()).await;

        assert!(acked, "a fully published fan-out acknowledges the CREATE");
        let published = publisher
            .await_events(3, std::time::Duration::from_secs(2))
            .await;
        assert_eq!(published.len(), 3);
        assert!(published
            .iter()
            .all(|event| event.event_type == crate::work_events::GENERATE_EVENT_TYPE));

        publisher.shutdown().await;
    }

    #[tokio::test]
    async fn a_retryable_publish_failure_withholds_the_create_ack() {
        // Installed because `publish_generate_event` reads the process-global
        // runtime config for its deadline. Without this the test passes only when
        // another test in the binary happened to install it first, which is the
        // ordering flake recorded as `thumbnail-service-review-remediation` §11.7.
        crate::test_fixtures::init_test_runtime_config("thumbnail-common-event-processing-tests");
        use crate::test_fixtures::mock_publisher_server::{MockPublisherServer, PublisherBehavior};

        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let publisher =
            MockPublisherServer::start_with(PublisherBehavior::FailWith(tonic::Code::Unavailable))
                .await;

        let event = make_event(
            "https://source.example/assets/image.png",
            Some("identity-2"),
        );
        let acked = handle_storage_event(event, &runtime, publisher.outbound()).await;

        assert!(
            !acked,
            "work that was never enqueued must not be acknowledged as enqueued"
        );

        publisher.shutdown().await;
    }

    /// A template that cannot render is a property of the configuration, so
    /// redelivering reproduces it. ACK rather than loop forever.
    #[tokio::test]
    async fn a_permanent_dispatch_failure_acks_the_create() {
        // Installed because `publish_generate_event` reads the process-global
        // runtime config for its deadline. Without this the test passes only when
        // another test in the binary happened to install it first, which is the
        // ordering flake recorded as `thumbnail-service-review-remediation` §11.7.
        crate::test_fixtures::init_test_runtime_config("thumbnail-common-event-processing-tests");
        use crate::test_fixtures::mock_publisher_server::{MockPublisherServer, PublisherBehavior};

        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let publisher = MockPublisherServer::start_with(PublisherBehavior::FailWith(
            tonic::Code::InvalidArgument,
        ))
        .await;

        let event = make_event(
            "https://source.example/assets/image.png",
            Some("identity-3"),
        );
        let acked = handle_storage_event(event, &runtime, publisher.outbound()).await;

        assert!(
            acked,
            "a permanently rejected publish must not redeliver forever"
        );

        publisher.shutdown().await;
    }

    #[test]
    fn a_traceparent_on_the_incoming_event_is_carried_through() {
        let mut fields = event_fields_with_address("https://source.example/assets/image.png");
        fields.insert(
            "traceparent".to_string(),
            string_value("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
        );

        let storage_event =
            convert_grpc_event_to_storage_event(make_grpc_event_with_fields(fields)).unwrap();

        assert_eq!(
            storage_event.message.traceparent.as_deref(),
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")
        );
    }

    #[test]
    fn a_blank_traceparent_is_treated_as_absent() {
        let mut fields = event_fields_with_address("https://source.example/assets/image.png");
        fields.insert("traceparent".to_string(), string_value("   "));

        let storage_event =
            convert_grpc_event_to_storage_event(make_grpc_event_with_fields(fields)).unwrap();

        assert_eq!(storage_event.message.traceparent, None);
    }

    /// The W3C context is two fields, and this hop used to read one. Vendor
    /// sampling decisions ride in `tracestate`, so dropping it honors the trace
    /// while quietly overriding the decision about whether to record it.
    #[test]
    fn a_tracestate_travels_with_the_traceparent_it_arrived_with() {
        let mut fields = event_fields_with_address("https://source.example/assets/image.png");
        fields.insert(
            "traceparent".to_string(),
            string_value("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
        );
        fields.insert(
            "tracestate".to_string(),
            string_value("vendor=opaque,other=1"),
        );

        let storage_event =
            convert_grpc_event_to_storage_event(make_grpc_event_with_fields(fields)).unwrap();

        assert_eq!(
            storage_event.message.tracestate.as_deref(),
            Some("vendor=opaque,other=1")
        );
    }

    /// Same blank-is-absent rule as `traceparent`, so an empty vendor string does
    /// not reach the propagator as a value it has to reject.
    #[test]
    fn a_blank_tracestate_is_treated_as_absent() {
        let mut fields = event_fields_with_address("https://source.example/assets/image.png");
        fields.insert(
            "traceparent".to_string(),
            string_value("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
        );
        fields.insert("tracestate".to_string(), string_value("  "));

        let storage_event =
            convert_grpc_event_to_storage_event(make_grpc_event_with_fields(fields)).unwrap();

        assert_eq!(storage_event.message.tracestate, None);
    }

    /// The hop between the converter and the publish: the delivery span adopts
    /// **both** halves of the event's context, not just `traceparent`.
    ///
    /// This is the argument list at the `set_remote_parent` call, which is exactly
    /// what regressed into `None` before — so it is worth a test of its own rather
    /// than being covered by reading. Synchronous on purpose: `delivery_span` has no
    /// awaits, so unlike an assertion driven through `handle_storage_event` there is
    /// nothing here for parallel test execution to disturb.
    #[test]
    fn the_delivery_span_adopts_both_halves_of_the_events_context() {
        use crate::test_fixtures::trace_probe;

        let _probe = trace_probe::probe();

        let mut fields = event_fields_with_address("https://source.example/assets/image.png");
        fields.insert(
            "traceparent".to_string(),
            string_value("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
        );
        fields.insert("tracestate".to_string(), string_value("vendor=opaque"));
        let storage_event =
            convert_grpc_event_to_storage_event(make_grpc_event_with_fields(fields)).unwrap();

        let span = delivery_span(&storage_event);
        let carried = crate::work_events::CapturedTraceContext::from_span(&span);

        let traceparent = carried
            .traceparent
            .as_deref()
            .expect("the probe subscriber must give the delivery span a real context");
        assert_eq!(
            trace_probe::trace_id_of(traceparent),
            Some("4bf92f3577b34da6a3ce929d0e0e4736"),
            "the delivery must continue the trace the storage event named"
        );
        assert_eq!(
            carried.tracestate.as_deref(),
            Some("vendor=opaque"),
            "and must carry the vendor state with it — passing `None` here is the \
             regression this pins, and it is invisible from the trace id alone"
        );
    }

    /// A `tracestate` with no `traceparent` is not a context, and must not become
    /// one. The combination the round-trip documentation calls out and no test
    /// covered: without a parent there is nothing for the vendor state to qualify.
    #[test]
    fn a_tracestate_without_a_traceparent_parents_nothing() {
        let _probe = crate::test_fixtures::trace_probe::probe();

        let mut fields = event_fields_with_address("https://source.example/assets/image.png");
        fields.insert("tracestate".to_string(), string_value("vendor=opaque"));

        let storage_event =
            convert_grpc_event_to_storage_event(make_grpc_event_with_fields(fields)).unwrap();
        assert_eq!(storage_event.message.traceparent, None);
        assert_eq!(
            storage_event.message.tracestate.as_deref(),
            Some("vendor=opaque")
        );

        let span = tracing::info_span!("delivery");
        assert!(
            !crate::work_events::set_remote_parent(
                &span,
                storage_event.message.traceparent.as_deref(),
                storage_event.message.tracestate.as_deref(),
            ),
            "vendor state alone does not identify a parent, so the span must root \
             its own trace"
        );
    }

    #[tokio::test]
    async fn handle_storage_event_acks_rejected_preconditions() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let outbound = Arc::new(crate::outbound::OutboundClients::for_tests());

        let missing_address = make_event("   ", Some("id"));
        assert!(
            handle_storage_event(missing_address, &runtime, outbound.clone()).await,
            "missing address is permanent and should ACK"
        );

        let missing_identity = make_event("omniverse://storage/projects/test/image.png", None);
        assert!(
            handle_storage_event(missing_identity, &runtime, outbound).await,
            "missing identity is permanent and should ACK"
        );
    }

    #[tokio::test]
    async fn handle_storage_event_acks_skipped_preconditions() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let outbound = Arc::new(crate::outbound::OutboundClients::for_tests());

        for event in [
            make_event("omniverse://storage/projects/test/", Some("id")),
            make_event("omniverse://storage/projects/test/README", Some("id")),
            make_event("omniverse://storage/projects/test/model.usd", Some("id")),
        ] {
            assert!(
                handle_storage_event(event, &runtime, outbound.clone()).await,
                "skipped events should ACK without task submission"
            );
        }
    }

    #[tokio::test]
    async fn handle_storage_event_sweeps_deleted_resource_without_requiring_identity() {
        use crate::test_fixtures::mock_storage_server::{
            mock_storage, unique_test_address, MetadataOp,
        };

        let runtime = crate::test_fixtures::init_test_runtime_config(
            "thumbnail-common-event-processing-tests",
        );
        let outbound = Arc::new(crate::outbound::OutboundClients::for_tests());
        let resource_address = unique_test_address("deleted-source.jpg");
        let mut event = make_event(&resource_address, None);
        event.event_type = StorageEventType::Deleted.as_str().to_string();

        assert!(
            handle_storage_event(event, runtime, outbound).await,
            "an empty but successfully enumerated association set should ACK"
        );

        assert!(
            mock_storage()
                .calls_for(&resource_address)
                .iter()
                .any(|call| call.op == MetadataOp::Get),
            "deleted-event handling must invoke the disassociate-all metadata sweep"
        );
    }

    #[test]
    fn convert_grpc_event_defaults_missing_timestamps_to_unknown() {
        let event = make_grpc_event_with_fields(event_fields_with_address(
            "omniverse://storage/projects/test/image.png",
        ));

        let storage_event = convert_grpc_event_to_storage_event(event).unwrap();

        assert_eq!(storage_event.occurred_at, "unknown");
        assert_eq!(storage_event.published_at, "unknown");
    }

    #[test]
    fn convert_grpc_event_ignores_non_string_optional_fields() {
        let mut fields = event_fields_with_address("omniverse://storage/projects/test/image.png");
        fields.insert(
            "resource_identity".to_string(),
            Value {
                kind: Some(Kind::NumberValue(123.0)),
            },
        );
        fields.insert(
            "mime_type".to_string(),
            Value {
                kind: Some(Kind::BoolValue(true)),
            },
        );

        let storage_event =
            convert_grpc_event_to_storage_event(make_grpc_event_with_fields(fields)).unwrap();

        assert_eq!(storage_event.message.resource_identity, None);
        assert_eq!(storage_event.message.mime_type, None);
    }

    #[test]
    fn precondition_skips_folder_trailing_slash() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let event = make_event("omniverse://storage/projects/test/", Some("id"));
        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Skip
        ));
    }

    #[test]
    fn precondition_skips_folder_dot_folder_suffix() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let event = make_event("omniverse://storage/projects/test.folder", Some("id"));
        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Skip
        ));
    }

    #[test]
    fn precondition_skips_dot_folder_path_segment() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let event = make_event("omniverse://storage/projects/.folder/image.png", Some("id"));
        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Skip
        ));
    }

    #[test]
    fn precondition_rejects_blank_resource_address() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let event = make_event("   ", Some("id"));
        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Reject(CreateEventRejection::MissingResourceAddress)
        ));
    }

    #[test]
    fn precondition_rejects_missing_resource_identity() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let event = make_event("omniverse://storage/projects/test/image.png", None);
        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Reject(CreateEventRejection::MissingResourceIdentity)
        ));
    }

    #[test]
    fn precondition_rejects_missing_resource_identity_before_folder_skip() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let event = make_event("omniverse://storage/projects/test/", None);
        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Reject(CreateEventRejection::MissingResourceIdentity)
        ));
    }

    #[test]
    fn precondition_rejects_blank_resource_identity() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let event = make_event("omniverse://storage/projects/test/image.png", Some("   "));
        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Reject(CreateEventRejection::MissingResourceIdentity)
        ));
    }

    #[test]
    fn precondition_eligible_for_valid_image() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let event = make_event(
            "omniverse://storage/projects/test/image.png",
            Some("valid-id"),
        );
        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Eligible { .. }
        ));
    }

    #[test]
    fn precondition_skips_generated_thumbnail_marker_true() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let mut event = make_event(
            "https://unrelated.example/arbitrary/output.png",
            Some("generated-id"),
        );
        event.message.generated_by_thumbnail_pipeline = Some(true);

        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Skip
        ));
    }

    #[test]
    fn precondition_marker_false_remains_eligible() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let mut event = make_event("https://source.example/assets/image.png", Some("source-id"));
        event.message.generated_by_thumbnail_pipeline = Some(false);

        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Eligible { .. }
        ));
    }

    #[test]
    fn convert_grpc_event_parses_generated_thumbnail_marker() {
        let mut fields = event_fields_with_address("https://storage.example/output.png");
        fields.insert(
            "generated_by_thumbnail_pipeline".to_string(),
            Value {
                kind: Some(Kind::BoolValue(true)),
            },
        );

        let storage_event =
            convert_grpc_event_to_storage_event(make_grpc_event_with_fields(fields)).unwrap();

        assert_eq!(
            storage_event.message.generated_by_thumbnail_pipeline,
            Some(true)
        );
    }

    #[test]
    fn precondition_skips_generated_thumbnail_principal_at_any_destination() {
        let mut runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        runtime
            .event_consumer
            .generated_thumbnail_principal_identity = Some("thumbnail-task-worker".to_string());
        let mut event = make_event(
            "https://unrelated.example/arbitrary/output.png",
            Some("generated-id"),
        );
        event.principal_identity = "thumbnail-task-worker".to_string();

        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Skip
        ));
    }

    #[test]
    fn precondition_without_generated_thumbnail_principal_keeps_source_eligible() {
        let mut runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        runtime
            .event_consumer
            .generated_thumbnail_principal_identity = None;
        let mut event = make_event("https://source.example/assets/image.png", Some("source-id"));
        event.principal_identity = String::new();

        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Eligible { .. }
        ));
    }

    /// The loop this guards against: `task-worker` writes a thumbnail into
    /// watched storage, storage emits a CREATE for it, and that CREATE arrives
    /// back on StorageEventQueue. Left unguarded the service thumbnails its own output,
    /// appending another `.auto.<ext>` on every pass until storage fills.
    ///
    /// Two independent checks stop it, and this asserts each one *alone*,
    /// because only one of them currently works. The writer-principal check is
    /// inert — storage does not yet stamp a writer principal on CREATE events —
    /// so template-output matching is carrying the whole guard today, and a
    /// combined test would pass even if that one silently broke.
    #[test]
    fn a_worker_written_thumbnail_create_is_skipped() {
        let mut runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        runtime
            .event_consumer
            .generated_thumbnail_principal_identity = Some("thumbnail-task-worker".to_string());

        // The address the worker actually wrote to: the rendered output of the
        // configured template, not an arbitrary path.
        let destination = runtime
            .thumbnail_path
            .thumbnail_path_template
            .render(&crate::thumbnail_path::ThumbnailPathContext {
                resource_address: "https://source.example/assets/photo.jpg",
                resource_identity: "https://source.example/assets/photo.jpg?version=1",
                width: 256,
                height: 256,
                output_encoding: "PNG",
                quality: 85,
            })
            .expect("the configured test template must render");

        // 1. Template-output matching alone, with the writer principal absent —
        //    which is the situation in every environment today.
        let mut unattributed = make_event(&destination, Some("generated-id"));
        unattributed.principal_identity = "storage-service".to_string();
        assert!(
            matches!(
                check_create_event_preconditions(&unattributed, &runtime),
                CreateEventPrecondition::Skip
            ),
            "the rendered destination must be skipped on shape alone; this check is \
             the only active recursion guard"
        );

        // 2. Writer-principal matching alone, at an address the template would
        //    never produce — the path that starts working once storage
        //    propagates the writer.
        let mut attributed = make_event(
            "https://unrelated.example/somewhere/else.png",
            Some("generated-id"),
        );
        attributed.principal_identity = "thumbnail-task-worker".to_string();
        assert!(
            matches!(
                check_create_event_preconditions(&attributed, &runtime),
                CreateEventPrecondition::Skip
            ),
            "a CREATE attributed to the worker must be skipped wherever it was written"
        );

        // And the guard must not be so broad that real sources stop generating.
        let genuine = make_event("https://source.example/assets/photo.jpg", Some("source-id"));
        assert!(
            matches!(
                check_create_event_preconditions(&genuine, &runtime),
                CreateEventPrecondition::Eligible { .. }
            ),
            "the source itself must remain eligible, or nothing is ever generated"
        );
    }

    #[test]
    fn precondition_keeps_normal_source_in_former_thumbnail_prefix_eligible() {
        let mut runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        runtime
            .event_consumer
            .generated_thumbnail_principal_identity = Some("thumbnail-task-worker".to_string());
        let event = make_event(
            "https://storage.example/thumbnails/ordinary-source.png",
            Some("source-id"),
        );

        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Eligible { .. }
        ));
    }

    #[test]
    fn precondition_skips_configured_template_output_address() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let template = &runtime.thumbnail_path.thumbnail_path_template;
        let rendered = template
            .render(&crate::thumbnail_path::ThumbnailPathContext {
                resource_address: "https://source.example/path/image.jpg",
                resource_identity: "https://source.example/path/image.jpg?version=1",
                width: 256,
                height: 256,
                output_encoding: "PNG",
                quality: 85,
            })
            .expect("rendered destination");
        let event = make_event(&rendered, Some("generated-id"));

        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Skip
        ));
    }

    #[test]
    fn precondition_skips_file_with_no_extension() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let event = make_event("omniverse://storage/projects/test/README", Some("valid-id"));
        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Skip
        ));
    }

    #[test]
    fn precondition_skips_unsupported_extension() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        let event = make_event(
            "omniverse://storage/projects/test/model.usd",
            Some("valid-id"),
        );
        assert!(matches!(
            check_create_event_preconditions(&event, &runtime),
            CreateEventPrecondition::Skip
        ));
    }

    /// The allowlist and the conversion library's decoder set are one set. An
    /// extension no decoder covers must be skipped here rather than published as
    /// generate work the worker can only permanently reject.
    #[test]
    fn precondition_skips_extensions_with_no_decoder() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();

        for extension in ["svg", "heic", "heif", "avif"] {
            let address = format!("https://storage.example/assets/source.{extension}");
            assert!(
                matches!(
                    check_create_event_preconditions(&make_event(&address, Some("id")), &runtime),
                    CreateEventPrecondition::Skip
                ),
                ".{extension} has no decoder in crates/thumbnail-convert, so it must not \
                 become generate work"
            );
        }
    }

    #[test]
    fn precondition_accepts_every_decodable_extension() {
        let runtime = crate::tests::parse_runtime_config_from_env_for_tests();

        for extension in [
            "jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp", "ico",
        ] {
            let address = format!("https://storage.example/assets/source.{extension}");
            assert!(
                matches!(
                    check_create_event_preconditions(&make_event(&address, Some("id")), &runtime),
                    CreateEventPrecondition::Eligible { .. }
                ),
                ".{extension} is decodable and must remain eligible"
            );
        }
    }
}
