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

//! The worker pipeline against in-process storage and thumbnail-service doubles.
//!
//! These cover the behaviors that only exist at the seams — which failures
//! withhold an acknowledgement, that association goes through the service RPC
//! and never through storage metadata, and that a duplicate delivery converges
//! rather than corrupting anything. None of them reach a live dependency.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use image::{Rgba, RgbaImage};
use thumbnail_common::spec_enums::{CropStrategy, EncodingFormat, TransformType};
use thumbnail_common::storage::AssociationTargetKind;
use thumbnail_common::test_fixtures::mock_storage_server::{
    mock_identity_for, mock_storage, unique_test_address, FileOp, MetadataOp,
};
use thumbnail_common::test_fixtures::trace_probe;
use thumbnail_common::work_events::{
    CapturedTraceContext, GenerateWorkMessage, ResolvedGenerateSpec, WorkResource,
};
use thumbnail_common::OutboundClients;
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::thumbnail_service_server::{
    ThumbnailService, ThumbnailServiceServer,
};
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::AssociationFailureKind;
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::{
    AssociateThumbnailRequest, AssociateThumbnailResponse, CreateThumbnailsRequest,
    CreateThumbnailsResponse, DisassociateThumbnailRequest, DisassociateThumbnailResponse,
    GetThumbnailsRequest, GetThumbnailsResponse,
};
use thumbnail_task_worker::outcome::{FailureClass, WorkOutcome};
use thumbnail_task_worker::pipeline::{MessageProcessor, Pipeline};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, Response, Status};

// --- thumbnail-service double ---------------------------------------------

/// How the association double answers.
#[derive(Clone, Debug)]
enum AssociateBehavior {
    Succeed,
    /// Fail the RPC with this code.
    FailWith(tonic::Code),
    /// Return `success = false` in band, as the proto's failure model allows,
    /// with no classification — an older service, or one that declines to
    /// classify. The worker must treat this as retryable.
    RejectInBand(String),
    /// Return `success = false` in band, classified. This is what lets the worker
    /// acknowledge a rejection that retrying cannot fix.
    RejectInBandClassified(String, AssociationFailureKind),
}

#[derive(Default)]
struct AssociateLog {
    requests: Vec<AssociateThumbnailRequest>,
    /// The `traceparent` each request arrived with, if any. Recorded so a test can
    /// assert on what actually reached the wire rather than on what the client was
    /// configured to send.
    traceparents: Vec<Option<String>>,
    /// The `authorization` header each request arrived with. Recorded for the same
    /// reason and one more: the trace layer had to be composed *outside* the auth
    /// interceptor, and without this the suite would stay green if the interceptor
    /// were dropped from that stack altogether.
    authorizations: Vec<Option<String>>,
}

struct MockThumbnailService {
    behavior: AssociateBehavior,
    log: Arc<Mutex<AssociateLog>>,
}

// The two echo fields are deprecated and never populated by the real service;
// the double mirrors that rather than pretending they are still meaningful.
#[allow(deprecated)]
#[tonic::async_trait]
impl ThumbnailService for MockThumbnailService {
    async fn associate_thumbnail(
        &self,
        request: Request<AssociateThumbnailRequest>,
    ) -> Result<Response<AssociateThumbnailResponse>, Status> {
        {
            let header = |name: &str| {
                request
                    .metadata()
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string)
            };
            let traceparent = header("traceparent");
            let authorization = header("authorization");
            let mut log = self.log.lock().expect("associate log must not be poisoned");
            log.traceparents.push(traceparent);
            log.authorizations.push(authorization);
            log.requests.push(request.into_inner());
        }

        match &self.behavior {
            AssociateBehavior::Succeed => Ok(Response::new(AssociateThumbnailResponse {
                thumbnail: None,
                association_target: None,
                success: true,
                error_message: None,
                failure_kind: None,
            })),
            AssociateBehavior::FailWith(code) => Err(Status::new(
                *code,
                "mock thumbnail-service: injected failure",
            )),
            AssociateBehavior::RejectInBand(reason) => {
                Ok(Response::new(AssociateThumbnailResponse {
                    thumbnail: None,
                    association_target: None,
                    success: false,
                    error_message: Some(reason.clone()),
                    failure_kind: None,
                }))
            }
            AssociateBehavior::RejectInBandClassified(reason, kind) => {
                Ok(Response::new(AssociateThumbnailResponse {
                    thumbnail: None,
                    association_target: None,
                    success: false,
                    error_message: Some(reason.clone()),
                    failure_kind: Some(*kind as i32),
                }))
            }
        }
    }

    async fn disassociate_thumbnail(
        &self,
        _: Request<DisassociateThumbnailRequest>,
    ) -> Result<Response<DisassociateThumbnailResponse>, Status> {
        Err(Status::unimplemented("not used by the worker"))
    }

    async fn create_thumbnails(
        &self,
        _: Request<CreateThumbnailsRequest>,
    ) -> Result<Response<CreateThumbnailsResponse>, Status> {
        Err(Status::unimplemented("not used by the worker"))
    }

    async fn get_thumbnails(
        &self,
        _: Request<GetThumbnailsRequest>,
    ) -> Result<Response<GetThumbnailsResponse>, Status> {
        Err(Status::unimplemented("not used by the worker"))
    }
}

