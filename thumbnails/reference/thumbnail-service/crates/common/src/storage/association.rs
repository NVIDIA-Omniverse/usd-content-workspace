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

use crate::notifications::event_publisher::{self, AssociationEventKind, AssociationEventParams};
use crate::outbound::OutboundClients;
use crate::resource::{canonicalize_resource_address, validate_resource, ResourceValidationError};
use crate::spec_enums::{CropStrategy, EncodingFormat, TransformType};
use crate::storage::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;
use thiserror::Error;
use tracing::{error, info, warn, Instrument as _};
use utoipa::ToSchema;

/// The specification for a thumbnail association. Each variant carries exactly the
/// fields required for its type; `"type"` appears as the JSON discriminant.
///
/// `IMAGE_TO_IMAGE` is accepted by both `AssociateThumbnail` and `CreateThumbnails`.
/// `MANUAL` is accepted by `AssociateThumbnail` only — the caller has already produced
/// the thumbnail bytes and is registering the association without conversion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum AssociationSpec {
    /// Thumbnail produced by the image-to-image conversion pipeline.
    ImageToImage {
        #[schema(minimum = 1, maximum = 8192, format = Int32)]
        width: u32,
        #[schema(minimum = 1, maximum = 8192, format = Int32)]
        height: u32,
        encoding_format: EncodingFormat,
        transform_type: TransformType,
        crop_strategy: CropStrategy,
        /// Optional JPEG/WebP encode quality in `[1, 100]`. `None` means the caller
        /// omitted it (distinct from an explicit value); `normalize` fills the
        /// runtime default so stored metadata and events always carry a concrete value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quality: Option<f64>,
        /// Optional preserve-aspect-ratio flag. `None` means the caller omitted it
        /// (distinct from an explicit `false`); `normalize` fills the runtime default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preserve_aspect_ratio: Option<bool>,
    },
    /// Pre-generated thumbnail registered directly by the caller without conversion.
    /// Only `width`, `height`, and `encoding_format` are present; conversion
    /// parameters (transform, crop, quality) are absent because no conversion occurs.
    Manual {
        #[schema(minimum = 1, maximum = 8192, format = Int32)]
        width: u32,
        #[schema(minimum = 1, maximum = 8192, format = Int32)]
        height: u32,
        encoding_format: EncodingFormat,
    },
}

impl AssociationSpec {
    /// Matches OpenAPI `maximum` on width/height and the rest of the thumbnail
    /// HTTP contract. Enforced in `AssociationRequest::validate` so both
    /// transports reject oversized dimensions (schema attrs alone are not).
    pub const MAX_DIMENSION: u32 = 8192;

    /// The `"type"` field value for this variant, matching the JSON discriminant.
    pub fn spec_type(&self) -> &'static str {
        match self {
            Self::ImageToImage { .. } => "IMAGE_TO_IMAGE",
            Self::Manual { .. } => "MANUAL",
        }
    }

    /// Output width, present on every variant.
    pub fn width(&self) -> u32 {
        match self {
            Self::ImageToImage { width, .. } | Self::Manual { width, .. } => *width,
        }
    }

    /// Output height, present on every variant.
    pub fn height(&self) -> u32 {
        match self {
            Self::ImageToImage { height, .. } | Self::Manual { height, .. } => *height,
        }
    }

    /// Output encoding format, present on every variant.
    pub fn encoding_format(&self) -> &EncodingFormat {
        match self {
            Self::ImageToImage {
                encoding_format, ..
            }
            | Self::Manual {
                encoding_format, ..
            } => encoding_format,
        }
    }

    /// Fill omitted optional values from the installed runtime configuration.
    ///
    /// Only applies to `ImageToImage` — `Manual` has no optional fields.
    /// Presence is preserved: an explicit value — including
    /// `preserve_aspect_ratio: false` — is never overwritten.
    pub fn normalize(&mut self) {
        if let Self::ImageToImage {
            quality,
            preserve_aspect_ratio,
            ..
        } = self
        {
            let defaults = &crate::current_runtime_config().thumbnail_defaults;
            quality.get_or_insert(f64::from(defaults.thumbnail_quality));
            preserve_aspect_ratio.get_or_insert(defaults.thumbnail_preserve_aspect_ratio);
        }
        // Manual has no optional fields to normalize.
    }

    fn resolved_quality(&self) -> Result<f64, AssociationInvariant> {
        match self {
            Self::ImageToImage { quality, .. } => {
                quality.ok_or(AssociationInvariant::UnnormalizedQuality)
            }
            Self::Manual { .. } => Err(AssociationInvariant::UnnormalizedQuality),
        }
    }

    fn resolved_preserve_aspect_ratio(&self) -> Result<bool, AssociationInvariant> {
        match self {
            Self::ImageToImage {
                preserve_aspect_ratio,
                ..
            } => preserve_aspect_ratio.ok_or(AssociationInvariant::UnnormalizedPreserveAspectRatio),
            Self::Manual { .. } => Err(AssociationInvariant::UnnormalizedPreserveAspectRatio),
        }
    }
}

#[derive(Debug, PartialEq, Error)]
pub enum AssociationValidationError {
    #[error("invalid thumbnail resource: {0}")]
    InvalidThumbnailResource(ResourceValidationError),
    #[error("invalid association_target resource: {0}")]
    InvalidAssociationTargetResource(ResourceValidationError),
    #[error("specification is required")]
    MissingSpecification,
    #[error(
        "specification width and height must both be between 1 and {}",
        AssociationSpec::MAX_DIMENSION
    )]
    InvalidDimensions,
    #[error("specification quality must be between 1 and 100 for JPEG/WebP")]
    InvalidQuality,
}

/// A guarantee that validation and normalization should already have established.
///
/// Reaching one of these means an output builder was called out of order, not that
/// the caller sent something wrong.
#[derive(Debug, PartialEq, Error)]
pub enum AssociationInvariant {
    #[error("association specification must be normalized before output construction: quality is absent")]
    UnnormalizedQuality,
    #[error("association specification must be normalized before output construction: preserve_aspect_ratio is absent")]
    UnnormalizedPreserveAspectRatio,
    #[error("Failed to serialize specification: {0}")]
    SpecificationSerialization(String),

    /// The canonical message is always a JSON object, so a conversion that produces
    /// anything else means the shape changed without this conversion being updated.
    #[error("event payload converted to a non-struct protobuf value: {0}")]
    EventPayloadNotAStruct(String),
    #[error("Failed converting event payload JSON to protobuf value: {0}")]
    EventPayloadEncoding(String),

    /// `AssociationRequest::validate` guarantees these identities, so reaching an
    /// output builder without one means validation was skipped.
    #[error("unvalidated association request: {resource} carries no resource_identity")]
    MissingResourceIdentity { resource: &'static str },
}

/// Why an association could not be stored, or a disassociation completed.
///
/// Each variant keeps the failure's identity so a transport can choose a status
/// from it instead of parsing a message.
#[derive(Debug, Error)]
pub enum AssociationError {
    #[error(transparent)]
    Validation(#[from] AssociationValidationError),

    /// The downstream rejected the caller's token — most often one that expired between
    /// being issued and being used. This service checks only that a token is present and
    /// never that it is still valid, so a rejection downstream is where expiry surfaces.
    #[error("{0}")]
    DownstreamUnauthenticated(String),

    #[error("{0}")]
    StorageWrite(StorageMetadataError),

    /// The thumbnail-side write failed and rolling back the association-target-side key
    /// failed too, so a read-visible association with no thumbnail metadata behind it is
    /// still on the association target. Names both failures, leading with the original,
    /// so a caller's retry behavior is driven by the same cause it would have been.
    #[error(
        "{write_error} (and the association-target-side key '{key}' on {resource_address} \
         could not be rolled back: {compensation_error})"
    )]
    UncompensatedWrite {
        resource_address: String,
        key: String,
        write_error: StorageMetadataError,
        compensation_error: StorageMetadataError,
    },

    #[error(transparent)]
    Invariant(#[from] AssociationInvariant),
}

/// What a transport should tell the caller about a failed association.
///
/// The remedies differ — get a new token, or try the same request again later — and both
/// transports have to reach the same conclusion from the same evidence. Classifying here
/// keeps that judgment in one place; each transport only translates it into its own
/// status model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssociationFailure {
    /// The caller's credentials were rejected. A new token may work; the same one will not.
    Unauthenticated,
    /// The request was fine; something downstream or internal was not.
    Internal,
}

impl AssociationError {
    /// Whether retrying the identical request could succeed.
    ///
    /// Reported to callers so a worker deciding acknowledgements does not have to
    /// guess from the error prose. Everything that is a property of the request or
    /// of the stored state is permanent; only a storage fault that may clear on
    /// its own is retryable.
    ///
    /// `NotFound` is permanent: the association target the caller named does not
    /// exist, and replaying the same request will keep finding it absent.
    /// `PermissionDenied` is permanent for the same reason a permission decision
    /// is not a transient fault. `Failed` is the catch-all storage error and is
    /// the one genuinely worth retrying.
    pub fn is_retryable(&self) -> bool {
        fn storage_is_retryable(error: &StorageMetadataError) -> bool {
            match error {
                StorageMetadataError::Failed(_) => true,
                StorageMetadataError::NotFound
                | StorageMetadataError::Unauthenticated(_)
                | StorageMetadataError::PermissionDenied(_) => false,
            }
        }

        match self {
            // A malformed or invalid request reproduces exactly on replay.
            Self::Validation(_) | Self::Invariant(_) => false,
            // Surfaced as UNAUTHENTICATED rather than in-band, so this arm is not
            // normally reached; classified permanent because the same token will
            // fail again.
            Self::DownstreamUnauthenticated(_) => false,
            Self::StorageWrite(error) => storage_is_retryable(error),
            // The rollback left a key behind, which is an operator's problem
            // rather than the caller's. Classified by the write that caused it,
            // because that is what determines whether a retry can succeed.
            Self::UncompensatedWrite { write_error, .. } => storage_is_retryable(write_error),
        }
    }

    pub fn failure(&self) -> AssociationFailure {
        match self {
            Self::DownstreamUnauthenticated(_) => AssociationFailure::Unauthenticated,

            // A rejected token fails the write and its rollback alike, so a failed
            // rollback is classified by the write that caused it: the caller's remedy is
            // a new token either way, and the key left behind is an operator's concern,
            // already logged as one.
            Self::UncompensatedWrite {
                write_error: StorageMetadataError::Unauthenticated(_),
                ..
            } => AssociationFailure::Unauthenticated,

            // Validation and invariant failures should be unreachable — both transports
            // validate before calling — but if one arrives it is this service's bug, not
            // a problem with the caller's credentials.
            Self::Validation(_)
            | Self::StorageWrite(_)
            | Self::UncompensatedWrite { .. }
            | Self::Invariant(_) => AssociationFailure::Internal,
        }
    }
}

/// Classify a failed metadata write.
///
/// A rejected token is the caller's to fix and is reported as such; anything else is an
/// upstream fault they can only retry.
fn write_failure(error: StorageMetadataError) -> AssociationError {
    match error {
        StorageMetadataError::Unauthenticated(message) => {
            AssociationError::DownstreamUnauthenticated(message)
        }
        other => AssociationError::StorageWrite(other),
    }
}

fn deterministic_key_suffix(
    association_target: &str,
    thumbnail_resource_identity: &str,
    spec: &AssociationSpec,
) -> Result<String, AssociationInvariant> {
    let mut hasher = Sha256::new();
    hasher.update(association_target.len().to_le_bytes());
    hasher.update(association_target.as_bytes());
    hasher.update(thumbnail_resource_identity.len().to_le_bytes());
    hasher.update(thumbnail_resource_identity.as_bytes());
    hasher.update(spec.width().to_le_bytes());
    hasher.update(spec.height().to_le_bytes());
    hasher.update(spec.encoding_format().to_string().as_bytes());
    // Include the spec type so IMAGE_TO_IMAGE and MANUAL keys never collide.
    hasher.update(spec.spec_type().as_bytes());
    match spec {
        AssociationSpec::ImageToImage {
            transform_type,
            crop_strategy,
            ..
        } => {
            hasher.update(transform_type.to_string().as_bytes());
            hasher.update(crop_strategy.to_string().as_bytes());
            hasher.update(spec.resolved_quality()?.to_le_bytes());
            hasher.update([spec.resolved_preserve_aspect_ratio()? as u8]);
        }
        AssociationSpec::Manual { .. } => {
            // No transform, crop, quality, or preserve_aspect_ratio for Manual.
        }
    }
    let hash = hasher.finalize();

    const SUFFIX_BYTES: usize = 12;
    let mut suffix = String::with_capacity(SUFFIX_BYTES * 2);
    for byte in &hash[..SUFFIX_BYTES] {
        write!(suffix, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(suffix)
}

/// Storage metadata key written on the thumbnail's own resource to record that it is
/// associated. This is the thumbnail-side half of the two-part association write; the
/// association-target-side key is the dynamic `thumbnail_*` key built by
/// [`build_thumbnail_key`].
pub const THUMBNAIL_METADATA_KEY: &str = "thumbnail_metadata";

fn build_thumbnail_key(
    width: u32,
    height: u32,
    encoding_format: &EncodingFormat,
    suffix: &str,
) -> String {
    format!("thumbnail_{width}x{height}_{encoding_format}_{suffix}")
}

/// One deterministic metadata write produced by the canonical association output builder.
#[derive(Debug)]
pub struct AssociationMetadataWrite {
    /// Address-keyed storage URI the metadata is written on (ADR 0002 §4).
    pub resource_address: String,
    pub key: String,
    pub value: serde_json::Value,
}

/// The two metadata writes an association performs, named by the side each lands on.
///
/// Both carry the same value; they differ in where they live and how they are found.
#[derive(Debug)]
pub struct AssociationMetadataWrites {
    /// The deterministic `thumbnail_*` key on the association target. Disassociation
    /// enumerates these keys to rediscover the thumbnail.
    pub association_target_side: AssociationMetadataWrite,
    /// `thumbnail_metadata` on the thumbnail's own resource.
    pub thumbnail_side: AssociationMetadataWrite,
}

/// A storage resource in an associate request (file or folder).
///
/// `resource_address` is always required. `resource_identity` is present for files
/// and omitted for folders on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct AssociationResource {
    #[schema(max_length = 2048)]
    pub resource_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(max_length = 2048)]
    pub resource_identity: Option<String>,
}

/// Whether metadata is stored on a source **file** identity or a **folder** URI (manual association).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AssociationTargetKind {
    #[default]
    File,
    Folder,
}

impl AssociationTargetKind {
    /// The wire string used for `target_type` in stored metadata and events.
    pub fn as_str(self) -> &'static str {
        match self {
            AssociationTargetKind::File => "FILE",
            AssociationTargetKind::Folder => "FOLDER",
        }
    }

    /// Parse a stored/wire `target_type` string back into a kind.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "FILE" => Some(AssociationTargetKind::File),
            "FOLDER" => Some(AssociationTargetKind::Folder),
            _ => None,
        }
    }
}

impl TryFrom<i32> for AssociationTargetKind {
    type Error = String;

    fn try_from(target_type: i32) -> Result<Self, Self::Error> {
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::TargetType;

        match TargetType::try_from(target_type) {
            Ok(TargetType::Unspecified) | Ok(TargetType::File) => Ok(Self::File),
            Ok(TargetType::Folder) => Ok(Self::Folder),
            Err(_) => Err(format!("Unknown target_type enum value: {target_type}")),
        }
    }
}

fn required_association_spec_schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
    utoipa::openapi::schema::AllOfBuilder::new()
        .item(utoipa::openapi::Ref::from_schema_name("AssociationSpec"))
        .into()
}

/// Bundled parameters for associate calls, produced by each transport after
/// converting its protocol-specific request body.
///
/// Auth is intentionally not part of this type — it is transport metadata and is
/// passed separately to `store_association`. Transports should call `validate`
/// early so they can map `AssociationValidationError` to protocol-specific
/// status codes. `store_association` also validates before writing.
///
/// JSON wire shape matches the established HTTP contract: nested `thumbnail` and
/// `association_target` resources, `target_type`, and a flat `specification` object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct AssociationRequest {
    #[serde(default, rename = "target_type")]
    #[schema(default = "FILE")]
    pub target_kind: AssociationTargetKind,
    pub thumbnail: AssociationResource,
    pub association_target: AssociationResource,
    /// `None` when the caller omitted the spec; `validate` rejects this as
    /// `MissingSpecification`.
    #[serde(default)]
    #[schema(required = true, schema_with = required_association_spec_schema)]
    pub specification: Option<AssociationSpec>,
}

