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

use std::sync::Arc;

use discovery_proto::nvidia::omniverse::discovery::capabilities::v2alpha::FILE_DESCRIPTOR_SET as DISCOVERY_FILE_DESCRIPTOR_SET;
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha as proto;

use thumbnail_common::{
    create_thumbnails, disassociate_thumbnail, store_association, AssociationError,
    AssociationFailure, AssociationRequest, AssociationResource, AssociationSpec,
    AssociationTargetKind, CreateThumbnailsError, DisassociateRequest, FailedSubmission,
    OutboundClients,
};
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::{
    thumbnail_service_server::ThumbnailService, AssociateThumbnailRequest,
    AssociateThumbnailResponse, CreateThumbnailFailure, CreateThumbnailsRequest,
    CreateThumbnailsResponse, DisassociateResult, DisassociateThumbnailRequest,
    DisassociateThumbnailResponse,
};
use tonic::{Request, Response, Status};
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct ThumbnailServiceImpl {
    /// Shared outbound-auth + gRPC plumbing constructed at startup.
    pub outbound: Arc<OutboundClients>,
}

impl ThumbnailServiceImpl {
    pub fn new(outbound: Arc<OutboundClients>) -> Self {
        Self { outbound }
    }
}

pub mod capabilities;
pub use capabilities::CapabilitiesServiceImpl;

#[cfg(test)]
mod tests;

/// Every descriptor set this server hosts a service for, registered in one place
/// so both reflection protocol versions describe the same server.
///
/// `build_v1` and `build_v1alpha` each consume the builder, so a caller takes a
/// fresh one per protocol version. Building either fails when a registered set
/// is missing a transitive dependency, which is why both are built explicitly
/// and covered by a test rather than assumed to work.
pub fn reflection_builder() -> tonic_reflection::server::Builder<'static> {
    tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .register_encoded_file_descriptor_set(DISCOVERY_FILE_DESCRIPTOR_SET)
        // The health service is registered on the server as well, so reflection
        // should be able to describe it too.
        .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
}

/// The single place an association failure becomes a gRPC associate outcome.
///
/// Associate reports failures in-band, as `success: false` carrying the returned message.
/// A rejected token is the exception: it gets an `UNAUTHENTICATED` status, so a caller
/// that handles auth once for every RPC sees it where it already looks.
fn associate_error_message(error: &AssociationError) -> Result<String, Status> {
    match error.failure() {
        AssociationFailure::Unauthenticated => Err(Status::unauthenticated(error.to_string())),
        AssociationFailure::Internal => Ok(error.to_string()),
    }
}

// gRPC auth arrives in request metadata rather than an HTTP header map.
fn extract_auth_token<T>(request: &tonic::Request<T>) -> Option<String> {
    request
        .metadata()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(crate::bearer_token_from_header)
}

/// Parse a proto enum field that must be present and not its `UNSPECIFIED`
/// variant. In proto3 the unspecified variant is always discriminant `0`, so a
/// zero value is reported with `unspecified_msg` and an out-of-range value with
/// `invalid_msg`. Returns `String` so the caller can map to any error type.
fn require_specified_enum<T: TryFrom<i32>>(
    raw: i32,
    invalid_msg: &str,
    unspecified_msg: &str,
) -> Result<T, String> {
    if raw == 0 {
        return Err(unspecified_msg.to_string());
    }
    T::try_from(raw).map_err(|_| invalid_msg.to_string())
}

