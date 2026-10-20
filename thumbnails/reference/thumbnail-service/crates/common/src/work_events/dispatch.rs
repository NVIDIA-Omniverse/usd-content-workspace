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

//! Turning an eligible source into published generate work.
//!
//! Both producers of thumbnail work land here: the storage-CREATE consumer,
//! which fans out one event per configured size, and the `CreateThumbnails` API,
//! which fans out one per caller-supplied specification. They share the
//! destination template, the message builder, and the publisher, and differ only
//! in how they report a partial failure — which follows from who is listening.
//!
//! A CREATE delivery has an acknowledgement to decide and nobody to tell, so any
//! failure fails the batch. An API caller can act on knowing which specification
//! failed, so that path reports per-specification outcomes.

use tracing::{error, info};

use super::{
    build_generate_message, publish_generate_event, resolve_generate_spec, GenerateContractError,
    GenerateMessageParams, GeneratePublishError, WorkResource,
};
use crate::config::RuntimeConfigArgs;
use crate::outbound::OutboundClients;
use crate::storage::AssociationTargetKind;
use crate::thumbnail_path::{ThumbnailPathContext, ThumbnailPathError};
use crate::types::ImageProcessingSpec;

/// The source one or more generate events are being dispatched for.
///
/// Carries no trace context: every published message takes it from the span
/// active at the point of publish, so both producers are traced by being inside a
/// span rather than by remembering to pass one along.
#[derive(Debug, Clone, Copy)]
pub struct GenerateSource<'a> {
    pub resource_identity: &'a str,
    pub resource_address: &'a str,
}

/// Why one generate event was not dispatched.
#[derive(Debug, thiserror::Error)]
pub enum GenerateDispatchError {
    #[error("generate dispatch requires a non-empty resource_identity")]
    MissingResourceIdentity,

    #[error("generate dispatch requires a non-empty resource_address")]
    MissingResourceAddress,