impl AssociationRequest {
    /// Copy the request with both address-keyed resources serialized as valid URLs.
    /// Resource identities remain opaque and are never used as metadata keys.
    fn with_canonical_resource_addresses(&self) -> Self {
        let mut normalized = self.clone();
        normalized.thumbnail.resource_address =
            canonicalize_resource_address(&normalized.thumbnail.resource_address);
        normalized.association_target.resource_address =
            canonicalize_resource_address(&normalized.association_target.resource_address);
        normalized
    }

    /// Validate resources, specification presence, dimensions, and quality.
    pub fn validate(&self) -> Result<(), AssociationValidationError> {
        validate_resource(
            &self.thumbnail.resource_address,
            self.thumbnail.resource_identity.as_deref(),
            true,
        )
        .map_err(AssociationValidationError::InvalidThumbnailResource)?;

        validate_resource(
            &self.association_target.resource_address,
            self.association_target.resource_identity.as_deref(),
            matches!(self.target_kind, AssociationTargetKind::File),
        )
        .map_err(AssociationValidationError::InvalidAssociationTargetResource)?;

        let spec = self
            .specification
            .as_ref()
            .ok_or(AssociationValidationError::MissingSpecification)?;

        let dim_range = 1..=AssociationSpec::MAX_DIMENSION;
        if !dim_range.contains(&spec.width()) || !dim_range.contains(&spec.height()) {
            return Err(AssociationValidationError::InvalidDimensions);
        }

        // Quality range validation only applies to IMAGE_TO_IMAGE (Manual has no quality).
        if let AssociationSpec::ImageToImage {
            quality: Some(quality),
            encoding_format,
            ..
        } = spec
        {
            let violates_lossy_range =
                matches!(encoding_format, EncodingFormat::Jpeg | EncodingFormat::Webp)
                    && !(1.0..=100.0).contains(quality);
            if violates_lossy_range {
                return Err(AssociationValidationError::InvalidQuality);
            }
        }

        Ok(())
    }

    /// Fill omitted optional specification values (`quality`,
    /// `preserve_aspect_ratio`) from the installed runtime configuration so both
    /// transports produce identical deterministic keys, stored metadata, and
    /// events regardless of which optional fields the caller supplied. Presence
    /// is preserved: an explicit value — including `preserve_aspect_ratio: false`
    /// — is never overwritten. This is the single normalization seam; transports
    /// no longer inject their own defaults.
    pub fn normalize_specification(&mut self) {
        if let Some(spec) = self.specification.as_mut() {
            spec.normalize();
        }
    }

    /// The thumbnail's identity.
    ///
    /// Always present after [`Self::validate`], which validates the thumbnail as a
    /// file and so requires one; its absence here means the request reached an output
    /// builder without being validated.
    fn thumbnail_identity(&self) -> Result<&str, AssociationInvariant> {
        self.thumbnail.resource_identity.as_deref().ok_or(
            AssociationInvariant::MissingResourceIdentity {
                resource: "thumbnail",
            },
        )
    }

    /// The association target's identity, empty for a FOLDER target.
    ///
    /// [`Self::validate`] requires an identity for a FILE target and rejects one for a
    /// FOLDER target, so a FILE target without an identity means the request reached
    /// an output builder unvalidated.
    fn association_target_identity(&self) -> Result<&str, AssociationInvariant> {
        match self.association_target.resource_identity.as_deref() {
            Some(identity) => Ok(identity),
            None if matches!(self.target_kind, AssociationTargetKind::Folder) => Ok(""),
            None => Err(AssociationInvariant::MissingResourceIdentity {
                resource: "association_target",
            }),
        }
    }

    /// Resolve the validated identities into the parameter set that the stored
    /// metadata and the published event are both built from.
    ///
    /// This is where the two identity invariants are discharged, so callers that hold
    /// the result can read identities without re-checking them.
    pub fn message_params<'a>(
        &'a self,
        specification: Option<&'a AssociationSpec>,
    ) -> Result<AssociationMessageParams<'a>, AssociationInvariant> {
        // Validate identity invariants before constructing params; callers that hold
        // the result can read identities without re-checking them.
        self.thumbnail_identity()?;
        self.association_target_identity()?;
        Ok(AssociationMessageParams {
            thumbnail: self.thumbnail.clone(),
            association_target: self.association_target.clone(),
            target_kind: self.target_kind,
            specification,
        })
    }
}

/// The one association a message describes: which thumbnail, which target, and the
/// specification it was produced under.
///
/// Using [`AssociationResource`] for each participant groups address and identity
/// together, making it impossible to accidentally transpose one resource's identity
/// with another's address.
pub struct AssociationMessageParams<'a> {
    pub thumbnail: AssociationResource,
    pub association_target: AssociationResource,
    pub target_kind: AssociationTargetKind,
    /// Absent on a disassociation, where no specification is being asserted.
    pub specification: Option<&'a AssociationSpec>,
}

/// Build the canonical nested association message (ADR 0002 §2/§3) shared by
/// stored metadata and emitted `omni.thumbnail.*` events. `thumbnail` and
/// `association_target` are `{resource_address, resource_identity?}` objects;
/// FOLDER targets omit `association_target.resource_identity` entirely;
/// `specification` is included as `{ "image_to_image": <spec> }` when provided.
/// This is the single source of truth for the shape so stored metadata and events
/// cannot drift.
pub fn build_association_message(
    params: &AssociationMessageParams<'_>,
) -> Result<serde_json::Value, AssociationInvariant> {
    let mut association_target_obj = serde_json::Map::new();
    association_target_obj.insert(
        "resource_address".to_string(),
        serde_json::Value::String(params.association_target.resource_address.clone()),
    );
    // Folders have no identity — omit it entirely (not null/empty). The caller
    // signals this by leaving association_target.resource_identity as None.
    if let Some(identity) = &params.association_target.resource_identity {
        association_target_obj.insert(
            "resource_identity".to_string(),
            serde_json::Value::String(identity.clone()),
        );
    }

    let mut message = serde_json::json!({
        "thumbnail": {
            "resource_identity": params.thumbnail.resource_identity.as_deref().unwrap_or(""),
            "resource_address": params.thumbnail.resource_address,
        },
        "association_target": serde_json::Value::Object(association_target_obj),
        "target_type": params.target_kind.as_str(),
    });

    if let Some(spec) = params.specification {
        // Guard normalization invariant for ImageToImage (Manual has no optional fields).
        if matches!(spec, AssociationSpec::ImageToImage { .. }) {
            spec.resolved_quality()?;
            spec.resolved_preserve_aspect_ratio()?;
        }
        let specification = serde_json::to_value(spec)
            .map_err(|error| AssociationInvariant::SpecificationSerialization(error.to_string()))?;
        message["specification"] = specification;
    }

    Ok(message)
}

/// Build the storage write plan for an association from an already validated and
/// normalized request.
///
/// The stored value is the unified nested shape (ADR 0002 §2): `thumbnail` and
/// `association_target` are objects carrying `resource_address` (always) and
/// `resource_identity` (files only). For a FOLDER target,
/// `association_target.resource_identity` is omitted entirely. Both writes are keyed
/// on the resource address (ADR 0002 §4).
///
/// This remains pure so transport contract tests can compare the exact deterministic
/// key and stored message produced after their real HTTP/protobuf mappings.
///
/// `create_time` is passed in rather than read from the clock so the whole plan is a
/// function of its arguments; the one production caller supplies
/// [`crate::current_timestamp_rfc3339`].
pub fn build_association_metadata_writes(
    req: &AssociationRequest,
    spec: &AssociationSpec,
    create_time: &str,
) -> Result<AssociationMetadataWrites, AssociationInvariant> {
    let params = req.message_params(Some(spec))?;

    let mut metadata_value = build_association_message(&params)?;
    metadata_value["create_time"] = serde_json::Value::String(create_time.to_string());

    // Key derivation (storage layout) is unchanged — see ADR 0001, out of scope here.
    let key_suffix = deterministic_key_suffix(
        &req.association_target.resource_address,
        params.thumbnail.resource_identity.as_deref().unwrap_or(""),
        spec,
    )?;
    let thumbnail_key = build_thumbnail_key(
        spec.width(),
        spec.height(),
        spec.encoding_format(),
        &key_suffix,
    );

    Ok(AssociationMetadataWrites {
        association_target_side: AssociationMetadataWrite {
            resource_address: req.association_target.resource_address.clone(),
            key: thumbnail_key,
            value: metadata_value.clone(),
        },
        thumbnail_side: AssociationMetadataWrite {
            resource_address: req.thumbnail.resource_address.clone(),
            key: THUMBNAIL_METADATA_KEY.to_string(),
            value: metadata_value,
        },
    })
}

/// Undo the association-target-side write after the thumbnail-side write failed.
///
/// `GET /thumbnails` resolves association state from the association-target-side key
/// alone, so leaving that key behind would report an association whose thumbnail
/// metadata was never written — a failure the caller was told about but readers were
/// not. Removing it is what makes a failed association leave nothing behind.
///
/// Returns the error to report: the original write failure, or one naming both when the
/// rollback also failed and the leftover key became an operator's problem.
async fn roll_back_association_target_write(
    association_target_side: &AssociationMetadataWrite,
    write_error: StorageMetadataError,
    auth_token: &str,
) -> AssociationError {
    match remove_storage_metadata(
        &association_target_side.resource_address,
        &association_target_side.key,
        auth_token,
    )
    .await
    {
        // An already-absent key is the outcome the rollback wanted, not a failure.
        Ok(()) | Err(StorageMetadataError::NotFound) => {
            warn!(
                resource_address = %association_target_side.resource_address,
                key = %association_target_side.key,
                %write_error,
                "Thumbnail-side metadata write failed; rolled back the association-target-side key"
            );
            write_failure(write_error)
        }
        Err(compensation_error) => {
            error!(
                resource_address = %association_target_side.resource_address,
                key = %association_target_side.key,
                %write_error,
                %compensation_error,
                "Thumbnail-side metadata write failed and its rollback failed too; this key \
                 still reports an association with no thumbnail metadata and must be removed \
                 by hand"
            );
            AssociationError::UncompensatedWrite {
                resource_address: association_target_side.resource_address.clone(),
                key: association_target_side.key.clone(),
                write_error,
                compensation_error,
            }
        }
    }
}

pub async fn store_association(
    req: &AssociationRequest,
    auth_token: &str,
    outbound: Arc<OutboundClients>,
) -> Result<(), AssociationError> {
    // Storage's MetadataService parses these strings as RFC URIs and rejects raw
    // path spaces/Unicode even though upstream event and HTTP layers accept them.
    // Normalize once at the shared store boundary so gRPC, HTTP, and retried work
    // messages all write, persist, and emit the same canonical resource addresses.
    let normalized_req = req.with_canonical_resource_addresses();
    let req = &normalized_req;

    // Both transports validate before calling, so this is defense in depth — but it is
    // a pure check on a public function, and it is what makes the identity and
    // specification accesses below invariants rather than guesses. Keeping it means a
    // future caller cannot reach the write path unvalidated.
    req.validate()?;

    // Normalize once here — the single shared write path — so omitted optional spec
    // values are resolved to runtime defaults before any deterministic key, stored
    // metadata, or event payload is derived. Both transports converge on this identical
    // normalized specification. Normalization only touches the specification, so only
    // the specification is copied; everything else is read off the caller's request.
    let mut spec = req
        .specification
        .clone()
        .ok_or(AssociationValidationError::MissingSpecification)?;
    spec.normalize();

    // Discharges both identity invariants once; the logs, the write plan and the event
    // below all read their identities from here rather than re-resolving them.
    let params = req.message_params(Some(&spec))?;
    let thumb_identity = params
        .thumbnail
        .resource_identity
        .as_deref()
        .unwrap_or_default();
    let target_identity = params
        .association_target
        .resource_identity
        .as_deref()
        .unwrap_or_default();

    info!(
        "[CONNECT] Associating thumbnail {} with {:?} target {}",
        thumb_identity, req.target_kind, target_identity
    );

    let metadata_writes =
        build_association_metadata_writes(req, &spec, &crate::current_timestamp_rfc3339())?;

    info!(
        "Writing association-target-side thumbnail metadata on resource: {}",
        req.association_target.resource_address
    );
    let association_target_side = &metadata_writes.association_target_side;
    update_storage_metadata(
        &association_target_side.resource_address,
        &association_target_side.key,
        &association_target_side.value,
        auth_token,
    )
    .await
    .map_err(write_failure)?;

    info!(
        "Writing thumbnail_metadata on thumbnail resource: {}",
        req.thumbnail.resource_address
    );
    let thumbnail_side = &metadata_writes.thumbnail_side;
    if let Err(write_error) = update_storage_metadata(
        &thumbnail_side.resource_address,
        &thumbnail_side.key,
        &thumbnail_side.value,
        auth_token,
    )
    .await
    {
        return Err(roll_back_association_target_write(
            association_target_side,
            write_error,
            auth_token,
        )
        .await);
    }

    info!(
        "[OK] Successfully associated thumbnail {} with {}",
        thumb_identity, target_identity
    );

    let thumbnail = AssociationResource {
        resource_address: req.thumbnail.resource_address.clone(),
        resource_identity: Some(thumb_identity.to_string()),
    };
    let association_target = AssociationResource {
        resource_address: req.association_target.resource_address.clone(),
        resource_identity: Some(target_identity.to_string()),
    };
    let target_kind = req.target_kind;
    // Emit the same normalized spec that was stored, so the event payload and the
    // stored metadata cannot drift.
    let event_specification = spec;
    spawn_association_publish(
        AssociationEventKind::Associated,
        thumbnail,
        association_target,
        target_kind,
        Some(event_specification),
        outbound,
    );

    Ok(())
}

/// Publish an association lifecycle event off the caller's critical path, in the
/// caller's trace but not under the caller's span.
///
/// The distinction is the whole point of this function existing. `tokio::spawn` does
/// not carry the tracing context across the task boundary, and the obvious repair —
/// `.in_current_span()` — makes the detached task a *descendant* of the request span.
/// `Instrumented::poll` enters and exits the span on every poll and
/// `tracing-opentelemetry` writes `end_time` on every exit, so the last exit wins: the
/// request's exported span would end when this publish's final poll ended, reporting a
/// 20 ms associate as however long notifications took to accept the event.
///
/// Capturing the W3C context and making it the *remote* parent of a span this task owns
/// gets the linkage without the descendancy. The parentage is real — same trace, this
/// publish named as a child of the request span — but it is expressed in the
/// OpenTelemetry context rather than in the `tracing` registry, so the request span
/// neither is re-entered nor is held open waiting for a child to close.
fn spawn_association_publish(
    event_kind: AssociationEventKind,
    thumbnail: AssociationResource,
    association_target: AssociationResource,
    target_kind: AssociationTargetKind,
    specification: Option<AssociationSpec>,
    outbound: Arc<OutboundClients>,
) {
    let captured = crate::work_events::CapturedTraceContext::capture();
    // Read here, not in the task. `current_runtime_config()` panics when the config
    // is not installed, and this task's `JoinHandle` is dropped — a panic inside it
    // reaches stderr and nothing else, losing the event with no failure anyone can
    // observe. On this side of the spawn the same panic is on the request path.
    let request_timeout = crate::current_runtime_config()
        .service
        .outbound_request_timeout;

    tokio::spawn(async move {
        let span = tracing::info_span!(
            "thumbnail.association_event",
            event_type = event_kind.as_str()
        );
        crate::work_events::set_remote_parent(
            &span,
            captured.traceparent.as_deref(),
            captured.tracestate.as_deref(),
        );

        async move {
            let params = AssociationEventParams {
                event_kind,
                message: AssociationMessageParams {
                    thumbnail,
                    association_target,
                    target_kind,
                    specification: specification.as_ref(),
                },
            };
            event_publisher::publish_event(outbound, &params, request_timeout).await;
        }
        .instrument(span)
        .await;
    });
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum DisassociateValidationError {
    #[error("invalid association_target_resource_address: {0}")]
    InvalidAssociationTargetResource(ResourceValidationError),
    #[error("invalid thumbnail_resource_address: {0}")]
    InvalidThumbnailResource(ResourceValidationError),
}

/// Bundled parameters for a targeted disassociate, produced by each transport after
/// converting its protocol-specific request.
///
/// Removes the single thumbnail recorded on the association target whose
/// `thumbnail.resource_address` matches `thumbnail_resource_address`. Resource identities
/// are optional and do not participate in matching; only addresses locate and remove entries.
///
/// Auth is intentionally not part of this type, matching `AssociationRequest` — it is
/// transport metadata and is passed separately to `disassociate_thumbnail`.
///
/// The wire shape is the established HTTP query contract, which this type is
/// deserialized from directly rather than through a transport-local copy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DisassociateRequest {
    /// Type of association target: FILE or FOLDER. Optional; defaults to FILE.
    #[serde(default, rename = "target_type")]
    #[param(rename = "target_type", default = "FILE")]
    #[schema(default = "FILE")]
    pub target_kind: AssociationTargetKind,
    /// Address of the association target from which to remove the thumbnail.
    #[param(
        example = "https://example.blob.core.windows.net/container/sample-image.jpg",
        max_length = 2048
    )]
    #[schema(
        example = "https://example.blob.core.windows.net/container/sample-image.jpg",
        max_length = 2048
    )]
    pub association_target_resource_address: String,
    /// Optional opaque identity of the association target. It does not participate in matching;
    /// only the address is used to locate the entry.
    #[serde(default)]
    #[param(max_length = 2048)]
    #[schema(max_length = 2048)]
    pub association_target_resource_identity: Option<String>,
    /// Address of the specific thumbnail to remove.
    #[param(
        example = "https://example.blob.core.windows.net/container/.thumbnails/sample-image.jpg.png",
        max_length = 2048
    )]
    #[schema(
        example = "https://example.blob.core.windows.net/container/.thumbnails/sample-image.jpg.png",
        max_length = 2048
    )]
    pub thumbnail_resource_address: String,
    /// Optional opaque identity of the thumbnail resource. It does not participate in matching;
    /// only the address is used to locate the entry.
    #[serde(default)]
    #[param(max_length = 2048)]
    #[schema(max_length = 2048)]
    pub thumbnail_resource_identity: Option<String>,
}

