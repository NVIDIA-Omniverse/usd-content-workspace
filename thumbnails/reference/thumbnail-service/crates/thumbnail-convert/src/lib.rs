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

//! Source bytes plus a resolved specification to thumbnail bytes.
//!
//! This is the whole conversion core, and it knows nothing about where its input
//! came from or what happens to its output. That is the point: the worker
//! consumes a queue and decides acknowledgements, and the approved synchronous
//! `createThumbnail` mode will answer an HTTP request, and both need exactly this
//! function. Building it inside the worker binary would force an extraction the
//! moment the second caller arrived.
//!
//! # Parity with the Pillow implementation it replaces
//!
//! The Python worker used Pillow 10.4, and the observable pixel behavior here is
//! deliberately the same for every value the contract can carry: all eight
//! transforms, all nine crop anchors, `preserve_aspect_ratio` per transform,
//! alpha and palette flattened onto white for JPEG, and the first frame only for
//! animated sources. Lanczos resampling matches Pillow's `LANCZOS`.
//!
//! One divergence, documented rather than hidden: **WebP is encoded losslessly
//! and `quality` is ignored on WebP output.** The pure-Rust encoder has no lossy
//! mode, and linking libwebp is a poor trade for a non-default encoding — every
//! deployment configures PNG today. WebP thumbnails are therefore larger than the
//! Python worker's.

mod bounds;
mod encode;
mod spec;
mod transform;

use std::io::Cursor;

use image::{DynamicImage, ImageReader};

pub use bounds::DecodeLimits;
pub use spec::{CropStrategy, ImageSpec, OutputFormat, Transform};

/// File extensions this build can decode, lowercase and without the dot.
///
/// The authoritative list, published so the eligibility gate on the CREATE path
/// does not have to keep a second copy in step by hand. An extension that is
/// eligible but undecodable costs a download and a permanent rejection to reach
/// the same outcome as never publishing the work; one that is decodable but
/// ineligible silently produces no thumbnail at all. Both are drift between two
/// lists, so there is one list.
///
/// Adding an entry means adding the decoder — see the module docs on why `svg`,
/// `heic`, `heif`, and `avif` are absent.
pub const SUPPORTED_INPUT_EXTENSIONS: [&str; 9] = [
    "bmp", "gif", "ico", "jpeg", "jpg", "png", "tif", "tiff", "webp",
];

/// Why a conversion did not produce bytes.
///
/// Every variant is a property of the input or the specification, so none of
/// them improves on a retry. Callers that acknowledge deliveries can treat the
/// whole enum as permanent; [`ConversionError::failure_class`] gives them the
/// label to record it under.
#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error("source is {actual} bytes, over the {limit}-byte bound")]
    SourceTooLarge { actual: u64, limit: u64 },

    #[error("source declares {width}x{height} ({pixels} pixels), over the {limit}-pixel bound")]
    PixelBudgetExceeded {
        width: u32,
        height: u32,
        pixels: u64,
        limit: u64,
    },

    #[error(
        "resizing {source_width}x{source_height} to {new_width}x{new_height} would peak at \
         {peak} bytes, over the {limit}-byte bound"
    )]
    TransformBudgetExceeded {
        source_width: u32,
        source_height: u32,
        new_width: u64,
        new_height: u64,
        peak: u64,
        limit: u64,
    },

    #[error("{what} would peak at {peak} bytes, over the {limit}-byte bound")]
    AllocationBudgetExceeded {
        what: &'static str,
        peak: u64,
        limit: u64,
    },

    #[error("source is empty")]
    EmptySource,

    #[error("source format could not be recognized")]
    UnrecognizedSourceFormat,

    #[error("source format {0:?} has no decoder in this build")]
    UnsupportedSourceFormat(image::ImageFormat),

    #[error("source could not be decoded: {0}")]
    Decode(String),

    #[error("thumbnail could not be encoded as {format:?}: {reason}")]
    Encode {
        format: OutputFormat,
        reason: String,
    },

    #[error("specification is not usable: {0}")]
    InvalidSpecification(String),
}

/// The coarse class a failure is counted under, so metrics and logs agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionFailureClass {
    /// The specification could not produce an image at all.
    Specification,
    /// The source exceeded a configured decode bound.
    Bounds,
    /// No decoder in this build handles the source.
    UnsupportedFormat,
    /// A decoder exists but the bytes did not decode.
    Decode,
    /// The transform succeeded but the output could not be written.
    Encode,
}

