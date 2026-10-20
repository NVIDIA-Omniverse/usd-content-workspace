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

//! Alpha handling and the four output encodings.

use std::io::Cursor;

use image::codecs::gif::GifEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{DynamicImage, ImageEncoder};

use crate::spec::{ImageSpec, OutputFormat};
use crate::ConversionError;

/// Composite alpha and palette sources onto white before a JPEG encode.
///
/// JPEG cannot carry alpha, and the alternative to compositing is a black
/// background — which is what a naive drop of the alpha channel produces, and
/// not what the Python worker emitted. Formats that can carry alpha are left
/// untouched.
pub(crate) fn flatten_for_output(image: DynamicImage, format: OutputFormat) -> DynamicImage {
    if format.supports_alpha() || !image.color().has_alpha() {
        return image;
    }

    // into_rgba8() consumes `image` and returns its inner RgbaImage when the
    // layout is already ImageRgba8 (no extra allocation). Any other alpha
    // layout allocates a new 4 B/px RGBA8 buffer while the original is still
    // alive, then drops the original before this blend loop. Peak is therefore
    // the decoded buffer plus at most one 4 B/px buffer: 4 B/px for Rgba8,
    // 8 B/px for La16, 12 B/px for Rgba16. The 64 MP worst case is a 16-bit
    // gray+alpha PNG (La16): Rgba16 is refused by max_alloc before 64 MP.
    // Chart-default PNG output never reaches this function. The previous
    // to_rgba8() borrow plus a third blended buffer was 3× the decoded size.
    let mut buf = image.into_rgba8();
    for pixel in buf.pixels_mut() {
        let alpha = f32::from(pixel[3]) / 255.0;
        let blend = |over: u8| (f32::from(over) * alpha + 255.0 * (1.0 - alpha)).round() as u8;
        pixel[0] = blend(pixel[0]);
        pixel[1] = blend(pixel[1]);
        pixel[2] = blend(pixel[2]);
        pixel[3] = 255;
    }
    DynamicImage::ImageRgba8(buf)
}

/// Encode the transformed image.
///
/// Takes ownership so the layout conversions below can consume the buffer
/// instead of copying it. `into_rgb8`/`into_rgba8` are moves when the variant
/// already matches and conversions otherwise, so the saving is one full-image
/// allocation on the matching path — which is the common one, since the transform
/// produces RGBA8 and chart-default output is PNG. `image.color()` is read before
/// the move because the conversion consumes it.
pub(crate) fn encode(image: DynamicImage, spec: &ImageSpec) -> Result<Vec<u8>, ConversionError> {
    let has_alpha = image.color().has_alpha();
    let mut buffer = Vec::new();
    let failed = |reason: String| ConversionError::Encode {
        format: spec.format,
        reason,
    };

    match spec.format {
        OutputFormat::Jpeg => {
            let rgb = image.into_rgb8();
            let quality = u8::try_from(spec.quality).unwrap_or(u8::MAX);
            JpegEncoder::new_with_quality(&mut buffer, quality)
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(|e| failed(e.to_string()))?;
        }
        OutputFormat::Png => {
            // Keep the channel layout the transform produced rather than forcing
            // RGBA: a source with no alpha should not gain one on the way out.
            let encoder = PngEncoder::new(&mut buffer);
            if has_alpha {
                let rgba = image.into_rgba8();
                encoder
                    .write_image(
                        rgba.as_raw(),
                        rgba.width(),
                        rgba.height(),
                        image::ExtendedColorType::Rgba8,
                    )
                    .map_err(|e| failed(e.to_string()))?;
            } else {
                let rgb = image.into_rgb8();
                encoder
                    .write_image(
                        rgb.as_raw(),
                        rgb.width(),
                        rgb.height(),
                        image::ExtendedColorType::Rgb8,
                    )
                    .map_err(|e| failed(e.to_string()))?;
            }
        }
        OutputFormat::Gif => {
            let rgba = image.into_rgba8();
            let mut encoder = GifEncoder::new(Cursor::new(&mut buffer));
            encoder
                .encode(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|e| failed(e.to_string()))?;
        }
        OutputFormat::Webp => {
            // Lossless only: the pure-Rust encoder has no lossy mode, so
            // `quality` is deliberately unused here. See the crate docs.
            let encoder = WebPEncoder::new_lossless(&mut buffer);
            if has_alpha {
                let rgba = image.into_rgba8();
                encoder
                    .write_image(
                        rgba.as_raw(),
                        rgba.width(),
                        rgba.height(),
                        image::ExtendedColorType::Rgba8,
                    )
                    .map_err(|e| failed(e.to_string()))?;
            } else {
                let rgb = image.into_rgb8();
                encoder
                    .write_image(
                        rgb.as_raw(),
                        rgb.width(),
                        rgb.height(),
                        image::ExtendedColorType::Rgb8,
                    )
                    .map_err(|e| failed(e.to_string()))?;
            }
        }
    }

    Ok(buffer)
}
