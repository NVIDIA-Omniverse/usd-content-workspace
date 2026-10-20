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

use std::time::Duration;

use prost_types::{value::Kind, Struct, Value};

use super::*;
use crate::spec_enums::{CropStrategy, EncodingFormat, TransformType};
use crate::test_fixtures::mock_publisher_server::{MockPublisherServer, PublisherBehavior};
use crate::types::ImageProcessingSpec;
use tracing::Instrument as _;

const SOURCE_ADDRESS: &str = "https://storage.example/container/assets/image.png";
const SOURCE_IDENTITY: &str = "https://storage.example/container/assets/image.png?etag=abc123";
const DESTINATION: &str = "https://storage.example/container/thumbs/image_256x256.png";

fn complete_spec() -> ImageProcessingSpec {
    ImageProcessingSpec {
        width: Some(256),
        height: Some(256),
        encoding_format: Some(EncodingFormat::Png),
        transform_type: Some(TransformType::Square),
        crop_strategy: Some(CropStrategy::Center),
        quality: Some(85),
        preserve_aspect_ratio: Some(true),
    }
}

fn build(spec: &ImageProcessingSpec) -> Result<GenerateWorkMessage, GenerateContractError> {
    build_generate_message(GenerateMessageParams {
        source: WorkResource::file(SOURCE_ADDRESS, SOURCE_IDENTITY),
        destination_resource_address: DESTINATION,
        target_type: AssociationTargetKind::File,
        specification: spec,
    })
}

fn valid_message() -> GenerateWorkMessage {
    build(&complete_spec()).expect("a complete specification must build")
}

// --- Contract shape -------------------------------------------------------

#[test]
fn envelope_resource_id_is_the_source_address_not_the_identity() {
    let message = valid_message();

    assert_eq!(message.envelope_resource_id(), SOURCE_ADDRESS);
    assert_ne!(
        message.envelope_resource_id(),
        SOURCE_IDENTITY,
        "the envelope filter key is the address; the identity lives in the message Resource"
    );
    assert_eq!(
        message.source.resource_identity.as_deref(),
        Some(SOURCE_IDENTITY)
    );
}

/// The envelope and the message are separate layers. Encoding the message must
/// not smuggle the envelope's single-string filter key into the payload, or the
/// worker would have two disagreeing sources of truth for the source.
#[test]
fn encoded_message_carries_the_resource_shape_and_no_envelope_key() {
    let encoded = valid_message().to_struct();

    assert!(
        !encoded.fields.contains_key("resource_id"),
        "envelope resource_id must not appear in the message body"
    );

    let source = match &encoded.fields["source"].kind {
        Some(Kind::StructValue(source)) => source,
        other => panic!("source must be a struct, got {other:?}"),
    };
    assert!(source.fields.contains_key("resource_address"));
    assert!(source.fields.contains_key("resource_identity"));
}

#[test]
fn file_source_without_an_identity_is_rejected() {
    let error = build_generate_message(GenerateMessageParams {
        source: WorkResource {
            resource_address: SOURCE_ADDRESS.to_string(),
            resource_identity: None,
        },
        destination_resource_address: DESTINATION,
        target_type: AssociationTargetKind::File,
        specification: &complete_spec(),
    })
    .expect_err("a FILE source must carry an identity");

    assert!(matches!(error, GenerateContractError::InvalidSource(_)));
}

#[test]
fn every_resolved_specification_field_travels_in_the_message() {
    let message = valid_message();
    let encoded = message.to_struct();
    let specification = match &encoded.fields["specification"].kind {
        Some(Kind::StructValue(specification)) => specification,
        other => panic!("specification must be a struct, got {other:?}"),
    };

    for field in [
        "width",
        "height",
        "encoding_format",
        "transform_type",
        "crop_strategy",
        "quality",
        "preserve_aspect_ratio",
    ] {
        assert!(
            specification.fields.contains_key(field),
            "the worker has no THUMBNAIL_* config of its own, so '{field}' must be on the wire"
        );
    }
    assert!(
        !specification.fields.contains_key("processor_identity"),
        "processor_identity was removed; the type is always IMAGE_TO_IMAGE by construction"
    );

    assert!(encoded.fields.contains_key("target_type"));
    assert!(encoded.fields.contains_key("destination_resource_address"));
}

/// Clears one field of a specification, so a test can walk every required field.
type ClearField = fn(&mut ImageProcessingSpec);

#[test]
fn each_missing_specification_field_is_named_in_the_error() {
    let cases: [(&str, ClearField); 7] = [
        ("width", |s| s.width = None),
        ("height", |s| s.height = None),
        ("encoding_format", |s| s.encoding_format = None),
        ("transform_type", |s| s.transform_type = None),
        ("crop_strategy", |s| s.crop_strategy = None),
        ("quality", |s| s.quality = None),
        ("preserve_aspect_ratio", |s| s.preserve_aspect_ratio = None),
    ];

    for (field, clear) in cases {
        let mut spec = complete_spec();
        clear(&mut spec);

        match build(&spec) {
            Err(GenerateContractError::MissingSpecificationField { field: named }) => {
                assert_eq!(named, field)
            }
            other => panic!("omitting {field} must be a contract error, got {other:?}"),
        }
    }
}

