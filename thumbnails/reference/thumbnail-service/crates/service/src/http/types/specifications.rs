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

//! Thumbnail specification types for HTTP API
//!
//! The HTTP association contract accepts a flat typed specification object
//! with a `"type"` discriminant: `{"type": "IMAGE_TO_IMAGE", ...}` or
//! `{"type": "MANUAL", ...}`. The `CreateThumbnails` endpoint uses the
//! outer-keyed `{"image_to_image": {...}}` shape for backward compatibility
//! with existing clients, and rejects MANUAL specifications.

use serde::{Deserialize, Serialize};
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha as proto;
use utoipa::ToSchema;

use super::enums::{CropStrategy, ImageEncodingFormat, TransformType};

/// HTTP request specification for `CreateThumbnails`. Uses the outer-keyed
/// `{"image_to_image": {...}}` shape. Only `image_to_image` is valid here;
/// MANUAL is rejected before any work queue submissions.
#[derive(Deserialize, Serialize, Debug, Clone, ToSchema)]
pub struct ThumbnailSpecification {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(required = true, schema_with = required_image_to_image_schema)]
    pub image_to_image: Option<ImageToImageSpecification>,
}

fn required_image_to_image_schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
    utoipa::openapi::schema::AllOfBuilder::new()
        .item(utoipa::openapi::Ref::from_schema_name(
            "ImageToImageSpecification",
        ))
        .into()
}

#[derive(Deserialize, Serialize, Debug, Clone, ToSchema)]
pub struct ImageToImageSpecification {
    #[serde(default = "default_encoding_format")]
    pub encoding_format: ImageEncodingFormat,
    #[serde(default = "default_width")]
    #[schema(minimum = 1, maximum = 8192)]
    pub width: u32,
    #[serde(default = "default_height")]
    #[schema(minimum = 1, maximum = 8192)]
    pub height: u32,
    #[serde(default = "default_transform_type")]
    pub transform_type: TransformType,
    #[serde(default = "default_crop_strategy")]
    pub crop_strategy: CropStrategy,
    /// Optional encode quality: must be between 1 and 100 for JPEG/WebP.
    /// PNG/GIF do not apply this range. When omitted, the service applies
    /// its configured default (THUMBNAIL_QUALITY).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    // These are the HTTP i32 wire limits; the lossy-quality range depends on format.
    #[schema(minimum = -2147483648, maximum = 2147483647)]
    pub quality: Option<i32>,
    /// Optional flag controlling whether the source aspect ratio is preserved. Omission is
    /// distinct from an explicit `false`: omitting the field applies the service's configured
    /// default (THUMBNAIL_PRESERVE_ASPECT_RATIO), while `false` forces the requested
    /// dimensions regardless of that default.
    // Same semantics the proto has declared all along on `ImageToImageSpecification`; this
    // type simply had no field, so the handler could only ever send the configured default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserve_aspect_ratio: Option<bool>,
}

fn default_encoding_format() -> ImageEncodingFormat {
    thumbnail_common::current_runtime_config()
        .thumbnail_defaults
        .thumbnail_encoding_format
        .into()
}

fn default_width() -> u32 {
    256
}

fn default_height() -> u32 {
    256
}

fn default_transform_type() -> TransformType {
    thumbnail_common::current_runtime_config()
        .thumbnail_defaults
        .thumbnail_transform_type
        .into()
}

fn default_crop_strategy() -> CropStrategy {
    thumbnail_common::current_runtime_config()
        .thumbnail_defaults
        .thumbnail_crop_strategy
        .into()
}

impl ImageToImageSpecification {
    fn proto_encoding_format(&self) -> i32 {
        match self.encoding_format {
            ImageEncodingFormat::Jpeg => proto::ImageEncodingFormat::Jpeg as i32,
            ImageEncodingFormat::Png => proto::ImageEncodingFormat::Png as i32,
            ImageEncodingFormat::Gif => proto::ImageEncodingFormat::Gif as i32,
            ImageEncodingFormat::Webp => proto::ImageEncodingFormat::Webp as i32,
        }
    }

    fn proto_transform_type(&self) -> i32 {
        match self.transform_type {
            TransformType::Square => proto::TransformType::Square as i32,
            TransformType::ScaleByWidth => proto::TransformType::ScaleByWidth as i32,
            TransformType::ScaleByHeight => proto::TransformType::ScaleByHeight as i32,
            TransformType::Crop => proto::TransformType::Crop as i32,
            TransformType::Fit => proto::TransformType::Fit as i32,
            TransformType::ScaleAndCrop => proto::TransformType::ScaleAndCrop as i32,
            TransformType::Thumbnail => proto::TransformType::Thumbnail as i32,
            TransformType::Resize => proto::TransformType::Resize as i32,
        }
    }

    fn proto_crop_strategy(&self) -> i32 {
        match self.crop_strategy {
            CropStrategy::Center => proto::CropStrategy::Center as i32,
            CropStrategy::Top => proto::CropStrategy::Top as i32,
            CropStrategy::Bottom => proto::CropStrategy::Bottom as i32,
            CropStrategy::Left => proto::CropStrategy::Left as i32,
            CropStrategy::Right => proto::CropStrategy::Right as i32,
            CropStrategy::TopLeft => proto::CropStrategy::TopLeft as i32,
            CropStrategy::TopRight => proto::CropStrategy::TopRight as i32,
            CropStrategy::BottomLeft => proto::CropStrategy::BottomLeft as i32,
            CropStrategy::BottomRight => proto::CropStrategy::BottomRight as i32,
            CropStrategy::Smart => proto::CropStrategy::Smart as i32,
        }
    }
}