impl ConversionError {
    pub fn failure_class(&self) -> ConversionFailureClass {
        match self {
            Self::SourceTooLarge { .. }
            | Self::PixelBudgetExceeded { .. }
            | Self::TransformBudgetExceeded { .. }
            | Self::AllocationBudgetExceeded { .. } => ConversionFailureClass::Bounds,
            Self::UnrecognizedSourceFormat | Self::UnsupportedSourceFormat(_) => {
                ConversionFailureClass::UnsupportedFormat
            }
            Self::EmptySource | Self::Decode(_) => ConversionFailureClass::Decode,
            Self::Encode { .. } => ConversionFailureClass::Encode,
            Self::InvalidSpecification(_) => ConversionFailureClass::Specification,
        }
    }
}

/// Convert `source` into thumbnail bytes.
///
/// Takes ownership of `source` so the encoded source-byte buffer can be
/// explicitly released immediately after decode completes, before transform
/// and encode run. This keeps peak memory lower: without an early drop the
/// source bytes, the decoded pixel buffer, and the encoded output buffer
/// would all be resident at once for the back half of every conversion.
///
/// Bounds are checked before anything is allocated for the decode, because an
/// out-of-memory abort is strictly worse than a rejection: the process dies with
/// the work unacknowledged, and the same bytes reproduce the crash on every
/// redelivery.
pub fn convert(
    source: Vec<u8>,
    spec: &ImageSpec,
    limits: &DecodeLimits,
) -> Result<Vec<u8>, ConversionError> {
    spec.validate()?;

    let decoded = decode_within_bounds(&source, limits)?;
    // Release the encoded source bytes now that decode is done. Transform and
    // encode work on the decoded pixel buffer (`decoded`), not the original
    // bytes, so dropping here prevents the source allocation from overlapping
    // with the decoded-pixel and encoded-output buffers for the rest of the
    // conversion.
    drop(source);
    // Flattening happens before the transform, matching Pillow: a letterbox
    // added by FIT is then already opaque and needs no second flatten.
    let prepared = encode::flatten_for_output(decoded, spec.format);
    let transformed = transform::apply(prepared, spec, limits)?;

    encode::encode(transformed, spec)
}

/// Decode `source`, refusing anything past the configured bounds.
///
/// Animated GIF and WebP sources decode their first frame, which is what the
/// Python worker produced and what `image` returns for a `DynamicImage`.
fn decode_within_bounds(
    source: &[u8],
    limits: &DecodeLimits,
) -> Result<DynamicImage, ConversionError> {
    if source.is_empty() {
        return Err(ConversionError::EmptySource);
    }
    limits.check_source_bytes(source.len() as u64)?;

    let format = ImageReader::new(Cursor::new(source))
        .with_guessed_format()
        .map_err(|e| ConversionError::Decode(e.to_string()))?
        .format()
        .ok_or(ConversionError::UnrecognizedSourceFormat)?;

    // Dimensions come from the header, so an oversized image is refused before a
    // buffer for it is ever allocated.
    let (width, height) = ImageReader::new(Cursor::new(source))
        .with_guessed_format()
        .map_err(|e| ConversionError::Decode(e.to_string()))?
        .into_dimensions()
        .map_err(|e| match e {
            image::ImageError::Unsupported(_) => ConversionError::UnsupportedSourceFormat(format),
            other => ConversionError::Decode(other.to_string()),
        })?;
    limits.check_decoded_pixels(width, height)?;

    let mut reader = ImageReader::new(Cursor::new(source))
        .with_guessed_format()
        .map_err(|e| ConversionError::Decode(e.to_string()))?;
    // A second, independent bound inside the decoder: a header can lie about its
    // dimensions, and this one is enforced against what is actually allocated.
    reader.limits(limits.to_image_limits());

    reader.decode().map_err(|e| match e {
        image::ImageError::Unsupported(_) => ConversionError::UnsupportedSourceFormat(format),
        image::ImageError::Limits(_) => ConversionError::PixelBudgetExceeded {
            width,
            height,
            pixels: u64::from(width) * u64::from(height),
            limit: limits.max_decoded_pixels,
        },
        other => ConversionError::Decode(other.to_string()),
    })
}

#[cfg(test)]
mod tests;
