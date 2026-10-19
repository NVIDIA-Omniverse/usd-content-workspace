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

//! Thumbnail query operations — storage metadata retrieval, filtering, and parsing.

use super::parsing::{extract_match_info_from_metadata, parse_thumbnail_key};
use super::types::{ThumbnailFilters, ThumbnailMatchInfo};
use crate::current_runtime_config;
use crate::storage::{metadata::get_all_storage_metadata_values, StorageMetadataError};
use prost_types;
use std::collections::HashMap;
use storage_proto::nvidia::omniverse::storage::metadata::v1alpha::UserMetadataValue;
use thiserror::Error;
use tracing::{debug, error, info, instrument, warn};

#[derive(Debug, Default)]
struct FilterStats {
    total_metadata_count: usize,
    total_thumbnail_keys: usize,
    parsed_thumbnail_keys: usize,
    filtered_out_count: usize,
    extraction_failures: usize,
}

/// Shared thumbnail metadata filtering pipeline that supports different extraction outputs.
#[instrument(skip_all, fields(total_keys = user_metadata.len()))]
fn filter_and_parse_thumbnails_with<T, F>(
    user_metadata: HashMap<String, UserMetadataValue>,
    filters: &ThumbnailFilters,
    source_filter: Option<&str>,
    mut extractor: F,
) -> (Vec<T>, FilterStats)
where
    F: FnMut(&str, &prost_types::Value) -> Option<T>,
{
    let mut matching_results = Vec::new();
    let mut stats = FilterStats {
        total_metadata_count: user_metadata.len(),
        ..Default::default()
    };

    for (key, metadata_value) in user_metadata {
        if !key.starts_with("thumbnail_") {
            continue;
        }

        stats.total_thumbnail_keys += 1;
        debug!(key, "Found thumbnail key");

        // Parse the thumbnail key
        let Some(thumbnail_info) = parse_thumbnail_key(&key) else {
            warn!(key, "Could not parse thumbnail key");
            continue;
        };

        stats.parsed_thumbnail_keys += 1;
        debug!(
            key,
            width = thumbnail_info.width,
            height = thumbnail_info.height,
            format = %thumbnail_info.format,
            "Parsed thumbnail dimensions"
        );

        let matches_height = filters.height.is_none_or(|h| thumbnail_info.height == h);
        let matches_width = filters.width.is_none_or(|w| thumbnail_info.width == w);
        let matches_format = filters
            .format
            .as_deref()
            .is_none_or(|f| thumbnail_info.format.eq_ignore_ascii_case(f));

        if !matches_height || !matches_width || !matches_format {
            stats.filtered_out_count += 1;
            debug!(
                key,
                matches_height, matches_width, matches_format, "Thumbnail filtered out by criteria"
            );
            continue;
        }

        let Some(value) = &metadata_value.value else {
            stats.extraction_failures += 1;
            warn!(key, "No value found in metadata");
            continue;
        };

        if !metadata_specification_matches(value, filters) {
            stats.filtered_out_count += 1;
            continue;
        }

        if let Some(expected_source) = source_filter {
            if !metadata_association_target_matches(value, expected_source) {
                stats.filtered_out_count += 1;
                continue;
            }
        }

        match extractor(&key, value) {
            Some(result) => {
                debug!(key, "Found matching thumbnail");
                matching_results.push(result);
            }
            None => {
                stats.extraction_failures += 1;
                warn!(key, "Could not extract resource identity from metadata");
            }
        }
    }

    (matching_results, stats)
}

fn metadata_specification_matches(value: &prost_types::Value, filters: &ThumbnailFilters) -> bool {
    let has_specification_filters = filters.specification.is_some()
        || filters.transform_type.is_some()
        || filters.crop_strategy.is_some();
    if !has_specification_filters {
        return true;
    }

    let value = crate::utils::prost_to_json_value(value);
    let Some(specification) = value.get("specification") else {
        return false;
    };

    // Match the specification `type` field if a spec type filter was supplied.
    if let Some(expected_type) = filters.specification.as_deref() {
        let actual_type = specification
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !actual_type.eq_ignore_ascii_case(expected_type) {
            return false;
        }
    }

    // Transform and crop fields live directly on the flat specification object.
    enum_field_matches(
        specification,
        "transform_type",
        filters.transform_type.as_deref(),
    ) && enum_field_matches(
        specification,
        "crop_strategy",
        filters.crop_strategy.as_deref(),
    )
}

fn enum_field_matches(value: &serde_json::Value, field: &str, expected: Option<&str>) -> bool {
    expected.is_none_or(|expected| {
        value
            .get(field)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
    })
}

