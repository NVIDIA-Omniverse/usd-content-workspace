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

//! The `omni.thumbnail.generate` work-event contract.
//!
//! One definition of the message, used by `server` when it publishes generation
//! work and by `task-worker` when it consumes it. Both binaries build and parse
//! through the types here rather than reshaping JSON at each end, so a field
//! added on one side cannot silently go missing on the other.
//!
//! Two layers are deliberately distinct and easy to conflate:
//!
//! - The **Notification envelope** carries `event.resource.resource_id`, a single
//!   string that durable queues filter on and dead-letter triage reads. For
//!   generate it is the source `resource_address`, matching the associated-event
//!   convention in ADR 0002 §3.
//! - The **message** carries the ADR 0002 `Resource` shape for the source
//!   (address required, identity required for files) plus the destination and the
//!   fully resolved specification.
//!
//! The consumer-side `Event` has no `resource` field at all, so the worker reads
//! the source from the message. Teardown never keys off the envelope alone.

mod dispatch;
mod publish;
mod retry;
mod trace_context;

use std::collections::BTreeMap;

use prost_types::{value::Kind, Struct, Value};
use thiserror::Error;

use crate::resource::{validate_resource, ResourceValidationError};
use crate::spec_enums::{CropStrategy, EncodingFormat, TransformType};
use crate::storage::AssociationTargetKind;
use crate::types::ImageProcessingSpec;

pub use dispatch::{
    dispatch_generate_event, dispatch_generate_for_configured_sizes,
    dispatch_generate_per_specification, GenerateDispatchError, GenerateFanOutFailure,
    GenerateSource, SpecificationDispatchFailure, SpecificationDispatchOutcome,
};
pub use publish::{publish_generate_event, GeneratePublishError};
pub use retry::is_retryable_tonic_code;
pub use trace_context::{set_remote_parent, CapturedTraceContext};

/// Notification event type carrying exactly one unit of thumbnail work.
pub const GENERATE_EVENT_TYPE: &str = "omni.thumbnail.generate";

/// The schema version this build publishes and accepts.
///
/// Bumping it is deploy-ordered: every worker replica that can consume WorkerEventQueue
/// must accept the new version before `server` starts publishing it. A worker
/// that meets a *newer* version withholds the ACK rather than discarding the
/// message as poison — see [`GenerateMessageRejection::withholds_ack`].
pub const GENERATE_SCHEMA_VERSION: u32 = 1;

/// Largest width or height the contract accepts, matching the documented range on
/// `ImageToImageSpecification` and `AssociationSpec::MAX_DIMENSION`.
///
/// Public so the transport boundaries can reject an out-of-range dimension as a
/// caller error before paying for a storage `Stat`, rather than restating the
/// literal per transport and letting the contract layer be the only enforcer.
pub const MAX_DIMENSION: u32 = 8192;

/// Most specifications one `CreateThumbnails` request may supply, on either transport.
///
/// A bound on enqueued work rather than on message size: each specification becomes
/// one published generate event and then one conversion on a worker pool that runs a
/// small number at a time, so an unbounded list lets a single request fill a shared
/// queue.
///
/// Public, and here beside [`MAX_DIMENSION`], for the reason that constant is: the
/// number lived in the HTTP request types, so gRPC enforced nothing and the two
/// transports disagreed about which requests were legal. 1,000 is the figure HTTP
/// already enforced, carried over deliberately rather than derived — raising it later
/// is additive, lowering it is not.
pub const MAX_CREATE_THUMBNAIL_SPECIFICATIONS: usize = 1000;

/// The source a thumbnail is generated from, in the ADR 0002 `Resource` shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkResource {
    /// Always required and non-empty.
    pub resource_address: String,
    /// Required for file resources, omitted for folders.
    pub resource_identity: Option<String>,
}

impl WorkResource {
    pub fn file(resource_address: impl Into<String>, resource_identity: impl Into<String>) -> Self {
        Self {
            resource_address: resource_address.into(),
            resource_identity: Some(resource_identity.into()),
        }
    }
}

