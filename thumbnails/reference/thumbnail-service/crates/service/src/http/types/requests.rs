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

//! Request types for HTTP API

use serde::{Deserialize, Serialize};
use thiserror::Error;
use thumbnail_common::ThumbnailFilters;
use utoipa::ToSchema;

use super::enums::{CropStrategy, ImageEncodingFormat, ThumbnailSpecificationType, TransformType};
use super::specifications::ThumbnailSpecification;

/// A storage resource (file or folder) identified by its address.
///
/// `resource_address` is always required and non-empty. `resource_identity` is
/// present for files and omitted for folders (folders have no identity concept).
#[derive(Deserialize, Serialize, Debug, Clone, ToSchema)]
pub struct Resource {
    #[schema(max_length = 2048)]
    pub resource_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(max_length = 2048)]
    pub resource_identity: Option<String>,
}

pub use thumbnail_common::resource::{validate_resource, ResourceValidationError};

impl Resource {
    /// Validate this resource. `require_identity` is true when the resource is a
    /// file (thumbnails always; association targets when `target_type` is FILE),
    /// false when it is a folder. Delegates to the shared rule in `thumbnail_common`
    /// so the HTTP and gRPC transports cannot diverge.
    pub fn validate(&self, require_identity: bool) -> Result<(), ResourceValidationError> {
        validate_resource(
            &self.resource_address,
            self.resource_identity.as_deref(),
            require_identity,
        )
    }
}

#[cfg(test)]
mod resource_tests {
    use super::*;

    #[test]
    fn file_resource_with_address_and_identity_is_valid() {
        let resource = Resource {
            resource_address: "s3://bucket/image.jpg".to_string(),
            resource_identity: Some("encoded-identity".to_string()),
        };
        assert_eq!(resource.validate(true), Ok(()));
    }

    #[test]
    fn file_resource_without_identity_is_rejected() {
        let resource = Resource {
            resource_address: "s3://bucket/image.jpg".to_string(),
            resource_identity: None,
        };
        assert_eq!(
            resource.validate(true),
            Err(ResourceValidationError::MissingIdentity)
        );
    }

    #[test]
    fn folder_resource_without_identity_is_valid() {
        let resource = Resource {
            resource_address: "s3://bucket/folder/".to_string(),
            resource_identity: None,
        };
        assert_eq!(resource.validate(false), Ok(()));
    }

    #[test]
    fn folder_resource_carrying_identity_is_rejected() {
        let resource = Resource {
            resource_address: "s3://bucket/folder/".to_string(),
            resource_identity: Some("encoded-identity".to_string()),
        };
        assert_eq!(
            resource.validate(false),
            Err(ResourceValidationError::UnexpectedIdentity)
        );
    }

    #[test]
    fn blank_address_is_rejected_before_identity_checks() {
        let resource = Resource {
            resource_address: "   ".to_string(),
            resource_identity: Some("encoded-identity".to_string()),
        };
        assert_eq!(
            resource.validate(true),
            Err(ResourceValidationError::MissingAddress)
        );
    }

    #[test]
    fn target_type_file_requires_identity_folder_does_not() {
        use super::super::enums::TargetType;
        assert!(TargetType::File.requires_identity());
        assert!(!TargetType::Folder.requires_identity());
    }
}

/// Request body for `POST /v1alpha/thumbnails` — trigger thumbnail generation.
#[derive(Deserialize, Serialize, Debug, Clone, ToSchema)]
pub struct CreateThumbnailsRequest {
    /// The resource_address of the source file to generate thumbnails for (required, non-empty).
    #[schema(max_length = 2048)]
    pub resource_address: String,

    /// Optional thumbnail specifications. If absent or empty, the service fans out one task per
    /// configured `THUMBNAIL_SIZES` entry using application defaults (identical to the
    /// notification-driven path). Only `image_to_image` entries are supported; supplying any
    /// other spec type returns 400. At most 1000 entries.
    // `max_items` must stay in step with
    // `thumbnail_common::work_events::MAX_CREATE_THUMBNAIL_SPECIFICATIONS`, which is what the
    // handlers enforce. `#[schema]` takes a literal, so the two are kept aligned by
    // `create_thumbnails_max_items_matches_the_enforced_bound` rather than by construction.
    // A non-doc comment: this belongs to whoever edits the number, not in the published spec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(max_items = 1000)]
    pub specifications: Option<Vec<ThumbnailSpecification>>,
}

/// Why a `CreateThumbnails` request cannot be served as written.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CreateThumbnailsValidationError {
    #[error("specification {index}: quality must be between 1 and 100 for JPEG/WebP")]
    InvalidQuality { index: usize },
    #[error("specification {index}: {field} must be between 1 and {maximum}")]
    DimensionOutOfRange {
        index: usize,
        field: &'static str,
        maximum: u32,
    },
}