#[test]
fn out_of_range_dimensions_and_quality_are_rejected() {
    for (width, height) in [(0, 256), (256, 0), (8193, 256), (256, 8193)] {
        let mut spec = complete_spec();
        spec.width = Some(width);
        spec.height = Some(height);
        assert!(matches!(
            build(&spec),
            Err(GenerateContractError::InvalidDimensions)
        ));
    }

    for quality in [0, 101, 200, -1] {
        let mut spec = complete_spec();
        spec.encoding_format = Some(EncodingFormat::Jpeg);
        spec.quality = Some(quality);
        assert!(matches!(
            build(&spec),
            Err(GenerateContractError::InvalidQuality)
        ));
    }
}

/// A message can name every field with the right type and still ask for
/// something the contract forbids. Shape-checking alone would carry it to the
/// worker, and the dimension bound is the one that does damage there: decode
/// limits cap the source an image is read from, nothing else caps the output a
/// specification asks for, so an enormous requested size would decode a small
/// source happily and then OOM the replica resizing it.
#[test]
fn a_parsed_message_is_held_to_the_same_contract_as_a_built_one() {
    let mut spec = complete_spec();
    spec.encoding_format = Some(EncodingFormat::Webp);
    let valid = build(&spec).expect("the fixture is a valid message");

    // Re-encode the valid message with one specification number replaced, then
    // parse it back.
    let with_spec_number = |field: &str, value: f64| {
        let mut encoded = valid.to_struct();
        let Some(Kind::StructValue(spec)) = encoded
            .fields
            .get_mut("specification")
            .and_then(|v| v.kind.as_mut())
        else {
            panic!("the fixture carries a specification struct");
        };
        spec.fields.insert(
            field.to_string(),
            Value {
                kind: Some(Kind::NumberValue(value)),
            },
        );
        GenerateWorkMessage::from_struct(&encoded)
    };

    // Past `MAX_DIMENSION`: the OOM path.
    let huge = with_spec_number("width", 65535.0);
    assert!(
        matches!(
            huge,
            Err(GenerateMessageRejection::Contract(
                GenerateContractError::InvalidDimensions
            ))
        ),
        "an out-of-range width must be refused on parse, got {huge:?}"
    );

    let zero = with_spec_number("height", 0.0);
    assert!(matches!(
        zero,
        Err(GenerateMessageRejection::Contract(
            GenerateContractError::InvalidDimensions
        ))
    ));

    let quality = with_spec_number("quality", 250.0);
    assert!(
        matches!(
            quality,
            Err(GenerateMessageRejection::Contract(
                GenerateContractError::InvalidQuality
            ))
        ),
        "quality outside 1..=100 must be refused on parse, got {quality:?}"
    );

    // And the valid message still round-trips, so the new checks did not just
    // reject everything.
    assert_eq!(
        GenerateWorkMessage::from_struct(&valid.to_struct()).expect("valid round-trip"),
        valid
    );
}

#[test]
fn a_fractional_schema_version_is_rejected_instead_of_truncated() {
    let mut encoded = valid_message().to_struct();
    encoded.fields.insert(
        "schema_version".to_string(),
        Value {
            kind: Some(Kind::NumberValue(1.5)),
        },
    );

    assert_eq!(
        GenerateWorkMessage::from_struct(&encoded),
        Err(GenerateMessageRejection::MissingSchemaVersion)
    );
}

#[test]
fn parsed_messages_validate_source_and_destination_like_built_messages() {
    let valid = valid_message().to_struct();

    let mut missing_identity = valid.clone();
    let source = match missing_identity
        .fields
        .get_mut("source")
        .and_then(|value| value.kind.as_mut())
    {
        Some(Kind::StructValue(source)) => source,
        other => panic!("source must be a struct, got {other:?}"),
    };
    source.fields.remove("resource_identity");
    assert!(matches!(
        GenerateWorkMessage::from_struct(&missing_identity),
        Err(GenerateMessageRejection::Contract(
            GenerateContractError::InvalidSource(_)
        ))
    ));

    let mut relative_destination = valid.clone();
    relative_destination.fields.insert(
        "destination_resource_address".to_string(),
        string_value("thumb.png".to_string()),
    );
    assert!(matches!(
        GenerateWorkMessage::from_struct(&relative_destination),
        Err(GenerateMessageRejection::Contract(
            GenerateContractError::RelativeDestination(_)
        ))
    ));

    let mut aliased_destination = valid;
    aliased_destination.fields.insert(
        "destination_resource_address".to_string(),
        string_value(SOURCE_ADDRESS.to_string()),
    );
    assert_eq!(
        GenerateWorkMessage::from_struct(&aliased_destination),
        Err(GenerateMessageRejection::Contract(
            GenerateContractError::DestinationEqualsSource
        ))
    );
}

