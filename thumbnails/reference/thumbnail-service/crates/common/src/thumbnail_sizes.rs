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

//! Parser for the `THUMBNAIL_SIZES` env var.
//!
//! The deployer-controlled list of thumbnail dimensions the service produces
//! for every source asset. The grammar is documented in
//! `openspec/changes/configure-thumbnail-sizes/design.md` (Decisions §4):
//!
//! ```text
//! THUMBNAIL_SIZES = size ("," size)*
//! size            = WS* uint "x" uint WS*
//! 1 ≤ each ≤ 4096
//! at least 1 entry, at most 10, no duplicates
//! ```

use std::str::FromStr;
use thiserror::Error;

/// Env var name that this module reads.
pub const ENV_VAR: &str = "THUMBNAIL_SIZES";

/// Inclusive lower bound on each dimension.
pub const DIM_MIN: u32 = 1;

/// Inclusive upper bound on each dimension.
///
/// Deliberately tighter than the 8192 a caller-supplied specification may ask
/// for (`thumbnail_common::work_events::MAX_DIMENSION`). These are the sizes
/// every eligible upload is fanned out to, so the cost of a large one is paid on
/// every CREATE rather than once per request.
pub const DIM_MAX: u32 = 4096;

/// Maximum number of `(width, height)` pairs accepted.
pub const MAX_ENTRIES: usize = 10;

/// Parsed list of `(width, height)` thumbnail sizes.
///
/// Guarantees, after successful construction:
/// - non-empty
/// - at most `MAX_ENTRIES` entries
/// - every `(width, height)` is unique
/// - every dimension satisfies `DIM_MIN ..= DIM_MAX`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailSizes(Vec<(u32, u32)>);

impl ThumbnailSizes {
    /// Borrow the parsed sizes as a slice.
    pub fn as_slice(&self) -> &[(u32, u32)] {
        &self.0
    }

    /// Render this list back to the canonical env-var form (`WxH,WxH,...`).
    pub fn to_env_value(&self) -> String {
        self.0
            .iter()
            .map(|(w, h)| format!("{w}x{h}"))
            .collect::<Vec<String>>()
            .join(",")
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if there are zero entries. Never `true` for a value
    /// constructed via the public constructors; provided for API symmetry.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterator over the `(width, height)` pairs.
    pub fn iter(&self) -> std::slice::Iter<'_, (u32, u32)> {
        self.0.iter()
    }
}

impl IntoIterator for ThumbnailSizes {
    type Item = (u32, u32);
    type IntoIter = std::vec::IntoIter<(u32, u32)>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a ThumbnailSizes {
    type Item = &'a (u32, u32);
    type IntoIter = std::slice::Iter<'a, (u32, u32)>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl FromStr for ThumbnailSizes {
    type Err = ThumbnailSizesError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ThumbnailSizesError::Empty);
        }

        let mut sizes: Vec<(u32, u32)> = Vec::new();

        for raw_entry in trimmed.split(',') {
            let entry = raw_entry.trim();
            if entry.is_empty() {
                return Err(ThumbnailSizesError::Invalid {
                    reason: format!("empty entry in `{}` (check for stray commas)", trimmed),
                });
            }

            let (w_raw, h_raw) =
                entry
                    .split_once('x')
                    .ok_or_else(|| ThumbnailSizesError::Invalid {
                        reason: format!(
                            "entry `{}` is missing the `x` separator (expected `WxH`)",
                            entry
                        ),
                    })?;

            let w_str = w_raw.trim();
            let h_str = h_raw.trim();

            if w_str.is_empty() || h_str.is_empty() {
                return Err(ThumbnailSizesError::Invalid {
                    reason: format!(
                        "entry `{}` is missing width or height around the `x`",
                        entry
                    ),
                });
            }

            let width: u32 = w_str.parse().map_err(|_| ThumbnailSizesError::Invalid {
                reason: format!(
                    "entry `{}`: width `{}` is not a non-negative integer",
                    entry, w_str
                ),
            })?;
            let height: u32 = h_str.parse().map_err(|_| ThumbnailSizesError::Invalid {
                reason: format!(
                    "entry `{}`: height `{}` is not a non-negative integer",
                    entry, h_str
                ),
            })?;

            if !(DIM_MIN..=DIM_MAX).contains(&width) || !(DIM_MIN..=DIM_MAX).contains(&height) {
                return Err(ThumbnailSizesError::OutOfRange {
                    entry: entry.to_string(),
                    min: DIM_MIN,
                    max: DIM_MAX,
                });
            }

            if sizes.contains(&(width, height)) {
                return Err(ThumbnailSizesError::Duplicate {
                    entry: entry.to_string(),
                });
            }

            sizes.push((width, height));

            if sizes.len() > MAX_ENTRIES {
                return Err(ThumbnailSizesError::TooManyEntries {
                    count: sizes.len(),
                    max: MAX_ENTRIES,
                });
            }
        }

        Ok(ThumbnailSizes(sizes))
    }
}

/// Errors produced when reading or parsing the `THUMBNAIL_SIZES` env var.
///
/// Every variant's `Display` includes:
/// - the env var name (`THUMBNAIL_SIZES`),
/// - the offending substring or condition,
/// - a valid example value.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ThumbnailSizesError {
    #[error(
        "THUMBNAIL_SIZES is required but not set. Set it to a comma-separated `WxH` list, e.g. `64x64,128x128,256x256`"
    )]
    Missing,

    #[error(
        "THUMBNAIL_SIZES is empty. Set it to a comma-separated `WxH` list, e.g. `64x64,128x128,256x256`"
    )]
    Empty,

    #[error(
        "THUMBNAIL_SIZES value is malformed: {reason}. Example of a valid value: `64x64,128x128,256x256`"
    )]
    Invalid { reason: String },

    #[error(
        "THUMBNAIL_SIZES entry `{entry}` has a dimension outside the allowed range {min}..={max}. Example of a valid value: `64x64,128x128,256x256`"
    )]
    OutOfRange { entry: String, min: u32, max: u32 },

    #[error(
        "THUMBNAIL_SIZES contains duplicate entry `{entry}`. Each `WxH` pair must appear at most once. Example: `64x64,128x128,256x256`"
    )]
    Duplicate { entry: String },

    #[error(
        "THUMBNAIL_SIZES has {count} entries; the maximum allowed is {max}. Example: `64x64,128x128,256x256`"
    )]
    TooManyEntries { count: usize, max: usize },
}

impl From<ThumbnailSizesError> for String {
    fn from(err: ThumbnailSizesError) -> String {
        err.to_string()
    }
}