async fn start_thumbnail_service(
    behavior: AssociateBehavior,
) -> (SocketAddr, Arc<Mutex<AssociateLog>>) {
    let log = Arc::new(Mutex::new(AssociateLog::default()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock thumbnail-service must bind");
    let addr = listener.local_addr().expect("bound addr");

    let service = MockThumbnailService {
        behavior,
        log: log.clone(),
    };
    tokio::spawn(async move {
        let _ = tonic::transport::Server::builder()
            .add_service(ThumbnailServiceServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await;
    });

    (addr, log)
}

// --- harness ---------------------------------------------------------------

fn worker_args(
    configure: impl FnOnce(&mut thumbnail_task_worker::config::WorkerArgs),
) -> thumbnail_task_worker::config::WorkerArgs {
    // Built directly, not parsed: clap reads the environment and CI exports
    // several of these names.
    let mut args = thumbnail_task_worker::config::WorkerArgs::for_tests();
    configure(&mut args);
    args
}

fn lazy_channel(addr: SocketAddr) -> tonic::transport::Channel {
    tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
        .expect("valid endpoint")
        .connect_lazy()
}

struct Harness {
    pipeline: Pipeline,
    associate_log: Arc<Mutex<AssociateLog>>,
}

async fn harness(
    behavior: AssociateBehavior,
    configure: impl FnOnce(&mut thumbnail_task_worker::config::WorkerArgs),
) -> Harness {
    let runtime = thumbnail_common::test_fixtures::init_test_runtime_config(
        "thumbnail-task-worker-pipeline-tests",
    );
    let storage_addr: SocketAddr = runtime
        .endpoints
        .storage_grpc_endpoint
        .parse()
        .expect("the fixture points storage at the in-process double");

    let (service_addr, associate_log) = start_thumbnail_service(behavior).await;

    let outbound = Arc::new(OutboundClients {
        storage_channel: lazy_channel(storage_addr),
        ..OutboundClients::for_tests()
    });

    Harness {
        pipeline: Pipeline {
            args: worker_args(configure),
            outbound,
            thumbnail_service: lazy_channel(service_addr),
            metrics: Arc::new(thumbnail_task_worker::metrics::WorkerMetrics::new_for_tests()),
        },
        associate_log,
    }
}

/// A harness whose outbound auth yields no token, for the credential path.
///
/// Everything else is left working, so a failure here can only come from the
/// credential — which is the point: a credential outage is not a property of
/// the message and must not be reported as one.
async fn harness_without_credentials() -> Harness {
    let mut harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    harness.pipeline.outbound = Arc::new(OutboundClients {
        storage_channel: harness.pipeline.outbound.storage_channel.clone(),
        ..OutboundClients::for_tests_without_credentials()
    });
    harness
}

impl Harness {
    fn associate_traceparents(&self) -> Vec<Option<String>> {
        self.associate_log
            .lock()
            .expect("associate log must not be poisoned")
            .traceparents
            .clone()
    }

    fn associate_requests(&self) -> Vec<AssociateThumbnailRequest> {
        self.associate_log
            .lock()
            .expect("associate log must not be poisoned")
            .requests
            .clone()
    }

    fn associate_authorizations(&self) -> Vec<Option<String>> {
        self.associate_log
            .lock()
            .expect("associate log must not be poisoned")
            .authorizations
            .clone()
    }
}

// --- fixtures --------------------------------------------------------------

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut image = RgbaImage::new(width, height);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255]);
    }
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("fixture must encode");
    bytes
}

/// Build a message the way the real publisher does, trace context included.
///
/// Capturing from the ambient span rather than hardcoding `None` is what lets one
/// builder serve both the uninstrumented tests — no subscriber, so nothing to
/// capture — and the propagation test, which opens a span around this call.
fn message(source: &str, destination: &str) -> prost_types::Struct {
    let trace_context = CapturedTraceContext::capture();
    GenerateWorkMessage {
        schema_version: thumbnail_common::work_events::GENERATE_SCHEMA_VERSION,
        source: WorkResource {
            resource_address: source.to_string(),
            resource_identity: Some(mock_identity_for(source)),
        },
        destination_resource_address: destination.to_string(),
        target_type: AssociationTargetKind::File,
        specification: ResolvedGenerateSpec {
            width: 64,
            height: 64,
            encoding_format: EncodingFormat::Png,
            transform_type: TransformType::Resize,
            crop_strategy: CropStrategy::Center,
            quality: 85,
            preserve_aspect_ratio: true,
        },
        traceparent: trace_context.traceparent,
        tracestate: trace_context.tracestate,
    }
    .to_struct()
}