    #[error("destination template render failed: {0}")]
    Template(#[from] ThumbnailPathError),

    #[error(transparent)]
    Contract(#[from] GenerateContractError),

    #[error(transparent)]
    Publish(#[from] GeneratePublishError),
}

impl GenerateDispatchError {
    /// Whether the originating delivery should be left unacknowledged.
    ///
    /// Only a publish failure can be retryable. A blank identity, a template that
    /// will not render, or a specification that fails validation are fixed
    /// properties of the input and the configuration: redelivering reproduces
    /// them, so the CREATE is acknowledged rather than looping forever.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Publish(error) => error.is_retryable(),
            Self::MissingResourceIdentity
            | Self::MissingResourceAddress
            | Self::Template(_)
            | Self::Contract(_) => false,
        }
    }
}

/// Dispatch exactly one generate event and return the destination it was
/// published for.
pub async fn dispatch_generate_event(
    source: GenerateSource<'_>,
    specification: &ImageProcessingSpec,
    runtime: &RuntimeConfigArgs,
    outbound: &OutboundClients,
) -> Result<String, GenerateDispatchError> {
    let resource_identity = source.resource_identity.trim();
    if resource_identity.is_empty() {
        return Err(GenerateDispatchError::MissingResourceIdentity);
    }
    let resource_address = source.resource_address.trim();
    if resource_address.is_empty() {
        return Err(GenerateDispatchError::MissingResourceAddress);
    }

    // Resolved before rendering because the template reads the dimensions and
    // encoding from it, and the message must carry exactly what was rendered.
    let resolved = resolve_generate_spec(specification)?;

    let destination =
        runtime
            .thumbnail_path
            .thumbnail_path_template
            .render(&ThumbnailPathContext {
                resource_address,
                resource_identity,
                width: resolved.width,
                height: resolved.height,
                output_encoding: &resolved.encoding_format.to_string(),
                quality: resolved.quality,
            })?;

    let message = build_generate_message(GenerateMessageParams {
        source: WorkResource {
            resource_address: resource_address.to_string(),
            resource_identity: Some(resource_identity.to_string()),
        },
        destination_resource_address: &destination,
        target_type: AssociationTargetKind::File,
        specification,
    })?;

    publish_generate_event(outbound, &message).await?;

    Ok(destination)
}

/// A CREATE fan-out that did not fully succeed.
///
/// There is no partial success: the CREATE path reports one batch outcome, so a
/// caller never sees "2 of 3 sizes published".
#[derive(Debug, thiserror::Error)]
#[error("{failed} of {total} generate publishes failed: {reason}")]
pub struct GenerateFanOutFailure {
    pub failed: usize,
    pub total: usize,
    pub reason: String,
    /// True when at least one failure could succeed on redelivery.
    pub retryable: bool,
}

/// Fan out one generate event per configured `THUMBNAIL_SIZES` entry.
///
/// Every size is attempted in the same pass even after one fails, so a single
/// bad size does not hide the state of the others in the logs. Redelivery
/// re-publishes the whole matrix; duplicates are absorbed by the worker's
/// destination idempotency rather than by a per-size publish journal.
pub async fn dispatch_generate_for_configured_sizes(
    source: GenerateSource<'_>,
    runtime: &RuntimeConfigArgs,
    outbound: &OutboundClients,
) -> Result<Vec<String>, GenerateFanOutFailure> {
    let sizes = &runtime.thumbnail_defaults.thumbnail_sizes;

    info!(
        resource_address = source.resource_address,
        sizes = sizes.len(),
        "Dispatching generate work for an eligible source"
    );

    let mut destinations = Vec::with_capacity(sizes.len());
    let mut failures = Vec::new();
    let mut retryable = false;

    for &(width, height) in sizes.iter() {
        let specification = crate::converters::create_processing_spec_with(runtime, width, height);
        match dispatch_generate_event(source, &specification, runtime, outbound).await {
            Ok(destination) => {
                info!(
                    resource_address = source.resource_address,
                    destination, width, height, "Published generate work"
                );
                destinations.push(destination);
            }
            Err(error) => {
                retryable |= error.is_retryable();
                error!(
                    resource_address = source.resource_address,
                    width,
                    height,
                    %error,
                    "Failed to publish generate work"
                );
                failures.push(format!("{width}x{height}: {error}"));
            }
        }
    }

    if failures.is_empty() {
        return Ok(destinations);
    }

    Err(GenerateFanOutFailure {
        failed: failures.len(),
        total: sizes.len(),
        reason: failures.join("; "),
        retryable,
    })
}

/// One specification that could not be dispatched, reported back to an API caller.
#[derive(Debug)]
pub struct SpecificationDispatchFailure {
    /// The fully resolved specification that was attempted.
    pub specification: ImageProcessingSpec,
    pub error: GenerateDispatchError,
}

/// The per-specification result an API caller receives.
#[derive(Debug, Default)]
pub struct SpecificationDispatchOutcome {
    pub destinations: Vec<String>,
    pub failed: Vec<SpecificationDispatchFailure>,
}

/// Dispatch one generate event per supplied specification, reporting each
/// outcome separately.
pub async fn dispatch_generate_per_specification(
    source: GenerateSource<'_>,
    specifications: &[ImageProcessingSpec],
    runtime: &RuntimeConfigArgs,
    outbound: &OutboundClients,
) -> SpecificationDispatchOutcome {
    let mut outcome = SpecificationDispatchOutcome::default();

    for specification in specifications {
        match dispatch_generate_event(source, specification, runtime, outbound).await {
            Ok(destination) => outcome.destinations.push(destination),
            Err(error) => {
                error!(
                    resource_address = source.resource_address,
                    %error,
                    "Failed to publish caller-requested generate work"
                );
                outcome.failed.push(SpecificationDispatchFailure {
                    specification: specification.clone(),
                    error,
                });
            }
        }
    }

    outcome
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::test_fixtures::mock_publisher_server::{MockPublisherServer, PublisherBehavior};
    use crate::work_events::{CapturedTraceContext, GENERATE_EVENT_TYPE};
    use tracing::Instrument as _;

    const SOURCE_ADDRESS: &str = "https://source.example/assets/image.jpg";
    const SOURCE_IDENTITY: &str = "https://source.example/assets/image.jpg?version=1";

    fn runtime_with_sizes(sizes: &str) -> RuntimeConfigArgs {
        // Installed here rather than per test, because every test in this module
        // publishes and `publish_generate_event` reads the process-global config for
        // its deadline. Without it a test passes only when some *other* test in the
        // binary installed the global first: `cargo test -p thumbnail-common --lib
        // work_events` fails 14 tests on `main` for exactly that reason. Threading
        // `runtime` through instead is `thumbnail-service-review-remediation` §11.8;
        // this is the half that makes the tests self-sufficient today.
        crate::test_fixtures::init_test_runtime_config("thumbnail-common-work-events-tests");

        let mut runtime = crate::tests::parse_runtime_config_from_env_for_tests();
        runtime.thumbnail_defaults.thumbnail_sizes = sizes.parse().expect("test sizes must parse");
        runtime
    }

    fn source() -> GenerateSource<'static> {
        GenerateSource {
            resource_identity: SOURCE_IDENTITY,
            resource_address: SOURCE_ADDRESS,
        }
    }

