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

//! Caller-initiated thumbnail generation business logic.
//!
//! Encapsulates the `CreateThumbnails` operation shared by the HTTP and gRPC
//! transports: identity resolution via storage Stat, specification validation,
//! and generate-event fan-out.
//!
//! This is the second producer of thumbnail work alongside the storage-CREATE
//! consumer. It publishes the same `omni.thumbnail.generate` contract through the
//! same builder, so the worker cannot tell a caller-supplied specification from a
//! configuration-derived one. What it keeps that the CREATE path does not is
//! per-specification reporting: an API caller can act on knowing which
//! specification failed, where a CREATE delivery has only an acknowledgement to
//! decide.

use std::sync::Arc;

use thiserror::Error;
use tracing::info;

use crate::storage::StatError;
use crate::types::ImageProcessingSpec;
use crate::work_events::{dispatch_generate_per_specification, GenerateSource};
use crate::{OutboundClients, RuntimeConfigArgs};

/// One generate dispatch that failed during a `CreateThumbnails` call.
#[derive(Debug, Clone)]
pub struct FailedSubmission {
    /// The fully-resolved specification that was attempted but failed.
    /// For the defaults path this is populated from config (not a label).
    pub specification: ImageProcessingSpec,
    /// Human-readable description of why the dispatch failed.
    pub error_message: String,
}

/// Outcome of a successful (or partially-successful) `CreateThumbnails` call.
///
/// A call that returns `Ok(outcome)` was accepted: at least one generate event
/// was published. `failed` is empty on full success, non-empty on partial failure.
#[derive(Debug, Default)]
pub struct CreateThumbnailsOutcome {
    /// Per-spec failures. Empty on full success.
    pub failed: Vec<FailedSubmission>,
}

/// Why a `CreateThumbnails` call failed before or during dispatch.
#[derive(Debug, Error)]
pub enum CreateThumbnailsError {
    /// Caller input was invalid (empty address, or bad spec type).
    #[error("{0}")]
    InvalidArgument(String),

    /// The resource_address was not found in storage.
    #[error("{0}")]
    NotFound(String),

    /// The caller's token was missing or rejected by storage.
    #[error("{0}")]
    Unauthenticated(String),

    /// The token was accepted; the caller may not read the source resource.
    #[error("{0}")]
    PermissionDenied(String),

    /// Every generate publish failed — nothing was enqueued.
    #[error("{0}")]
    AllTasksFailed(String),
}

impl From<StatError> for CreateThumbnailsError {
    fn from(e: StatError) -> Self {
        match e {
            StatError::NotFound => Self::NotFound(
                "resource_address was not found in storage; cannot generate thumbnails".to_string(),
            ),
            StatError::Unauthenticated(msg) => Self::Unauthenticated(msg),
            StatError::PermissionDenied(msg) => Self::PermissionDenied(msg),
            StatError::Internal(msg) => Self::AllTasksFailed(format!(
                "storage Stat failed before any work was dispatched: {msg}"
            )),
        }
    }
}

/// Trigger asynchronous thumbnail generation for `resource_address`.
///
/// Workflow:
/// 1. Validate inputs (address non-empty, specs all `image_to_image`).
/// 2. Call `stat_file_object` to resolve `resource_identity`.
/// 3. If `specifications` is empty, fan out one generate event per configured
///    `THUMBNAIL_SIZES` entry; otherwise one per supplied specification.
/// 4. If all publishes succeed: `Ok(outcome)` with empty `failed`.
///    If some fail: `Ok(outcome)` with `failed` populated.
///    If all fail: `Err(AllTasksFailed)`.
///
/// `specifications` must contain only `image_to_image`-populated entries.
/// The caller is responsible for proto-to-`ImageProcessingSpec` conversion and
/// spec-type validation before calling this function; the shared generate builder
/// enforces it again rather than deferring to the worker's processor gate.
pub async fn create_thumbnails(
    resource_address: &str,
    specifications: &[ImageProcessingSpec],
    auth_token: &str,
    outbound: Arc<OutboundClients>,
    runtime: &RuntimeConfigArgs,
) -> Result<CreateThumbnailsOutcome, CreateThumbnailsError> {
    let resource_address = resource_address.trim();
    if resource_address.is_empty() {
        return Err(CreateThumbnailsError::InvalidArgument(
            "resource_address must not be empty".to_string(),
        ));
    }

    info!(
        resource_address,
        spec_count = specifications.len(),
        "create_thumbnails: resolving resource identity via Stat"
    );

    let resource_identity =
        crate::storage::stat_file_object(resource_address, auth_token, outbound.clone())
            .await
            .map_err(CreateThumbnailsError::from)?;

    // The defaults path submits exactly what the event-driven path would, but
    // reports each outcome separately instead of collapsing to a batch result.
    let owned_defaults: Vec<ImageProcessingSpec>;
    let specifications = if specifications.is_empty() {
        owned_defaults = runtime
            .thumbnail_defaults
            .thumbnail_sizes
            .iter()
            .map(|&(width, height)| {
                crate::converters::create_processing_spec_with(runtime, width, height)
            })
            .collect();
        &owned_defaults
    } else {
        specifications
    };

    fan_out(
        resource_address,
        &resource_identity,
        specifications,
        runtime,
        &outbound,
    )
    .await
}

async fn fan_out(
    resource_address: &str,
    resource_identity: &str,
    specifications: &[ImageProcessingSpec],
    runtime: &RuntimeConfigArgs,
    outbound: &OutboundClients,
) -> Result<CreateThumbnailsOutcome, CreateThumbnailsError> {
    info!(
        resource_address,
        spec_count = specifications.len(),
        "create_thumbnails: publishing generate work"
    );

    let resolved: Vec<ImageProcessingSpec> = specifications
        .iter()
        .map(|spec| resolve_with_runtime_defaults(spec, runtime))
        .collect();

    let outcome = dispatch_generate_per_specification(
        GenerateSource {
            resource_identity,
            resource_address,
        },
        &resolved,
        runtime,
        outbound,
    )
    .await;

    if outcome.destinations.is_empty() && !outcome.failed.is_empty() {
        return Err(CreateThumbnailsError::AllTasksFailed(format!(
            "all {} thumbnail generate publishes failed",
            outcome.failed.len()
        )));
    }

    Ok(CreateThumbnailsOutcome {
        failed: outcome
            .failed
            .into_iter()
            .map(|failure| FailedSubmission {
                error_message: failure.error.to_string(),
                specification: failure.specification,
            })
            .collect(),
    })
}

/// Fill configurable fields omitted by an explicit transport request before
/// dispatching it, preserving every value the caller supplied.
fn resolve_with_runtime_defaults(
    spec: &ImageProcessingSpec,
    runtime: &RuntimeConfigArgs,
) -> ImageProcessingSpec {
    let defaults = &runtime.thumbnail_defaults;
    let mut resolved = spec.clone();
    resolved
        .encoding_format
        .get_or_insert(defaults.thumbnail_encoding_format);
    resolved
        .transform_type
        .get_or_insert(defaults.thumbnail_transform_type);
    resolved
        .crop_strategy
        .get_or_insert(defaults.thumbnail_crop_strategy);
    resolved.quality.get_or_insert(defaults.thumbnail_quality);
    resolved
        .preserve_aspect_ratio
        .get_or_insert(defaults.thumbnail_preserve_aspect_ratio);
    resolved
}