impl CreateThumbnailsRequest {
    /// Range-check caller-supplied dimensions and JPEG/WebP quality before any outbound call.
    ///
    /// The `#[schema(minimum, maximum)]` annotations on
    /// `ImageToImageSpecification` are documentation; nothing enforced them. An
    /// out-of-range width survived to the work-event contract, which rejects it —
    /// but only after the source `Stat` had been paid for, and then reported it
    /// either as a per-specification failure inside a 202 or, when it was the only
    /// specification, as the upstream-fault status reserved for "every publish
    /// failed". Both tell the caller the service is broken when the input was.
    ///
    /// The bound is the contract's own `MAX_DIMENSION` rather than a fourth
    /// copy of the literal, and matches what the read path already enforces.
    pub fn validate(&self) -> Result<(), CreateThumbnailsValidationError> {
        let Some(specifications) = &self.specifications else {
            return Ok(());
        };

        for (index, specification) in specifications.iter().enumerate() {
            let Some(image_to_image) = &specification.image_to_image else {
                // A missing or wrong spec type is rejected further in, with its
                // own message; this check is only about ranges.
                continue;
            };

            let encoding_format: thumbnail_common::EncodingFormat =
                image_to_image.encoding_format.into();
            if encoding_format.requires_lossy_quality_range()
                && image_to_image
                    .quality
                    .is_some_and(|quality| !(1..=100).contains(&quality))
            {
                return Err(CreateThumbnailsValidationError::InvalidQuality { index });
            }

            for (field, value) in [
                ("width", image_to_image.width),
                ("height", image_to_image.height),
            ] {
                if !(1..=thumbnail_common::work_events::MAX_DIMENSION).contains(&value) {
                    return Err(CreateThumbnailsValidationError::DimensionOutOfRange {
                        index,
                        field,
                        maximum: thumbnail_common::work_events::MAX_DIMENSION,
                    });
                }
            }
        }

        Ok(())
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GetThumbnailsRequest {
    #[schema(
        example = "https://example.blob.core.windows.net/container/sample-image.jpg",
        max_length = 2048
    )]
    pub association_target_resource_address: String,

    #[serde(default)]
    pub specification: Option<ThumbnailSpecificationType>,
    #[serde(default)]
    #[schema(minimum = 1, maximum = 8192, format = Int32)]
    pub width: Option<u32>,
    #[serde(default)]
    #[schema(minimum = 1, maximum = 8192, format = Int32)]
    pub height: Option<u32>,
    #[serde(default)]
    pub encoding_format: Option<ImageEncodingFormat>,

    #[serde(default)]
    pub transform_type: Option<TransformType>,
    #[serde(default)]
    pub crop_strategy: Option<CropStrategy>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GetThumbnailsValidationError {
    #[error("association_target_resource_address is required")]
    MissingAddress,
    #[error("association_target_resource_address must not exceed 2048 characters")]
    AddressTooLong,
    #[error("{field} must be between 1 and {maximum}")]
    OutOfRange { field: &'static str, maximum: u32 },
}

impl GetThumbnailsRequest {
    pub fn validate(&self) -> Result<(), GetThumbnailsValidationError> {
        if self.association_target_resource_address.trim().is_empty() {
            return Err(GetThumbnailsValidationError::MissingAddress);
        }
        if self.association_target_resource_address.chars().count() > 2048 {
            return Err(GetThumbnailsValidationError::AddressTooLong);
        }

        validate_optional_range("width", self.width, 8192)?;
        validate_optional_range("height", self.height, 8192)?;
        Ok(())
    }
}

impl From<&GetThumbnailsRequest> for ThumbnailFilters {
    fn from(request: &GetThumbnailsRequest) -> Self {
        Self {
            specification: request
                .specification
                .as_ref()
                .map(|value| value.storage_key().to_string()),
            width: request.width,
            height: request.height,
            format: request
                .encoding_format
                .as_ref()
                .map(|value| value.as_str().to_string()),
            transform_type: request
                .transform_type
                .as_ref()
                .map(|value| value.as_str().to_string()),
            crop_strategy: request
                .crop_strategy
                .as_ref()
                .map(|value| value.as_str().to_string()),
        }
    }
}

fn validate_optional_range(
    field: &'static str,
    value: Option<u32>,
    maximum: u32,
) -> Result<(), GetThumbnailsValidationError> {
    if value.is_some_and(|value| !(1..=maximum).contains(&value)) {
        return Err(GetThumbnailsValidationError::OutOfRange { field, maximum });
    }
    Ok(())
}