/// A specification with every configurable field resolved by the publisher.
///
/// The worker substitutes nothing: it has no `THUMBNAIL_*` configuration of its
/// own, so a field missing here is a validation failure rather than a default.
/// Always IMAGE_TO_IMAGE by construction — MANUAL specs never enter the work queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedGenerateSpec {
    pub width: u32,
    pub height: u32,
    pub encoding_format: EncodingFormat,
    pub transform_type: TransformType,
    pub crop_strategy: CropStrategy,
    pub quality: u32,
    pub preserve_aspect_ratio: bool,
}

impl ResolvedGenerateSpec {
    /// The contract's value rules, applied wherever a specification is produced.
    ///
    /// Both the publisher building a message and the worker parsing one run this,
    /// which matters most for the dimension bound. Decode limits cap the *source*
    /// an image is read from; nothing else caps the *output* a specification asks
    /// for, so a message requesting 65535x65535 would decode a small source
    /// happily and then allocate seventeen gigabytes resizing it — an OOM kill
    /// with the delivery unacknowledged, reproducing on every redelivery. That is
    /// precisely the crash loop the decode bounds exist to prevent, reached
    /// through the one door they do not cover.
    pub fn validate(&self) -> Result<(), GenerateContractError> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_DIMENSION
            || self.height > MAX_DIMENSION
        {
            return Err(GenerateContractError::InvalidDimensions);
        }
        if self.encoding_format.requires_lossy_quality_range() && !(1..=100).contains(&self.quality)
        {
            return Err(GenerateContractError::InvalidQuality);
        }
        Ok(())
    }
}

/// One `omni.thumbnail.generate` message: one source, one resolved
/// specification, one destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateWorkMessage {
    pub schema_version: u32,
    pub source: WorkResource,
    pub destination_resource_address: String,
    /// The target type the resulting association will use.
    pub target_type: AssociationTargetKind,
    pub specification: ResolvedGenerateSpec,
    /// W3C trace context, as read from a received message.
    ///
    /// **Populated on the consume side only.** `build_generate_message` leaves
    /// both empty and `publish_generate_event` writes the context of the span
    /// active when it serializes, because the parent the consumer should join is
    /// the span that actually published — not whichever span happened to be
    /// active when the message was constructed. The two coincide on today's only
    /// path, where dispatch builds and publishes in the same breath, but both
    /// functions are public and a caller that separated them would otherwise send
    /// a parent span that never published anything and attribute queue dwell to
    /// the wrong request.
    ///
    /// Flat rather than nested because the wire is flat: two top-level message
    /// keys, mirroring the two headers the same context travels in over HTTP.
    pub traceparent: Option<String>,
    pub tracestate: Option<String>,
}

/// Why a generate message could not be built from its inputs.
///
/// Every variant is permanent: none of them improves on redelivery, so a CREATE
/// that hits one is ACKed rather than retried.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GenerateContractError {
    #[error("source resource is invalid: {0}")]
    InvalidSource(ResourceValidationError),

    #[error("destination_resource_address is required and non-empty")]
    MissingDestination,

    #[error("destination_resource_address must be an absolute storage address, got '{0}'")]
    RelativeDestination(String),

    #[error("destination_resource_address must not match the source resource_address")]
    DestinationEqualsSource,

    #[error("specification {field} is required and was not resolved")]
    MissingSpecificationField { field: &'static str },

    #[error("specification width and height must both be between 1 and {MAX_DIMENSION}")]
    InvalidDimensions,

    #[error("specification quality must be between 1 and 100 for JPEG/WebP")]
    InvalidQuality,
}