/// The publisher and the worker both gate on this, so it has to agree with
/// itself about the awkward cases rather than each side approximating.
#[test]
fn the_absolute_address_rule_is_one_rule() {
    for absolute in [
        "https://storage.example/a/b.png",
        "http://storage.example:8080/a/b.png",
        "omniverse://server/project/a.png",
        "  https://storage.example/a/b.png  ",
    ] {
        assert!(
            is_absolute_address(absolute),
            "{absolute} names a storage object and must be accepted"
        );
    }

    for relative in ["a/b.png", "/a/b.png", "", "   ", "://missing-scheme"] {
        assert!(
            !is_absolute_address(relative),
            "{relative:?} does not name a storage object and must be refused"
        );
    }
}

#[test]
fn destination_must_be_absolute() {
    let error = build_generate_message(GenerateMessageParams {
        source: WorkResource::file(SOURCE_ADDRESS, SOURCE_IDENTITY),
        destination_resource_address: "/container/thumbs/image_256x256.png",
        target_type: AssociationTargetKind::File,
        specification: &complete_spec(),
    })
    .expect_err("a relative destination is not a storage address");

    assert!(matches!(
        error,
        GenerateContractError::RelativeDestination(_)
    ));
}

/// A source spelled with an explicit default port or a mixed-case host names the
/// same object as its canonical form. Comparing raw strings would let the guard
/// pass and the thumbnail overwrite its own source.
#[test]
fn destination_equal_to_source_is_rejected_after_normalization() {
    for source in [
        "https://storage.example/container/assets/image.png",
        "https://storage.example:443/container/assets/image.png",
        "HTTPS://STORAGE.EXAMPLE/container/assets/image.png",
    ] {
        let error = build_generate_message(GenerateMessageParams {
            source: WorkResource::file(source, SOURCE_IDENTITY),
            destination_resource_address: "https://storage.example/container/assets/image.png",
            target_type: AssociationTargetKind::File,
            specification: &complete_spec(),
        })
        .expect_err("a destination naming the source must be refused");

        assert!(
            matches!(error, GenerateContractError::DestinationEqualsSource),
            "'{source}' names the same object as the destination"
        );
    }
}

/// Caller-supplied and configuration-derived specifications go through the same
/// builder, so the worker cannot tell the two origins apart.
#[test]
fn caller_and_configuration_specifications_produce_the_same_shape() {
    let from_configuration = complete_spec();
    let from_caller = ImageProcessingSpec {
        // A different field order and a fresh value set, resolved identically.
        preserve_aspect_ratio: Some(true),
        quality: Some(85),
        crop_strategy: Some(CropStrategy::Center),
        transform_type: Some(TransformType::Square),
        encoding_format: Some(EncodingFormat::Png),
        height: Some(256),
        width: Some(256),
    };

    assert_eq!(
        build(&from_configuration).unwrap().to_struct(),
        build(&from_caller).unwrap().to_struct()
    );
}

// --- Round trip and schema evolution --------------------------------------

#[test]
fn message_round_trips_through_the_struct_encoding() {
    let mut original = valid_message();
    original.traceparent = Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into());
    original.tracestate = Some("vendor=opaque".into());

    let decoded = GenerateWorkMessage::from_struct(&original.to_struct())
        .expect("a message this build produced must parse");

    assert_eq!(decoded, original);
}

/// The three states trace context can be in on the wire. `tracestate` without a
/// `traceparent` is not among them — a vendor state with no trace to attach it to
/// is meaningless — but it must still decode rather than fail, because parsing is
/// not the layer that judges telemetry.
#[test]
fn trace_context_round_trips_in_every_combination() {
    let cases = [
        (
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
            Some("vendor=opaque,other=1"),
        ),
        (
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
            None,
        ),
        (None, None),
    ];

    for (traceparent, tracestate) in cases {
        let mut original = valid_message();
        original.traceparent = traceparent.map(str::to_string);
        original.tracestate = tracestate.map(str::to_string);

        let encoded = original.to_struct();
        assert_eq!(
            encoded.fields.contains_key("traceparent"),
            traceparent.is_some(),
            "an absent traceparent must be omitted, not written as an empty string"
        );
        assert_eq!(
            encoded.fields.contains_key("tracestate"),
            tracestate.is_some()
        );

        let decoded = GenerateWorkMessage::from_struct(&encoded)
            .expect("trace context must never make a message unparseable");
        assert_eq!(decoded, original);
    }
}

/// A malformed `traceparent` must not fail the parse.
///
/// `from_struct` failing means the delivery is rejected, and for a rejection that
/// withholds nothing that means an ACK-and-discard of real conversion work — over
/// a telemetry field that nothing downstream depends on. The value is carried as
/// received and refused later by the propagator, which owns the grammar.
#[test]
fn a_malformed_traceparent_does_not_fail_the_parse() {
    let mut encoded = valid_message().to_struct();
    encoded.fields.insert(
        "traceparent".to_string(),
        Value {
            kind: Some(Kind::StringValue("not-a-traceparent".to_string())),
        },
    );

    let decoded =
        GenerateWorkMessage::from_struct(&encoded).expect("a bad traceparent is not a bad message");

    assert_eq!(decoded.traceparent.as_deref(), Some("not-a-traceparent"));
}