impl DisassociateRequest {
    fn with_canonical_resource_addresses(&self) -> Self {
        let mut normalized = self.clone();
        normalized.association_target_resource_address =
            canonicalize_resource_address(&normalized.association_target_resource_address);
        normalized.thumbnail_resource_address =
            canonicalize_resource_address(&normalized.thumbnail_resource_address);
        normalized
    }

    /// Validate both resource addresses through the same resource validator associate uses.
    pub fn validate(&self) -> Result<(), DisassociateValidationError> {
        validate_resource(&self.association_target_resource_address, None, false)
            .map_err(DisassociateValidationError::InvalidAssociationTargetResource)?;
        validate_resource(&self.thumbnail_resource_address, None, false)
            .map_err(DisassociateValidationError::InvalidThumbnailResource)
    }
}

/// A thumbnail resource touched by a disassociation operation.
#[derive(Debug, Clone)]
pub struct DisassociatedThumbnail {
    pub resource_address: String,
    pub resource_identity: Option<String>,
}

/// Why one thumbnail could not be disassociated.
///
/// Transports format these for callers; the shared operation stores no prose.
#[derive(Debug, Error)]
pub enum DisassociateEntryError {
    #[error(
        "stored entry '{key}' has no thumbnail.resource_address; thumbnail_metadata not removed"
    )]
    MalformedEntry { key: String },

    #[error("failed to remove thumbnail metadata: {source}")]
    ThumbnailSideRemoval { source: StorageMetadataError },

    #[error("failed to remove source key '{key}': {source}")]
    AssociationTargetSideRemoval {
        key: String,
        source: StorageMetadataError,
    },
}

impl DisassociateEntryError {
    /// Whether this failure is worth retrying by redelivering the event.
    ///
    /// `MalformedEntry` is permanent: the stored data is corrupt and will keep
    /// failing regardless of how many times the event is replayed. Storage-side
    /// removal failures (`ThumbnailSideRemoval`, `AssociationTargetSideRemoval`)
    /// are usually transient storage errors that may succeed on the next attempt
    /// — except a "duplicate batch entry" rejection, which is a structural
    /// property of the stored data (the same address was named twice in one
    /// batch) and will fail identically on every replay. Treating that one as
    /// retryable would NACK the event forever with no path to success; see
    /// [`is_permanent_duplicate_batch_entry`].
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::MalformedEntry { .. } => false,
            Self::ThumbnailSideRemoval { source }
            | Self::AssociationTargetSideRemoval { source, .. } => {
                !is_permanent_duplicate_batch_entry(source)
            }
        }
    }
}

/// Whether a storage-side batch-delete failure is the deterministic "duplicate
/// batch entry" rejection rather than a genuine transient fault.
///
/// Storage does not give this its own typed [`StorageMetadataError`] variant —
/// it comes back as `Failed` with the phrase in the message — so this is a
/// best-effort text match, not a typed classification. `disassociate_all_thumbnails`
/// dedupes phase-1 requests before sending them, which is what stops this
/// condition from occurring in the first place; this exists as the safety net
/// that keeps a recurrence from being misclassified as retryable and NACKed
/// forever.
fn is_permanent_duplicate_batch_entry(source: &StorageMetadataError) -> bool {
    matches!(
        source,
        StorageMetadataError::Failed(message) if message.to_lowercase().contains("duplicate")
    )
}

/// A thumbnail that could not be disassociated, and why.
#[derive(Debug)]
pub struct DisassociateEntryFailure {
    pub thumbnail: DisassociatedThumbnail,
    pub error: DisassociateEntryError,
}

