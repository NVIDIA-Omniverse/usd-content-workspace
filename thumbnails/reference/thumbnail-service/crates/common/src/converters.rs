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

//! Shared conversions, defaults, and helper functions for HTTP and gRPC services.

use crate::types::ImageProcessingSpec;
use crate::{current_runtime_config, CropStrategy, EncodingFormat, TransformType};
use chrono::Utc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// `DEFAULT_WIDTH` / `DEFAULT_HEIGHT` were removed: thumbnail dimensions now
// come exclusively from the parsed `THUMBNAIL_SIZES` value on the installed
// `RuntimeConfigArgs`, read directly via `current_runtime_config()` at each
// real call site (both HTTP and gRPC always have it installed by the time
// they need it). See `openspec/changes/configure-thumbnail-sizes/design.md` §8.

pub fn current_timestamp_duration() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|e| {
            tracing::error!("System time error: {}", e);
            Duration::from_secs(0)
        })
}

pub fn current_timestamp_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

/// Build an `ImageProcessingSpec` for the given `(width, height)`, leaving the
/// non-dimension fields populated from required `THUMBNAIL_*` config.
///
/// Width and height come from the caller (via the parsed `THUMBNAIL_SIZES`);
/// they are no longer read from the env. The legacy `DEFAULT_THUMBNAIL_WIDTH`
/// / `DEFAULT_THUMBNAIL_HEIGHT` env vars are intentionally ignored here and
/// will be removed entirely in the deprecation cleanup task.
pub fn create_processing_spec_for(width: u32, height: u32) -> ImageProcessingSpec {
    create_processing_spec_with(current_runtime_config(), width, height)
}

/// As [`create_processing_spec_for`], but reading defaults from the supplied
/// configuration rather than the process-wide one.
///
/// Dispatch takes a `&RuntimeConfigArgs` so tests can drive a fan-out against an
/// injected configuration; reaching for the global here would ignore it.
pub fn create_processing_spec_with(
    runtime: &crate::RuntimeConfigArgs,
    width: u32,
    height: u32,
) -> ImageProcessingSpec {
    let config = &runtime.thumbnail_defaults;

    ImageProcessingSpec {
        width: Some(width),
        height: Some(height),
        encoding_format: Some(config.thumbnail_encoding_format),
        transform_type: Some(config.thumbnail_transform_type),
        crop_strategy: Some(config.thumbnail_crop_strategy),
        quality: Some(config.thumbnail_quality),
        preserve_aspect_ratio: Some(config.thumbnail_preserve_aspect_ratio),
    }
}

// `create_default_processing_spec` was removed in favor of
// `create_processing_spec_for(width, height)` plus a `ThumbnailSizes` fan-out
// sourced from the installed `RuntimeConfigArgs`. See
// openspec/changes/configure-thumbnail-sizes/design.md §8.

#[derive(Debug, Clone, Copy)]
pub struct SpecRenderingOptions {
    pub quality: i32,
    pub preserve_aspect_ratio: bool,
}

pub fn create_spec_json(
    width: u32,
    height: u32,
    encoding_format: EncodingFormat,
    transform_type: TransformType,
    crop_strategy: CropStrategy,
    options: SpecRenderingOptions,
) -> serde_json::Value {
    serde_json::json!({
        "type": "IMAGE_TO_IMAGE",
        "width": width as f64,
        "height": height as f64,
        "encoding_format": encoding_format,
        "transform_type": transform_type,
        "crop_strategy": crop_strategy,
        "quality": options.quality as f64,
        "preserve_aspect_ratio": options.preserve_aspect_ratio,
    })
}