    #[tokio::test]
    async fn three_sizes_publish_three_distinct_events() {
        let runtime = runtime_with_sizes("64x64,128x128,256x256");
        let publisher = MockPublisherServer::start().await;
        let outbound = publisher.outbound();

        let destinations = dispatch_generate_for_configured_sizes(source(), &runtime, &outbound)
            .await
            .expect("a healthy publisher accepts every size");

        assert_eq!(destinations.len(), 3);

        let published = publisher.await_events(3, Duration::from_secs(2)).await;
        assert_eq!(
            published.len(),
            3,
            "one event per size, not one carrying three"
        );
        for event in &published {
            assert_eq!(event.event_type, GENERATE_EVENT_TYPE);
            assert_eq!(event.resource_id, SOURCE_ADDRESS);
        }

        let mut dimensions: Vec<(f64, f64)> = published
            .iter()
            .map(|event| {
                (
                    event
                        .message
                        .pointer("/specification/width")
                        .unwrap()
                        .as_f64()
                        .unwrap(),
                    event
                        .message
                        .pointer("/specification/height")
                        .unwrap()
                        .as_f64()
                        .unwrap(),
                )
            })
            .collect();
        dimensions.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(dimensions, [(64.0, 64.0), (128.0, 128.0), (256.0, 256.0)]);

        let mut unique: Vec<&str> = published
            .iter()
            .map(|event| event.message_str("/destination_resource_address").unwrap())
            .collect();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 3, "each size gets its own destination");