/// Why a disassociation operation stopped before inspecting association metadata.
///
/// An aborted operation and an unmatched association target both yield empty result lists, so
/// this is what tells them apart — and which abort cause it was, since a request the
/// caller got wrong and an upstream that would not answer warrant different statuses.
#[derive(Debug, Error)]
pub enum DisassociateAbort {
    #[error(transparent)]
    Validation(#[from] DisassociateValidationError),

    #[error("could not enumerate the association target's metadata: {0}")]
    Enumeration(StorageMetadataError),
}

impl DisassociateAbort {
    /// Whether the downstream rejected the credentials the operation forwarded to it.
    ///
    /// That means the caller needs a new token, which is not something an upstream-gateway
    /// status would tell them. A caller who sent no token at all never reaches this operation:
    /// both transports answer that themselves.
    pub fn is_unauthenticated(&self) -> bool {
        match self {
            Self::Enumeration(error) => {
                matches!(error, StorageMetadataError::Unauthenticated(_))
            }
            Self::Validation(_) => false,
        }
    }
}

/// Result of a disassociation operation. Each affected thumbnail lands in
/// `succeeded` or in `failed` with its error.
#[derive(Debug, Default)]
pub struct DisassociateOutcome {
    pub succeeded: Vec<DisassociatedThumbnail>,
    pub failed: Vec<DisassociateEntryFailure>,
    /// Set when the operation could not inspect association metadata, leaving both
    /// lists empty for a reason other than an unmatched or empty association target.
    pub aborted: Option<DisassociateAbort>,
}

/// One entry's outcome: the thumbnail it disassociated, or the one it could not.
type SweepResult = Result<DisassociatedThumbnail, DisassociateEntryFailure>;

impl Extend<SweepResult> for DisassociateOutcome {
    fn extend<I: IntoIterator<Item = SweepResult>>(&mut self, results: I) {
        for result in results {
            match result {
                Ok(thumbnail) => self.succeeded.push(thumbnail),
                Err(failure) => self.failed.push(failure),
            }
        }
    }
}

impl FromIterator<SweepResult> for DisassociateOutcome {
    fn from_iter<I: IntoIterator<Item = SweepResult>>(results: I) -> Self {
        let mut outcome = Self::default();
        outcome.extend(results);
        outcome
    }
}

/// The fields a disassociation operation needs from one stored `thumbnail_*` entry.
struct StoredAssociationEntry {
    /// `None` when the stored entry carries no `thumbnail.resource_address`. That
    /// address is the only way to locate the `thumbnail_metadata` blob, so an entry
    /// without one is malformed rather than already clean.
    thumbnail_address: Option<String>,
    thumbnail_identity: Option<String>,
    association_target_identity: String,
    target_kind: AssociationTargetKind,
    /// The stored `specification.type` discriminant, when the entry carries one.
    ///
    /// Read solely to tell a generated thumbnail from one a caller registered:
    /// see [`StoredAssociationEntry::blob_is_service_generated`]. The write path
    /// has always stored this; nothing used to read it back.
    specification_type: Option<String>,
}

impl StoredAssociationEntry {
    fn read(value: &serde_json::Value, fallback_target_kind: AssociationTargetKind) -> Self {
        let thumbnail = value.get("thumbnail");
        Self {
            thumbnail_address: thumbnail
                .and_then(|t| t.get("resource_address"))
                .and_then(|v| v.as_str())
                .filter(|address| !address.is_empty())
                .map(|address| address.to_string()),
            thumbnail_identity: thumbnail
                .and_then(|t| t.get("resource_identity"))
                .and_then(|v| v.as_str())
                .map(|identity| identity.to_string()),
            association_target_identity: value
                .get("association_target")
                .and_then(|t| t.get("resource_identity"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            // Prefer the entry's own stored target_type — it is what the association
            // actually recorded — and fall back to the operation's kind when the entry has
            // none or stored something unreadable.
            target_kind: value
                .get("target_type")
                .and_then(|v| v.as_str())
                .and_then(AssociationTargetKind::from_wire)
                .unwrap_or(fallback_target_kind),
            specification_type: value
                .get("specification")
                .and_then(|s| s.get("type"))
                .and_then(|v| v.as_str())
                .map(|kind| kind.to_string()),
        }
    }

    /// Whether this entry's blob is one this service wrote, and therefore one the
    /// service identity may delete.
    ///
    /// Deletion runs under the service account, which holds rights the caller may
    /// not. Without this test, a caller who can write association metadata naming
    /// an address they do not own can have the service delete that address for
    /// them: registering it as the "thumbnail" of a resource they do own, then
    /// deleting that resource, is enough. That escalates metadata-write into
    /// delete.
    ///
    /// Two signals, joined by OR rather than AND:
    ///
    /// - The stored specification says `IMAGE_TO_IMAGE`, i.e. this association
    ///   records a conversion this service performed. `MANUAL` is exactly the
    ///   case to exclude — the caller supplied pre-existing bytes.
    /// - The address is one the configured path template could have rendered,
    ///   which is the same predicate the CREATE path uses as its recursion guard.
    ///
    /// OR, because AND would orphan every blob generated under a previous
    /// `THUMBNAIL_PATH_TEMPLATE`: after a template change their addresses stop
    /// matching, and a template change would then silently convert into an
    /// unbounded orphan population. An entry that records a conversion is
    /// trustworthy regardless of which template rendered its address.
    ///
    /// An entry matching neither is left in storage. That is the safe direction:
    /// an orphaned blob costs storage, where a wrong deletion costs a caller's
    /// data.
    fn blob_is_service_generated(&self, address: &str) -> bool {
        if self.specification_type.as_deref() == Some("IMAGE_TO_IMAGE") {
            return true;
        }

        crate::current_runtime_config()
            .thumbnail_path
            .thumbnail_path_template
            .matches_rendered_output(address)
    }

    /// What the operation reports for this entry either way. A malformed entry is still
    /// named in `failed`, with the empty address it was stored with.
    fn reported_thumbnail(&self) -> DisassociatedThumbnail {
        DisassociatedThumbnail {
            resource_address: self.thumbnail_address.clone().unwrap_or_default(),
            resource_identity: self.thumbnail_identity.clone(),
        }
    }
}

/// Announce one successful removal off the disassociation critical path.
fn publish_disassociated_event(
    entry: &StoredAssociationEntry,
    thumbnail: &DisassociatedThumbnail,
    association_target_resource_address: &str,
    outbound: Arc<OutboundClients>,
) {
    let thumbnail_resource = AssociationResource {
        resource_address: thumbnail.resource_address.clone(),
        resource_identity: thumbnail.resource_identity.clone(),
    };
    let association_target_id = entry.association_target_identity.clone();
    let association_target_resource = AssociationResource {
        resource_address: association_target_resource_address.to_string(),
        // Folder entries store an empty string for the identity; map that to None
        // so the event message omits the field entirely (ADR 0002 §3).
        resource_identity: if association_target_id.is_empty() {
            None
        } else {
            Some(association_target_id)
        },
    };
    let target_type = entry.target_kind;

    // One of these is spawned per thumbnail on the DELETE sweep, which is why the
    // parentage question matters more here than at the single-associate site: under
    // `.in_current_span()` the `storage.event` span's duration became the slowest of N.
    spawn_association_publish(
        AssociationEventKind::Disassociated,
        thumbnail_resource,
        association_target_resource,
        target_type,
        None,
        outbound,
    );
}

/// Remove a single specific thumbnail from its association with a resource address.
///
/// Enumerates `thumbnail_*` keys on the association target, finds the entry whose
/// `thumbnail.resource_address` matches `req.thumbnail_resource_address`, and removes
/// only the source-side `thumbnail_*` key. The thumbnail resource and its
/// `thumbnail_metadata` are left intact — callers are responsible for deleting the
/// thumbnail itself. If no matching entry is found the call succeeds with empty arrays
/// (idempotent). A `NOT_FOUND` on the source-side removal is treated as already-removed
/// and counted as success, making retries safe.
///
/// One `omni.thumbnail.disassociated` event is emitted on a full success.
pub async fn disassociate_thumbnail(
    req: &DisassociateRequest,
    auth_token: &str,
    outbound: Arc<OutboundClients>,
) -> DisassociateOutcome {
    let normalized_req = req.with_canonical_resource_addresses();
    let req = &normalized_req;
    let association_target_resource_address = req.association_target_resource_address.as_str();
    let target_kind = req.target_kind;

    info!(
        "[CONNECT] Disassociate {} from {} ({:?})",
        req.thumbnail_resource_address, association_target_resource_address, target_kind
    );

    let aborted = |cause: DisassociateAbort| {
        warn!(
            "Disassociate could not start on {}: {}",
            association_target_resource_address, cause
        );
        DisassociateOutcome {
            aborted: Some(cause),
            ..Default::default()
        }
    };

    // Both transports validate before calling. Repeating it here keeps this function
    // safe to call from anywhere without relying on the caller to have remembered.
    if let Err(e) = req.validate() {
        return aborted(DisassociateAbort::Validation(e));
    }

    let raw_entries =
        match get_all_storage_metadata(association_target_resource_address, auth_token).await {
            Ok(entries) => entries,
            Err(e) => return aborted(DisassociateAbort::Enumeration(e)),
        };

    // Find the single entry whose stored thumbnail.resource_address matches the request.
    // Entries that have no thumbnail address are skipped (they cannot match), not reported
    // as failures: only the targeted thumbnail matters.
    let matching = raw_entries
        .into_iter()
        .filter(|(k, _)| k.starts_with("thumbnail_"))
        .find_map(|(key, value)| {
            let entry = StoredAssociationEntry::read(&value, target_kind);
            if entry.thumbnail_address.as_deref() == Some(req.thumbnail_resource_address.as_str()) {
                Some((key, entry))
            } else {
                None
            }
        });

    let (source_key, entry) = match matching {
        None => {
            // Already removed or never associated — idempotent success.
            info!(
                "[OK] Disassociate {}: not found on {}, nothing to remove",
                req.thumbnail_resource_address, association_target_resource_address
            );
            return DisassociateOutcome::default();
        }
        Some(found) => found,
    };

    let thumbnail = entry.reported_thumbnail();

    // Remove the source-side key. The thumbnail resource and its thumbnail_metadata are
    // left intact; callers manage the thumbnail's lifecycle. A NOT_FOUND counts as
    // success so a retry after a partial failure completes cleanly.
    match remove_storage_metadata(association_target_resource_address, &source_key, auth_token)
        .await
    {
        Ok(()) | Err(StorageMetadataError::NotFound) => {}
        Err(e) => {
            warn!(
                "Disassociate: source-side removal failed for key '{source_key}' on {}: {}",
                association_target_resource_address, e
            );
            return DisassociateOutcome {
                failed: vec![DisassociateEntryFailure {
                    thumbnail,
                    error: DisassociateEntryError::AssociationTargetSideRemoval {
                        key: source_key,
                        source: e,
                    },
                }],
                ..Default::default()
            };
        }
    }

    publish_disassociated_event(
        &entry,
        &thumbnail,
        association_target_resource_address,
        outbound.clone(),
    );

    info!(
        "[OK] Disassociate {} from {} complete",
        thumbnail.resource_address, association_target_resource_address
    );

    DisassociateOutcome {
        succeeded: vec![thumbnail],
        ..Default::default()
    }
}

/// Remove ALL thumbnails associated with a resource address (sweep semantics).
///
/// Used by the `omni.storage.deleted` event handler to clean up every thumbnail
/// linked to a source file when that file is deleted. For API-driven removal of a
/// single specific thumbnail, use `disassociate_thumbnail`.
///
/// Collects every `thumbnail_*` key on the source and issues two ordered
/// `DeleteMetadataBatch` RPCs: first the thumbnail-side `thumbnail_metadata` keys,
/// then the source-side keys only for entries that completed phase 1. Blob deletion
/// via the task queue follows after both metadata phases succeed. One
/// `omni.thumbnail.disassociated` event is emitted per successful removal.
///
/// Best-effort per thumbnail: one entry's failure neither stops the sweep nor hides
/// the entries that did succeed. The source-side key remains discoverable whenever
/// thumbnail-side removal fails or is unresolved. `NOT_FOUND` entries are treated as
/// already-removed successes, so a later sweep can safely retry either phase.
pub async fn disassociate_all_thumbnails(
    association_target_resource_address: &str,
    target_kind: AssociationTargetKind,
    auth_token: &str,
    outbound: Arc<OutboundClients>,
) -> DisassociateOutcome {
    let association_target_resource_address =
        canonicalize_resource_address(association_target_resource_address);
    let association_target_resource_address = association_target_resource_address.as_str();

    info!(
        "[CONNECT] Disassociate sweep on {} ({:?})",
        association_target_resource_address, target_kind
    );

    let aborted = |cause: DisassociateAbort| {
        warn!(
            "Disassociate sweep could not start on {}: {}",
            association_target_resource_address, cause
        );
        DisassociateOutcome {
            aborted: Some(cause),
            ..Default::default()
        }
    };

    let raw_entries =
        match get_all_storage_metadata(association_target_resource_address, auth_token).await {
            Ok(entries) => entries,
            Err(e) => return aborted(DisassociateAbort::Enumeration(e)),
        };

    // Parse thumbnail_* entries; separate valid (have an address) from malformed.
    // valid: (source_key, thumbnail_address, stored_entry)
    let mut valid: Vec<(String, String, StoredAssociationEntry)> = Vec::new();
    let mut outcome = DisassociateOutcome::default();

    for (key, value) in raw_entries
        .into_iter()
        .filter(|(k, _)| k.starts_with("thumbnail_"))
    {
        let entry = StoredAssociationEntry::read(&value, target_kind);
        match entry.thumbnail_address.clone() {
            Some(thumbnail_address) => valid.push((key, thumbnail_address, entry)),
            None => {
                warn!("Disassociate sweep: malformed entry '{key}' has no thumbnail address");
                let thumbnail = entry.reported_thumbnail();
                outcome.failed.push(DisassociateEntryFailure {
                    thumbnail,
                    error: DisassociateEntryError::MalformedEntry { key },
                });
            }
        }
    }

    if valid.is_empty() {
        info!(
            "[OK] Disassociate sweep on {} complete: {} succeeded, {} failed",
            association_target_resource_address,
            outcome.succeeded.len(),
            outcome.failed.len()
        );
        return outcome;
    }

    let mut entry_errors: Vec<Option<DisassociateEntryError>> =
        (0..valid.len()).map(|_| None).collect();

    // Phase 1 removes thumbnail-side metadata. Source keys are deliberately not
    // submitted yet: they are the durable lookup a later sweep needs if this phase
    // fails or ends before returning a result for an entry.
    //
    // Multiple `thumbnail_*` entries on the association target can reference the
    // same thumbnail resource address — most commonly a target that was associated
    // more than once, producing distinct deterministic-key variants that still point
    // at the same output file. Sending that address twice in one
    // `DeleteMetadataBatch` request is rejected by storage as a "duplicate batch
    // entry": a permanent, deterministic failure, not a transient one, so an event
    // carrying it would NACK and redeliver forever without ever clearing. Dedup by
    // thumbnail address before sending, and fan each request's outcome back out to
    // every `valid` index that shares it.
    let mut thumbnail_deletions: Vec<(&str, &str)> = Vec::new();
    let mut thumbnail_batch_groups: Vec<Vec<usize>> = Vec::new();
    {
        let mut batch_index_of: HashMap<&str, usize> = HashMap::new();
        for (valid_index, (_, thumb_addr, _)) in valid.iter().enumerate() {
            let batch_index = *batch_index_of
                .entry(thumb_addr.as_str())
                .or_insert_with(|| {
                    thumbnail_deletions.push((thumb_addr.as_str(), THUMBNAIL_METADATA_KEY));
                    thumbnail_batch_groups.push(Vec::new());
                    thumbnail_batch_groups.len() - 1
                });
            thumbnail_batch_groups[batch_index].push(valid_index);
        }
    }

    let thumbnail_batch =
        match remove_storage_metadata_batch(&thumbnail_deletions, auth_token).await {
            Ok(outcome) => outcome,
            Err(e) => {
                warn!(
                    "Disassociate sweep: thumbnail-side batch delete RPC failed for {}: {}",
                    association_target_resource_address, e
                );
                for (_, _, entry) in &valid {
                    outcome.failed.push(DisassociateEntryFailure {
                        thumbnail: entry.reported_thumbnail(),
                        error: DisassociateEntryError::ThumbnailSideRemoval { source: e.clone() },
                    });
                }
                info!(
                    "[FAIL] Disassociate sweep on {} complete: {} succeeded, {} failed",
                    association_target_resource_address,
                    outcome.succeeded.len(),
                    outcome.failed.len()
                );
                return outcome;
            }
        };

    if let Some(error) = &thumbnail_batch.terminal_error {
        warn!(
            "Disassociate sweep: thumbnail-side batch delete stream failed for {}: {}",
            association_target_resource_address, error
        );
    }

    let mut source_candidates = Vec::with_capacity(valid.len());
    for (batch_index, entry_outcome) in thumbnail_batch.entries.into_iter().enumerate() {
        let member_indices = &thumbnail_batch_groups[batch_index];
        match entry_outcome {
            StorageMetadataBatchEntryOutcome::Succeeded => {
                source_candidates.extend_from_slice(member_indices);
            }
            StorageMetadataBatchEntryOutcome::Failed(source) => {
                warn!(
                    "Disassociate sweep: thumbnail-side removal failed for batch index \
                     {batch_index} ({} valid entries deduped onto it): {source}",
                    member_indices.len()
                );
                for &index in member_indices {
                    entry_errors[index] = Some(DisassociateEntryError::ThumbnailSideRemoval {
                        source: source.clone(),
                    });
                }
            }
            StorageMetadataBatchEntryOutcome::Unresolved => {
                let source = thumbnail_batch.terminal_error.clone().unwrap_or_else(|| {
                    StorageMetadataError::Failed(format!(
                        "thumbnail-side batch returned no result for request index {batch_index}"
                    ))
                });
                warn!(
                    "Disassociate sweep: thumbnail-side removal unresolved for batch index {batch_index}: {source}"
                );
                for &index in member_indices {
                    entry_errors[index] = Some(DisassociateEntryError::ThumbnailSideRemoval {
                        source: source.clone(),
                    });
                }
            }
        }
    }

    // Phase 2 removes source-side keys only for phase-1 successes.
    let mut source_succeeded = vec![false; valid.len()];
    if !source_candidates.is_empty() {
        let source_deletions: Vec<(&str, &str)> = source_candidates
            .iter()
            .map(|&index| (association_target_resource_address, valid[index].0.as_str()))
            .collect();

        match remove_storage_metadata_batch(&source_deletions, auth_token).await {
            Err(error) => {
                warn!(
                    "Disassociate sweep: source-side batch delete RPC failed for {}: {}",
                    association_target_resource_address, error
                );
                for &index in &source_candidates {
                    entry_errors[index] =
                        Some(DisassociateEntryError::AssociationTargetSideRemoval {
                            key: valid[index].0.clone(),
                            source: error.clone(),
                        });
                }
            }
            Ok(source_batch) => {
                if let Some(error) = &source_batch.terminal_error {
                    warn!(
                        "Disassociate sweep: source-side batch delete stream failed for {}: {}",
                        association_target_resource_address, error
                    );
                }
                for (batch_index, entry_outcome) in source_batch.entries.into_iter().enumerate() {
                    let valid_index = source_candidates[batch_index];
                    match entry_outcome {
                        StorageMetadataBatchEntryOutcome::Succeeded => {
                            source_succeeded[valid_index] = true;
                        }
                        StorageMetadataBatchEntryOutcome::Failed(source) => {
                            let key = valid[valid_index].0.clone();
                            warn!(
                                "Disassociate sweep: source-side removal failed for key '{key}': {source}"
                            );
                            entry_errors[valid_index] =
                                Some(DisassociateEntryError::AssociationTargetSideRemoval {
                                    key,
                                    source,
                                });
                        }
                        StorageMetadataBatchEntryOutcome::Unresolved => {
                            let source = source_batch.terminal_error.clone().unwrap_or_else(|| {
                                StorageMetadataError::Failed(format!(
                                    "source-side batch returned no result for request index {batch_index}"
                                ))
                            });
                            let key = valid[valid_index].0.clone();
                            warn!(
                                "Disassociate sweep: source-side removal unresolved for key '{key}': {source}"
                            );
                            entry_errors[valid_index] =
                                Some(DisassociateEntryError::AssociationTargetSideRemoval {
                                    key,
                                    source,
                                });
                        }
                    }
                }
            }
        }
    }

    // Blobs this service wrote, and may therefore delete under the service
    // identity. Decided per entry here rather than derived from
    // `outcome.succeeded` afterwards, because the decision needs the stored
    // specification and `DisassociatedThumbnail` does not carry it.
    let mut deletable_blobs: Vec<String> = Vec::new();
    let mut caller_owned_blobs: Vec<String> = Vec::new();

    for (index, ((source_key, _, entry), entry_error)) in
        valid.into_iter().zip(entry_errors).enumerate()
    {
        let thumbnail = entry.reported_thumbnail();
        if let Some(error) = entry_error {
            outcome
                .failed
                .push(DisassociateEntryFailure { thumbnail, error });
        } else if source_succeeded[index] {
            if !thumbnail.resource_address.is_empty() {
                if entry.blob_is_service_generated(&thumbnail.resource_address) {
                    deletable_blobs.push(thumbnail.resource_address.clone());
                } else {
                    caller_owned_blobs.push(thumbnail.resource_address.clone());
                }
            }
            publish_disassociated_event(
                &entry,
                &thumbnail,
                association_target_resource_address,
                outbound.clone(),
            );
            outcome.succeeded.push(thumbnail);
        } else {
            outcome.failed.push(DisassociateEntryFailure {
                thumbnail,
                error: DisassociateEntryError::AssociationTargetSideRemoval {
                    key: source_key,
                    source: StorageMetadataError::Failed(
                        "source-side batch produced no outcome for this entry".to_string(),
                    ),
                },
            });
        }
    }

    for address in &caller_owned_blobs {
        info!(
            "Disassociate sweep: leaving '{address}' in storage — its association records no \
             conversion by this service and its address is not one the configured path template \
             renders, so it is treated as caller-owned. Metadata was still removed."
        );
    }

    if !deletable_blobs.is_empty() {
        crate::storage::delete_thumbnail_blobs(deletable_blobs, outbound.as_ref()).await;
    }

    // The skipped count belongs in the summary: without it, a sweep that deleted
    // nothing because everything was caller-owned reads identically to one that
    // had nothing to delete.
    info!(
        "[OK] Disassociate sweep on {} complete: {} succeeded, {} failed, {} blobs left as \
         caller-owned",
        association_target_resource_address,
        outcome.succeeded.len(),
        outcome.failed.len(),
        caller_owned_blobs.len()
    );

    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed clock for write-plan tests, so a plan is reproducible across runs.
    const TEST_CREATE_TIME: &str = "2026-01-01T00:00:00Z";

    fn request(
        target_addr: &str,
        target_id: &str,
        thumb_addr: &str,
        thumb_id: &str,
        target_kind: AssociationTargetKind,
    ) -> AssociationRequest {
        AssociationRequest {
            target_kind,
            association_target: AssociationResource {
                resource_address: target_addr.to_string(),
                resource_identity: if target_id.is_empty() {
                    None
                } else {
                    Some(target_id.to_string())
                },
            },
            thumbnail: AssociationResource {
                resource_address: thumb_addr.to_string(),
                resource_identity: if thumb_id.is_empty() {
                    None
                } else {
                    Some(thumb_id.to_string())
                },
            },
            specification: Some(AssociationSpec::ImageToImage {
                width: 256,
                height: 256,
                encoding_format: EncodingFormat::Png,
                transform_type: TransformType::Square,
                crop_strategy: CropStrategy::Center,
                quality: Some(85.0),
                preserve_aspect_ratio: Some(true),
            }),
        }
    }

    fn task_queue_http_payload() -> serde_json::Value {
        serde_json::json!({
            "target_type": "FILE",
            "thumbnail": {
                "resource_address": "https://storage.example.com/container/thumbnails/image-256x256.png?sig=yyy",
                "resource_identity": "thumb-identity-encoded"
            },
            "association_target": {
                "resource_address": "https://storage.example.com/container/source/image.png?sig=xxx",
                "resource_identity": "src-identity-encoded"
            },
            "specification": {
                "type": "IMAGE_TO_IMAGE",
                "width": 256,
                "height": 256,
                "encoding_format": "JPEG",
                "transform_type": "SQUARE",
                "crop_strategy": "CENTER",
                "quality": 85,
                "preserve_aspect_ratio": true
            }
        })
    }

    #[test]
    fn association_request_deserializes_task_queue_http_payload() {
        let req: AssociationRequest = serde_json::from_value(task_queue_http_payload())
            .expect("task-queue associate payload should deserialize");

        assert_eq!(req.target_kind, AssociationTargetKind::File);
        assert_eq!(
            req.thumbnail.resource_address,
            "https://storage.example.com/container/thumbnails/image-256x256.png?sig=yyy"
        );
        assert_eq!(
            req.thumbnail.resource_identity.as_deref(),
            Some("thumb-identity-encoded")
        );
        assert_eq!(
            req.association_target.resource_address,
            "https://storage.example.com/container/source/image.png?sig=xxx"
        );
        assert_eq!(req.validate(), Ok(()));
        let spec = req.specification.expect("specification should be present");
        if let AssociationSpec::ImageToImage {
            encoding_format,
            quality,
            preserve_aspect_ratio,
            ..
        } = spec
        {
            assert_eq!(encoding_format.to_string(), "JPEG");
            assert_eq!(quality, Some(85.0));
            assert_eq!(preserve_aspect_ratio, Some(true));
        } else {
            panic!("expected IMAGE_TO_IMAGE spec");
        }
    }

    #[test]
    fn association_request_accepts_all_canonical_task_queue_transforms() {
        for transform in ["THUMBNAIL", "RESIZE", "FIT"] {
            let mut payload = task_queue_http_payload();
            payload["specification"]["transform_type"] =
                serde_json::Value::String(transform.to_string());

            let request: AssociationRequest = serde_json::from_value(payload)
                .unwrap_or_else(|error| panic!("{transform} should deserialize: {error}"));
            if let Some(AssociationSpec::ImageToImage { transform_type, .. }) =
                request.specification
            {
                assert_eq!(transform_type.to_string(), transform);
            } else {
                panic!("{transform}: expected IMAGE_TO_IMAGE spec");
            }
        }
    }

    #[test]
    fn association_spec_optional_fields_preserve_presence() {
        // Omitting the optional fields deserializes as `None` (distinct from a value).
        let mut payload = task_queue_http_payload();
        let spec = payload["specification"].as_object_mut().unwrap();
        spec.remove("quality");
        spec.remove("preserve_aspect_ratio");
        let req: AssociationRequest =
            serde_json::from_value(payload).expect("payload without optionals should deserialize");
        let spec = req.specification.expect("specification present");
        if let AssociationSpec::ImageToImage {
            quality,
            preserve_aspect_ratio,
            ..
        } = spec
        {
            assert_eq!(quality, None);
            assert_eq!(preserve_aspect_ratio, None);
        } else {
            panic!("expected IMAGE_TO_IMAGE spec");
        }

        // An explicit `preserve_aspect_ratio: false` is retained as `Some(false)`.
        let mut payload = task_queue_http_payload();
        payload["specification"]["preserve_aspect_ratio"] = serde_json::Value::Bool(false);
        let req: AssociationRequest =
            serde_json::from_value(payload).expect("explicit false should deserialize");
        if let Some(AssociationSpec::ImageToImage {
            preserve_aspect_ratio,
            ..
        }) = req.specification
        {
            assert_eq!(preserve_aspect_ratio, Some(false));
        } else {
            panic!("expected IMAGE_TO_IMAGE spec");
        }
    }

    #[test]
    fn association_http_quality_uses_builtin_numeric_deserialization() {
        for quality in [
            serde_json::json!(85),
            serde_json::json!(85.0),
            serde_json::json!(85.5),
        ] {
            let mut payload = task_queue_http_payload();
            payload["specification"]["quality"] = quality.clone();

            let request: AssociationRequest = serde_json::from_value(payload)
                .unwrap_or_else(|error| panic!("numeric quality should deserialize: {error}"));
            if let Some(AssociationSpec::ImageToImage { quality: q, .. }) = request.specification {
                assert!(
                    q.is_some(),
                    "quality {quality} should deserialize through built-in f64 handling"
                );
            } else {
                panic!("expected IMAGE_TO_IMAGE spec");
            }
        }

        let mut payload = task_queue_http_payload();
        payload["specification"]["quality"] = serde_json::Value::Null;
        let request: AssociationRequest =
            serde_json::from_value(payload).expect("null quality should mean omission");
        if let Some(AssociationSpec::ImageToImage { quality, .. }) = request.specification {
            assert_eq!(quality, None);
        } else {
            panic!("expected IMAGE_TO_IMAGE spec");
        }

        let mut payload = task_queue_http_payload();
        payload["specification"]["quality"] = serde_json::json!(85.0);
        let request: AssociationRequest = serde_json::from_value(payload)
            .expect("an integral JSON number should remain backward compatible");
        if let Some(AssociationSpec::ImageToImage { quality, .. }) = request.specification {
            assert_eq!(quality, Some(85.0));
        } else {
            panic!("expected IMAGE_TO_IMAGE spec");
        }

        for invalid_quality in [serde_json::json!("85"), serde_json::json!(true)] {
            let mut payload = task_queue_http_payload();
            payload["specification"]["quality"] = invalid_quality;
            assert!(serde_json::from_value::<AssociationRequest>(payload).is_err());
        }
    }

    #[test]
    fn normalize_specification_fills_defaults_but_preserves_explicit_values() {
        crate::test_fixtures::init_test_runtime_config("thumbnail-common-association-tests");

        // Omitted optionals are resolved to the installed runtime defaults.
        let mut req = request(
            "https://storage.example.test/container/assets/source-file.usd",
            "source-id",
            "https://storage.example.test/container/thumbnails/thumb.png",
            "thumb-id",
            AssociationTargetKind::File,
        );
        if let Some(AssociationSpec::ImageToImage {
            quality,
            preserve_aspect_ratio,
            ..
        }) = req.specification.as_mut()
        {
            *quality = None;
            *preserve_aspect_ratio = None;
        }
        req.normalize_specification();
        if let Some(AssociationSpec::ImageToImage {
            quality,
            preserve_aspect_ratio,
            ..
        }) = req.specification.as_ref()
        {
            assert_eq!(*quality, Some(85.0));
            assert_eq!(*preserve_aspect_ratio, Some(true));
        } else {
            panic!("expected IMAGE_TO_IMAGE spec");
        }

        // Explicit values (including `false`) survive normalization untouched.
        let mut req = request(
            "https://storage.example.test/container/assets/source-file.usd",
            "source-id",
            "https://storage.example.test/container/thumbnails/thumb.png",
            "thumb-id",
            AssociationTargetKind::File,
        );
        if let Some(AssociationSpec::ImageToImage {
            quality,
            preserve_aspect_ratio,
            ..
        }) = req.specification.as_mut()
        {
            *quality = Some(42.0);
            *preserve_aspect_ratio = Some(false);
        }
        req.normalize_specification();
        if let Some(AssociationSpec::ImageToImage {
            quality,
            preserve_aspect_ratio,
            ..
        }) = req.specification.as_ref()
        {
            assert_eq!(*quality, Some(42.0));
            assert_eq!(*preserve_aspect_ratio, Some(false));
        } else {
            panic!("expected IMAGE_TO_IMAGE spec");
        }
    }

    #[test]
    fn normalized_specification_is_serialized_into_metadata() {
        crate::test_fixtures::init_test_runtime_config("thumbnail-common-association-tests");

        let mut req = request(
            "https://storage.example.test/container/assets/source-file.usd",
            "source-id",
            "https://storage.example.test/container/thumbnails/thumb.png",
            "thumb-id",
            AssociationTargetKind::File,
        );
        if let Some(AssociationSpec::ImageToImage {
            quality,
            preserve_aspect_ratio,
            ..
        }) = req.specification.as_mut()
        {
            *quality = None;
            *preserve_aspect_ratio = None;
        }
        req.normalize_specification();
        let spec = req.specification.clone().unwrap();

        let writes = build_association_metadata_writes(&req, &spec, TEST_CREATE_TIME)
            .expect("metadata write plan builds");
        let stored_spec = &writes.association_target_side.value["specification"];
        assert_eq!(stored_spec["type"], "IMAGE_TO_IMAGE");
        assert_eq!(stored_spec["quality"], 85.0);
        assert_eq!(stored_spec["preserve_aspect_ratio"], true);
    }

    fn association_output_snapshot(
        mut req: AssociationRequest,
    ) -> (String, serde_json::Value, serde_json::Value) {
        req.normalize_specification();
        req.validate().expect("normalized request should validate");
        let spec = req.specification.as_ref().expect("specification");
        let writes = build_association_metadata_writes(&req, spec, TEST_CREATE_TIME)
            .expect("metadata output should build");
        let mut stored_message = writes.association_target_side.value.clone();
        stored_message
            .as_object_mut()
            .expect("stored message object")
            .remove("create_time");
        let event_message = build_association_message(
            &req.message_params(Some(spec))
                .expect("validated request resolves its identities"),
        )
        .expect("normalized event message");

        (
            writes.association_target_side.key.clone(),
            stored_message,
            event_message,
        )
    }

    #[test]
    fn http_and_grpc_canonical_outputs_match_keys_storage_and_events() {
        crate::test_fixtures::init_test_runtime_config("thumbnail-common-association-tests");

        for (wire_format, encoding_format) in [
            ("JPEG", EncodingFormat::Jpeg),
            ("PNG", EncodingFormat::Png),
            ("WEBP", EncodingFormat::Webp),
        ] {
            for (quality, preserve_aspect_ratio) in [(None, None), (Some(63.0), Some(false))] {
                let mut http_payload = task_queue_http_payload();
                http_payload["specification"]["encoding_format"] =
                    serde_json::Value::String(wire_format.to_string());
                let http_spec = http_payload["specification"]
                    .as_object_mut()
                    .expect("specification object");
                match quality {
                    Some(value) => {
                        http_spec.insert("quality".to_string(), serde_json::json!(value));
                    }
                    None => {
                        http_spec.remove("quality");
                    }
                }
                match preserve_aspect_ratio {
                    Some(value) => {
                        http_spec.insert(
                            "preserve_aspect_ratio".to_string(),
                            serde_json::Value::Bool(value),
                        );
                    }
                    None => {
                        http_spec.remove("preserve_aspect_ratio");
                    }
                }
                let from_http: AssociationRequest =
                    serde_json::from_value(http_payload).expect("HTTP request");

                let mut from_grpc = request(
                    "https://storage.example.com/container/source/image.png?sig=xxx",
                    "src-identity-encoded",
                    "https://storage.example.com/container/thumbnails/image-256x256.png?sig=yyy",
                    "thumb-identity-encoded",
                    AssociationTargetKind::File,
                );
                if let Some(AssociationSpec::ImageToImage {
                    encoding_format: ef,
                    quality: q,
                    preserve_aspect_ratio: par,
                    ..
                }) = from_grpc.specification.as_mut()
                {
                    *ef = encoding_format;
                    *q = quality;
                    *par = preserve_aspect_ratio;
                }

                assert_eq!(
                    association_output_snapshot(from_http),
                    association_output_snapshot(from_grpc),
                    "{wire_format} outputs should match for quality={quality:?}, preserve_aspect_ratio={preserve_aspect_ratio:?}"
                );
            }
        }
    }

    #[test]
    fn association_request_rejects_invalid_typed_http_fields() {
        let cases = [
            ("target_type", "UNKNOWN"),
            ("encoding_format", "TIFF"),
            ("transform_type", "STRETCH"),
            ("crop_strategy", "MIDDLE"),
        ];

        for (field, invalid_value) in cases {
            let mut payload = task_queue_http_payload();
            if field == "target_type" {
                payload[field] = serde_json::Value::String(invalid_value.to_string());
            } else {
                payload["specification"][field] =
                    serde_json::Value::String(invalid_value.to_string());
            }

            assert!(
                serde_json::from_value::<AssociationRequest>(payload).is_err(),
                "{field} should reject unsupported value {invalid_value}"
            );
        }
    }

    #[test]
    fn association_http_contract_requires_specification_and_defaults_target_type_to_file() {
        let mut without_target_type = task_queue_http_payload();
        without_target_type
            .as_object_mut()
            .expect("payload object")
            .remove("target_type");
        let request: AssociationRequest = serde_json::from_value(without_target_type)
            .expect("omitted target_type should default to FILE");
        assert_eq!(request.target_kind, AssociationTargetKind::File);

        let mut without_specification = task_queue_http_payload();
        without_specification
            .as_object_mut()
            .expect("payload object")
            .remove("specification");
        let request: AssociationRequest = serde_json::from_value(without_specification)
            .expect("missing specification must reach canonical validation");
        assert_eq!(
            request.validate(),
            Err(AssociationValidationError::MissingSpecification)
        );
    }

    #[test]
    fn association_request_validates_itself() {
        let req = request(
            "https://storage.example.test/container/assets/source-file.usd",
            "source-id",
            "https://storage.example.test/container/thumbnails/thumb.png",
            "thumb-id",
            AssociationTargetKind::File,
        );

        assert_eq!(req.validate(), Ok(()));
    }

    #[test]
    fn association_request_rejects_out_of_range_dimensions_before_storage() {
        let max = AssociationSpec::MAX_DIMENSION;
        for (width, height) in [
            (0, 256),
            (256, 0),
            (max + 1, 256),
            (256, max + 1),
            (100_000, 256),
        ] {
            let mut req = request(
                "https://storage.example.test/container/assets/source-file.usd",
                "source-id",
                "https://storage.example.test/container/thumbnails/thumb.png",
                "thumb-id",
                AssociationTargetKind::File,
            );
            let (test_w, test_h) = (width, height);
            if let Some(AssociationSpec::ImageToImage {
                width: w,
                height: h,
                ..
            }) = req.specification.as_mut()
            {
                *w = test_w;
                *h = test_h;
            }

            assert_eq!(
                req.validate(),
                Err(AssociationValidationError::InvalidDimensions)
            );
        }

        let mut at_max = request(
            "https://storage.example.test/container/assets/source-file.usd",
            "source-id",
            "https://storage.example.test/container/thumbnails/thumb.png",
            "thumb-id",
            AssociationTargetKind::File,
        );
        if let Some(AssociationSpec::ImageToImage {
            width: w,
            height: h,
            ..
        }) = at_max.specification.as_mut()
        {
            *w = max;
            *h = max;
        }
        assert_eq!(at_max.validate(), Ok(()));
    }

    #[test]
    fn association_request_applies_quality_range_only_to_lossy_formats() {
        for encoding_format in [EncodingFormat::Jpeg, EncodingFormat::Webp] {
            for quality in [0.0, 101.0] {
                let (test_ef, test_q) = (encoding_format, quality);
                let mut req = request(
                    "https://storage.example.test/container/assets/source-file.usd",
                    "source-id",
                    "https://storage.example.test/container/thumbnails/thumb.png",
                    "thumb-id",
                    AssociationTargetKind::File,
                );
                if let Some(AssociationSpec::ImageToImage {
                    encoding_format: ef,
                    quality: q,
                    ..
                }) = req.specification.as_mut()
                {
                    *ef = test_ef;
                    *q = Some(test_q);
                }

                assert_eq!(
                    req.validate(),
                    Err(AssociationValidationError::InvalidQuality)
                );
            }
        }

        for encoding_format in [EncodingFormat::Png, EncodingFormat::Gif] {
            for quality in [-1.0, 85.5, 101.0] {
                let (test_ef, test_q) = (encoding_format, quality);
                let mut req = request(
                    "https://storage.example.test/container/assets/source-file.usd",
                    "source-id",
                    "https://storage.example.test/container/thumbnails/thumb.png",
                    "thumb-id",
                    AssociationTargetKind::File,
                );
                if let Some(AssociationSpec::ImageToImage {
                    encoding_format: ef,
                    quality: q,
                    ..
                }) = req.specification.as_mut()
                {
                    *ef = test_ef;
                    *q = Some(test_q);
                }

                assert_eq!(
                    req.validate(),
                    Ok(()),
                    "{encoding_format} should not apply lossy quality validation to {quality}"
                );
            }
        }
    }

    #[test]
    fn association_output_builders_reject_unnormalized_optionals() {
        let mut req = request(
            "https://storage.example.test/container/assets/source-file.usd",
            "source-id",
            "https://storage.example.test/container/thumbnails/thumb.png",
            "thumb-id",
            AssociationTargetKind::File,
        );
        if let Some(AssociationSpec::ImageToImage {
            quality,
            preserve_aspect_ratio,
            ..
        }) = req.specification.as_mut()
        {
            *quality = None;
            *preserve_aspect_ratio = None;
        }
        let spec = req.specification.clone().expect("test specification");

        let error = build_association_metadata_writes(&req, &spec, TEST_CREATE_TIME)
            .expect_err("output builders must require normalized optionals");
        assert_eq!(error, AssociationInvariant::UnnormalizedQuality);
    }

    #[test]
    fn association_event_message_builder_rejects_unnormalized_specification() {
        let specification = AssociationSpec::ImageToImage {
            width: 256,
            height: 256,
            encoding_format: EncodingFormat::Png,
            transform_type: TransformType::Square,
            crop_strategy: CropStrategy::Center,
            quality: None,
            preserve_aspect_ratio: None,
        };
        let result = build_association_message(&AssociationMessageParams {
            thumbnail: AssociationResource {
                resource_address: "https://store.test/thumb.png".to_string(),
                resource_identity: Some("thumb-id".to_string()),
            },
            association_target: AssociationResource {
                resource_address: "https://store.test/source.png".to_string(),
                resource_identity: Some("source-id".to_string()),
            },
            target_kind: AssociationTargetKind::File,
            specification: Some(&specification),
        });

        assert!(
            result.is_err(),
            "event message construction must reject missing normalized optionals"
        );
    }

    #[test]
    fn integral_quality_preserves_existing_deterministic_key() {
        let target_addr = "https://storage.example.test/container/assets/source-file.usd";
        let thumb_id =
            "https://storage.example.test/container/thumbnails/thumb.png?version-id=thumb";
        let req = request(
            target_addr,
            "source-id",
            "https://storage.example.test/container/thumbnails/thumb.png",
            thumb_id,
            AssociationTargetKind::File,
        );
        let writes = build_association_metadata_writes(
            &req,
            req.specification.as_ref().expect("test specification"),
            TEST_CREATE_TIME,
        )
        .expect("default specification is normalized");

        // Key reflects the new hash ordering (spec_type hashed before transform/crop).
        assert_eq!(
            writes.association_target_side.key,
            "thumbnail_256x256_PNG_1272de75c2b0a3bfdc524f4c"
        );
    }

    #[test]
    fn association_target_kind_converts_from_proto_values() {
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::TargetType;

        assert_eq!(
            AssociationTargetKind::try_from(TargetType::Unspecified as i32),
            Ok(AssociationTargetKind::File)
        );
        assert_eq!(
            AssociationTargetKind::try_from(TargetType::File as i32),
            Ok(AssociationTargetKind::File)
        );
        assert_eq!(
            AssociationTargetKind::try_from(TargetType::Folder as i32),
            Ok(AssociationTargetKind::Folder)
        );
        assert!(AssociationTargetKind::try_from(i32::MAX).is_err());
    }

    #[tokio::test]
    async fn file_association_writes_nested_shape_on_both_sides() {
        let target_addr = "https://storage.example.test/container/assets/source-file.usd";
        let target_id =
            "https://storage.example.test/container/assets/source-file.usd?version-id=source";
        let thumb_addr = "https://storage.example.test/container/thumbnails/thumb.png";
        let thumb_id =
            "https://storage.example.test/container/thumbnails/thumb.png?version-id=thumb";

        let writes = build_association_metadata_writes(
            &request(
                target_addr,
                target_id,
                thumb_addr,
                thumb_id,
                AssociationTargetKind::File,
            ),
            &AssociationSpec::ImageToImage {
                width: 256,
                height: 256,
                encoding_format: EncodingFormat::Png,
                transform_type: TransformType::Square,
                crop_strategy: CropStrategy::Center,
                quality: Some(85.0),
                preserve_aspect_ratio: Some(true),
            },
            TEST_CREATE_TIME,
        )
        .expect("metadata write plan should build");

        // Both sides keyed on address (ADR 0002 §4).
        assert_eq!(writes.association_target_side.resource_address, target_addr);
        assert!(writes
            .association_target_side
            .key
            .starts_with("thumbnail_256x256_PNG_"));
        assert_eq!(writes.thumbnail_side.resource_address, thumb_addr);
        assert_eq!(writes.thumbnail_side.key, THUMBNAIL_METADATA_KEY);

        // Same nested value written on both sides.
        assert_eq!(
            writes.association_target_side.value,
            writes.thumbnail_side.value
        );
        let value = &writes.association_target_side.value;
        assert_eq!(value["target_type"], "FILE");
        assert_eq!(value["thumbnail"]["resource_identity"], thumb_id);
        assert_eq!(value["thumbnail"]["resource_address"], thumb_addr);
        assert_eq!(value["association_target"]["resource_identity"], target_id);
        assert_eq!(value["association_target"]["resource_address"], target_addr);
        assert!(
            value["create_time"].is_string(),
            "stored association metadata should include create_time"
        );
    }

    #[tokio::test]
    async fn folder_association_omits_target_identity() {
        let folder_addr = "https://storage.example.test/container/assets/";
        let thumb_addr = "https://storage.example.test/container/thumbnails/thumb.png";
        let thumb_id =
            "https://storage.example.test/container/thumbnails/thumb.png?version-id=thumb";

        let writes = build_association_metadata_writes(
            &request(
                folder_addr,
                "", // folders have no identity
                thumb_addr,
                thumb_id,
                AssociationTargetKind::Folder,
            ),
            &AssociationSpec::ImageToImage {
                width: 256,
                height: 256,
                encoding_format: EncodingFormat::Png,
                transform_type: TransformType::Square,
                crop_strategy: CropStrategy::Center,
                quality: Some(85.0),
                preserve_aspect_ratio: Some(true),
            },
            TEST_CREATE_TIME,
        )
        .expect("metadata write plan should build");

        let value = &writes.association_target_side.value;
        assert_eq!(value["target_type"], "FOLDER");
        assert_eq!(value["association_target"]["resource_address"], folder_addr);
        // resource_identity must be omitted entirely for folders, not null/empty.
        assert!(value["association_target"]
            .as_object()
            .unwrap()
            .get("resource_identity")
            .is_none());
    }

    fn stored_entry(thumbnail: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "thumbnail": thumbnail,
            "association_target": {
                "resource_address": "https://storage.example.test/container/assets/source.usd",
                "resource_identity": "https://storage.example.test/container/assets/source.usd?version-id=v1",
            },
            "target_type": "FILE",
        })
    }