/// Convert a gRPC `AssociateThumbnailRequest` into the common `AssociationRequest`
/// type. Validates proto-specific invariants (UNSPECIFIED enum variants) and passes
/// the additive optional fields (`quality`, `preserve_aspect_ratio`) through with
/// their presence preserved; omitted values are resolved to runtime defaults by the
/// shared `AssociationRequest::normalize_specification` seam rather than here.
///
/// Auth is not part of this conversion — it remains transport metadata and is
/// supplied separately to `store_association`.
fn association_request_from_proto(
    req: &proto::AssociateThumbnailRequest,
) -> Result<AssociationRequest, String> {
    let thumbnail = req
        .thumbnail
        .as_ref()
        .ok_or_else(|| "thumbnail resource is required".to_string())?;
    let association_target = req
        .association_target
        .as_ref()
        .ok_or_else(|| "association_target resource is required".to_string())?;

    let target_kind = AssociationTargetKind::try_from(req.target_type.unwrap_or_default())?;

    let specification = match req
        .specification
        .as_ref()
        .and_then(|spec| spec.specification.as_ref())
    {
        Some(proto::thumbnail_specification::Specification::ImageToImage(image_spec)) => {
            let processor_identity = require_specified_enum::<proto::ProcessorIdentity>(
                image_spec.processor_identity,
                "specification.image_to_image.processor_identity is invalid",
                "specification.image_to_image.processor_identity must not be UNSPECIFIED",
            )?;
            if processor_identity != proto::ProcessorIdentity::ImageToImage {
                return Err(
                    "specification.image_to_image.processor_identity must be IMAGE_TO_IMAGE"
                        .to_string(),
                );
            }
            let encoding_format: thumbnail_common::EncodingFormat =
                require_specified_enum::<proto::ImageEncodingFormat>(
                    image_spec.encoding_format,
                    "specification.image_to_image.encoding_format is invalid",
                    "specification.image_to_image.encoding_format must not be UNSPECIFIED",
                )
                .map(Into::into)?;
            let transform_type: thumbnail_common::TransformType =
                require_specified_enum::<proto::TransformType>(
                    image_spec.transform_type,
                    "specification.image_to_image.transform_type is invalid",
                    "specification.image_to_image.transform_type must not be UNSPECIFIED",
                )
                .map(Into::into)?;
            let crop_strategy: thumbnail_common::CropStrategy =
                require_specified_enum::<proto::CropStrategy>(
                    image_spec.crop_strategy,
                    "specification.image_to_image.crop_strategy is invalid",
                    "specification.image_to_image.crop_strategy must not be UNSPECIFIED",
                )
                .map(Into::into)?;

            // Pass the additive optional fields through verbatim, preserving
            // presence: an omitted value stays `None` and is resolved to the
            // runtime default by `AssociationRequest::normalize_specification`
            // in the shared store path — the gRPC handler no longer injects
            // transport-specific defaults.
            Some(AssociationSpec::ImageToImage {
                width: image_spec.width,
                height: image_spec.height,
                encoding_format,
                transform_type,
                crop_strategy,
                quality: image_spec.quality.map(f64::from),
                preserve_aspect_ratio: image_spec.preserve_aspect_ratio,
            })
        }
        Some(proto::thumbnail_specification::Specification::Manual(manual_spec)) => {
            let encoding_format: thumbnail_common::EncodingFormat =
                require_specified_enum::<proto::ImageEncodingFormat>(
                    manual_spec.encoding_format,
                    "specification.manual.encoding_format is invalid",
                    "specification.manual.encoding_format must not be UNSPECIFIED",
                )
                .map(Into::into)?;
            Some(AssociationSpec::Manual {
                width: manual_spec.width,
                height: manual_spec.height,
                encoding_format,
            })
        }
        None => None,
    };

    Ok(AssociationRequest {
        target_kind,
        thumbnail: AssociationResource {
            resource_address: thumbnail.resource_address.clone(),
            resource_identity: thumbnail.resource_identity.clone(),
        },
        association_target: AssociationResource {
            resource_address: association_target.resource_address.clone(),
            resource_identity: association_target.resource_identity.clone(),
        },
        specification,
    })
}