// --- Trace continuation semantics -----------------------------------------
//
// Asserted on span contexts rather than on exported spans, because a span's own
// context is readable while it is open. The alternative — reading the consumer's
// exported parent — cannot be used at this layer: `tracing_subscriber::Registry`
// holds a span open while any descendant is open, so an operation's outermost
// span exports late and a test that waits for it is asserting on scheduling.

/// The property the whole change turns on: a consumer given a published context
/// ends up in the *same trace* under a *different span id*.
///
/// Both halves matter and each alone is satisfiable by a bug. Matching trace ids
/// alone is what verbatim-forwarding produces, and it leaves the consumer parented
/// to a span in the producer's process. A differing span id alone is just an
/// unrelated span.
#[test]
fn a_consumer_continues_the_publishers_trace_under_its_own_span_id() {
    let _probe = crate::test_fixtures::trace_probe::probe();

    let publisher_span = tracing::info_span!("publisher");
    let published = CapturedTraceContext::from_span(&publisher_span);
    let published_traceparent = published
        .traceparent
        .as_deref()
        .expect("the probe must give the publishing span a real context");

    let consumer_span = tracing::info_span!("consumer");
    let applied = set_remote_parent(&consumer_span, Some(published_traceparent), None);
    assert!(applied, "a valid traceparent must be adopted as the parent");

    let consumer = CapturedTraceContext::from_span(&consumer_span)
        .traceparent
        .expect("the consumer span has a context of its own");

    assert_eq!(
        trace_id_of(&consumer),
        trace_id_of(published_traceparent),
        "the consumer must join the publisher's trace"
    );
    assert_ne!(
        span_id_of(&consumer),
        span_id_of(published_traceparent),
        "and must be its own span in it, not a second span claiming the publisher's id"
    );
}

/// Absent context is not an error and not a parent. The span keeps the context it
/// would have had anyway.
#[test]
fn no_incoming_context_leaves_the_consumer_rooting_its_own_trace() {
    let _probe = crate::test_fixtures::trace_probe::probe();

    let publisher_span = tracing::info_span!("publisher");
    let published = CapturedTraceContext::from_span(&publisher_span)
        .traceparent
        .expect("probe must produce a context");

    let consumer_span = tracing::info_span!("consumer");
    assert!(!set_remote_parent(&consumer_span, None, None));

    let consumer = CapturedTraceContext::from_span(&consumer_span)
        .traceparent
        .expect("a span with no remote parent still has its own context");

    assert_ne!(
        trace_id_of(&consumer),
        trace_id_of(&published),
        "with nothing to continue, the consumer starts a trace of its own"
    );
}

/// A malformed traceparent is refused whole rather than half-applied. Adopting the
/// trace id from a value that failed to parse would attach the consumer to a trace
/// that may not exist, which is worse than starting a new one.
#[test]
fn a_malformed_incoming_context_is_refused_rather_than_partly_applied() {
    let _probe = crate::test_fixtures::trace_probe::probe();

    for malformed in [
        "not-a-traceparent",
        "",
        "00-tooshort-00f067aa0ba902b7-01",
        // Well-formed grammar, all-zero ids: the W3C spec calls this invalid.
        "00-00000000000000000000000000000000-0000000000000000-01",
    ] {
        // A span per case, and its context read before and after. Asserting only
        // the `false` return leaves the interesting failure uncovered: a partial
        // application that adopts the trace id and reports refusal would pass.
        let consumer_span = tracing::info_span!("consumer");
        let before = CapturedTraceContext::from_span(&consumer_span)
            .traceparent
            .expect("the probe gives every span a context of its own");

        assert!(
            !set_remote_parent(&consumer_span, Some(malformed), None),
            "'{malformed}' must not be adopted as a parent"
        );

        let after = CapturedTraceContext::from_span(&consumer_span);
        assert_eq!(
            after.traceparent.as_deref(),
            Some(before.as_str()),
            "'{malformed}' was refused, so the span must keep exactly the context \
             it had — not a trace id salvaged from a value that failed to parse"
        );
        assert_eq!(
            after.tracestate, None,
            "'{malformed}' carried no tracestate, so none may appear"
        );
    }
}

/// An over-long `traceparent` is refused before it reaches the propagator.
///
/// The value is not merely invalid, it is a log-volume problem: `otel-tools` logs a
/// rejected value at ERROR with the value inline and unbounded, once per delivery,
/// and that emit site is upstream. Refusing here is what bounds it.
#[test]
fn an_over_long_traceparent_is_refused_without_reaching_the_propagator() {
    let _probe = crate::test_fixtures::trace_probe::probe();

    let consumer_span = tracing::info_span!("consumer");
    let before = CapturedTraceContext::from_span(&consumer_span)
        .traceparent
        .expect("probe context");

    // A valid prefix followed by megabytes of padding: the shape that would
    // otherwise be logged in full, and the one a length check catches while a
    // grammar check would too, only after building the header.
    let padded = format!(
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01{}",
        "0".repeat(64 * 1024)
    );

    assert!(
        !set_remote_parent(&consumer_span, Some(&padded), None),
        "an over-long traceparent must be refused"
    );
    assert_eq!(
        CapturedTraceContext::from_span(&consumer_span)
            .traceparent
            .as_deref(),
        Some(before.as_str()),
        "refusal must leave the span rooting the trace it already had"
    );
}