    #[test]
    fn stored_association_entry_reads_the_fields_the_removal_and_event_need() {
        let entry = StoredAssociationEntry::read(
            &stored_entry(serde_json::json!({
                "resource_address": "https://storage.example.test/container/thumbnails/t.png",
                "resource_identity": "https://storage.example.test/container/thumbnails/t.png?version-id=v2",
            })),
            AssociationTargetKind::Folder,
        );

        assert_eq!(
            entry.thumbnail_address.as_deref(),
            Some("https://storage.example.test/container/thumbnails/t.png")
        );
        assert_eq!(
            entry.thumbnail_identity.as_deref(),
            Some("https://storage.example.test/container/thumbnails/t.png?version-id=v2")
        );
        assert_eq!(
            entry.association_target_identity,
            "https://storage.example.test/container/assets/source.usd?version-id=v1"
        );
        // The entry's own stored target_type wins over the caller's fallback kind.
        assert_eq!(entry.target_kind, AssociationTargetKind::File);
    }

    #[test]
    fn stored_association_entry_uses_the_fallback_kind_when_the_entry_has_none_readable() {
        for stored_target_type in [
            serde_json::json!(null),
            serde_json::json!("NOT_A_TARGET_TYPE"),
            serde_json::json!(7),
        ] {
            let mut value = stored_entry(serde_json::json!({
                "resource_address": "https://storage.example.test/container/thumbnails/t.png",
            }));
            value["target_type"] = stored_target_type.clone();

            let entry = StoredAssociationEntry::read(&value, AssociationTargetKind::Folder);

            assert_eq!(
                entry.target_kind,
                AssociationTargetKind::Folder,
                "unreadable stored target_type {stored_target_type} should use the fallback kind"
            );
        }
    }

