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

//! The conversion crate's own view of a resolved specification.
//!
//! These mirror the contract enums in `thumbnail-common` but are declared here so
//! this crate stays a leaf. Callers map their enum onto these with an exhaustive
//! `match`, which turns a new variant upstream into a compile error at the
//! mapping rather than a silent fallthrough here.

use crate::ConversionError;

/// The encodings the contract defines. Inputs are wider than outputs: BMP, TIFF,
/// and ICO decode but are never written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Jpeg,
    Png,
    Gif,
    Webp,
}

impl OutputFormat {
    /// Whether `quality` means anything for this encoding.
    ///
    /// JPEG only. PNG and GIF are lossless by nature, and WebP is lossless here
    /// because the pure-Rust encoder has no lossy mode.
    pub fn honors_quality(self) -> bool {
        matches!(self, OutputFormat::Jpeg)
    }

    /// Whether this encoding can carry an alpha channel.
    pub fn supports_alpha(self) -> bool {
        !matches!(self, OutputFormat::Jpeg)
    }
}

/// How the source is fitted to the requested dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    /// Crop to a centred square at the smaller dimension, then resize.
    Square,
    /// Scale to the target width, deriving height from the aspect ratio.
    ScaleByWidth,
    /// Scale to the target height, deriving width from the aspect ratio.
    ScaleByHeight,
    /// Crop to the target box at the requested anchor.
    Crop,
    /// Fit inside the target box, letterboxing the remainder.
    Fit,
    /// Scale to fill the target box, then crop the overflow.
    ScaleAndCrop,
    /// Fit inside the target box without ever upscaling.
    Thumbnail,
    /// Resize to exactly the target dimensions, ignoring aspect ratio.
    Resize,
}

/// Which part of an oversized image a crop keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropStrategy {
    Center,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// Content-aware crop: uses smartcrop2 to find the most visually interesting
    /// region. Falls back to [`CropStrategy::Center`] if analysis fails.
    Smart,
}

/// A fully resolved conversion specification. Nothing here is optional: the
/// publisher resolved every field before the work was dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSpec {
    pub width: u32,
    pub height: u32,
    pub format: OutputFormat,
    pub transform: Transform,
    pub crop: CropStrategy,
    /// Applied to JPEG output only; see [`OutputFormat::honors_quality`].
    pub quality: u32,
    pub preserve_aspect_ratio: bool,
}

impl ImageSpec {
    pub(crate) fn validate(&self) -> Result<(), ConversionError> {
        if self.width == 0 || self.height == 0 {
            return Err(ConversionError::InvalidSpecification(format!(
                "width and height must both be non-zero, got {}x{}",
                self.width, self.height
            )));
        }
        if self.format.honors_quality() && !(1..=100).contains(&self.quality) {
            return Err(ConversionError::InvalidSpecification(format!(
                "quality must be between 1 and 100 for {:?}, got {}",
                self.format, self.quality
            )));
        }
        Ok(())
    }
}