/// Why a received generate message was not accepted.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GenerateMessageRejection {
    #[error("schema_version is missing or is not a number")]
    MissingSchemaVersion,

    #[error("schema_version {received} is newer than the supported version {supported}")]
    NewerSchemaVersion { received: u32, supported: u32 },

    #[error(
        "schema_version {received} is not supported by this build (which accepts {supported})"
    )]
    UnsupportedSchemaVersion { received: u32, supported: u32 },

    #[error("field '{0}' is missing or has the wrong type")]
    MalformedField(&'static str),

    #[error("field '{field}' has an unrecognized value '{value}'")]
    UnrecognizedValue { field: &'static str, value: String },

    #[error(transparent)]
    Contract(#[from] GenerateContractError),
}

impl GenerateMessageRejection {
    /// Whether the delivery must be left unacknowledged rather than discarded.
    ///
    /// Only a *newer* schema version qualifies. Treating one as poison would have
    /// an old replica ACK-and-discard real work during a rolling deploy, so it is
    /// preserved for a replica that understands it or for the dead-letter queue.
    /// Everything else here is malformed input that no redelivery improves.
    pub fn withholds_ack(&self) -> bool {
        matches!(self, Self::NewerSchemaVersion { .. })
    }
}

/// Inputs to [`build_generate_message`].
pub struct GenerateMessageParams<'a> {
    pub source: WorkResource,
    /// The destination already rendered from `THUMBNAIL_PATH_TEMPLATE`. The
    /// worker never re-derives it.
    pub destination_resource_address: &'a str,
    pub target_type: AssociationTargetKind,
    /// The specification to resolve and validate. Caller-supplied and
    /// configuration-derived specifications both come through here, so they
    /// produce the same message shape and the worker cannot tell them apart.
    pub specification: &'a ImageProcessingSpec,
}

/// Build and validate one generate message.
///
/// This is the only way a generate message is created. Rejecting a
/// non-image-to-image processor here rather than at the worker keeps unsupported
/// work off WorkerEventQueue entirely instead of publishing it and discarding it later.
///
/// Trace context is captured from the span active here rather than accepted as a
/// parameter. A caller-supplied value would be a second source of truth for the
/// same field, and the one it competes with — a context received from upstream —
/// must not be forwarded verbatim: its span id names a span in another process, so
/// using it directly parents the consumer to that span and omits this service from
/// the trace entirely. Continuing a trace means being a span in it.
pub fn build_generate_message(
    params: GenerateMessageParams<'_>,
) -> Result<GenerateWorkMessage, GenerateContractError> {
    let (source, destination_resource_address) = validate_generate_resources(
        params.source,
        params.destination_resource_address,
        params.target_type,
    )?;

    let specification = resolve_generate_spec(params.specification)?;

    Ok(GenerateWorkMessage {
        schema_version: GENERATE_SCHEMA_VERSION,
        source,
        destination_resource_address,
        target_type: params.target_type,
        specification,
        // Left empty deliberately. Trace context belongs to the act of putting the
        // message on the wire, so `publish_generate_event` sets it; see the field
        // docs on `GenerateWorkMessage`.
        traceparent: None,
        tracestate: None,
    })
}

fn validate_generate_resources(
    source: WorkResource,
    destination_resource_address: &str,
    target_type: AssociationTargetKind,
) -> Result<(WorkResource, String), GenerateContractError> {
    let requires_identity = matches!(target_type, AssociationTargetKind::File);
    validate_resource(
        &source.resource_address,
        source.resource_identity.as_deref(),
        requires_identity,
    )
    .map_err(GenerateContractError::InvalidSource)?;

    let destination = destination_resource_address.trim();
    if destination.is_empty() {
        return Err(GenerateContractError::MissingDestination);
    }
    if !is_absolute_address(destination) {
        return Err(GenerateContractError::RelativeDestination(
            destination.to_string(),
        ));
    }
    if canonicalize_address(destination) == canonicalize_address(&source.resource_address) {
        return Err(GenerateContractError::DestinationEqualsSource);
    }

    Ok((
        WorkResource {
            resource_address: source.resource_address.trim().to_string(),
            resource_identity: source
                .resource_identity
                .map(|identity| identity.trim().to_string()),
        },
        destination.to_string(),
    ))
}

/// Canonicalize a storage address for comparison.
///
/// Two addresses that differ only in scheme or host case, or in an explicitly
/// spelled default port, name the same stored object. Comparing them raw would
/// let a non-canonical source slip past the destination-equals-source guard and
/// overwrite itself. An address that does not parse as a URL compares trimmed,
/// which is the strictest thing available for it.
pub fn canonicalize_address(address: &str) -> String {
    crate::resource::canonicalize_resource_address(address)
}

/// Whether an address is absolute enough to write to.
///
/// One definition, called by the publisher when it builds a message and by the
/// worker before it writes. They used to answer this separately — the publisher
/// with `reqwest::Url::parse`, the worker with a hand-rolled approximation,
/// because `task-worker` does not depend on `reqwest` — which is two rules
/// guarding one invariant and no guarantee they agreed.
pub fn is_absolute_address(address: &str) -> bool {
    reqwest::Url::parse(address.trim()).is_ok()
}

/// Validate a possibly-partial specification into one with every field present.
///
/// Public because the destination template needs the resolved dimensions and
/// encoding before the message can be built, and re-deriving them at that call
/// site is how the two drift.
pub fn resolve_generate_spec(
    spec: &ImageProcessingSpec,
) -> Result<ResolvedGenerateSpec, GenerateContractError> {
    fn required<T>(value: Option<T>, field: &'static str) -> Result<T, GenerateContractError> {
        value.ok_or(GenerateContractError::MissingSpecificationField { field })
    }

    // Presence first, so a missing field is named as missing rather than failing
    // a range check on whatever a default would have been.
    let resolved = ResolvedGenerateSpec {
        width: required(spec.width, "width")?,
        height: required(spec.height, "height")?,
        encoding_format: required(spec.encoding_format, "encoding_format")?,
        transform_type: required(spec.transform_type, "transform_type")?,
        crop_strategy: required(spec.crop_strategy, "crop_strategy")?,
        quality: required(spec.quality, "quality")? as u32,
        preserve_aspect_ratio: required(spec.preserve_aspect_ratio, "preserve_aspect_ratio")?,
    };

    resolved.validate()?;
    Ok(resolved)
}

impl GenerateWorkMessage {
    /// The Notification envelope's `resource_id` for this message.
    ///
    /// A single filter string, not the message's `Resource`: the two are separate
    /// layers and the worker resolves the source from the message.
    pub fn envelope_resource_id(&self) -> &str {
        &self.source.resource_address
    }

    pub fn to_struct(&self) -> Struct {
        let spec = &self.specification;
        let mut specification = BTreeMap::new();
        specification.insert("width".to_string(), number_value(spec.width));
        specification.insert("height".to_string(), number_value(spec.height));
        specification.insert(
            "encoding_format".to_string(),
            string_value(spec.encoding_format.to_string()),
        );
        specification.insert(
            "transform_type".to_string(),
            string_value(spec.transform_type.to_string()),
        );
        specification.insert(
            "crop_strategy".to_string(),
            string_value(spec.crop_strategy.to_string()),
        );
        specification.insert("quality".to_string(), number_value(spec.quality));
        specification.insert(
            "preserve_aspect_ratio".to_string(),
            Value {
                kind: Some(Kind::BoolValue(spec.preserve_aspect_ratio)),
            },
        );

        let mut source = BTreeMap::new();
        source.insert(
            "resource_address".to_string(),
            string_value(self.source.resource_address.clone()),
        );
        if let Some(identity) = &self.source.resource_identity {
            source.insert(
                "resource_identity".to_string(),
                string_value(identity.clone()),
            );
        }

        let mut fields = BTreeMap::new();
        fields.insert(
            "schema_version".to_string(),
            number_value(self.schema_version),
        );
        fields.insert("source".to_string(), struct_value(source));
        fields.insert(
            "destination_resource_address".to_string(),
            string_value(self.destination_resource_address.clone()),
        );
        fields.insert(
            "target_type".to_string(),
            string_value(self.target_type.as_str().to_string()),
        );
        fields.insert("specification".to_string(), struct_value(specification));
        if let Some(traceparent) = &self.traceparent {
            fields.insert("traceparent".to_string(), string_value(traceparent.clone()));
        }
        // Only ever present alongside a traceparent, and omitted when empty, which
        // is what the W3C format says about an empty tracestate.
        if let Some(tracestate) = &self.tracestate {
            fields.insert("tracestate".to_string(), string_value(tracestate.clone()));
        }

        Struct { fields }
    }

    /// Parse a received message.
    ///
    /// Unrecognized fields are ignored rather than rejected, so an additive change
    /// on the publisher does not break a worker that predates it.
    pub fn from_struct(message: &Struct) -> Result<Self, GenerateMessageRejection> {
        let schema_version = read_number(&message.fields, "schema_version")
            .ok_or(GenerateMessageRejection::MissingSchemaVersion)?;
        if !schema_version.is_finite() || schema_version < 0.0 || schema_version.fract() != 0.0 {
            return Err(GenerateMessageRejection::MissingSchemaVersion);
        }
        let schema_version = u32::try_from(schema_version as u64)
            .map_err(|_| GenerateMessageRejection::MissingSchemaVersion)?;

        if schema_version > GENERATE_SCHEMA_VERSION {
            return Err(GenerateMessageRejection::NewerSchemaVersion {
                received: schema_version,
                supported: GENERATE_SCHEMA_VERSION,
            });
        }
        if schema_version != GENERATE_SCHEMA_VERSION {
            return Err(GenerateMessageRejection::UnsupportedSchemaVersion {
                received: schema_version,
                supported: GENERATE_SCHEMA_VERSION,
            });
        }

        let source_struct = read_struct(&message.fields, "source")
            .ok_or(GenerateMessageRejection::MalformedField("source"))?;
        let source = WorkResource {
            resource_address: read_string(&source_struct.fields, "resource_address").ok_or(
                GenerateMessageRejection::MalformedField("source.resource_address"),
            )?,
            resource_identity: read_string(&source_struct.fields, "resource_identity"),
        };

        let destination_resource_address =
            read_string(&message.fields, "destination_resource_address").ok_or(
                GenerateMessageRejection::MalformedField("destination_resource_address"),
            )?;

        let target_type_wire = read_string(&message.fields, "target_type")
            .ok_or(GenerateMessageRejection::MalformedField("target_type"))?;
        let target_type = AssociationTargetKind::from_wire(&target_type_wire).ok_or_else(|| {
            GenerateMessageRejection::UnrecognizedValue {
                field: "target_type",
                value: target_type_wire.clone(),
            }
        })?;
        let (source, destination_resource_address) =
            validate_generate_resources(source, &destination_resource_address, target_type)?;

        let spec_struct = read_struct(&message.fields, "specification")
            .ok_or(GenerateMessageRejection::MalformedField("specification"))?;
        let specification = ResolvedGenerateSpec {
            width: read_dimension(&spec_struct.fields, "specification.width")?,
            height: read_dimension(&spec_struct.fields, "specification.height")?,
            encoding_format: read_enum(&spec_struct.fields, "specification.encoding_format")?,
            transform_type: read_enum(&spec_struct.fields, "specification.transform_type")?,
            crop_strategy: if spec_struct.fields.contains_key("crop_strategy") {
                read_enum(&spec_struct.fields, "specification.crop_strategy")?
            } else {
                read_enum(&spec_struct.fields, "specification.crop_direction")?
            },
            quality: read_dimension(&spec_struct.fields, "specification.quality")?,
            preserve_aspect_ratio: read_flag(
                &spec_struct.fields,
                "specification.preserve_aspect_ratio",
            )?,
        };

        // Shape is not enough. A message can name every field with the right
        // type and still ask for something the contract forbids, and a consumer
        // that only checked shape would carry it all the way to the point where
        // it does damage.
        specification.validate()?;

        Ok(GenerateWorkMessage {
            schema_version,
            source,
            destination_resource_address,
            target_type,
            specification,
            // Read but not validated. A malformed traceparent is rejected where it
            // is used, by the propagator that owns the grammar, which logs it once
            // and leaves the consumer rooting its own trace. Validating here would
            // duplicate that grammar and risk turning a bad telemetry field into a
            // parse failure — which for this message means an unacknowledged
            // delivery and a redelivery loop over a field nothing depends on.
            traceparent: read_string(&message.fields, "traceparent"),
            tracestate: read_string(&message.fields, "tracestate"),
        })
    }
}

fn string_value(value: String) -> Value {
    Value {
        kind: Some(Kind::StringValue(value)),
    }
}

fn number_value(value: u32) -> Value {
    Value {
        kind: Some(Kind::NumberValue(f64::from(value))),
    }
}

fn struct_value(fields: BTreeMap<String, Value>) -> Value {
    Value {
        kind: Some(Kind::StructValue(Struct { fields })),
    }
}

fn read_string(fields: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    match fields.get(key).and_then(|value| value.kind.as_ref()) {
        Some(Kind::StringValue(value)) if !value.trim().is_empty() => Some(value.clone()),
        _ => None,
    }
}

fn read_number(fields: &BTreeMap<String, Value>, key: &str) -> Option<f64> {
    match fields.get(key).and_then(|value| value.kind.as_ref()) {
        Some(Kind::NumberValue(value)) => Some(*value),
        _ => None,
    }
}

fn read_bool(fields: &BTreeMap<String, Value>, key: &str) -> Option<bool> {
    match fields.get(key).and_then(|value| value.kind.as_ref()) {
        Some(Kind::BoolValue(value)) => Some(*value),
        _ => None,
    }
}

fn read_struct<'a>(fields: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a Struct> {
    match fields.get(key).and_then(|value| value.kind.as_ref()) {
        Some(Kind::StructValue(value)) => Some(value),
        _ => None,
    }
}