    #[test]
    fn stored_association_entry_treats_an_absent_or_blank_thumbnail_address_as_no_address() {
        for thumbnail in [
            serde_json::json!({}),
            serde_json::json!({ "resource_address": "" }),
            serde_json::json!({ "resource_address": null }),
        ] {
            let entry =
                StoredAssociationEntry::read(&stored_entry(thumbnail), AssociationTargetKind::File);

            assert!(
                entry.thumbnail_address.is_none(),
                "an entry with no usable thumbnail address must not present one"
            );
            // It is still named in the outcome, with the empty address it was stored with.
            assert_eq!(entry.reported_thumbnail().resource_address, "");
        }
    }

    #[tokio::test]
    async fn malformed_entry_is_skipped_when_targeting_a_specific_thumbnail() {
        use crate::test_fixtures::mock_publisher_server::MockPublisherServer;
        use crate::test_fixtures::mock_storage_server::{mock_storage, unique_test_address};

        crate::test_fixtures::init_test_runtime_config("thumbnail-common-association-tests");

        let target_address = unique_test_address("malformed-entry-source.usd");
        let thumbnail_address = unique_test_address("malformed-entry-thumb.png");
        // Seed a thumbnail_* key with no thumbnail.resource_address (malformed).
        mock_storage().seed(
            &target_address,
            "thumbnail_abc123",
            serde_json::json!({
                "thumbnail": {},
                "association_target": { "resource_address": target_address },
                "target_type": "FILE",
            }),
        );
        let publisher = MockPublisherServer::start().await;

        // A targeted request for a specific thumbnail that is not listed (because the
        // stored entry has no address) returns empty success — nothing to remove.
        let outcome = disassociate_thumbnail(
            &targeted_request(&target_address, &thumbnail_address),
            "test-token",
            publisher.outbound(),
        )
        .await;

        assert!(outcome.succeeded.is_empty());
        assert!(outcome.failed.is_empty());
        assert!(outcome.aborted.is_none());
        // The malformed entry is left untouched.
        assert!(mock_storage().has_key(&target_address, "thumbnail_abc123"));
    }

    #[test]
    fn entry_errors_render_the_messages_callers_already_receive() {
        assert_eq!(
            DisassociateEntryError::MalformedEntry {
                key: "thumbnail_abc123".to_string()
            }
            .to_string(),
            "stored entry 'thumbnail_abc123' has no thumbnail.resource_address; thumbnail_metadata not removed"
        );
        assert_eq!(
            DisassociateEntryError::ThumbnailSideRemoval {
                source: StorageMetadataError::Failed("upstream exploded".to_string())
            }
            .to_string(),
            "failed to remove thumbnail metadata: upstream exploded"
        );
        assert_eq!(
            DisassociateEntryError::AssociationTargetSideRemoval {
                key: "thumbnail_abc123".to_string(),
                source: StorageMetadataError::Failed("upstream exploded".to_string()),
            }
            .to_string(),
            "failed to remove source key 'thumbnail_abc123': upstream exploded"
        );
    }

    #[test]
    fn sweep_results_partition_into_the_outcomes_two_lists() {
        let thumbnail = |address: &str| DisassociatedThumbnail {
            resource_address: address.to_string(),
            resource_identity: None,
        };

        let outcome: DisassociateOutcome = vec![
            Ok(thumbnail("first")),
            Err(DisassociateEntryFailure {
                thumbnail: thumbnail("second"),
                error: DisassociateEntryError::MalformedEntry {
                    key: "thumbnail_second".to_string(),
                },
            }),
            Ok(thumbnail("third")),
        ]
        .into_iter()
        .collect();

        assert_eq!(
            outcome
                .succeeded
                .iter()
                .map(|t| t.resource_address.as_str())
                .collect::<Vec<_>>(),
            ["first", "third"]
        );
        assert_eq!(
            outcome
                .failed
                .iter()
                .map(|f| f.thumbnail.resource_address.as_str())
                .collect::<Vec<_>>(),
            ["second"]
        );
        assert!(
            outcome.aborted.is_none(),
            "a sweep that produced per-entry results did not abort"
        );
    }

    /// A `FILE`-target disassociate request targeting one specific thumbnail.
    fn targeted_request(
        association_target_resource_address: &str,
        thumbnail_resource_address: &str,
    ) -> DisassociateRequest {
        DisassociateRequest {
            target_kind: AssociationTargetKind::File,
            association_target_resource_address: association_target_resource_address.to_string(),
            association_target_resource_identity: None,
            thumbnail_resource_address: thumbnail_resource_address.to_string(),
            thumbnail_resource_identity: None,
        }
    }

    /// Stand up a disassociation test against the storage double: one target holding one
    /// `thumbnail_*` entry, plus that thumbnail's own `thumbnail_metadata` blob.
    ///
    /// Returns the target address, the thumbnail address, and the source-side key.
    fn seed_one_association(label: &str) -> (String, String, String) {
        use crate::test_fixtures::mock_storage_server::{mock_storage, unique_test_address};

        crate::test_fixtures::init_test_runtime_config("thumbnail-common-association-tests");

        let target_address = unique_test_address(&format!("{label}-source.usd"));
        let thumbnail_address = unique_test_address(&format!("{label}-thumb.png"));
        let source_key = "thumbnail_abc123";

        mock_storage().seed(
            &target_address,
            source_key,
            serde_json::json!({
                "thumbnail": {
                    "resource_address": thumbnail_address,
                    "resource_identity": format!("{thumbnail_address}?version-id=v1"),
                },
                "association_target": {
                    "resource_address": target_address,
                    "resource_identity": format!("{target_address}?version-id=v1"),
                },
                "target_type": "FILE",
            }),
        );
        mock_storage().seed(
            &thumbnail_address,
            THUMBNAIL_METADATA_KEY,
            serde_json::json!({ "status": "SUCCESS" }),
        );

        (target_address, thumbnail_address, source_key.to_string())
    }

    /// How long the publisher double holds an association publish open. Large
    /// enough that a caller span which waited for it is unmistakable, and the
    /// duration assertion below leaves half of it as margin rather than racing.
    const SLOW_PUBLISH: std::time::Duration = std::time::Duration::from_millis(800);

    /// The defect MR 211 shipped: `.in_current_span()` made the detached publish a
    /// descendant of the caller's span, and `tracing-opentelemetry` writes
    /// `end_time` on every span exit, so the caller's *exported* duration became
    /// the publish's. A 20 ms associate reported as however long notifications
    /// took to accept the event, with no child span accounting for the gap.
    ///
    /// Both halves are asserted, because either alone is satisfiable by a
    /// regression: a caller span bounded by its own return, and a publish that is
    /// still a named child of it. Dropping the parentage to fix the duration would
    /// leave the publish in a trace of its own, which is the other bug.
    #[tokio::test]
    async fn a_slow_association_publish_neither_extends_nor_leaves_the_callers_trace() {
        use crate::test_fixtures::mock_publisher_server::{MockPublisherServer, PublisherBehavior};
        use crate::test_fixtures::trace_probe;

        let probe = trace_probe::probe();
        let (target_address, thumbnail_address, _) = seed_one_association("slow-publish-span");
        let publisher =
            MockPublisherServer::start_with(PublisherBehavior::SucceedAfter(SLOW_PUBLISH)).await;

        // Scoped so the caller's span closes when the call returns, which is the
        // boundary the exported duration is being compared against.
        {
            let caller = tracing::info_span!("test.caller");
            let outcome = disassociate_thumbnail(
                &targeted_request(&target_address, &thumbnail_address),
                "test-token",
                publisher.outbound(),
            )
            .instrument(caller)
            .await;
            assert!(
                outcome.failed.is_empty(),
                "the disassociate itself must succeed: {:?}",
                outcome.failed
            );
        }

        // Awaited rather than read outright, and that is a deliberate weakening: a
        // caller span that *is* a descendant of the publish stays open until the
        // publish closes, so reading immediately would fail with "no such span"
        // and say nothing about why. Waiting lets the assertion below report the
        // inflated duration, which is the actual defect.
        let caller = probe
            .await_span_named("test.caller", std::time::Duration::from_secs(5))
            .await;
        let publish = probe
            .await_span_named(
                "thumbnail.association_event",
                std::time::Duration::from_secs(5),
            )
            .await;

        trace_probe::ExpectedParent::of(&caller).assert_parents(&publish);

        // Compared against the publish's *end*, not against a fixed budget. The
        // caller's own storage work is as slow as the machine is loaded — an
        // earlier version of this assertion capped the caller's total duration and
        // failed at 1.07s inside the full suite while passing alone. The delay is
        // measured from the spawn, so the gap between the two end times is what
        // the fix controls and load does not.
        let gap = publish
            .end_time
            .duration_since(caller.end_time)
            .unwrap_or_default();
        assert!(
            gap >= SLOW_PUBLISH / 2,
            "the caller's exported span must end when the call returned, leaving \
             the detached publish to end {SLOW_PUBLISH:?} later; the two ended \
             {gap:?} apart, which is what a caller span waiting on its own \
             descendant looks like"
        );
        // Use the same margin as the caller-end check above. The exported span
        // uses wall-clock timestamps, while the injected delay uses Tokio's
        // timer; requiring the full delay exactly makes this a clock assertion.
        let publish_duration = trace_probe::exported_duration(&publish);
        assert!(
            publish_duration >= SLOW_PUBLISH / 2,
            "the publish's own span is where its duration belongs, so it should \
             report the time the caller's span no longer does; got {publish_duration:?} \
             for an injected delay of {SLOW_PUBLISH:?}"
        );
    }

    /// The third of the three re-composed clients with an `Unauthenticated`
    /// refresh-and-reissue branch, and the one whose failure is quietest: the
    /// association publish is fire-and-forget, so a reissue broken by the layered
    /// stack would lose the event with nothing but a log line to show for it.
    #[tokio::test]
    async fn an_unauthenticated_association_publish_is_reissued_and_succeeds() {
        use crate::test_fixtures::mock_publisher_server::{MockPublisherServer, PublisherBehavior};
        use crate::test_fixtures::trace_probe;

        let _probe = trace_probe::probe();
        let (target_address, thumbnail_address, _) = seed_one_association("publish-reissue");
        let publisher =
            MockPublisherServer::start_with(PublisherBehavior::UnauthenticatedThenSucceed).await;

        let outcome = disassociate_thumbnail(
            &targeted_request(&target_address, &thumbnail_address),
            "test-token",
            publisher.outbound_with_token_refresh(),
        )
        .await;
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);

        let published = publisher
            .await_events(2, std::time::Duration::from_secs(5))
            .await;
        assert_eq!(
            published.len(),
            2,
            "the rejected attempt and the reissue are both real publishes; one \
             means the reissue never left the client"
        );
        for (attempt, event) in published.iter().enumerate() {
            assert!(
                event.traceparent.is_some(),
                "attempt {} must carry trace context",
                attempt + 1
            );
        }

        // A publish is fire-and-forget, so `outcome` cannot report whether the
        // reissue worked. The tokens are the only evidence, and they have to
        // *differ*: the double answers by call index, so resending the rejected
        // credential would look identical from every other angle.
        let first = format!("Bearer {}", crate::outbound::OutboundClients::TEST_TOKEN);
        let refreshed = format!(
            "Bearer {}",
            crate::outbound::OutboundClients::TEST_TOKEN_REFRESHED
        );
        assert_eq!(
            published
                .iter()
                .map(|event| event.authorization.as_deref())
                .collect::<Vec<_>>(),
            vec![Some(first.as_str()), Some(refreshed.as_str())],
            "the reissue must carry the refreshed credential"
        );
    }

    /// Outbound storage calls carry the caller's trace context and its bearer
    /// token, asserted on what the double received.
    ///
    /// Nothing could assert this before: the storage double read `authorization`
    /// for its own token checks and discarded every other header, so propagation
    /// to storage was untestable rather than untested. These are the calls made in
    /// the middle of a request, so an unattributed stall in one of them is the
    /// hardest kind to explain from a trace.
    #[tokio::test]
    async fn storage_calls_carry_the_callers_trace_context_and_token() {
        use crate::test_fixtures::mock_publisher_server::MockPublisherServer;
        use crate::test_fixtures::mock_storage_server::mock_storage;
        use crate::test_fixtures::trace_probe;

        let probe = trace_probe::probe();
        let (target_address, thumbnail_address, _) = seed_one_association("storage-propagation");
        let publisher = MockPublisherServer::start().await;

        let caller = tracing::info_span!("test.caller");
        let outcome = disassociate_thumbnail(
            &targeted_request(&target_address, &thumbnail_address),
            "test-token",
            publisher.outbound(),
        )
        .instrument(caller)
        .await;
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);