/// Convert a gRPC `GetThumbnailsRequest` into the same canonical request the HTTP
/// route parses its query string into.
///
/// Going through the HTTP type rather than building `ThumbnailFilters` directly is
/// what makes the two transports filter identically: the field-to-filter mapping
/// and `validate()` are each written once, so a filter that narrows on one
/// transport narrows the same way on the other. `disassociate_request_from_proto`
/// takes the same approach with its canonical request.
///
/// An `UNSPECIFIED` enum is no filter, not an error. Proto3 cannot distinguish a
/// caller that set the zero variant from one that set nothing meaningful, and the
/// useful reading of "no specification type" on a filter field is "do not narrow".
/// A value outside the enum is different — that is a caller sending something this
/// service does not define, and it is reported.
fn get_thumbnails_request_from_proto(
    req: &proto::GetThumbnailsRequest,
) -> Result<crate::http::GetThumbnailsRequest, String> {
    /// `None` for absent or `UNSPECIFIED`; an error for an undefined value.
    fn optional_filter<P, T>(raw: Option<i32>, field: &str) -> Result<Option<T>, String>
    where
        P: TryFrom<i32> + Into<T>,
    {
        match raw {
            None | Some(0) => Ok(None),
            Some(value) => P::try_from(value)
                .map(|parsed| Some(parsed.into()))
                .map_err(|_| format!("{field} is invalid")),
        }
    }

    let specification = match req.specification {
        None | Some(0) => None,
        Some(value) => match proto::ThumbnailSpecificationType::try_from(value) {
            Ok(proto::ThumbnailSpecificationType::ImageToImage) => {
                Some(crate::http::ThumbnailSpecificationType::ImageToImage)
            }
            // Unreachable while UNSPECIFIED is the only other variant, and
            // answered rather than asserted so adding a variant to the proto
            // without teaching this mapping is a rejected filter, not a filter
            // that silently stops narrowing.
            Ok(_) | Err(_) => return Err("specification is invalid".to_string()),
        },
    };

    let encoding_format: Option<thumbnail_common::EncodingFormat> =
        optional_filter::<proto::ImageEncodingFormat, _>(req.encoding_format, "encoding_format")?;
    let transform_type: Option<thumbnail_common::TransformType> =
        optional_filter::<proto::TransformType, _>(req.transform_type, "transform_type")?;
    let crop_strategy: Option<thumbnail_common::CropStrategy> =
        optional_filter::<proto::CropStrategy, _>(req.crop_strategy, "crop_strategy")?;

    Ok(crate::http::GetThumbnailsRequest {
        association_target_resource_address: req.association_target_resource_address.clone(),
        specification,
        width: req.width,
        height: req.height,
        encoding_format: encoding_format.map(Into::into),
        transform_type: transform_type.map(Into::into),
        crop_strategy: crop_strategy.map(Into::into),
    })
}

/// Render one shared `ThumbnailEntry` as its proto counterpart.
///
/// The stored specification crosses as a `Struct`, which can hold a JSON object
/// and nothing else. Association metadata is always written with an object there,
/// so anything else is a malformed or legacy record — and one of those must not
/// fail the read. It is omitted from its own entry instead, which is why the field
/// is optional and why the entry's addresses still come back: the same rule
/// `thumbnail_entries_from_matches` applies to a record it cannot read at all, and
/// the rule the disassociate sweep applies to one it cannot remove.
///
/// Returning `INTERNAL` here instead — which this did — meant one bad record on a
/// target made every good thumbnail on it unreadable over gRPC while HTTP returned
/// them, so the transports disagreed about whether the target had any thumbnails.
fn get_thumbnails_entry_to_proto(
    entry: crate::http::ThumbnailEntry,
) -> proto::GetThumbnailsThumbnailEntry {
    let specification = entry.specification.and_then(|value| {
        match thumbnail_common::utils::json_to_prost_value(&value) {
            Ok(prost_types::Value {
                kind: Some(prost_types::value::Kind::StructValue(fields)),
            }) => Some(fields),
            converted => {
                // The JSON kind only, never the value: a stored specification can
                // carry a signed address, and the read path already declines to
                // put those in logs. The kind is what says which record shape to
                // go looking for.
                warn!(
                    stored_kind = json_kind_name(&value),
                    encodable = converted.is_ok(),
                    "Omitting a stored thumbnail specification that is not a JSON object; \
                     the entry and the remaining thumbnails are unaffected"
                );
                None
            }
        }
    });

    proto::GetThumbnailsThumbnailEntry {
        target_type: match entry.target_type {
            crate::http::TargetType::File => proto::TargetType::File as i32,
            crate::http::TargetType::Folder => proto::TargetType::Folder as i32,
        },
        association_target: Some(proto::Resource {
            resource_address: entry.association_target.resource_address,
            resource_identity: entry.association_target.resource_identity,
        }),
        thumbnail: Some(proto::Resource {
            resource_address: entry.thumbnail.resource_address,
            resource_identity: entry.thumbnail.resource_identity,
        }),
        specification,
        create_time: entry.create_time,
    }
}