        publisher.shutdown().await;
    }

    /// Every size is attempted in the same pass, and the result is one batch
    /// failure — there is no "2 of 3 published" outcome for a CREATE to report.
    #[tokio::test]
    async fn a_publish_failure_fails_the_whole_batch_after_attempting_every_size() {
        let runtime = runtime_with_sizes("64x64,128x128,256x256");
        let publisher =
            MockPublisherServer::start_with(PublisherBehavior::FailWith(tonic::Code::Unavailable))
                .await;
        let outbound = publisher.outbound();

        let failure = dispatch_generate_for_configured_sizes(source(), &runtime, &outbound)
            .await
            .expect_err("an unavailable publisher must fail the dispatch");

        assert_eq!(failure.total, 3);
        assert_eq!(failure.failed, 3);
        assert!(failure.retryable, "Unavailable is worth another delivery");

        // The recorder saw all three attempts, so a mid-fan-out failure did not
        // abandon the sizes after it.
        let attempted = publisher.settle(Duration::from_millis(100)).await;
        assert_eq!(
            attempted.len(),
            3,
            "every size is attempted in the same pass"
        );

        publisher.shutdown().await;
    }

    #[tokio::test]
    async fn a_permanent_publish_rejection_is_not_retryable() {
        let runtime = runtime_with_sizes("256x256");
        let publisher = MockPublisherServer::start_with(PublisherBehavior::FailWith(
            tonic::Code::InvalidArgument,
        ))
        .await;
        let outbound = publisher.outbound();

        let failure = dispatch_generate_for_configured_sizes(source(), &runtime, &outbound)
            .await
            .expect_err("InvalidArgument must fail the dispatch");

        assert!(
            !failure.retryable,
            "redelivering the same rejected event reproduces the rejection"
        );

        publisher.shutdown().await;
    }

    #[tokio::test]
    async fn a_blank_identity_is_a_permanent_dispatch_failure() {
        let runtime = runtime_with_sizes("256x256");
        let publisher = MockPublisherServer::start().await;
        let outbound = publisher.outbound();

        let failure = dispatch_generate_for_configured_sizes(
            GenerateSource {
                resource_identity: "   ",
                resource_address: SOURCE_ADDRESS,
            },
            &runtime,
            &outbound,
        )
        .await
        .expect_err("a blank identity cannot produce work");

        assert!(!failure.retryable);
        assert!(
            publisher.settle(Duration::from_millis(50)).await.is_empty(),
            "nothing should be published for an unusable source"
        );

        publisher.shutdown().await;
    }

    /// Every event in a fan-out carries the context of the one span that published
    /// them, so a caller can find all N conversions from the request that asked for
    /// them — and so the trace does not fork into N unrelated traces.
    #[tokio::test]
    async fn the_publishing_span_reaches_every_event_in_a_fan_out() {
        use crate::test_fixtures::trace_probe;

        let _probe = trace_probe::probe();
        let runtime = runtime_with_sizes("64x64,128x128");
        let publisher = MockPublisherServer::start().await;
        let outbound = publisher.outbound();

        let span = tracing::info_span!("test.publisher");
        let expected = CapturedTraceContext::from_span(&span);
        let expected_traceparent = expected
            .traceparent
            .expect("the probe subscriber must give the span a real span context");

        dispatch_generate_for_configured_sizes(source(), &runtime, &outbound)
            .instrument(span)
            .await
            .expect("dispatch should succeed");

        let published = publisher.await_events(2, Duration::from_secs(2)).await;
        assert_eq!(published.len(), 2);

        for event in &published {
            let traceparent = event
                .message_str("/traceparent")
                .expect("a publish inside a span must carry trace context");
            assert_eq!(
                trace_probe::trace_id_of(traceparent),
                trace_probe::trace_id_of(&expected_traceparent),
                "every event belongs to the publishing span's trace"
            );
            assert_eq!(
                trace_probe::span_id_of(traceparent),
                trace_probe::span_id_of(&expected_traceparent),
                "and names the publishing span as the parent, not one event's sibling"
            );
        }

        publisher.shutdown().await;
    }

    /// The clause the storage-event requirement asserts and nothing was proving: a
    /// `tracestate` arriving on a delivery reaches the generate events it publishes.
    ///
    /// Nothing forwards the field, which is why it needs asserting rather than
    /// reading: the delivery span takes the event's context as its remote parent, an
    /// OpenTelemetry span context carries `TraceState`, and the publish injects the
    /// *current* span. The vendor state survives by inheritance, so it would break
    /// silently the moment the remote parent stopped being applied.
    ///
    /// The span is built here exactly as `handle_storage_event` builds its own —
    /// `info_span!` then `set_remote_parent` — rather than by calling that function.
    /// Driving the real delivery was tried first and is **flaky under parallelism**:
    /// the probe captured no spans at all (`exported spans []`) on three runs in four,
    /// while passing every time under `--test-threads=1`. That is the thread-local
    /// subscriber hazard `test_fixtures::trace_probe` documents, and asserting through
    /// it would have shipped exactly the kind of flake `CLAUDE.md` says to fix rather
    /// than tolerate. What is left uncovered is one hop: that `handle_storage_event`
    /// passes the event's `tracestate` into `set_remote_parent`, which the converter
    /// tests cover from the other side.
    #[tokio::test]
    async fn an_upstream_tracestate_reaches_every_event_in_a_fan_out() {
        use crate::test_fixtures::trace_probe;
        use crate::work_events::set_remote_parent;

        const UPSTREAM: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

        let _probe = trace_probe::probe();
        let runtime = runtime_with_sizes("64x64,128x128");
        let publisher = MockPublisherServer::start().await;
        let outbound = publisher.outbound();

        let span = tracing::info_span!("storage.event");
        assert!(
            set_remote_parent(&span, Some(UPSTREAM), Some("vendor=opaque")),
            "the upstream context must be adopted, or everything below asserts nothing"
        );
        // Read synchronously, and loudly: if the probe subscriber is not in effect
        // this is `None`, and every assertion after it would pass vacuously.
        let captured = CapturedTraceContext::from_span(&span);
        assert_eq!(
            captured.tracestate.as_deref(),
            Some("vendor=opaque"),
            "the delivery span must carry the vendor state before anything publishes"
        );

        dispatch_generate_for_configured_sizes(source(), &runtime, &outbound)
            .instrument(span)
            .await
            .expect("dispatch should succeed");

        let published = publisher.await_events(2, Duration::from_secs(2)).await;
        assert_eq!(published.len(), 2);

        for event in &published {
            assert_eq!(
                event.message_str("/tracestate"),
                Some("vendor=opaque"),
                "the upstream vendor state must reach the work the delivery published, \
                 or a sampling decision made upstream is silently overridden here"
            );
            assert_eq!(
                event
                    .message_str("/traceparent")
                    .and_then(trace_probe::trace_id_of),
                Some("4bf92f3577b34da6a3ce929d0e0e4736"),
                "and it must ride on this service's own span inside the upstream \
                 trace, not on a forwarded copy of the upstream span"
            );
        }

        publisher.shutdown().await;
    }

    /// The uninstrumented case: no active span means no trace fields, rather than a
    /// syntactically valid all-zero traceparent that a backend would try to join.
    #[tokio::test]
    async fn a_publish_outside_a_span_carries_no_trace_context() {
        let _probe = crate::test_fixtures::trace_probe::probe();
        let runtime = runtime_with_sizes("64x64");
        let publisher = MockPublisherServer::start().await;
        let outbound = publisher.outbound();

        dispatch_generate_for_configured_sizes(source(), &runtime, &outbound)
            .await
            .expect("dispatch should succeed without a span");

        for event in publisher.await_events(1, Duration::from_secs(2)).await {
            assert_eq!(event.message_str("/traceparent"), None);
            assert_eq!(event.message_str("/tracestate"), None);
        }

        publisher.shutdown().await;
    }

    /// Both producers render destinations the same way, because they share this
    /// function. A caller-supplied specification and the configured default for the
    /// same dimensions must land on the same address.
    #[tokio::test]
    async fn both_producers_resolve_the_same_destination() {
        let runtime = runtime_with_sizes("150x150");
        let publisher = MockPublisherServer::start().await;
        let outbound = publisher.outbound();

        let from_sizes = dispatch_generate_for_configured_sizes(source(), &runtime, &outbound)
            .await
            .expect("configured-size dispatch");

        let caller_spec = crate::converters::create_processing_spec_with(&runtime, 150, 150);
        let from_caller =
            dispatch_generate_per_specification(source(), &[caller_spec], &runtime, &outbound)
                .await;

        assert!(from_caller.failed.is_empty());
        assert_eq!(from_sizes, from_caller.destinations);

        publisher.shutdown().await;
    }

    /// The API path reports each specification separately rather than collapsing to
    /// a batch result, because its caller can act on knowing which one failed.
    #[tokio::test]
    async fn per_specification_dispatch_reports_each_outcome() {
        let runtime = runtime_with_sizes("256x256");
        let publisher = MockPublisherServer::start().await;
        let outbound = publisher.outbound();

        let valid_a = crate::converters::create_processing_spec_with(&runtime, 128, 128);
        let valid_b = crate::converters::create_processing_spec_with(&runtime, 256, 256);

        let outcome =
            dispatch_generate_per_specification(source(), &[valid_a, valid_b], &runtime, &outbound)
                .await;

        assert_eq!(
            outcome.destinations.len(),
            2,
            "both valid specs dispatch successfully"
        );
        assert!(outcome.failed.is_empty());

        publisher.shutdown().await;
    }

    /// The failure branch the test above is named for but never reaches.
    ///
    /// Both of its specifications are valid, so it only ever asserts
    /// `failed.is_empty()` — the per-specification reporting it exists to cover
    /// was untested. A dimension past the contract bound is the cheapest way to
    /// make one specification fail while its neighbours succeed.
    ///
    /// The transports now reject an out-of-range dimension before dispatch, so
    /// this is defence in depth rather than a reachable path: it pins that the
    /// contract still refuses what the boundary is supposed to have caught, and
    /// that one refusal does not take the valid specifications down with it.
    #[tokio::test]
    async fn per_specification_dispatch_isolates_a_failing_specification() {
        let runtime = runtime_with_sizes("256x256");
        let publisher = MockPublisherServer::start().await;
        let outbound = publisher.outbound();

        let valid = crate::converters::create_processing_spec_with(&runtime, 128, 128);
        let out_of_range = crate::converters::create_processing_spec_with(
            &runtime,
            crate::work_events::MAX_DIMENSION + 1,
            128,
        );
        let also_valid = crate::converters::create_processing_spec_with(&runtime, 256, 256);

        let outcome = dispatch_generate_per_specification(
            source(),
            &[valid, out_of_range, also_valid],
            &runtime,
            &outbound,
        )
        .await;

        assert_eq!(
            outcome.destinations.len(),
            2,
            "the two valid specifications must still dispatch"
        );
        assert_eq!(
            outcome.failed.len(),
            1,
            "the out-of-range specification must be reported, not silently dropped: {:?}",
            outcome.failed
        );

        publisher.shutdown().await;
    }
}