impl From<&ImageToImageSpecification> for proto::ImageToImageSpecification {
    fn from(http: &ImageToImageSpecification) -> Self {
        proto::ImageToImageSpecification {
            processor_identity: proto::ProcessorIdentity::ImageToImage as i32,
            encoding_format: http.proto_encoding_format(),
            width: http.width,
            height: http.height,
            transform_type: http.proto_transform_type(),
            crop_strategy: http.proto_crop_strategy(),
            // Presence is preserved rather than resolved here: an omitted optional
            // stays `None` and is filled from runtime configuration by the shared
            // dispatch seam, so an explicit `false` survives the trip.
            quality: http.quality.and_then(|quality| u32::try_from(quality).ok()),
            preserve_aspect_ratio: http.preserve_aspect_ratio,
        }
    }
}

/// Pure builder for the default `specifications` array given the parsed
/// `THUMBNAIL_SIZES` (or `None` to fall back to the legacy single 256x256).
/// Extracted from [`default_specifications`] purely so unit tests can hand it
/// a synthetic [`thumbnail_common::ThumbnailSizes`] without touching the
/// process-wide global.
pub(crate) fn build_default_specifications(
    sizes: Option<&thumbnail_common::ThumbnailSizes>,
) -> Vec<ThumbnailSpecification> {
    let default_encoding_format = default_encoding_format();
    let default_transform_type = default_transform_type();
    let default_crop_strategy = default_crop_strategy();

    let make = |width: u32, height: u32| ThumbnailSpecification {
        image_to_image: Some(ImageToImageSpecification {
            encoding_format: default_encoding_format,
            width,
            height,
            transform_type: default_transform_type,
            crop_strategy: default_crop_strategy,
            // Both resolved from config at submission time.
            quality: None,
            preserve_aspect_ratio: None,
        }),
    };

    match sizes {
        Some(sizes) => sizes.iter().map(|&(w, h)| make(w, h)).collect(),
        None => vec![make(256, 256)],
    }
}

/// Build the default `specifications` array for callers who omit it.
///
/// The binary installs `RuntimeConfigArgs` before any request handler runs
/// (and every test that reaches this path installs it via
/// `thumbnail_common::test_fixtures::init_test_runtime_config` first), so
/// this always has a real installed config to read and returns one
/// `ThumbnailSpecification` per parsed `THUMBNAIL_SIZES` entry.
pub fn default_specifications() -> Vec<ThumbnailSpecification> {
    let sizes = &thumbnail_common::current_runtime_config()
        .thumbnail_defaults
        .thumbnail_sizes;
    build_default_specifications(Some(sizes))
}

#[cfg(test)]
mod default_specifications_tests {
    use super::*;
    use std::str::FromStr;
    use thumbnail_common::ThumbnailSizes;

    #[test]
    fn fallback_yields_legacy_single_256x256_when_no_sizes_configured() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-http-tests");
        let specs = build_default_specifications(None);
        assert_eq!(specs.len(), 1, "fallback must produce exactly one spec");
        let s = specs[0].image_to_image.as_ref().unwrap();
        assert_eq!((s.width, s.height), (256, 256));
        assert_eq!(s.encoding_format, ImageEncodingFormat::Png);
        assert_eq!(s.transform_type, TransformType::Square);
        assert_eq!(s.crop_strategy, CropStrategy::Smart);
    }

    #[test]
    fn single_configured_size_yields_one_spec() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-http-tests");
        let sizes = ThumbnailSizes::from_str("128x128").unwrap();
        let specs = build_default_specifications(Some(&sizes));
        assert_eq!(specs.len(), 1);
        let s = specs[0].image_to_image.as_ref().unwrap();
        assert_eq!((s.width, s.height), (128, 128));
    }

    #[test]
    fn multi_configured_sizes_yield_one_spec_per_pair_in_input_order() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-http-tests");
        let sizes = ThumbnailSizes::from_str("64x64,128x128,256x256").unwrap();
        let specs = build_default_specifications(Some(&sizes));
        let dims: Vec<(u32, u32)> = specs
            .iter()
            .map(|s| {
                let i = s.image_to_image.as_ref().unwrap();
                (i.width, i.height)
            })
            .collect();
        assert_eq!(dims, vec![(64, 64), (128, 128), (256, 256)]);

        // Non-dimension fields are unchanged from the legacy defaults.
        for s in &specs {
            let i = s.image_to_image.as_ref().unwrap();
            assert_eq!(i.encoding_format, ImageEncodingFormat::Png);
            assert_eq!(i.transform_type, TransformType::Square);
            assert_eq!(i.crop_strategy, CropStrategy::Smart);
        }
    }

    #[test]
    fn non_square_pair_preserved() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-http-tests");
        let sizes = ThumbnailSizes::from_str("128x64").unwrap();
        let specs = build_default_specifications(Some(&sizes));
        let s = specs[0].image_to_image.as_ref().unwrap();
        assert_eq!((s.width, s.height), (128, 64));
    }
}