/// The leaf name of a dotted key. Nested readers take the fully qualified key so
/// their errors name the field as it appears on the wire, and look up the leaf.
fn leaf_of(qualified_key: &'static str) -> &'static str {
    qualified_key.rsplit('.').next().unwrap_or(qualified_key)
}

fn read_flag(
    fields: &BTreeMap<String, Value>,
    qualified_key: &'static str,
) -> Result<bool, GenerateMessageRejection> {
    read_bool(fields, leaf_of(qualified_key))
        .ok_or(GenerateMessageRejection::MalformedField(qualified_key))
}

/// Read a `u32` the message carries as a JSON number.
fn read_dimension(
    fields: &BTreeMap<String, Value>,
    qualified_key: &'static str,
) -> Result<u32, GenerateMessageRejection> {
    let raw = read_number(fields, leaf_of(qualified_key))
        .ok_or(GenerateMessageRejection::MalformedField(qualified_key))?;
    if raw.fract() != 0.0 || raw < 0.0 {
        return Err(GenerateMessageRejection::MalformedField(qualified_key));
    }
    u32::try_from(raw as u64).map_err(|_| GenerateMessageRejection::MalformedField(qualified_key))
}

fn read_enum<T: std::str::FromStr>(
    fields: &BTreeMap<String, Value>,
    qualified_key: &'static str,
) -> Result<T, GenerateMessageRejection> {
    let raw = read_string(fields, leaf_of(qualified_key))
        .ok_or(GenerateMessageRejection::MalformedField(qualified_key))?;
    raw.parse::<T>()
        .map_err(|_| GenerateMessageRejection::UnrecognizedValue {
            field: qualified_key,
            value: raw,
        })
}

#[cfg(test)]
mod tests;