/// A `traceparent` at exactly the standard length is unaffected by the bound.
///
/// The bound exists to stop an absurd value, not to second-guess the grammar, so
/// the case it must not touch is the ordinary one.
#[test]
fn a_standard_length_traceparent_is_unaffected_by_the_bound() {
    let _probe = crate::test_fixtures::trace_probe::probe();

    let publisher_span = tracing::info_span!("publisher");
    let published = CapturedTraceContext::from_span(&publisher_span);
    let traceparent = published.traceparent.as_deref().expect("probe context");
    assert_eq!(
        traceparent.len(),
        55,
        "a W3C traceparent is 55 characters; the bound's headroom is measured from here"
    );

    let consumer_span = tracing::info_span!("consumer");
    assert!(
        set_remote_parent(&consumer_span, Some(traceparent), None),
        "the ordinary case must still be adopted"
    );
}

/// `tracestate` rides along with `traceparent` rather than being dropped, which is
/// the reason this goes through the propagator instead of formatting one header.
#[test]
fn tracestate_survives_a_capture_and_apply_cycle() {
    let _probe = crate::test_fixtures::trace_probe::probe();

    let publisher_span = tracing::info_span!("publisher");
    let published = CapturedTraceContext::from_span(&publisher_span);
    let traceparent = published.traceparent.as_deref().expect("probe context");

    let consumer_span = tracing::info_span!("consumer");
    assert!(set_remote_parent(
        &consumer_span,
        Some(traceparent),
        Some("vendor=opaque")
    ));

    let carried = CapturedTraceContext::from_span(&consumer_span);
    assert_eq!(
        carried.tracestate.as_deref(),
        Some("vendor=opaque"),
        "vendor state must travel to the next hop, not stop at this one"
    );
}

fn trace_id_of(traceparent: &str) -> &str {
    crate::test_fixtures::trace_probe::trace_id_of(traceparent).expect("well-formed traceparent")
}

fn span_id_of(traceparent: &str) -> &str {
    crate::test_fixtures::trace_probe::span_id_of(traceparent).expect("well-formed traceparent")
}

/// Trace context is additive, so a message from a publisher that predates it
/// parses unchanged and reports no context. This is what lets the field ship
/// without a `schema_version` bump and its deploy ordering.
#[test]
fn a_message_without_trace_context_parses_as_absent() {
    let mut encoded = valid_message().to_struct();
    encoded.fields.remove("traceparent");
    encoded.fields.remove("tracestate");

    let decoded = GenerateWorkMessage::from_struct(&encoded).expect("must parse");

    assert_eq!(decoded.traceparent, None);
    assert_eq!(decoded.tracestate, None);
}

#[test]
fn round_trip_survives_a_json_hop() {
    let original = valid_message();
    let as_json = crate::utils::prost_to_json_value(&Value {
        kind: Some(Kind::StructValue(original.to_struct())),
    });
    let back = match crate::utils::json_to_prost_value(&as_json).unwrap().kind {
        Some(Kind::StructValue(back)) => back,
        other => panic!("expected a struct, got {other:?}"),
    };

    assert_eq!(GenerateWorkMessage::from_struct(&back).unwrap(), original);
}

#[test]
fn missing_schema_version_is_permanent() {
    let mut encoded = valid_message().to_struct();
    encoded.fields.remove("schema_version");

    let rejection = GenerateWorkMessage::from_struct(&encoded).unwrap_err();

    assert_eq!(rejection, GenerateMessageRejection::MissingSchemaVersion);
    assert!(
        !rejection.withholds_ack(),
        "an unparseable version is poison, not work to preserve"
    );
}

#[test]
fn unparseable_schema_version_is_permanent() {
    let mut encoded = valid_message().to_struct();
    encoded.fields.insert(
        "schema_version".to_string(),
        Value {
            kind: Some(Kind::StringValue("one".to_string())),
        },
    );

    assert_eq!(
        GenerateWorkMessage::from_struct(&encoded).unwrap_err(),
        GenerateMessageRejection::MissingSchemaVersion
    );
}

/// A rolling deploy puts an upgraded publisher next to replicas that predate it.
/// Those replicas must preserve the work rather than ACK it away as poison.
#[test]
fn newer_schema_version_withholds_the_ack() {
    let mut encoded = valid_message().to_struct();
    encoded.fields.insert(
        "schema_version".to_string(),
        Value {
            kind: Some(Kind::NumberValue(f64::from(GENERATE_SCHEMA_VERSION + 1))),
        },
    );

    let rejection = GenerateWorkMessage::from_struct(&encoded).unwrap_err();

    assert_eq!(
        rejection,
        GenerateMessageRejection::NewerSchemaVersion {
            received: GENERATE_SCHEMA_VERSION + 1,
            supported: GENERATE_SCHEMA_VERSION,
        }
    );
    assert!(rejection.withholds_ack());
}