/// The JSON kind of `value`, for a log line that must not carry the value itself.
fn json_kind_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Map a read failure to a gRPC status, mirroring the HTTP route's statuses.
///
/// `NotFound` is handled by the caller as an empty success and never reaches
/// here: a target with no thumbnails is a successful read of nothing.
fn get_thumbnails_error_to_status(error: thumbnail_common::ThumbnailQueryError) -> Status {
    use thumbnail_common::ThumbnailQueryError;
    match error {
        ThumbnailQueryError::NotFound => {
            debug_assert!(false, "NotFound is answered as an empty success");
            Status::internal("unreachable: NotFound is answered as an empty success")
        }
        ThumbnailQueryError::Unauthenticated => {
            Status::unauthenticated("authentication token was rejected by storage")
        }
        // The token was accepted and the authorization decision failed, so
        // UNAUTHENTICATED would invite a refresh that cannot help.
        ThumbnailQueryError::PermissionDenied => {
            Status::permission_denied("access to the requested resource was denied")
        }
        // An upstream fault worth retrying, which is what UNAVAILABLE says and
        // INTERNAL does not.
        ThumbnailQueryError::Storage(_) => Status::unavailable("storage service is unavailable"),
    }
}

/// Convert a gRPC `DisassociateThumbnailRequest` into the common `DisassociateRequest`.
///
/// `resource_identity` on either resource is forwarded but not used for matching — only
/// resource addresses are used to locate and remove the association entry.
fn disassociate_request_from_proto(
    req: &proto::DisassociateThumbnailRequest,
) -> Result<DisassociateRequest, String> {
    let association_target = req
        .association_target
        .as_ref()
        .ok_or_else(|| "association_target resource is required".to_string())?;

    let thumbnail = req
        .thumbnail
        .as_ref()
        .ok_or_else(|| "thumbnail resource is required".to_string())?;

    Ok(DisassociateRequest {
        target_kind: AssociationTargetKind::try_from(req.target_type.unwrap_or_default())?,
        association_target_resource_address: association_target.resource_address.clone(),
        association_target_resource_identity: association_target.resource_identity.clone(),
        thumbnail_resource_address: thumbnail.resource_address.clone(),
        thumbnail_resource_identity: thumbnail.resource_identity.clone(),
    })
}

