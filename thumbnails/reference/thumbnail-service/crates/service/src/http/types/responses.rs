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

//! Response types for HTTP API

use serde::{Deserialize, Serialize};
use thumbnail_common::AssociationSpec;
use utoipa::ToSchema;

use super::enums::TargetType;
use super::requests::Resource;
use super::specifications::ThumbnailSpecification;

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ErrorResponse {
    #[schema(max_length = 256)]
    pub source: String,
    #[schema(max_length = 4096)]
    pub error_message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

// --- GET /v1alpha/thumbnails ---

/// A single thumbnail entry in the GET /thumbnails response
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ThumbnailEntry {
    pub target_type: TargetType,
    pub association_target: Resource,
    pub thumbnail: Resource,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<AssociationSpec>)]
    pub specification: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(max_length = 64, format = DateTime)]
    pub create_time: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct GetThumbnailsResponse {
    #[schema(max_items = 1000)]
    pub thumbnails: Vec<ThumbnailEntry>,
}

// --- DELETE /v1alpha/associations (targeted removal) ---

/// The targeted thumbnail when its disassociation failed.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct DisassociateFailure {
    #[schema(max_length = 2048)]
    pub resource_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(max_length = 2048)]
    pub resource_identity: Option<String>,
    #[schema(max_length = 4096)]
    pub error: String,
}

/// Result of removing one targeted thumbnail association (ADR 0002 §5).
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct DisassociateThumbnailResponse {
    #[schema(max_items = 1)]
    pub succeeded: Vec<Resource>,
    #[schema(max_items = 1)]
    pub failed: Vec<DisassociateFailure>,
}

// --- POST /v1alpha/thumbnails (create / generate) ---

/// A single thumbnail task queue submission that failed during CreateThumbnails.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct CreateThumbnailsFailure {
    /// The full specification that was attempted (caller-supplied or resolved from defaults).
    pub specification: ThumbnailSpecification,
    /// Human-readable description of why the submission failed.
    #[schema(max_length = 4096)]
    pub error_message: String,
}

/// Acceptance response for `POST /v1alpha/thumbnails`.
/// `failed` is empty on full success and populated on partial acceptance.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct CreateThumbnailsResponse {
    /// Specifications that were not submitted. Empty on full acceptance.
    ///
    /// Populated when dispatch fails after request validation, including publisher
    /// failures. Invalid JPEG/WebP quality is rejected before storage lookup or dispatch.
    ///
    /// Only ever a *partial* result: at least one specification was accepted,
    /// because a call where none was returns `502` with an error body instead.
    ///
    /// Do not compare this length against the request's `specifications`. When
    /// that field is absent or empty the service fans out over its configured
    /// `THUMBNAIL_SIZES`, so `failed` describes specifications the caller never
    /// sent and can be longer than the list it did — an assertion of
    /// `failed.length < specifications.length` fails against a conformant
    /// deployment on that path.
    #[schema(max_items = 1000)]
    pub failed: Vec<CreateThumbnailsFailure>,
}

// --- Health ---

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct HealthCheckResponse {
    #[schema(max_length = 256)]
    pub status: String,
}