fn metadata_association_target_matches(value: &prost_types::Value, expected_source: &str) -> bool {
    // association_target is the nested Resource object (ADR 0002 §2); match on either
    // its address or its identity.
    if let Some(prost_types::value::Kind::StructValue(struct_val)) = &value.kind {
        if let Some(prost_types::value::Kind::StructValue(target)) = struct_val
            .fields
            .get("association_target")
            .and_then(|v| v.kind.as_ref())
        {
            for field in ["resource_address", "resource_identity"] {
                if let Some(prost_types::value::Kind::StringValue(s)) =
                    target.fields.get(field).and_then(|v| v.kind.as_ref())
                {
                    if s == expected_source {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn log_filter_summary(stats: &FilterStats) {
    info!(
        total_keys = stats.total_metadata_count,
        thumbnail_keys = stats.total_thumbnail_keys,
        parsed = stats.parsed_thumbnail_keys,
        filtered_out = stats.filtered_out_count,
        extraction_failures = stats.extraction_failures,
        "Thumbnail search completed"
    );
}

fn log_no_thumbnails_diagnostics(
    stats: &FilterStats,
    width: Option<u32>,
    height: Option<u32>,
    format: Option<&str>,
) {
    if stats.total_thumbnail_keys == 0 {
        debug!("No thumbnail metadata keys found - thumbnails may not be generated yet");
    } else if stats.parsed_thumbnail_keys == 0 {
        error!(
            thumbnail_keys = stats.total_thumbnail_keys,
            "Found thumbnail keys but none could be parsed - malformed keys"
        );
    } else if stats.filtered_out_count == stats.parsed_thumbnail_keys {
        debug!(
            thumbnails = stats.parsed_thumbnail_keys,
            ?width,
            ?height,
            format,
            "All thumbnails filtered out by criteria"
        );
    } else if stats.extraction_failures == stats.parsed_thumbnail_keys {
        error!(
            thumbnails = stats.parsed_thumbnail_keys,
            "Thumbnail metadata extraction failures - check metadata format"
        );
    } else {
        warn!(
            filtered_out = stats.filtered_out_count,
            extraction_failures = stats.extraction_failures,
            "Mixed issues filtering thumbnails"
        );
    }
}

#[derive(Debug, Error)]
pub enum ThumbnailQueryError {
    #[error("no thumbnail metadata exists for the requested association target")]
    NotFound,
    #[error("storage rejected the caller's authentication token")]
    Unauthenticated,
    #[error("storage denied access to the requested association target")]
    PermissionDenied,
    #[error("storage metadata query failed: {0}")]
    Storage(String),
}

impl From<StorageMetadataError> for ThumbnailQueryError {
    fn from(error: StorageMetadataError) -> Self {
        match error {
            StorageMetadataError::NotFound => Self::NotFound,
            StorageMetadataError::Unauthenticated(_) => Self::Unauthenticated,
            StorageMetadataError::PermissionDenied(_) => Self::PermissionDenied,
            StorageMetadataError::Failed(message) => Self::Storage(message),
        }
    }
}

/// Returns rich match info including
/// the specification metadata stored alongside each thumbnail association.
#[instrument(skip_all, fields(
    width = filters.width,
    height = filters.height,
    format = filters.format.as_deref()
))]
pub async fn get_thumbnails_for_resource_with_details(
    resource_address: &str,
    filters: &ThumbnailFilters,
    auth_token: &str,
    source_filter: Option<&str>,
) -> Result<Vec<ThumbnailMatchInfo>, ThumbnailQueryError> {
    if current_runtime_config().service.development_mode {
        warn!("Development mode: returning mock thumbnail data");
        return Ok(vec![ThumbnailMatchInfo {
            metadata_key: "thumbnail_development_mode_mock".to_string(),
            thumbnail_resource_identity: format!("mock_thumbnail_256x256_PNG_{}", resource_address),
            thumbnail_resource_address: Some(format!(
                "mock_thumbnail_256x256_PNG_{}",
                resource_address
            )),
            association_target_resource_identity: None,
            target_type: None,
            specification: Some(serde_json::json!({
                "type": "IMAGE_TO_IMAGE",
                "encoding_format": "PNG",
                "width": 256,
                "height": 256,
                "transform_type": "SQUARE",
                "crop_strategy": "CENTER"
            })),
            create_time: None,
        }]);
    }

    let user_metadata = get_all_storage_metadata_values(resource_address, auth_token).await?;
    if user_metadata.is_empty() {
        return Err(ThumbnailQueryError::NotFound);
    }

    let (matches, stats) = filter_and_parse_thumbnails_with(
        user_metadata,
        filters,
        source_filter,
        extract_match_info_from_metadata,
    );

    log_filter_summary(&stats);

    if stats.total_thumbnail_keys == 0 {
        log_no_thumbnails_diagnostics(
            &stats,
            filters.width,
            filters.height,
            filters.format.as_deref(),
        );
        return Err(ThumbnailQueryError::NotFound);
    }

    if matches.is_empty() {
        log_no_thumbnails_diagnostics(
            &stats,
            filters.width,
            filters.height,
            filters.format.as_deref(),
        );
    } else {
        info!(matches = matches.len(), "Found matching thumbnails");
    }

    Ok(matches)
}