/// Convert proto `ThumbnailSpecification` slice to `ImageProcessingSpec` values.
/// Returns an error if any spec is not `image_to_image`, so callers can surface
/// a single `INVALID_ARGUMENT` before touching storage or the task queue.
fn create_thumbnails_specs_from_proto(
    proto_specs: &[proto::ThumbnailSpecification],
) -> Result<Vec<thumbnail_common::ImageProcessingSpec>, String> {
    use proto::thumbnail_specification::Specification;

    proto_specs
        .iter()
        .enumerate()
        .map(|(i, spec)| match spec.specification.as_ref() {
            Some(Specification::ImageToImage(image_spec)) => {
                // These have to be built, not written as literals: the arms below
                // interpolate `i`, and a caller sending several specifications needs to
                // know which one it got wrong.
                let invalid =
                    |field| format!("specifications[{i}].image_to_image.{field} is invalid");
                let unspecified = |field| {
                    format!("specifications[{i}].image_to_image.{field} must not be UNSPECIFIED")
                };

                let encoding_format: thumbnail_common::EncodingFormat =
                    require_specified_enum::<proto::ImageEncodingFormat>(
                        image_spec.encoding_format,
                        &invalid("encoding_format"),
                        &unspecified("encoding_format"),
                    )
                    .map(Into::into)?;
                let transform_type: thumbnail_common::TransformType =
                    require_specified_enum::<proto::TransformType>(
                        image_spec.transform_type,
                        &invalid("transform_type"),
                        &unspecified("transform_type"),
                    )
                    .map(Into::into)?;
                let crop_strategy: thumbnail_common::CropStrategy =
                    require_specified_enum::<proto::CropStrategy>(
                        image_spec.crop_strategy,
                        &invalid("crop_strategy"),
                        &unspecified("crop_strategy"),
                    )
                    .map(Into::into)?;

                // Range-checked here rather than left to the work-event contract:
                // reaching that layer means the source Stat has already been paid
                // for, and its rejection surfaces as the status reserved for
                // "every publish failed" — which tells the caller the service is
                // broken when the input was. Same bound the read path enforces.
                for (field, value) in [
                    ("width", image_spec.width),
                    ("height", image_spec.height),
                ] {
                    if !(1..=thumbnail_common::work_events::MAX_DIMENSION).contains(&value) {
                        return Err(format!(
                            "specifications[{i}].image_to_image.{field} must be between 1 and {}",
                            thumbnail_common::work_events::MAX_DIMENSION
                        ));
                    }
                }

                if encoding_format.requires_lossy_quality_range()
                    && image_spec.quality.is_some_and(|quality| !(1..=100).contains(&quality))
                {
                    return Err(format!(
                        "specifications[{i}].image_to_image.quality must be between 1 and 100 for JPEG/WebP"
                    ));
                }

                Ok(thumbnail_common::ImageProcessingSpec {
                    width: Some(image_spec.width),
                    height: Some(image_spec.height),
                    encoding_format: Some(encoding_format),
                    transform_type: Some(transform_type),
                    crop_strategy: Some(crop_strategy),
                    quality: image_spec.quality.map(|q| q as i32),
                    preserve_aspect_ratio: image_spec.preserve_aspect_ratio,
                })
            }
            Some(Specification::Manual(_)) => Err(format!(
                "specifications[{i}]: MANUAL specification is not supported for CreateThumbnails; \
                 use AssociateThumbnail to register a pre-generated thumbnail"
            )),
            None => Err(format!(
                "specifications[{i}]: specification oneof must be set (received empty ThumbnailSpecification)"
            )),
        })
        .collect()
}

/// Convert a `FailedSubmission` to the proto `CreateThumbnailFailure` message.
fn failed_submission_to_proto(f: &FailedSubmission) -> CreateThumbnailFailure {
    use proto::thumbnail_specification::Specification;
    use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha as proto;

    let spec = &f.specification;

    let image_spec = proto::ImageToImageSpecification {
        // Failed submissions always come from the IMAGE_TO_IMAGE path.
        processor_identity: proto::ProcessorIdentity::ImageToImage as i32,
        encoding_format: spec
            .encoding_format
            .map(|e| proto::ImageEncodingFormat::from(e) as i32)
            .unwrap_or_default(),
        width: spec.width.unwrap_or(0),
        height: spec.height.unwrap_or(0),
        transform_type: spec
            .transform_type
            .map(|t| proto::TransformType::from(t) as i32)
            .unwrap_or_default(),
        crop_strategy: spec
            .crop_strategy
            .map(|c| proto::CropStrategy::from(c) as i32)
            .unwrap_or_default(),
        quality: spec.quality.and_then(|q| u32::try_from(q).ok()),
        preserve_aspect_ratio: spec.preserve_aspect_ratio,
    };

    CreateThumbnailFailure {
        specification: Some(proto::ThumbnailSpecification {
            specification: Some(Specification::ImageToImage(image_spec)),
        }),
        error_message: f.error_message.clone(),
    }
}

/// Map a `CreateThumbnailsError` to a gRPC `Status`.
fn create_thumbnails_error_to_status(e: CreateThumbnailsError) -> Status {
    match e {
        CreateThumbnailsError::InvalidArgument(msg) => Status::invalid_argument(msg),
        CreateThumbnailsError::NotFound(msg) => Status::not_found(msg),
        CreateThumbnailsError::Unauthenticated(msg) => Status::unauthenticated(msg),
        CreateThumbnailsError::PermissionDenied(msg) => Status::permission_denied(msg),
        CreateThumbnailsError::AllTasksFailed(msg) => Status::internal(msg),
    }
}

