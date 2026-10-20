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

//! Thumbnail-related types and structures - contains type definitions for thumbnail operations including filters, debug information, and thumbnail metadata

/// Filters supported by the image-to-image thumbnail query.
#[derive(Debug, Clone, Default)]
pub struct ThumbnailFilters {
    /// Stored specification variant key (`image_to_image`, etc.).
    pub specification: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub format: Option<String>,

    pub transform_type: Option<String>,
    pub crop_strategy: Option<String>,
}

/// Parsed thumbnail information from metadata keys
#[derive(Debug)]
pub struct ThumbnailInfo {
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub uuid: String,
}

/// Rich thumbnail match returned from queries — includes specification metadata
/// stored alongside the thumbnail association in the storage service.
#[derive(Debug, Clone)]
pub struct ThumbnailMatchInfo {
    /// The `thumbnail_*` metadata key this entry was read from.
    ///
    /// Carried so a reader that has to skip an uninterpretable entry can name it.
    /// Without it the only available log line is "one entry was bad", which tells
    /// an operator nothing they can act on.
    pub metadata_key: String,
    pub thumbnail_resource_identity: String,
    /// Thumbnail address as stored in the association metadata (ADR 0002 §2).
    /// `None` for legacy/string-only entries that carry no nested address.
    pub thumbnail_resource_address: Option<String>,
    /// Association target's stored identity (files only; omitted for folders).
    pub association_target_resource_identity: Option<String>,
    /// Stored association target type (`FILE`/`FOLDER`) when present in metadata.
    pub target_type: Option<String>,
    pub specification: Option<serde_json::Value>,
    /// Timestamp captured when the association metadata entry was created.
    pub create_time: Option<String>,
}