// --- trace propagation across the queue ------------------------------------
//
// These assert on `thumbnail.convert` rather than on the outermost
// `thumbnail.generate` span, and the reason is a property of the tracing
// registry rather than a shortcut: `tracing_subscriber::Registry` keeps a span
// open while any descendant is open, so the outermost span of an operation
// exports only after everything beneath it — including connection-scoped spans
// belonging to pooled gRPC channels — has closed. Waiting for it makes the test
// an assertion about scheduling. `thumbnail.convert` has no children, closes on
// the blocking thread the moment the conversion returns, and inherits the trace
// this test is about, so it answers the same question deterministically.
//
// The span-level parentage semantics — same trace, own span id, malformed
// context refused whole — are asserted directly in `thumbnail-common`'s
// `work_events` tests, where a span's context is readable without export.

/// The join this change exists for, at the boundary rather than by inspection.
///
/// Neither side's own tests can see a break: the publisher's pass as long as it
/// writes *a* traceparent, and the worker's pass as long as it produces *a* span.
/// Only comparing the two ends of one message distinguishes a continued trace
/// from two traces that both exist and look healthy in isolation.
#[tokio::test]
async fn a_published_message_is_converted_inside_the_publishers_trace() {
    let probe = trace_probe::probe();
    let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    let source = unique_test_address("trace-source.png");
    let destination = unique_test_address("trace-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(256, 256));

    // Scoped so the publishing span closes and exports before the assertions.
    let published = {
        let publisher_span = tracing::info_span!("test.publisher");
        publisher_span.in_scope(|| message(&source, &destination))
    };

    let outcome = harness.pipeline.process(&published).await;
    assert!(
        matches!(outcome, WorkOutcome::Succeeded { .. }),
        "expected success, got {outcome:?}"
    );

    let publisher = probe
        .await_span_named("test.publisher", std::time::Duration::from_secs(5))
        .await;
    let convert = probe
        .await_span_named("thumbnail.convert", std::time::Duration::from_secs(5))
        .await;

    assert_eq!(
        convert.span_context.trace_id(),
        publisher.span_context.trace_id(),
        "the conversion — where nearly all the elapsed time goes — must land in \
         the trace of whatever published the work"
    );
    assert_ne!(
        convert.parent_span_id,
        publisher.span_context.span_id(),
        "the worker's own processing span sits between them, so the conversion is \
         not parented directly to the publisher"
    );
}

/// The far end of the round trip: the worker's call back into `thumbnail-service`
/// carries the trace, so the association RPC is attributable to the delivery that
/// produced it instead of arriving as a call from nowhere.
///
/// Asserted on the metadata the server actually received, not on how the client
/// was built — the composition order that makes this work is subtle enough
/// (the trace layer must sit outside the auth interceptor) that only the received
/// header proves it.
#[tokio::test]
async fn the_associate_call_carries_the_workers_trace_context() {
    let probe = trace_probe::probe();
    let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    let source = unique_test_address("associate-trace-source.png");
    let destination = unique_test_address("associate-trace-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(256, 256));

    let published = {
        let publisher_span = tracing::info_span!("test.publisher");
        publisher_span.in_scope(|| message(&source, &destination))
    };

    let outcome = harness.pipeline.process(&published).await;
    assert!(
        matches!(outcome, WorkOutcome::Succeeded { .. }),
        "expected success, got {outcome:?}"
    );

    let traceparents = harness.associate_traceparents();
    assert_eq!(traceparents.len(), 1, "exactly one associate call");
    let received = traceparents[0]
        .as_deref()
        .expect("the associate call must carry a traceparent");

    let publisher = probe
        .await_span_named("test.publisher", std::time::Duration::from_secs(5))
        .await;
    assert_eq!(
        trace_probe::trace_id_of(received),
        Some(publisher.span_context.trace_id().to_string().as_str()),
        "the association must be attributable to the trace that requested the work"
    );

    // The auth header, on the same received request. Both or neither is the real
    // property: the trace layer had to be composed outside the auth interceptor,
    // and a stack that lost the interceptor in that rearrangement would still
    // satisfy every traceparent assertion above.
    let authorization = harness.associate_authorizations();
    assert_eq!(
        authorization.len(),
        1,
        "one call, so one recorded authorization"
    );
    assert!(
        authorization[0]
            .as_deref()
            .is_some_and(|value| value.starts_with("Bearer ")),
        "the interceptor must still put a bearer token on the wire alongside the \
         trace context, got {:?}",
        authorization[0]
    );
}

/// The `Unauthenticated` refresh-and-reissue branch, which is the only place the
/// layered `TonicTraceService<InterceptedService<…>>` stack is driven twice on one
/// client object: a new client span is opened, the interceptor is re-invoked for a
/// refreshed token, and `poll_ready` runs again on a service that already errored.
///
/// Nothing exercised it before, and the failure it would hide is not cosmetic. A
/// reissue that failed through the new stack would surface as a permanent
/// `Unauthenticated`, which classifies as retryable — so the delivery NACKs and the
/// whole download, convert and upload pipeline repeats until the platform's
/// delivery limit dead-letters it, with the suite green throughout.
#[tokio::test]
async fn an_unauthenticated_associate_is_reissued_through_the_layered_stack() {
    // Required, not incidental: with no OpenTelemetry layer in the subscriber a
    // span has no span context, the propagator injects nothing, and the
    // traceparent assertions below would pass against `None` on a completely
    // broken stack. Written without it first, and it failed for exactly that
    // reason.
    let _probe = trace_probe::probe();
    let mut harness = harness(
        AssociateBehavior::FailWith(tonic::Code::Unauthenticated),
        |_| {},
    )
    .await;
    // The default fixture's refresher is a no-op, which leaves the rejected token
    // in place — so both attempts would carry the same bearer and the assertion
    // below could not tell a refresh from a resend.
    harness.pipeline.outbound = Arc::new(OutboundClients {
        storage_channel: harness.pipeline.outbound.storage_channel.clone(),
        ..OutboundClients::for_tests_with_token_refresh()
    });
    let source = unique_test_address("reissue-source.png");
    let destination = unique_test_address("reissue-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(256, 256));

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;
    assert!(
        !matches!(outcome, WorkOutcome::Succeeded { .. }),
        "the double rejects both attempts, so the delivery cannot succeed"
    );

    let traceparents = harness.associate_traceparents();
    assert_eq!(
        traceparents.len(),
        2,
        "the first call is rejected and the second is the reissue after refresh; \
         one call means the reissue never left the client"
    );
    for (attempt, traceparent) in traceparents.iter().enumerate() {
        assert!(
            traceparent.is_some(),
            "attempt {} must carry trace context — the reissue goes through the \
             same layered stack, so losing it there is the regression this catches",
            attempt + 1
        );
    }
    // The tokens must *differ*, which is the only evidence that this was a refresh
    // rather than a resend: the double rejects by code, not by credential, so a
    // second call carrying the token the server just refused looks identical from
    // every other angle.
    let first = format!("Bearer {}", OutboundClients::TEST_TOKEN);
    let refreshed = format!("Bearer {}", OutboundClients::TEST_TOKEN_REFRESHED);
    assert_eq!(
        harness.associate_authorizations(),
        vec![Some(first), Some(refreshed)],
        "the reissue must carry the refreshed credential, not the one the server \
         just rejected"
    );
}

/// Without context the worker still converts, in a trace of its own rather than
/// attaching to whatever happened to be nearby.
#[tokio::test]
async fn a_message_without_trace_context_starts_a_new_trace() {
    let probe = trace_probe::probe();
    let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    let source = unique_test_address("untraced-source.png");
    let destination = unique_test_address("untraced-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(256, 256));

    // Built inside a span, then stripped: the publisher had a context and the
    // message arrives without one, which is what a pre-upgrade publisher looks
    // like from here.
    let mut published = {
        let publisher_span = tracing::info_span!("test.publisher");
        publisher_span.in_scope(|| message(&source, &destination))
    };
    published.fields.remove("traceparent");
    published.fields.remove("tracestate");

    let outcome = harness.pipeline.process(&published).await;
    assert!(
        matches!(outcome, WorkOutcome::Succeeded { .. }),
        "a message with no trace context must still be processed: {outcome:?}"
    );

    let publisher = probe
        .await_span_named("test.publisher", std::time::Duration::from_secs(5))
        .await;
    let convert = probe
        .await_span_named("thumbnail.convert", std::time::Duration::from_secs(5))
        .await;

    assert_ne!(
        convert.span_context.trace_id(),
        publisher.span_context.trace_id(),
        "with nothing to continue, the worker roots a trace of its own"
    );
}

/// A malformed traceparent is a telemetry problem, not a message problem: it must
/// not cost a conversion, and must not be half-adopted into a trace that may not
/// exist.
#[tokio::test]
async fn a_malformed_traceparent_costs_neither_the_conversion_nor_a_bogus_trace() {
    let probe = trace_probe::probe();
    let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    let source = unique_test_address("badtrace-source.png");
    let destination = unique_test_address("badtrace-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(256, 256));

    let mut published = {
        let publisher_span = tracing::info_span!("test.publisher");
        publisher_span.in_scope(|| message(&source, &destination))
    };
    published.fields.insert(
        "traceparent".to_string(),
        prost_types::Value {
            kind: Some(prost_types::value::Kind::StringValue(
                "obviously-not-a-traceparent".to_string(),
            )),
        },
    );

    let outcome = harness.pipeline.process(&published).await;
    assert!(
        matches!(outcome, WorkOutcome::Succeeded { .. }),
        "a bad traceparent must not cost a conversion: {outcome:?}"
    );

    let publisher = probe
        .await_span_named("test.publisher", std::time::Duration::from_secs(5))
        .await;
    let convert = probe
        .await_span_named("thumbnail.convert", std::time::Duration::from_secs(5))
        .await;

    assert_ne!(
        convert.span_context.trace_id(),
        publisher.span_context.trace_id(),
        "an unparseable context is refused whole, not mined for a trace id"
    );
}

// --- tests -----------------------------------------------------------------

#[tokio::test]
async fn a_valid_message_converts_uploads_and_associates() {
    let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    let source = unique_test_address("worker-source.png");
    let destination = unique_test_address("worker-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(256, 256));

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert!(
        matches!(outcome, WorkOutcome::Succeeded { .. }),
        "expected success, got {outcome:?}"
    );

    let written = mock_storage()
        .blob(&destination)
        .expect("the thumbnail should have been written to its destination");
    let decoded = image::load_from_memory(&written).expect("output must be a real image");
    assert_eq!(
        image::GenericImageView::dimensions(&decoded),
        (64, 64),
        "the resolved specification's dimensions are what get written"
    );

    let requests = harness.associate_requests();
    assert_eq!(
        requests.len(),
        1,
        "association goes through exactly one RPC"
    );
    let request = &requests[0];
    assert_eq!(
        request.thumbnail.as_ref().unwrap().resource_address,
        destination
    );
    assert_eq!(
        request.thumbnail.as_ref().unwrap().resource_identity,
        Some(mock_identity_for(&destination)),
        "the identity must be the one storage returned from the write, not a synthesized one"
    );
    assert_eq!(
        request
            .association_target
            .as_ref()
            .unwrap()
            .resource_address,
        source
    );
}

/// The worker must not establish the association itself. Any metadata write
/// against the thumbnail or the source would be the service's job done twice.
#[tokio::test]
async fn no_association_metadata_is_written_by_the_worker() {
    let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    let source = unique_test_address("no-metadata-source.png");
    let destination = unique_test_address("no-metadata-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));

    harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    for address in [&source, &destination] {
        let writes: Vec<_> = mock_storage()
            .calls_for(address)
            .into_iter()
            .filter(|call| matches!(call.op, MetadataOp::Update | MetadataOp::Delete))
            .collect();
        assert!(
            writes.is_empty(),
            "the worker wrote association metadata for {address}: {writes:?}"
        );
    }
}

#[tokio::test]
async fn a_missing_source_is_permanent() {
    let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    let source = unique_test_address("absent-source.png");
    let destination = unique_test_address("absent-thumb.png");
    // Deliberately not seeded.

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert!(
        outcome.should_ack(),
        "a source that does not exist will not appear"
    );
    assert_eq!(outcome.failure_class(), Some(FailureClass::SourceMissing));
}

#[tokio::test]
async fn a_transient_storage_read_failure_withholds_the_ack_after_one_attempt() {
    let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    let source = unique_test_address("flaky-source.png");
    let destination = unique_test_address("flaky-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));
    mock_storage().fail_file(FileOp::Read, &source, tonic::Code::Unavailable);

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert!(!outcome.should_ack(), "a transient failure must come back");
    assert_eq!(
        outcome.failure_class(),
        Some(FailureClass::Download),
        "a source that exists but could not be transferred is a download failure, \
         not a decode failure — they point at different systems"
    );

    // Exactly one attempt: no in-process retry loop, no Celery-style countdown.
    let reads: Vec<_> = mock_storage()
        .file_calls_for(&source)
        .into_iter()
        .filter(|call| call.op == FileOp::Read)
        .collect();
    assert_eq!(
        reads.len(),
        1,
        "the pipeline must make a single attempt and let redelivery do the retrying"
    );
}

/// A credential outage fails every message identically and says nothing about
/// any of them, so it gets its own class rather than being counted as a sudden
/// epidemic of undecodable images. It also has to fail before the source is
/// fetched — there is no point paying for a transfer that cannot be authorized.
#[tokio::test]
async fn a_credential_failure_is_its_own_class_and_costs_no_storage_call() {
    let harness = harness_without_credentials().await;
    let source = unique_test_address("credential-source.png");
    let destination = unique_test_address("credential-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert_eq!(outcome.failure_class(), Some(FailureClass::Credential));
    assert!(
        !outcome.should_ack(),
        "a token that could not be fetched is transient; the work is still good"
    );
    assert!(
        mock_storage().file_calls_for(&source).is_empty(),
        "the source must not be read when the call could not have been authorized"
    );
    assert!(
        harness.associate_requests().is_empty(),
        "nothing was produced, so nothing may be associated"
    );
}

#[tokio::test]
async fn an_upload_failure_is_classified_as_upload() {
    let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    let source = unique_test_address("upload-fail-source.png");
    let destination = unique_test_address("upload-fail-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));
    mock_storage().fail_file(FileOp::Write, &destination, tonic::Code::Unavailable);

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert!(!outcome.should_ack());
    assert_eq!(outcome.failure_class(), Some(FailureClass::Upload));
    assert!(
        harness.associate_requests().is_empty(),
        "nothing should be associated when the upload failed"
    );
}

/// Associating a thumbnail whose write reported no identity would record
/// something nobody confirmed exists.
#[tokio::test]
async fn a_write_without_an_identity_fails_rather_than_associating() {
    let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    let source = unique_test_address("no-identity-source.png");
    let destination = unique_test_address("no-identity-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));
    mock_storage().write_without_identity(&destination);

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert_eq!(outcome.failure_class(), Some(FailureClass::Upload));
    assert!(
        harness.associate_requests().is_empty(),
        "an unverified thumbnail must not be associated"
    );
}

#[tokio::test]
async fn a_transient_associate_failure_withholds_the_ack() {
    let harness = harness(
        AssociateBehavior::FailWith(tonic::Code::Unavailable),
        |_| {},
    )
    .await;
    let source = unique_test_address("assoc-fail-source.png");
    let destination = unique_test_address("assoc-fail-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert!(!outcome.should_ack());
    assert_eq!(outcome.failure_class(), Some(FailureClass::Associate));
}

#[tokio::test]
async fn an_in_band_association_rejection_is_retryable() {
    let harness = harness(
        AssociateBehavior::RejectInBand("storage write failed".to_string()),
        |_| {},
    )
    .await;
    let source = unique_test_address("assoc-reject-source.png");
    let destination = unique_test_address("assoc-reject-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert!(!outcome.should_ack());
    assert_eq!(outcome.failure_class(), Some(FailureClass::Associate));
}

/// The worker's parse boundary rejects an aliased destination before any storage
/// operation. Both addresses are normalized, so a non-canonical source cannot
/// slip past.
#[tokio::test]
async fn a_destination_equal_to_the_source_is_refused_after_normalization() {
    let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    let source = "https://storage.example.test/container/self.png";
    mock_storage().seed_blob(source, png_bytes(128, 128));

    for spelling in [
        "https://storage.example.test/container/self.png",
        "https://storage.example.test:443/container/self.png",
        "HTTPS://STORAGE.EXAMPLE.TEST/container/self.png",
    ] {
        let outcome = harness.pipeline.process(&message(spelling, source)).await;

        assert_eq!(
            outcome.failure_class(),
            Some(FailureClass::Validation),
            "'{spelling}' names the destination and must be refused"
        );
    }
}

#[tokio::test]
async fn a_source_over_the_decode_bound_is_permanent() {
    let harness = harness(AssociateBehavior::Succeed, |a| a.max_decoded_pixels = 1024).await;
    let source = unique_test_address("bomb-source.png");
    let destination = unique_test_address("bomb-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(512, 512));

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert!(
        outcome.should_ack(),
        "a decode bomb must be acknowledged, or it crash-loops the replacement replica"
    );
    assert_eq!(outcome.failure_class(), Some(FailureClass::Decode));
}

#[tokio::test]
async fn an_oversized_source_is_refused_by_the_byte_bound() {
    let harness = harness(AssociateBehavior::Succeed, |a| a.max_source_bytes = 64).await;
    let source = unique_test_address("large-source.png");
    let destination = unique_test_address("large-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(256, 256));

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert!(outcome.should_ack());
    assert_eq!(
        outcome.failure_class(),
        Some(FailureClass::Decode),
        "the byte bound is a decode bound that the transfer happens to catch first; \
         it must not report as a download failure just because of where it fired"
    );
}

/// The consumer holds an `Arc<dyn MessageProcessor>`, so the trait impl is the
/// only path production ever takes through this type — and it was the one path
/// no test covered, because the unit tests call the inherent method directly and
/// the consumer tests substitute a scripted double. That gap is what made the
/// shadowed `self.process(..)` call inside the impl a live trap rather than a
/// curiosity. Drive a real pipeline through the trait object so it is covered.
#[tokio::test]
async fn the_trait_object_reaches_the_real_pipeline() {
    let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    let source = unique_test_address("dyn-source.png");
    let destination = unique_test_address("dyn-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));

    let processor: &dyn MessageProcessor = &harness.pipeline;
    // Bounded, because the regression this guards against is unbounded
    // recursion: `#[async_trait]` boxes each call, so a self-dispatching impl
    // allocates its way onward instead of overflowing promptly. Without a
    // deadline the failure mode is a hung suite rather than a failed test.
    let outcome = tokio::time::timeout(
        Duration::from_secs(30),
        processor.process(&message(&source, &destination)),
    )
    .await
    .expect("the trait impl must dispatch to the pipeline, not recurse");

    assert!(
        matches!(outcome, WorkOutcome::Succeeded { .. }),
        "the trait impl must run the conversion, not recurse or no-op: {outcome:?}"
    );
    assert_eq!(
        harness.associate_requests().len(),
        1,
        "the work really went all the way through"
    );
}

// --- Idempotency -----------------------------------------------------------

/// A redelivery or a CREATE re-fan-out produces the same message twice. The
/// second must not fail merely because the first already ran.
#[tokio::test]
async fn a_duplicate_delivery_succeeds_again() {
    let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
    let source = unique_test_address("dup-source.png");
    let destination = unique_test_address("dup-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));

    let first = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;
    let second = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert!(matches!(first, WorkOutcome::Succeeded { .. }));
    assert!(
        matches!(second, WorkOutcome::Succeeded { .. }),
        "the duplicate must not fail on account of being a duplicate, got {second:?}"
    );
    assert_eq!(
        harness.associate_requests().len(),
        2,
        "both associate, and the service's own write is idempotent"
    );
}

/// Two replicas can process duplicates at the same time. v1 declines a
/// distributed lease — the platform offers none — and relies on the upload
/// overwriting and the association being idempotent, so the pair converges on
/// one consistent result.
#[tokio::test]
async fn concurrent_duplicates_converge_on_one_consistent_association() {
    let harness = Arc::new(harness(AssociateBehavior::Succeed, |_| {}).await);
    let source = unique_test_address("race-source.png");
    let destination = unique_test_address("race-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));

    let one = {
        let harness = harness.clone();
        let message = message(&source, &destination);
        tokio::spawn(async move { harness.pipeline.process(&message).await })
    };
    let two = {
        let harness = harness.clone();
        let message = message(&source, &destination);
        tokio::spawn(async move { harness.pipeline.process(&message).await })
    };

    let (one, two) = (one.await.expect("task one"), two.await.expect("task two"));
    assert!(matches!(one, WorkOutcome::Succeeded { .. }), "{one:?}");
    assert!(matches!(two, WorkOutcome::Succeeded { .. }), "{two:?}");

    let requests = harness.associate_requests();
    assert_eq!(requests.len(), 2, "neither is excluded");
    let identities: Vec<_> = requests
        .iter()
        .map(|request| {
            request
                .thumbnail
                .as_ref()
                .unwrap()
                .resource_identity
                .clone()
        })
        .collect();
    assert_eq!(
        identities[0], identities[1],
        "both associate the same destination identity, so the result converges"
    );
    assert!(
        mock_storage().blob(&destination).is_some(),
        "one blob exists at the destination, last write winning"
    );
}

/// Restarting keeps no state, and duplicate handling still holds — it comes from
/// overwrite-safe upload and idempotent association, not a dedup table.
#[tokio::test]
async fn duplicate_handling_needs_no_worker_state() {
    let source = unique_test_address("stateless-source.png");
    let destination = unique_test_address("stateless-thumb.png");

    {
        let harness = harness(AssociateBehavior::Succeed, |_| {}).await;
        mock_storage().seed_blob(&source, png_bytes(128, 128));
        assert!(matches!(
            harness
                .pipeline
                .process(&message(&source, &destination))
                .await,
            WorkOutcome::Succeeded { .. }
        ));
    }

    // A brand-new pipeline, as if the pod had restarted.
    let restarted = harness(AssociateBehavior::Succeed, |_| {}).await;
    assert!(
        matches!(
            restarted
                .pipeline
                .process(&message(&source, &destination))
                .await,
            WorkOutcome::Succeeded { .. }
        ),
        "a fresh process must handle the duplicate the same way"
    );
}

/// A permanent associate failure after a successful upload leaves an orphan, and
/// the orphan must be countable.
///
/// The acknowledgement is the problem: a permanent failure ACKs, so nothing will
/// ever retry, and the uploaded blob sits at the destination with no association
/// metadata pointing at it. The storage-deleted sweep cannot reclaim it either —
/// that sweep enumerates `thumbnail_*` keys on the *source*, and this blob has
/// none. Unreferenced and unenumerable is the combination that accumulates
/// silently, so it gets its own counter rather than sharing the `associate`
/// class, which is identical either side of the upload.
#[tokio::test]
async fn a_permanent_associate_failure_after_upload_counts_an_orphan() {
    let harness = harness(
        // InvalidArgument is non-retryable, so this ACKs.
        AssociateBehavior::FailWith(tonic::Code::InvalidArgument),
        |_| {},
    )
    .await;
    let source = unique_test_address("assoc-orphan-source.png");
    let destination = unique_test_address("assoc-orphan-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert!(
        outcome.should_ack(),
        "a permanent associate failure acknowledges; that is what strands the blob"
    );
    assert_eq!(outcome.failure_class(), Some(FailureClass::Associate));

    assert!(
        mock_storage().has_blob(&destination),
        "the upload succeeded, so the blob is there — that is the orphan"
    );

    let rendered = harness.pipeline.metrics.render();
    assert!(
        rendered.contains("thumbnail_worker_orphaned_blobs_total 1"),
        "the orphan must be countable, not just logged:\n{rendered}"
    );
}

/// A *retryable* associate failure strands nothing, so it must not be counted as
/// an orphan: the acknowledgement is withheld, the delivery comes back, and the
/// redelivery overwrites the same destination and associates it.
#[tokio::test]
async fn a_retryable_associate_failure_counts_no_orphan() {
    let harness = harness(
        AssociateBehavior::FailWith(tonic::Code::Unavailable),
        |_| {},
    )
    .await;
    let source = unique_test_address("assoc-no-orphan-source.png");
    let destination = unique_test_address("assoc-no-orphan-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert!(!outcome.should_ack());

    let rendered = harness.pipeline.metrics.render();
    assert!(
        rendered.contains("thumbnail_worker_orphaned_blobs_total 0"),
        "a withheld ack means a redelivery will fix this; it is not an orphan:\n{rendered}"
    );
}

/// A classified-permanent in-band rejection is acknowledged.
///
/// This is the point of `failure_kind`. Before it, `error_message` was the only
/// signal and it is prose, so the worker could not tell "storage blinked" from
/// "this will never succeed" and withheld the acknowledgement for both. Safe for
/// the transient case; for the permanent one it means the delivery comes back and
/// re-downloads, re-converts and re-uploads the same source on every redelivery
/// until the platform's delivery limit dead-letters it.
#[tokio::test]
async fn a_permanent_in_band_rejection_is_acknowledged() {
    let harness = harness(
        AssociateBehavior::RejectInBandClassified(
            "association target no longer exists".to_string(),
            AssociationFailureKind::Permanent,
        ),
        |_| {},
    )
    .await;
    let source = unique_test_address("assoc-perm-source.png");
    let destination = unique_test_address("assoc-perm-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert!(
        outcome.should_ack(),
        "a rejection retrying cannot fix must not be redelivered"
    );
    assert_eq!(outcome.failure_class(), Some(FailureClass::Associate));

    // Acknowledging is what strands the blob, so this must also be counted.
    let rendered = harness.pipeline.metrics.render();
    assert!(
        rendered.contains("thumbnail_worker_orphaned_blobs_total 1"),
        "an acknowledged post-upload rejection leaves an orphan:\n{rendered}"
    );
}

/// A classified-retryable in-band rejection still withholds the acknowledgement,
/// so the classification cannot be read as "acknowledge everything in band".
#[tokio::test]
async fn a_classified_retryable_in_band_rejection_still_withholds_the_ack() {
    let harness = harness(
        AssociateBehavior::RejectInBandClassified(
            "storage briefly unavailable".to_string(),
            AssociationFailureKind::Retryable,
        ),
        |_| {},
    )
    .await;
    let source = unique_test_address("assoc-retry-source.png");
    let destination = unique_test_address("assoc-retry-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(128, 128));

    let outcome = harness
        .pipeline
        .process(&message(&source, &destination))
        .await;

    assert!(!outcome.should_ack());
    assert!(
        harness
            .pipeline
            .metrics
            .render()
            .contains("thumbnail_worker_orphaned_blobs_total 0"),
        "a withheld ack means a redelivery will associate it; not an orphan"
    );
}

/// A storage read that accepts the call and never answers is bounded by
/// OUTBOUND_REQUEST_TIMEOUT.
///
/// This is the case neither of the existing bounds covers: a connect timeout
/// bounds establishing the connection and HTTP/2 keepalive notices a peer that
/// has gone away, while this peer is alive and simply silent. Unbounded, it holds
/// a conversion permit for as long as the stall lasts, which at the concurrency
/// bound means the worker stops taking work.
#[tokio::test]
async fn a_stalled_storage_read_is_bounded_by_the_outbound_deadline() {
    let harness = harness(AssociateBehavior::Succeed, |a| {
        a.outbound_request_timeout = Duration::from_millis(400);
    })
    .await;
    let source = unique_test_address("stalled-read-source.png");
    let destination = unique_test_address("stalled-read-thumb.png");
    mock_storage().seed_blob(&source, png_bytes(64, 64));
    mock_storage().stall_file(FileOp::Read, &source);

    let started = std::time::Instant::now();
    // Generous against the 400ms deadline: passing means the deadline fired, not
    // that the harness gave up.
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        harness.pipeline.process(&message(&source, &destination)),
    )
    .await
    .expect("the deadline must end the call, not the test harness");

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the outbound deadline should have fired long before this; took {:?}",
        started.elapsed()
    );
    assert!(
        !outcome.should_ack(),
        "a stalled dependency is transient: the delivery must come back"
    );
    assert_eq!(outcome.failure_class(), Some(FailureClass::Download));
}