#[tonic::async_trait]
impl ThumbnailService for ThumbnailServiceImpl {
    async fn associate_thumbnail(
        &self,
        request: Request<AssociateThumbnailRequest>,
    ) -> Result<Response<AssociateThumbnailResponse>, Status> {
        let auth_token = extract_auth_token(&request)
            .ok_or_else(|| Status::unauthenticated("authentication token is required"))?;

        let assoc_req =
            association_request_from_proto(request.get_ref()).map_err(Status::invalid_argument)?;

        assoc_req
            .validate()
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        info!(
            "associate_thumbnail: association_target={} thumbnail={} target_kind={:?}",
            assoc_req.association_target.resource_address,
            assoc_req.thumbnail.resource_address,
            assoc_req.target_kind
        );

        // `thumbnail` and `association_target` are left unset on both paths: they only
        // ever echoed the request back, and no caller read them. The fields remain in
        // the proto so existing clients keep parsing, but nothing populates them.
        match store_association(&assoc_req, &auth_token, self.outbound.clone()).await {
            Ok(()) => Ok(Response::new(AssociateThumbnailResponse {
                success: true,
                error_message: None,
                ..Default::default()
            })),
            Err(e) => {
                error!("Failed to associate thumbnail: {}", e);
                // Classified so a caller deciding acknowledgements does not have
                // to match on the prose in error_message. task-worker withheld
                // its acknowledgement for every in-band rejection because it
                // could not tell the two apart, which meant a permanently
                // rejected association repeated the whole conversion pipeline on
                // every redelivery until the platform dead-lettered it.
                let failure_kind = if e.is_retryable() {
                    proto::AssociationFailureKind::Retryable
                } else {
                    proto::AssociationFailureKind::Permanent
                };
                Ok(Response::new(AssociateThumbnailResponse {
                    success: false,
                    error_message: Some(associate_error_message(&e)?),
                    failure_kind: Some(failure_kind as i32),
                    ..Default::default()
                }))
            }
        }
    }

    async fn disassociate_thumbnail(
        &self,
        request: Request<DisassociateThumbnailRequest>,
    ) -> Result<Response<DisassociateThumbnailResponse>, Status> {
        let auth_token = extract_auth_token(&request)
            .ok_or_else(|| Status::unauthenticated("authentication token is required"))?;
        // At full parity with HTTP: same canonical request, same validate(), same
        // address-only rule for every target_type.
        let disassoc_req =
            disassociate_request_from_proto(request.get_ref()).map_err(Status::invalid_argument)?;

        disassoc_req
            .validate()
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        info!(
            "disassociate_thumbnail: thumbnail={} association_target={} target_kind={:?}",
            disassoc_req.thumbnail_resource_address,
            disassoc_req.association_target_resource_address,
            disassoc_req.target_kind
        );

        let outcome =
            disassociate_thumbnail(&disassoc_req, &auth_token, self.outbound.clone()).await;

        // Failing to enumerate the target is the only outcome with no targeted result
        // to report, so it surfaces as an error Status. This also disambiguates it from
        // a thumbnail that is not associated, which succeeds with both arrays empty.
        // A removal failure returns Ok with one `failed` entry so the client receives
        // its detail. HTTP instead returns 502 with the same body. See
        // DisassociateThumbnail in the proto for the divergence and retry guidance.
        //
        // An abort the caller can act on keeps its own code: a rejected token is
        // UNAUTHENTICATED, not the UNAVAILABLE that would have them retry and wait.
        if let Some(abort) = &outcome.aborted {
            return Err(if abort.is_unauthenticated() {
                Status::unauthenticated(abort.to_string())
            } else {
                Status::unavailable(
                    "disassociate could not enumerate the association target; no entry was inspected",
                )
            });
        }

        let succeeded = outcome
            .succeeded
            .iter()
            .map(|t| DisassociateResult {
                thumbnail: Some(proto::Resource {
                    resource_address: t.resource_address.clone(),
                    resource_identity: t.resource_identity.clone(),
                }),
                error_message: None,
            })
            .collect();
        let failed = outcome
            .failed
            .iter()
            .map(|failure| DisassociateResult {
                thumbnail: Some(proto::Resource {
                    resource_address: failure.thumbnail.resource_address.clone(),
                    resource_identity: failure.thumbnail.resource_identity.clone(),
                }),
                error_message: Some(failure.error.to_string()),
            })
            .collect();

        Ok(Response::new(DisassociateThumbnailResponse {
            succeeded,
            failed,
        }))
    }

