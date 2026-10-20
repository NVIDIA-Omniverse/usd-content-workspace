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

use crate::spec_enums::{CropStrategy, EncodingFormat, TransformType};
use serde::Serialize;

/// A possibly-partial image specification, as it arrives from a transport or is
/// assembled from `THUMBNAIL_*` configuration.
///
/// Every field is optional because a request may omit one and let runtime
/// configuration fill the gap. `work_events::resolve_generate_spec` is the one
/// place that turns this into a complete specification, so a generate message
/// never carries a hole for the worker to guess at.
///
/// Always represents an IMAGE_TO_IMAGE specification — MANUAL specs never flow
/// through the generate pipeline.
#[derive(Debug, Serialize, Default, Clone)]
pub struct ImageProcessingSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<EncodingFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform_type: Option<TransformType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop_strategy: Option<CropStrategy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preserve_aspect_ratio: Option<bool>,
}