        let caller = probe
            .await_span_named("test.caller", std::time::Duration::from_secs(5))
            .await;
        let expected_trace_id = caller.span_context.trace_id().to_string();

        let calls = mock_storage().calls_for(&target_address);
        assert!(
            !calls.is_empty(),
            "the disassociate must have made storage metadata calls to assert on"
        );
        for call in &calls {
            let traceparent = call.traceparent.as_deref().unwrap_or_else(|| {
                panic!(
                    "storage {:?} on {} arrived with no traceparent",
                    call.op, call.address
                )
            });
            assert_eq!(
                trace_probe::trace_id_of(traceparent),
                Some(expected_trace_id.as_str()),
                "storage {:?} must be attributable to the caller's trace",
                call.op
            );
            assert!(
                call.authorization.is_some(),
                "storage {:?} must still carry its bearer token; the trace layer \
                 was composed around the auth path, not instead of it",
                call.op
            );
        }
    }

    /// The sweep spawns one publish per thumbnail, which is why this mattered more
    /// there than at the single-associate site: under the old parentage the
    /// `storage.event` delivery span's duration became the slowest of N publishes.
    #[tokio::test]
    async fn a_fan_out_of_detached_publishes_does_not_extend_the_delivery_span() {
        use crate::test_fixtures::mock_publisher_server::{MockPublisherServer, PublisherBehavior};
        use crate::test_fixtures::mock_storage_server::{mock_storage, unique_test_address};
        use crate::test_fixtures::trace_probe;

        let probe = trace_probe::probe();
        let (target_address, _, _) = seed_one_association("sweep-publish-span");
        let second_thumbnail = unique_test_address("sweep-publish-span-thumb-2.png");
        mock_storage().seed(
            &target_address,
            "thumbnail_def456",
            serde_json::json!({
                "thumbnail": {
                    "resource_address": second_thumbnail,
                    "resource_identity": format!("{second_thumbnail}?version-id=v1"),
                },
                "association_target": { "resource_address": target_address },
                "target_type": "FILE",
                "specification": { "type": "MANUAL" },
            }),
        );
        mock_storage().seed(
            &second_thumbnail,
            THUMBNAIL_METADATA_KEY,
            serde_json::json!({ "status": "SUCCESS" }),
        );
        let publisher =
            MockPublisherServer::start_with(PublisherBehavior::SucceedAfter(SLOW_PUBLISH)).await;

        {
            let delivery = tracing::info_span!("test.delivery");
            let outcome = disassociate_all_thumbnails(
                &target_address,
                AssociationTargetKind::File,
                "test-token",
                publisher.outbound(),
            )
            .instrument(delivery)
            .await;
            assert_eq!(
                outcome.succeeded.len(),
                2,
                "both thumbnails should be disassociated: {outcome:?}"
            );
        }

        publisher
            .await_events(2, std::time::Duration::from_secs(5))
            .await;
        let delivery = probe
            .await_span_named("test.delivery", std::time::Duration::from_secs(5))
            .await;
        let publishes = probe
            .await_spans_named(
                "thumbnail.association_event",
                2,
                std::time::Duration::from_secs(5),
            )
            .await;

        let expected = trace_probe::ExpectedParent::of(&delivery);
        for publish in &publishes {
            expected.assert_parents(publish);
        }

        // Against the last publish to end, which is the one the old parentage made
        // the delivery span wait for.
        let slowest_end = publishes
            .iter()
            .map(|publish| publish.end_time)
            .max()
            .expect("two publishes were just asserted");
        let gap = slowest_end
            .duration_since(delivery.end_time)
            .unwrap_or_default();
        assert!(
            gap >= SLOW_PUBLISH / 2,
            "the delivery span must end without waiting for the slowest of the \
             spawned publishes; the two ended {gap:?} apart"
        );
    }

    #[tokio::test]
    async fn retry_after_source_side_failure_still_removes_source_key() {
        use crate::test_fixtures::mock_publisher_server::MockPublisherServer;
        use crate::test_fixtures::mock_storage_server::mock_storage;

        let (target_address, thumbnail_address, source_key) =
            seed_one_association("already-removed");
        let publisher = MockPublisherServer::start().await;

        // First call removes the source-side key cleanly.
        let outcome = disassociate_thumbnail(
            &targeted_request(&target_address, &thumbnail_address),
            "test-token",
            publisher.outbound(),
        )
        .await;
        assert!(outcome.failed.is_empty());

        // Re-seed only the source key (simulating a re-association or a retry scenario).
        // The retry must still remove the source key; NOT_FOUND counts as success.
        mock_storage().seed(
            &target_address,
            &source_key,
            serde_json::json!({
                "thumbnail": { "resource_address": thumbnail_address },
                "target_type": "FILE",
            }),
        );

        let outcome = disassociate_thumbnail(
            &targeted_request(&target_address, &thumbnail_address),
            "test-token",
            publisher.outbound(),
        )
        .await;

        assert_eq!(
            outcome.succeeded.len(),
            1,
            "source key present means the association still exists: {:?}",
            outcome.failed
        );
        assert!(
            !mock_storage().has_key(&target_address, &source_key),
            "the source key must be removed"
        );
    }

    #[tokio::test]
    async fn targeting_one_thumbnail_leaves_other_thumbnails_on_the_target_untouched() {
        use crate::test_fixtures::mock_publisher_server::MockPublisherServer;
        use crate::test_fixtures::mock_storage_server::{mock_storage, unique_test_address};

        let (target_address, target_thumbnail_address, target_source_key) =
            seed_one_association("targeted-removal");
        let other_thumbnail_address = unique_test_address("other-thumb.png");
        let other_source_key = "thumbnail_def456";
        mock_storage().seed(
            &target_address,
            other_source_key,
            serde_json::json!({
                "thumbnail": {
                    "resource_address": other_thumbnail_address,
                    "resource_identity": format!("{other_thumbnail_address}?version-id=v1"),
                },
                "association_target": { "resource_address": target_address },
                "target_type": "FILE",
            }),
        );
        mock_storage().seed(
            &other_thumbnail_address,
            THUMBNAIL_METADATA_KEY,
            serde_json::json!({ "status": "SUCCESS" }),
        );
        let publisher = MockPublisherServer::start().await;

        // Target only the first thumbnail; the second must be untouched.
        let outcome = disassociate_thumbnail(
            &targeted_request(&target_address, &target_thumbnail_address),
            "test-token",
            publisher.outbound(),
        )
        .await;

        assert_eq!(outcome.succeeded.len(), 1);
        assert_eq!(
            outcome.succeeded[0].resource_address,
            target_thumbnail_address
        );
        assert!(outcome.failed.is_empty());

        // Source-side key is removed; thumbnail resource and its metadata are left intact.
        assert!(!mock_storage().has_key(&target_address, &target_source_key));
        assert!(mock_storage().has_key(&target_thumbnail_address, THUMBNAIL_METADATA_KEY));

        // The other thumbnail is untouched.
        assert!(mock_storage().has_key(&target_address, other_source_key));
        assert!(mock_storage().has_key(&other_thumbnail_address, THUMBNAIL_METADATA_KEY));

        assert_eq!(
            publisher
                .await_events(1, std::time::Duration::from_secs(5))
                .await
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn source_side_failure_stays_discoverable_and_succeeds_on_retry() {
        use crate::test_fixtures::mock_publisher_server::MockPublisherServer;
        use crate::test_fixtures::mock_storage_server::{mock_storage, MetadataOp};

        let (target_address, thumbnail_address, source_key) =
            seed_one_association("source-side-retry");
        mock_storage().fail_times(
            MetadataOp::Delete,
            &target_address,
            Some(&source_key),
            tonic::Code::Internal,
            1,
        );
        let publisher = MockPublisherServer::start().await;

        let first_outcome = disassociate_thumbnail(
            &targeted_request(&target_address, &thumbnail_address),
            "test-token",
            publisher.outbound(),
        )
        .await;

        assert!(first_outcome.succeeded.is_empty());
        assert!(matches!(
            first_outcome.failed[0].error,
            DisassociateEntryError::AssociationTargetSideRemoval { .. }
        ));
        assert!(
            mock_storage().has_key(&target_address, &source_key),
            "a source-side failure must leave the association discoverable"
        );
        assert!(
            mock_storage().has_key(&thumbnail_address, THUMBNAIL_METADATA_KEY),
            "thumbnail-side metadata is never touched by targeted disassociation"
        );
        assert!(
            publisher
                .settle(std::time::Duration::from_millis(150))
                .await
                .is_empty(),
            "a partially removed entry must not emit a disassociated event"
        );

        let retry_outcome = disassociate_thumbnail(
            &targeted_request(&target_address, &thumbnail_address),
            "test-token",
            publisher.outbound(),
        )
        .await;

        assert_eq!(
            retry_outcome.succeeded.len(),
            1,
            "retry failures: {:?}",
            retry_outcome.failed
        );
        assert!(retry_outcome.failed.is_empty());
        assert!(!mock_storage().has_key(&target_address, &source_key));
        assert_eq!(
            publisher
                .await_events(1, std::time::Duration::from_secs(5))
                .await
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn targeting_a_thumbnail_not_associated_returns_empty_success() {
        use crate::test_fixtures::mock_publisher_server::MockPublisherServer;
        use crate::test_fixtures::mock_storage_server::{mock_storage, unique_test_address};

        crate::test_fixtures::init_test_runtime_config("thumbnail-common-association-tests");

        let (target_address, _seeded_thumbnail, source_key) =
            seed_one_association("not-associated");
        let other_thumbnail_address = unique_test_address("not-associated-other.png");
        let publisher = MockPublisherServer::start().await;

        // Request removal of a thumbnail that is not in the target's association list.
        let outcome = disassociate_thumbnail(
            &targeted_request(&target_address, &other_thumbnail_address),
            "test-token",
            publisher.outbound(),
        )
        .await;

        assert!(outcome.succeeded.is_empty());
        assert!(outcome.failed.is_empty());
        assert!(outcome.aborted.is_none());

        // The existing association is untouched.
        assert!(mock_storage().has_key(&target_address, &source_key));
    }

    #[tokio::test]
    async fn a_successful_removal_emits_exactly_one_disassociated_event() {
        use crate::test_fixtures::mock_publisher_server::MockPublisherServer;
        use crate::test_fixtures::mock_storage_server::mock_storage;

        let (target_address, thumbnail_address, source_key) = seed_one_association("one-event");
        let publisher = MockPublisherServer::start().await;

        let outcome = disassociate_thumbnail(
            &targeted_request(&target_address, &thumbnail_address),
            "test-token",
            publisher.outbound(),
        )
        .await;

        assert_eq!(outcome.succeeded.len(), 1, "failures: {:?}", outcome.failed);
        assert!(!mock_storage().has_key(&target_address, &source_key));
        assert!(
            mock_storage().has_key(&thumbnail_address, THUMBNAIL_METADATA_KEY),
            "thumbnail-side metadata is left intact by targeted disassociation"
        );

        let events = publisher
            .await_events(1, std::time::Duration::from_secs(5))
            .await;
        assert_eq!(
            events.len(),
            1,
            "exactly one event per successful removal, got {events:?}"
        );
        assert_eq!(events[0].event_type, "omni.thumbnail.disassociated");
        assert_eq!(
            events[0].message_str("/thumbnail/resource_address"),
            Some(thumbnail_address.as_str())
        );
        assert_eq!(
            events[0].resource_id,
            format!("{thumbnail_address}?version-id=v1"),
            "a disassociation is filed under the thumbnail that went away"
        );
    }

    #[tokio::test]
    async fn a_downstream_rejected_token_aborts_before_any_removal() {
        use crate::test_fixtures::mock_publisher_server::MockPublisherServer;
        use crate::test_fixtures::mock_storage_server::mock_storage;

        let (target_address, thumbnail_address, source_key) =
            seed_one_association("rejected-token");
        mock_storage().reject_token("expired-token");
        let publisher = MockPublisherServer::start().await;

        let outcome = disassociate_thumbnail(
            &targeted_request(&target_address, &thumbnail_address),
            "expired-token",
            publisher.outbound(),
        )
        .await;

        // A rejected token on enumeration surfaces as an Enumeration abort carrying
        // Unauthenticated; transports turn that into a 401 / UNAUTHENTICATED rather than a 502.
        assert!(
            matches!(
                &outcome.aborted,
                Some(DisassociateAbort::Enumeration(
                    StorageMetadataError::Unauthenticated(_)
                ))
            ),
            "expected an enumeration abort carrying Unauthenticated, got {:?}",
            outcome.aborted
        );
        assert!(
            mock_storage().has_key(&target_address, &source_key),
            "an enumeration failure must not have removed anything"
        );
    }

    #[test]
    fn output_builders_refuse_an_unvalidated_request_instead_of_defaulting_its_identity() {
        crate::test_fixtures::init_test_runtime_config("thumbnail-common-association-tests");

        let base = request(
            "https://storage.example.test/container/assets/source-file.usd",
            "source-id",
            "https://storage.example.test/container/thumbnails/thumb.png",
            "thumb-id",
            AssociationTargetKind::File,
        );

        for (resource, mut req) in [("thumbnail", base.clone()), ("association_target", base)] {
            match resource {
                "thumbnail" => req.thumbnail.resource_identity = None,
                _ => req.association_target.resource_identity = None,
            }
            // validate() is what rejects this for real callers; the builder must not
            // quietly substitute an empty identity for one that reaches it anyway.
            assert!(req.validate().is_err());

            let error = build_association_metadata_writes(
                &req,
                &AssociationSpec::ImageToImage {
                    width: 256,
                    height: 256,
                    encoding_format: EncodingFormat::Png,
                    transform_type: TransformType::Square,
                    crop_strategy: CropStrategy::Center,
                    quality: Some(85.0),
                    preserve_aspect_ratio: Some(true),
                },
                TEST_CREATE_TIME,
            )
            .expect_err("an unvalidated request must not produce a write plan");

            assert!(
                matches!(
                    error,
                    AssociationInvariant::MissingResourceIdentity { resource: r } if r == resource
                ),
                "expected a missing-{resource} invariant, got {error:?}"
            );
        }
    }

    #[test]
    fn folder_targets_have_no_identity_rather_than_a_missing_one() {
        crate::test_fixtures::init_test_runtime_config("thumbnail-common-association-tests");

        let req = request(
            "https://storage.example.test/container/assets/",
            "", // folders have no identity
            "https://storage.example.test/container/thumbnails/thumb.png",
            "thumb-id",
            AssociationTargetKind::Folder,
        );

        assert_eq!(req.association_target_identity(), Ok(""));
        assert!(build_association_metadata_writes(
            &req,
            &AssociationSpec::ImageToImage {
                width: 256,
                height: 256,
                encoding_format: EncodingFormat::Png,
                transform_type: TransformType::Square,
                crop_strategy: CropStrategy::Center,
                quality: Some(85.0),
                preserve_aspect_ratio: Some(true),
            },
            TEST_CREATE_TIME
        )
        .is_ok());
    }

    // -----------------------------------------------------------------
    // Association write consistency: a failed association must not leave a
    // read-visible association behind it.
    // -----------------------------------------------------------------

    /// A valid association request against addresses no other test uses.
    fn association_against_fresh_addresses(label: &str) -> AssociationRequest {
        use crate::test_fixtures::mock_storage_server::unique_test_address;

        crate::test_fixtures::init_test_runtime_config("thumbnail-common-association-tests");

        let target_address = unique_test_address(&format!("{label}-source.usd"));
        let thumbnail_address = unique_test_address(&format!("{label}-thumb.png"));
        request(
            &target_address,
            &format!("{target_address}?version-id=v1"),
            &thumbnail_address,
            &format!("{thumbnail_address}?version-id=v1"),
            AssociationTargetKind::File,
        )
    }

    /// The read path these tests assert through returns canned data in development
    /// mode, which would make them pass or fail for reasons unrelated to the writes.
    /// The fixture pins the mode off; this fails once, and clearly, if that stops
    /// happening.
    #[test]
    fn the_test_runtime_is_not_in_development_mode() {
        crate::test_fixtures::init_test_runtime_config("thumbnail-common-association-tests");

        assert!(
            !crate::current_runtime_config().service.development_mode,
            "development mode serves canned thumbnails, so storage read-backs prove nothing"
        );
    }

    /// What `GET /thumbnails` reports for an association target.
    ///
    /// A target carrying no metadata at all reports as an error rather than an empty
    /// list, and for these tests both mean the same thing: nothing is associated.
    async fn readable_associations(address: &str) -> Vec<String> {
        crate::thumbnails::get_thumbnails_for_resource_with_details(
            address,
            &crate::thumbnails::ThumbnailFilters::default(),
            "test-token",
            None,
        )
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.thumbnail_resource_identity)
        .collect()
    }

    #[tokio::test]
    async fn both_writes_succeeding_leaves_a_readable_association() {
        use crate::test_fixtures::mock_storage_server::mock_storage;

        let req = association_against_fresh_addresses("both-writes-succeed");

        store_association(&req, "test-token", Arc::new(OutboundClients::for_tests()))
            .await
            .expect("both writes should succeed against the storage double");

        assert!(
            mock_storage().has_key(&req.thumbnail.resource_address, THUMBNAIL_METADATA_KEY),
            "the thumbnail's own metadata blob should exist"
        );
        assert_eq!(
            readable_associations(&req.association_target.resource_address)
                .await
                .len(),
            1,
            "the association target should report its one thumbnail"
        );
    }

    #[tokio::test]
    async fn association_canonicalizes_resource_addresses_before_storage_writes() {
        use crate::test_fixtures::mock_storage_server::mock_storage;

        crate::test_fixtures::init_test_runtime_config("thumbnail-common-association-tests");

        let target_raw = "file-storage://fileservice/samples/IMG_2062 (1).jpg";
        let target_canonical = "file-storage://fileservice/samples/IMG_2062%20(1).jpg";
        let thumbnail_raw = "file-storage://fileservice/thumbnails/puppyö.jpg";
        let thumbnail_canonical = "file-storage://fileservice/thumbnails/puppyo%CC%88.jpg";
        let req = request(
            target_raw,
            "file-storage://fileservice/samples/IMG_2062%20(1).jpg?etag=source",
            thumbnail_raw,
            "file-storage://fileservice/thumbnails/puppyo%CC%88.jpg?etag=thumbnail",
            AssociationTargetKind::File,
        );

        store_association(&req, "test-token", Arc::new(OutboundClients::for_tests()))
            .await
            .expect("raw path characters should be canonicalized before Storage metadata calls");

        let target_metadata = mock_storage().metadata(target_canonical);
        let stored = target_metadata
            .values()
            .next()
            .expect("association metadata should be written on the canonical target address");
        assert_eq!(
            stored["association_target"]["resource_address"],
            target_canonical
        );
        assert_eq!(stored["thumbnail"]["resource_address"], thumbnail_canonical);
        assert!(
            mock_storage().has_key(thumbnail_canonical, THUMBNAIL_METADATA_KEY),
            "thumbnail metadata should be written on the canonical thumbnail address"
        );
        assert!(mock_storage().calls_for(target_raw).is_empty());
        assert!(mock_storage().calls_for(thumbnail_raw).is_empty());
    }

    #[tokio::test]
    async fn a_failed_first_write_attempts_nothing_further() {
        use crate::test_fixtures::mock_storage_server::{mock_storage, MetadataOp};

        let req = association_against_fresh_addresses("first-write-fails");
        mock_storage().fail(
            MetadataOp::Update,
            &req.association_target.resource_address,
            None,
            tonic::Code::Internal,
        );

        let error = store_association(&req, "test-token", Arc::new(OutboundClients::for_tests()))
            .await
            .expect_err("a failed association-target-side write should fail the request");

        assert!(matches!(error, AssociationError::StorageWrite(_)));
        assert!(
            mock_storage()
                .calls_for(&req.thumbnail.resource_address)
                .is_empty(),
            "nothing was written to roll back, so the thumbnail side should be untouched"
        );
        assert!(
            readable_associations(&req.association_target.resource_address)
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_failed_second_write_rolls_back_the_first() {
        use crate::test_fixtures::mock_storage_server::{mock_storage, MetadataOp};

        let req = association_against_fresh_addresses("second-write-fails");
        mock_storage().fail(
            MetadataOp::Update,
            &req.thumbnail.resource_address,
            Some(THUMBNAIL_METADATA_KEY),
            tonic::Code::Internal,
        );

        let error = store_association(&req, "test-token", Arc::new(OutboundClients::for_tests()))
            .await
            .expect_err("a failed thumbnail-side write should fail the request");

        assert!(
            matches!(error, AssociationError::StorageWrite(_)),
            "a rolled-back write reports the original failure, not a rollback failure: {error}"
        );
        assert!(
            mock_storage()
                .metadata(&req.association_target.resource_address)
                .is_empty(),
            "the association-target-side key written moments earlier should have been removed"
        );
        assert!(
            readable_associations(&req.association_target.resource_address)
                .await
                .is_empty(),
            "a failed association must not be readable as a successful one"
        );
    }

    #[tokio::test]
    async fn a_rollback_finding_the_key_already_gone_is_a_successful_rollback() {
        use crate::test_fixtures::mock_storage_server::{mock_storage, MetadataOp};

        let req = association_against_fresh_addresses("rollback-already-gone");
        mock_storage().fail(
            MetadataOp::Update,
            &req.thumbnail.resource_address,
            Some(THUMBNAIL_METADATA_KEY),
            tonic::Code::Internal,
        );
        // Something else removed the key first — the rollback's goal, reached without it.
        mock_storage().fail(
            MetadataOp::Delete,
            &req.association_target.resource_address,
            None,
            tonic::Code::NotFound,
        );

        let error = store_association(&req, "test-token", Arc::new(OutboundClients::for_tests()))
            .await
            .expect_err("a failed thumbnail-side write should fail the request");

        assert!(
            matches!(error, AssociationError::StorageWrite(_)),
            "an absent key means the rollback succeeded, so the original error stands: {error}"
        );
    }

    #[tokio::test]
    async fn a_failed_rollback_names_both_failures_and_says_what_to_clean_up() {
        use crate::test_fixtures::mock_storage_server::{mock_storage, MetadataOp};

        let req = association_against_fresh_addresses("rollback-fails");
        mock_storage().fail(
            MetadataOp::Update,
            &req.thumbnail.resource_address,
            Some(THUMBNAIL_METADATA_KEY),
            tonic::Code::Internal,
        );
        mock_storage().fail(
            MetadataOp::Delete,
            &req.association_target.resource_address,
            None,
            tonic::Code::Internal,
        );

        let error = store_association(&req, "test-token", Arc::new(OutboundClients::for_tests()))
            .await
            .expect_err("a failed thumbnail-side write should fail the request");

        let AssociationError::UncompensatedWrite {
            resource_address,
            key,
            ..
        } = &error
        else {
            panic!("expected an uncompensated-write error, got {error:?}");
        };

        // The address and key an operator needs to reconcile the leftover by hand.
        assert_eq!(resource_address, &req.association_target.resource_address);
        assert!(mock_storage().has_key(&req.association_target.resource_address, key));

        let message = error.to_string();
        assert!(
            message.contains("could not be rolled back"),
            "the message should name the rollback failure: {message}"
        );
        assert!(
            message.starts_with("Storage service call failed"),
            "the message should lead with the original write failure so retry behavior is \
             driven by the same cause: {message}"
        );
    }

    #[tokio::test]
    async fn a_token_the_downstream_rejects_is_an_authentication_failure() {
        use crate::test_fixtures::mock_storage_server::mock_storage;

        let req = association_against_fresh_addresses("associate-expired-token");
        mock_storage().reject_token("associate-expired-token");

        let error = store_association(
            &req,
            "associate-expired-token",
            Arc::new(OutboundClients::for_tests()),
        )
        .await
        .expect_err("a rejected token should fail the association");

        assert_eq!(
            error.failure(),
            AssociationFailure::Unauthenticated,
            "a rejected token is the caller's to fix, not an internal fault: {error}"
        );
    }

    // A token can expire between the two writes, which fails the second write and then
    // the rollback that follows it. The caller still needs a new token, so the leftover
    // key must not turn the answer into a generic internal error.
    #[tokio::test]
    async fn a_rejected_token_that_also_blocks_the_rollback_stays_an_authentication_failure() {
        use crate::test_fixtures::mock_storage_server::{mock_storage, MetadataOp};

        let req = association_against_fresh_addresses("rollback-after-expiry");
        mock_storage().fail(
            MetadataOp::Update,
            &req.thumbnail.resource_address,
            Some(THUMBNAIL_METADATA_KEY),
            tonic::Code::Unauthenticated,
        );
        mock_storage().fail(
            MetadataOp::Delete,
            &req.association_target.resource_address,
            None,
            tonic::Code::Unauthenticated,
        );

        let error = store_association(&req, "test-token", Arc::new(OutboundClients::for_tests()))
            .await
            .expect_err("a failed thumbnail-side write should fail the request");

        assert!(
            matches!(error, AssociationError::UncompensatedWrite { .. }),
            "the leftover key should still be reported: {error}"
        );
        assert_eq!(
            error.failure(),
            AssociationFailure::Unauthenticated,
            "a failed rollback follows the classification of the write that caused it: {error}"
        );
    }

    #[tokio::test]
    async fn a_storage_failure_unrelated_to_the_token_stays_internal() {
        use crate::test_fixtures::mock_storage_server::{mock_storage, MetadataOp};

        let req = association_against_fresh_addresses("associate-upstream-down");
        mock_storage().fail(
            MetadataOp::Update,
            &req.association_target.resource_address,
            None,
            tonic::Code::Internal,
        );

        let error = store_association(&req, "test-token", Arc::new(OutboundClients::for_tests()))
            .await
            .expect_err("a failed write should fail the association");

        assert_eq!(
            error.failure(),
            AssociationFailure::Internal,
            "only a rejected token should be reported as an authentication failure: {error}"
        );
    }

    #[tokio::test]
    async fn an_enumeration_rejected_by_storage_aborts_on_authentication() {
        use crate::test_fixtures::mock_storage_server::{mock_storage, unique_test_address};

        crate::test_fixtures::init_test_runtime_config("thumbnail-common-association-tests");

        let target_address = unique_test_address("enumeration-auth-source.usd");
        let thumbnail_address = unique_test_address("enumeration-auth-thumb.png");
        mock_storage().reject_token("enumeration-auth-expired-token");

        let outcome = disassociate_thumbnail(
            &targeted_request(&target_address, &thumbnail_address),
            "enumeration-auth-expired-token",
            Arc::new(OutboundClients::for_tests()),
        )
        .await;

        let abort = outcome
            .aborted
            .expect("a rejected token should abort before any removal");
        assert!(
            abort.is_unauthenticated(),
            "should name the token as the cause, not a generic outage: {abort}"
        );
    }

    #[test]
    fn a_disassociate_request_validates_both_addresses() {
        for kind in [AssociationTargetKind::File, AssociationTargetKind::Folder] {
            // Valid addresses should be accepted regardless of target_kind.
            let req = targeted_request(
                "https://store.test/source.usd",
                "https://store.test/thumb.png",
            );
            let mut req = DisassociateRequest {
                target_kind: kind,
                ..req
            };
            assert_eq!(req.validate(), Ok(()));

            // Blank association_target address is rejected.
            for blank in ["", "   "] {
                req.association_target_resource_address = blank.to_string();
                req.thumbnail_resource_address = "https://store.test/thumb.png".to_string();
                assert_eq!(
                    req.validate(),
                    Err(
                        DisassociateValidationError::InvalidAssociationTargetResource(
                            ResourceValidationError::MissingAddress
                        )
                    )
                );
            }

            // Blank thumbnail address is rejected.
            req.association_target_resource_address = "https://store.test/source.usd".to_string();
            for blank in ["", "   "] {
                req.thumbnail_resource_address = blank.to_string();
                assert_eq!(
                    req.validate(),
                    Err(DisassociateValidationError::InvalidThumbnailResource(
                        ResourceValidationError::MissingAddress
                    ))
                );
            }
        }
    }

    // Both transports validate first, so this only fires for a caller that skipped it —
    // which is the point of keeping the check inside the function.
    #[tokio::test]
    async fn a_blank_address_aborts_before_calling_storage() {
        use crate::test_fixtures::mock_storage_server::mock_storage;

        crate::test_fixtures::init_test_runtime_config("thumbnail-common-association-tests");

        let outcome = disassociate_thumbnail(
            &targeted_request("   ", "https://store.test/thumb.png"),
            "test-token",
            Arc::new(OutboundClients::for_tests()),
        )
        .await;

        assert!(matches!(
            outcome.aborted,
            Some(DisassociateAbort::Validation(_))
        ));
        assert!(mock_storage().calls_for("   ").is_empty());
    }

    #[test]
    fn a_sweep_aborted_by_an_unreachable_upstream_is_not_an_authentication_failure() {
        let abort = DisassociateAbort::Enumeration(StorageMetadataError::Failed(
            "upstream down".to_string(),
        ));

        assert!(!abort.is_unauthenticated());
    }

    /// Task 3.4: MANUAL round-trips through serde_json to the flat typed shape
    /// with exactly three fields — no conversion fields (transform, crop, quality,
    /// preserve_aspect_ratio) are present.
    #[test]
    fn association_spec_manual_round_trips_to_flat_typed_json() {
        let spec = AssociationSpec::Manual {
            width: 512,
            height: 384,
            encoding_format: EncodingFormat::Jpeg,
        };

        let json = serde_json::to_value(&spec).expect("MANUAL must serialize");

        assert_eq!(json["type"], "MANUAL");
        assert_eq!(json["width"], 512);
        assert_eq!(json["height"], 384);
        assert_eq!(json["encoding_format"], "JPEG");

        // No conversion fields present.
        for unexpected in [
            "transform_type",
            "crop_strategy",
            "quality",
            "preserve_aspect_ratio",
        ] {
            assert!(
                json.get(unexpected).is_none(),
                "MANUAL should not carry {unexpected}"
            );
        }

        // Round-trip back to Rust.
        let round_tripped: AssociationSpec =
            serde_json::from_value(json).expect("MANUAL must deserialize");
        assert_eq!(round_tripped, spec);
    }

    #[test]
    fn association_spec_rejects_fields_outside_the_selected_variant() {
        let image = serde_json::json!({
            "type": "IMAGE_TO_IMAGE",
            "width": 256,
            "height": 256,
            "encoding_format": "PNG",
            "transform_type": "SQUARE",
            "crop_strategy": "CENTER",
            "processor_identity": "IMAGE_TO_IMAGE"
        });
        assert!(
            serde_json::from_value::<AssociationSpec>(image).is_err(),
            "the removed processor_identity field must not be silently ignored"
        );

        let manual = serde_json::json!({
            "type": "MANUAL",
            "width": 256,
            "height": 256,
            "encoding_format": "PNG",
            "quality": 85
        });
        assert!(
            serde_json::from_value::<AssociationSpec>(manual).is_err(),
            "MANUAL must reject conversion-only fields"
        );
    }

    /// The classification a caller keys its acknowledgements off.
    ///
    /// Getting this wrong is expensive in both directions: calling a permanent
    /// failure retryable makes the worker repeat the whole conversion pipeline
    /// until the platform dead-letters the delivery, and calling a transient one
    /// permanent loses the association outright.
    #[test]
    fn association_failures_are_classified_for_retry() {
        assert!(
            !AssociationError::Validation(AssociationValidationError::MissingSpecification)
                .is_retryable(),
            "an invalid request reproduces exactly on replay"
        );
        assert!(
            !AssociationError::StorageWrite(StorageMetadataError::NotFound).is_retryable(),
            "an association target that does not exist will keep not existing"
        );
        assert!(
            !AssociationError::StorageWrite(StorageMetadataError::PermissionDenied(
                "denied".to_string()
            ))
            .is_retryable(),
            "a permission decision is not a transient fault"
        );
        assert!(
            AssociationError::StorageWrite(StorageMetadataError::Failed(
                "storage down".to_string()
            ))
            .is_retryable(),
            "a generic storage fault is the case worth retrying"
        );
    }

    /// A failed rollback is classified by the write that caused it, because that
    /// is what decides whether a retry can succeed. The stranded key is an
    /// operator's concern and is already logged as one.
    #[test]
    fn an_uncompensated_write_is_classified_by_its_write_error() {
        let permanent = AssociationError::UncompensatedWrite {
            write_error: StorageMetadataError::NotFound,
            compensation_error: StorageMetadataError::Failed("rollback failed".to_string()),
            resource_address: "https://store.test/source.png".to_string(),
            key: "thumbnail_256x256_PNG_abc".to_string(),
        };
        assert!(!permanent.is_retryable());

        let retryable = AssociationError::UncompensatedWrite {
            write_error: StorageMetadataError::Failed("storage down".to_string()),
            compensation_error: StorageMetadataError::Failed("rollback failed".to_string()),
            resource_address: "https://store.test/source.png".to_string(),
            key: "thumbnail_256x256_PNG_abc".to_string(),
        };
        assert!(retryable.is_retryable());
    }
}