    async fn create_thumbnails(
        &self,
        request: Request<CreateThumbnailsRequest>,
    ) -> Result<Response<CreateThumbnailsResponse>, Status> {
        let auth_token = extract_auth_token(&request)
            .ok_or_else(|| Status::unauthenticated("authentication token is required"))?;

        let req = request.get_ref();

        // Validate resource_address.
        if req.resource_address.trim().is_empty() {
            return Err(Status::invalid_argument(
                "resource_address must not be empty",
            ));
        }

        // Bound the fan-out before the Stat, for the same reason the dimension
        // range check sits there: a request the service is going to refuse should
        // cost no outbound call. HTTP enforced this and gRPC enforced nothing, so
        // the same request was legal on one transport and not the other; the
        // number now lives in `thumbnail_common` where both read it.
        if req.specifications.len()
            > thumbnail_common::work_events::MAX_CREATE_THUMBNAIL_SPECIFICATIONS
        {
            return Err(Status::invalid_argument(format!(
                "specifications must contain at most {} entries",
                thumbnail_common::work_events::MAX_CREATE_THUMBNAIL_SPECIFICATIONS
            )));
        }

        // Validate and convert specs — must all be image_to_image.
        let specs = create_thumbnails_specs_from_proto(&req.specifications)
            .map_err(Status::invalid_argument)?;

        info!(
            resource_address = %req.resource_address,
            spec_count = specs.len(),
            "grpc create_thumbnails: initiating thumbnail generation"
        );

        match create_thumbnails(
            &req.resource_address,
            &specs,
            &auth_token,
            self.outbound.clone(),
            thumbnail_common::current_runtime_config(),
        )
        .await
        {
            Ok(outcome) => {
                let failed = outcome
                    .failed
                    .iter()
                    .map(failed_submission_to_proto)
                    .collect();
                Ok(Response::new(CreateThumbnailsResponse { failed }))
            }
            Err(e) => Err(create_thumbnails_error_to_status(e)),
        }
    }

    async fn get_thumbnails(
        &self,
        request: Request<proto::GetThumbnailsRequest>,
    ) -> Result<Response<proto::GetThumbnailsResponse>, Status> {
        // Before validation, as on every other RPC: an unauthenticated caller
        // sending a bad filter should be told about the token, not handed the
        // shape of a request it has not earned the right to make.
        let auth_token = extract_auth_token(&request)
            .ok_or_else(|| Status::unauthenticated("authentication token is required"))?;

        // Same canonical request and same validate() as the HTTP route, so the
        // two transports cannot reach different decisions about the same filters.
        let params = get_thumbnails_request_from_proto(request.get_ref())
            .map_err(Status::invalid_argument)?;
        params
            .validate()
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        info!("grpc get_thumbnails: getting thumbnails");

        let filters = thumbnail_common::ThumbnailFilters::from(&params);

        // Associations are written keyed on the target's resource_address
        // (ADR 0002 §4), so reads use the same key for FILE and FOLDER targets
        // alike — no identity resolution.
        let matches = match thumbnail_common::get_thumbnails_for_resource_with_details(
            &params.association_target_resource_address,
            &filters,
            &auth_token,
            None,
        )
        .await
        {
            Ok(matches) => matches,
            // Not NOT_FOUND: a target with no thumbnail metadata is a successful
            // read of nothing, and reporting it as missing sends the caller
            // looking for a resource that is there.
            Err(thumbnail_common::ThumbnailQueryError::NotFound) => {
                return Ok(Response::new(proto::GetThumbnailsResponse {
                    thumbnails: vec![],
                }))
            }
            Err(error) => {
                let status = get_thumbnails_error_to_status(error);
                error!(code = ?status.code(), "grpc get_thumbnails failed");
                return Err(status);
            }
        };

        // The skip rule, the sort key and the 1000-entry cap come from the one
        // implementation the HTTP route uses.
        let thumbnails = crate::http::thumbnail_entries_from_matches(
            &params.association_target_resource_address,
            matches,
        )
        .into_iter()
        .map(get_thumbnails_entry_to_proto)
        .collect();

        Ok(Response::new(proto::GetThumbnailsResponse { thumbnails }))
    }
}