#[test]
fn an_additive_field_does_not_break_parsing() {
    let mut encoded = valid_message().to_struct();
    encoded.fields.insert(
        "some_future_field".to_string(),
        Value {
            kind: Some(Kind::StringValue("ignored".to_string())),
        },
    );

    assert_eq!(
        GenerateWorkMessage::from_struct(&encoded).unwrap(),
        valid_message()
    );
}

#[test]
fn each_missing_required_field_is_rejected_by_name() {
    for field in [
        "source",
        "destination_resource_address",
        "target_type",
        "specification",
    ] {
        let mut encoded = valid_message().to_struct();
        encoded.fields.remove(field);

        assert_eq!(
            GenerateWorkMessage::from_struct(&encoded).unwrap_err(),
            GenerateMessageRejection::MalformedField(field),
            "'{field}' is required"
        );
    }
}

#[test]
fn missing_nested_specification_fields_are_rejected_by_qualified_name() {
    for (field, qualified) in [
        ("width", "specification.width"),
        ("height", "specification.height"),
        ("quality", "specification.quality"),
        (
            "preserve_aspect_ratio",
            "specification.preserve_aspect_ratio",
        ),
        ("encoding_format", "specification.encoding_format"),
    ] {
        let mut encoded = valid_message().to_struct();
        let specification = match encoded
            .fields
            .get_mut("specification")
            .unwrap()
            .kind
            .as_mut()
        {
            Some(Kind::StructValue(specification)) => specification,
            other => panic!("specification must be a struct, got {other:?}"),
        };
        specification.fields.remove(field);

        assert_eq!(
            GenerateWorkMessage::from_struct(&encoded).unwrap_err(),
            GenerateMessageRejection::MalformedField(qualified)
        );
    }
}

#[test]
fn an_unrecognized_enum_value_names_the_field_and_the_value() {
    let mut encoded = valid_message().to_struct();
    let specification = match encoded
        .fields
        .get_mut("specification")
        .unwrap()
        .kind
        .as_mut()
    {
        Some(Kind::StructValue(specification)) => specification,
        other => panic!("specification must be a struct, got {other:?}"),
    };
    specification.fields.insert(
        "transform_type".to_string(),
        Value {
            kind: Some(Kind::StringValue("SPIRALIZE".to_string())),
        },
    );

    assert_eq!(
        GenerateWorkMessage::from_struct(&encoded).unwrap_err(),
        GenerateMessageRejection::UnrecognizedValue {
            field: "specification.transform_type",
            value: "SPIRALIZE".to_string(),
        }
    );
}

#[test]
fn a_message_with_no_fields_at_all_is_rejected_rather_than_defaulted() {
    assert_eq!(
        GenerateWorkMessage::from_struct(&Struct::default()).unwrap_err(),
        GenerateMessageRejection::MissingSchemaVersion
    );
}

// --- Publishing -----------------------------------------------------------

/// Trace context comes from the span that publishes, not the one that built the
/// message.
///
/// The two coincide on the only production path, where dispatch builds and
/// publishes in the same breath, so nothing observable distinguished them — which
/// is exactly why this needs pinning. Both functions are public, and a caller that
/// built a batch under one span and published it under another would otherwise
/// hand the worker a parent span that never published anything, attributing the
/// queue dwell to the wrong request.
#[tokio::test]
async fn publish_captures_the_span_that_publishes_not_the_one_that_built() {
    use crate::test_fixtures::trace_probe;

    let _probe = trace_probe::probe();
    let publisher = MockPublisherServer::start().await;
    let outbound = publisher.outbound();

    let build_span = tracing::info_span!("test.builder");
    let built_context = CapturedTraceContext::from_span(&build_span);
    let built_traceparent = built_context
        .traceparent
        .as_deref()
        .expect("probe must give the build span a context");

    // Built under one span...
    let message = build_span.in_scope(valid_message);

    // ...published under another.
    let publish_span = tracing::info_span!("test.publisher");
    let publish_context = CapturedTraceContext::from_span(&publish_span);
    let publish_traceparent = publish_context
        .traceparent
        .as_deref()
        .expect("probe must give the publish span a context");

    publish_generate_event(&outbound, &message)
        .instrument(publish_span)
        .await
        .expect("a healthy publisher accepts the event");

    let published = publisher.await_events(1, Duration::from_secs(2)).await;
    let sent = published[0]
        .message_str("/traceparent")
        .expect("the event must carry trace context");

    assert_eq!(
        span_id_of(sent),
        span_id_of(publish_traceparent),
        "the parent must be the publishing span"
    );
    assert_ne!(
        span_id_of(sent),
        span_id_of(built_traceparent),
        "and specifically not the span the message was constructed under"
    );

    publisher.shutdown().await;
}

/// A message carrying stale trace context does not get to keep it.
///
/// Replacing rather than filling a gap: a value the message was carrying is not
/// evidence about the span putting it on the wire, and honouring it would let a
/// consumer be parented to something that never published.
#[tokio::test]
async fn publish_replaces_trace_context_the_message_was_carrying() {
    let publisher = MockPublisherServer::start().await;
    let outbound = publisher.outbound();

    // No probe, so nothing is capturable at publish: the stale value must be
    // dropped rather than passed through.
    let mut message = valid_message();
    message.traceparent =
        Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_string());
    message.tracestate = Some("vendor=stale".to_string());

    publish_generate_event(&outbound, &message)
        .await
        .expect("a healthy publisher accepts the event");

    let published = publisher.await_events(1, Duration::from_secs(2)).await;
    assert_eq!(published[0].message_str("/traceparent"), None);
    assert_eq!(published[0].message_str("/tracestate"), None);

    publisher.shutdown().await;
}

#[tokio::test]
async fn publish_emits_the_canonical_type_with_the_source_as_resource_id() {
    let publisher = MockPublisherServer::start().await;
    let outbound = publisher.outbound();

    publish_generate_event(&outbound, &valid_message())
        .await
        .expect("a healthy publisher accepts the event");

    let published = publisher.await_events(1, Duration::from_secs(2)).await;
    assert_eq!(published[0].event_type, GENERATE_EVENT_TYPE);
    assert_eq!(published[0].resource_id, SOURCE_ADDRESS);
    assert_eq!(
        published[0].message_str("/source/resource_identity"),
        Some(SOURCE_IDENTITY)
    );
    assert_eq!(
        published[0].message_str("/destination_resource_address"),
        Some(DESTINATION)
    );

    publisher.shutdown().await;
}

/// The whole point of a separate publisher: generate failures reach the caller
/// so the CREATE acknowledgement can be decided from them.
#[tokio::test]
async fn publish_propagates_transport_failures_with_their_classification() {
    for (code, expect_retryable) in [
        (tonic::Code::Unavailable, true),
        (tonic::Code::DeadlineExceeded, true),
        (tonic::Code::InvalidArgument, false),
        (tonic::Code::PermissionDenied, false),
    ] {
        let publisher = MockPublisherServer::start_with(PublisherBehavior::FailWith(code)).await;
        let outbound = publisher.outbound();

        let error = publish_generate_event(&outbound, &valid_message())
            .await
            .expect_err("an injected transport failure must reach the caller");

        assert!(
            matches!(error, GeneratePublishError::Transport(_)),
            "{code:?} should surface as a transport failure, got {error}"
        );
        assert_eq!(
            error.is_retryable(),
            expect_retryable,
            "{code:?} classification"
        );

        publisher.shutdown().await;
    }
}

/// The `Unauthenticated` refresh-and-reissue branch on the generate publish, which
/// is one of the three places the layered `TonicTraceService<InterceptedService<…>>`
/// stack is driven twice on one client. Nothing exercised any of them.
///
/// The reissue is asserted to *succeed*, not merely to happen: a second call that
/// went out with the same rejected token would satisfy a call-count assertion while
/// leaving the publish broken, and a generate publish that fails permanently is a
/// thumbnail that never gets made.
#[tokio::test]
async fn an_unauthenticated_generate_publish_is_reissued_and_succeeds() {
    // Installed explicitly rather than inherited. `publish_generate_event` reads
    // the process-global runtime config for its deadline, and most tests on this
    // path only pass because some *other* test in the binary installed it first —
    // the ordering flake recorded as `thumbnail-service-review-remediation` §11.7.
    // Written without this line first, and it panicked at `config.rs:677`.
    crate::test_fixtures::init_test_runtime_config("thumbnail-common-work-events-tests");

    // Without an OTel layer the propagator injects nothing and the traceparent
    // assertions below would hold against a stack that lost the trace layer.
    let _probe = crate::test_fixtures::trace_probe::probe();

    let publisher =
        MockPublisherServer::start_with(PublisherBehavior::UnauthenticatedThenSucceed).await;
    let outbound = publisher.outbound_with_token_refresh();

    publish_generate_event(&outbound, &valid_message())
        .await
        .expect("the reissue after refresh must carry the event through");

    let published = publisher.await_events(2, Duration::from_secs(2)).await;
    assert_eq!(
        published.len(),
        2,
        "the rejected attempt and the reissue are both real publishes"
    );
    for (attempt, event) in published.iter().enumerate() {
        assert!(
            event.traceparent.is_some(),
            "attempt {} must carry trace context; the reissue runs through the \
             same layered stack and is where losing it would go unnoticed",
            attempt + 1
        );
    }

    // The assertion that makes this about a *refresh* rather than a retry. The
    // double answers the second call by index, so a reissue resending the rejected
    // token would satisfy every count and header-shape check above.
    use crate::outbound::OutboundClients;
    let first = format!("Bearer {}", OutboundClients::TEST_TOKEN);
    let refreshed = format!("Bearer {}", OutboundClients::TEST_TOKEN_REFRESHED);
    let tokens: Vec<Option<&str>> = published
        .iter()
        .map(|event| event.authorization.as_deref())
        .collect();
    assert_eq!(
        tokens,
        vec![Some(first.as_str()), Some(refreshed.as_str())],
        "the second attempt must carry the refreshed credential, not the one the \
         server just rejected"
    );

    publisher.shutdown().await;
}

#[tokio::test]
async fn publish_treats_an_aggregation_rejection_as_retryable() {
    let publisher =
        MockPublisherServer::start_with(PublisherBehavior::Reject("queue full".to_string())).await;
    let outbound = publisher.outbound();

    let error = publish_generate_event(&outbound, &valid_message())
        .await
        .expect_err("success=false is not a successful dispatch");

    assert!(matches!(error, GeneratePublishError::Rejected { .. }));
    assert!(
        error.is_retryable(),
        "the reason is unclassifiable, and losing the thumbnail is worse than a duplicate"
    );

    publisher.shutdown().await;
}

/// An unconfigured channel must never look like a successful dispatch, because
/// ACKing the CREATE would assert work was enqueued that never left the process.
#[tokio::test]
async fn publish_without_a_channel_fails_retryably_rather_than_skipping() {
    let mut outbound = crate::outbound::OutboundClients::for_tests();
    outbound.event_publishing_channel = None;

    let error = publish_generate_event(&outbound, &valid_message())
        .await
        .expect_err("a missing channel is not a silent skip");

    assert!(matches!(error, GeneratePublishError::ChannelUnconfigured));
    assert!(error.is_retryable());
}

/// The contrast that motivates the separate API: association publishing keeps
/// swallowing its errors, because a missed notification costs a notification
/// while a missed generate costs the thumbnail.
#[tokio::test]
async fn association_publishing_stays_best_effort_under_the_same_failure() {
    use crate::notifications::event_publisher::{
        publish_event, AssociationEventKind, AssociationEventParams,
    };
    use crate::storage::{AssociationMessageParams, AssociationResource, AssociationTargetKind};

    let publisher =
        MockPublisherServer::start_with(PublisherBehavior::FailWith(tonic::Code::Unavailable))
            .await;
    let outbound = publisher.outbound();

    // Returns `()`: there is no error to propagate even when the publisher is down.
    publish_event(
        outbound.clone(),
        &AssociationEventParams {
            event_kind: AssociationEventKind::Associated,
            message: AssociationMessageParams {
                thumbnail: AssociationResource {
                    resource_address: DESTINATION.to_string(),
                    resource_identity: Some(format!("{DESTINATION}?etag=thumb")),
                },
                association_target: AssociationResource {
                    resource_address: SOURCE_ADDRESS.to_string(),
                    resource_identity: Some(SOURCE_IDENTITY.to_string()),
                },
                target_kind: AssociationTargetKind::File,
                specification: None,
            },
        },
        // Passed in rather than read from the global, which is the point of the
        // signature: this call runs detached in production, where a panicking
        // config read would lose the event with nothing observing the failure.
        Duration::from_secs(5),
    )
    .await;

    // Reaching this line is most of the point — the call returned rather than
    // propagating — but on its own that would also hold if `publish_event` did
    // nothing at all. Assert it genuinely attempted the publish and swallowed
    // the failure, so the test can fail if the best-effort path is ever quietly
    // removed.
    let attempted = publisher.settle(Duration::from_millis(100)).await;
    assert_eq!(
        attempted.len(),
        1,
        "the association publish should have been attempted against the failing publisher"
    );
    assert_eq!(attempted[0].event_type, "omni.thumbnail.associated");

    // And the contrast that motivates the separate generate API: the same
    // injected failure, on the same publisher, does reach the caller there.
    let generate = publish_generate_event(&outbound, &valid_message()).await;
    assert!(
        generate.is_err(),
        "generate publishing must propagate what association publishing swallows"
    );

    publisher.shutdown().await;
}

#[test]
fn quality_build_and_parse_rules_agree_for_every_format() {
    for format in [
        EncodingFormat::Png,
        EncodingFormat::Gif,
        EncodingFormat::Jpeg,
        EncodingFormat::Webp,
    ] {
        for quality in [0, 1, 85, 100, 200] {
            let mut spec = complete_spec();
            spec.encoding_format = Some(format);
            spec.quality = Some(quality);
            let expected_valid = matches!(format, EncodingFormat::Png | EncodingFormat::Gif)
                || (1..=100).contains(&quality);
            assert_eq!(build(&spec).is_ok(), expected_valid, "{format} {quality}");
            let mut message = valid_message();
            message.specification.encoding_format = format;
            message.specification.quality = quality as u32;
            let parsed = GenerateWorkMessage::from_struct(&message.to_struct());
            if expected_valid {
                assert_eq!(parsed.unwrap(), message);
            } else {
                assert!(matches!(
                    parsed,
                    Err(GenerateMessageRejection::Contract(
                        GenerateContractError::InvalidQuality
                    ))
                ));
            }
            message.specification.width = 0;
            assert!(matches!(
                message.specification.validate(),
                Err(GenerateContractError::InvalidDimensions)
            ));
        }
    }
    assert_eq!(
        GenerateContractError::InvalidQuality.to_string(),
        "specification quality must be between 1 and 100 for JPEG/WebP"
    );
}
