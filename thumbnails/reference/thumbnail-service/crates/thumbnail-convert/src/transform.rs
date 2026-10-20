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

//! The eight transforms, reproducing the Pillow implementation's geometry.
//!
//! The arithmetic here is copied from `app/core/image_processor.py` rather than
//! reinvented, down to the truncation and floor-division, because "roughly the
//! same family of operation" would silently move every thumbnail by a pixel.

use std::num::NonZeroU32;

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

use crate::bounds::DecodeLimits;
use crate::spec::{CropStrategy, ImageSpec, Transform};
use crate::ConversionError;

/// Pillow's `Image.Resampling.LANCZOS`.
const RESAMPLE: FilterType = FilterType::Lanczos3;

pub(crate) fn apply(
    image: DynamicImage,
    spec: &ImageSpec,
    limits: &DecodeLimits,
) -> Result<DynamicImage, ConversionError> {
    match spec.transform {
        Transform::Square => square(image, spec, limits),
        Transform::ScaleByWidth => scale_by_width(image, spec, limits),
        Transform::ScaleByHeight => scale_by_height(image, spec, limits),
        Transform::Crop => crop(
            image,
            spec.width,
            spec.height,
            spec.crop,
            spec.preserve_aspect_ratio,
            limits,
        ),
        Transform::Fit => fit(image, spec, limits),
        Transform::ScaleAndCrop => scale_and_crop(image, spec, limits),
        Transform::Thumbnail => thumbnail(image, spec, limits),
        Transform::Resize => {
            resize_checked(image, u64::from(spec.width), u64::from(spec.height), limits)
        }
    }
}

/// The single door every resample goes through.
///
/// `resize_exact` allocates a working buffer sized by the *source* width and the
/// *target* height, so a transform whose output dimensions are fixed by the
/// request is still unbounded in what it allocates to produce them. Checking
/// here rather than in each transform is what makes that true of all of them at
/// once, including the ones added later.
///
/// Targets are `u64` because a dimension derived from the source aspect ratio
/// can exceed `u32`; the narrowing happens inside the check, after the bound.
fn resize_checked(
    image: DynamicImage,
    new_width: u64,
    new_height: u64,
    limits: &DecodeLimits,
) -> Result<DynamicImage, ConversionError> {
    resize_checked_holding(image, new_width, new_height, 0, limits)
}

/// As [`resize_checked`], for a caller holding `extra_resident_bytes` of its own
/// buffers across the resample.
///
/// Only `smart_crop` needs it: it keeps a full RGB copy of the source alive while
/// resizing the region the analyzer chose, and that copy is resident alongside
/// the source, the working buffer and the result.
fn resize_checked_holding(
    image: DynamicImage,
    new_width: u64,
    new_height: u64,
    extra_resident_bytes: u64,
    limits: &DecodeLimits,
) -> Result<DynamicImage, ConversionError> {
    let (source_width, source_height) = image.dimensions();
    let bytes_per_pixel = u64::from(image.color().bytes_per_pixel());

    let (new_width, new_height) = limits.check_resample(
        source_width,
        source_height,
        bytes_per_pixel,
        new_width,
        new_height,
        extra_resident_bytes,
    )?;

    Ok(image.resize_exact(new_width, new_height, RESAMPLE))
}

/// Scale a dimension by `factor`, in `f64`, without narrowing.
///
/// Pillow truncates toward zero, and so does this; the result stays `u64` so an
/// overflow reaches [`DecodeLimits::check_resample`] as a rejection instead of
/// saturating at `u32::MAX` on the way there.
fn scaled(dimension: u32, factor: f64) -> u64 {
    let scaled = f64::from(dimension) * factor;
    if !scaled.is_finite() || scaled < 0.0 {
        return 0;
    }
    // Above u64::MAX the cast saturates, which check_resample rejects anyway.
    (scaled as u64).max(1)
}

/// Crop to the largest centred square the source allows, then resize to a square
/// of `min(width, height)` — Pillow forces the result square even when the
/// requested box is not.
fn square(
    image: DynamicImage,
    spec: &ImageSpec,
    limits: &DecodeLimits,
) -> Result<DynamicImage, ConversionError> {
    let (width, height) = image.dimensions();
    let side = width.min(height);

    let squared = crop(image, side, side, CropStrategy::Center, false, limits)?;
    let target = u64::from(spec.width.min(spec.height));
    resize_checked(squared, target, target, limits)
}

fn scale_by_width(
    image: DynamicImage,
    spec: &ImageSpec,
    limits: &DecodeLimits,
) -> Result<DynamicImage, ConversionError> {
    let (width, height) = image.dimensions();
    // `int(original_height * scale_factor)` in Python truncates toward zero.
    let scaled_height = scaled(height, f64::from(spec.width) / f64::from(width));
    resize_checked(image, u64::from(spec.width), scaled_height, limits)
}

fn scale_by_height(
    image: DynamicImage,
    spec: &ImageSpec,
    limits: &DecodeLimits,
) -> Result<DynamicImage, ConversionError> {
    let (width, height) = image.dimensions();
    let scaled_width = scaled(width, f64::from(spec.height) / f64::from(height));
    resize_checked(image, scaled_width, u64::from(spec.height), limits)
}

/// Fit inside the target box. With `preserve_aspect_ratio` the remainder is
/// letterboxed onto a white canvas; without it this is a plain exact resize.
fn fit(
    image: DynamicImage,
    spec: &ImageSpec,
    limits: &DecodeLimits,
) -> Result<DynamicImage, ConversionError> {
    if !spec.preserve_aspect_ratio {
        return resize_checked(image, u64::from(spec.width), u64::from(spec.height), limits);
    }

    let (width, height) = image.dimensions();
    let scale =
        (f64::from(spec.width) / f64::from(width)).min(f64::from(spec.height) / f64::from(height));
    let new_width = scaled(width, scale);
    let new_height = scaled(height, scale);
    let resized = resize_checked(image, new_width, new_height, limits)?;
    let (new_width, new_height) = resized.dimensions();
    let resized = resized.to_rgba8();

    // Opaque white for JPEG, transparent white otherwise — Pillow picks the
    // canvas mode from the output format, and the difference is visible wherever
    // the letterbox falls.
    let background = if spec.format.supports_alpha() {
        Rgba([255, 255, 255, 0])
    } else {
        Rgba([255, 255, 255, 255])
    };
    // The canvas is allocated after the resize, so the resample check above did not
    // see it, and both are resident together. At the 8192 ceiling the canvas alone
    // is 256 MiB.
    let resized_bytes = u64::from(new_width)
        .saturating_mul(u64::from(new_height))
        .saturating_mul(4);
    let canvas_bytes = u64::from(spec.width)
        .saturating_mul(u64::from(spec.height))
        .saturating_mul(4);
    limits.check_allocation(resized_bytes, canvas_bytes, "the FIT letterbox canvas")?;

    let mut canvas = RgbaImage::from_pixel(spec.width, spec.height, background);

    // Pillow's `paste` without a mask overwrites the destination pixels
    // outright, alpha included, which is what `replace` does.
    let x = (spec.width.saturating_sub(new_width) / 2) as i64;
    let y = (spec.height.saturating_sub(new_height) / 2) as i64;
    image::imageops::replace(&mut canvas, &resized, x, y);

    Ok(DynamicImage::ImageRgba8(canvas))
}

/// Scale to cover the target box, then crop the overflow at the anchor.
fn scale_and_crop(
    image: DynamicImage,
    spec: &ImageSpec,
    limits: &DecodeLimits,
) -> Result<DynamicImage, ConversionError> {
    let (width, height) = image.dimensions();
    let scale =
        (f64::from(spec.width) / f64::from(width)).max(f64::from(spec.height) / f64::from(height));
    let new_width = scaled(width, scale);
    let new_height = scaled(height, scale);
    let covered = resize_checked(image, new_width, new_height, limits)?;

    crop(covered, spec.width, spec.height, spec.crop, false, limits)
}

/// Crop to `target_width` × `target_height` at `anchor`.
///
/// With `preserve_aspect_ratio` the source is first scaled to cover the target,
/// so the crop takes a full box rather than whatever the source happened to
/// have. The target is clamped to the image, matching Pillow, so a crop larger
/// than the source yields the source rather than an error.
///
/// [`CropStrategy::Smart`] bypasses both the pre-scale and the fixed-origin
/// logic entirely: it calls [`smart_crop`] to let smartcrop2 pick the region.
fn crop(
    image: DynamicImage,
    target_width: u32,
    target_height: u32,
    anchor: CropStrategy,
    preserve_aspect_ratio: bool,
    limits: &DecodeLimits,
) -> Result<DynamicImage, ConversionError> {
    if anchor == CropStrategy::Smart {
        return smart_crop(
            image,
            target_width,
            target_height,
            preserve_aspect_ratio,
            limits,
        );
    }

    let image = if preserve_aspect_ratio {
        let (width, height) = image.dimensions();
        let scale = (f64::from(target_width) / f64::from(width))
            .max(f64::from(target_height) / f64::from(height));
        let new_width = scaled(width, scale);
        let new_height = scaled(height, scale);
        resize_checked(image, new_width, new_height, limits)?
    } else {
        image
    };

    let (image_width, image_height) = image.dimensions();
    let width = target_width.min(image_width);
    let height = target_height.min(image_height);
    let (x, y) = crop_origin(image_width, image_height, width, height, anchor);

    Ok(image.crop_imm(x, y, width, height))
}

/// Content-aware crop using smartcrop2.
///
/// Converts the image to RGB8 (the format smartcrop's trait bounds require),
/// runs the saliency analysis to find the best crop region at the requested
/// dimensions, then extracts and resizes that region from the original image.
///
/// Falls back to a center crop if `target_width` or `target_height` is zero, if
/// the analysis itself fails, or if the region it returns does not lie inside
/// the source, so callers always receive a valid thumbnail.
///
/// That last fallback is not defensive dressing. `crop_imm` panics on a region
/// that runs past the image, the analyzer is a third-party component under no
/// obligation to this crate's invariants, and the worker classifies a conversion
/// panic as *retryable* — so an out-of-range region would redeliver the whole
/// pipeline indefinitely rather than fail once. This path is also the shipped
/// default (`CROP` + `SMART`), which is the wrong place to trust an external
/// contract.
fn smart_crop(
    image: DynamicImage,
    target_width: u32,
    target_height: u32,
    preserve_aspect_ratio: bool,
    limits: &DecodeLimits,
) -> Result<DynamicImage, ConversionError> {
    let (Some(w), Some(h)) = (
        NonZeroU32::new(target_width),
        NonZeroU32::new(target_height),
    ) else {
        return crop(
            image,
            target_width,
            target_height,
            CropStrategy::Center,
            preserve_aspect_ratio,
            limits,
        );
    };

    // smartcrop requires a `GenericImage<Pixel = P>` with `P: Pixel<Subpixel = u8>`.
    // Converting to Rgb8 satisfies that bound without altering the source geometry.
    //
    // This copy is a full 3 B/px image and stays alive through the analysis *and*
    // the resize below, so it is resident alongside every buffer the resample
    // allocates. Checked before it is made, and passed to the resample check after.
    let (image_width, image_height) = image.dimensions();
    let rgb_copy_bytes = u64::from(image_width)
        .saturating_mul(u64::from(image_height))
        .saturating_mul(3);
    let source_bytes = u64::from(image_width)
        .saturating_mul(u64::from(image_height))
        .saturating_mul(u64::from(image.color().bytes_per_pixel()));
    limits.check_allocation(
        source_bytes,
        rgb_copy_bytes,
        "the content-aware crop's RGB copy",
    )?;

    let rgb = image.to_rgb8();

    match smartcrop::find_best_crop(&rgb, w, h) {
        Ok(result) if region_fits(&result.crop, image_width, image_height) => {
            let c = result.crop;
            let cropped = image.crop_imm(c.x, c.y, c.width, c.height);
            if c.width == target_width && c.height == target_height {
                Ok(cropped)
            } else {
                resize_checked_holding(
                    cropped,
                    u64::from(target_width),
                    u64::from(target_height),
                    rgb_copy_bytes,
                    limits,
                )
            }
        }
        _ => crop(
            image,
            target_width,
            target_height,
            CropStrategy::Center,
            preserve_aspect_ratio,
            limits,
        ),
    }
}

/// Whether a crop region lies wholly inside a `width` × `height` image.
fn region_fits(region: &smartcrop::Crop, width: u32, height: u32) -> bool {
    region
        .x
        .checked_add(region.width)
        .zip(region.y.checked_add(region.height))
        .is_some_and(|(right, bottom)| right <= width && bottom <= height)
}

/// Where the crop box starts, for each of the nine anchors.
fn crop_origin(
    image_width: u32,
    image_height: u32,
    width: u32,
    height: u32,
    anchor: CropStrategy,
) -> (u32, u32) {
    let centre_x = (image_width - width) / 2;
    let centre_y = (image_height - height) / 2;
    let right = image_width - width;
    let bottom = image_height - height;

    match anchor {
        CropStrategy::Center => (centre_x, centre_y),
        CropStrategy::Top => (centre_x, 0),
        CropStrategy::Bottom => (centre_x, bottom),
        CropStrategy::Left => (0, centre_y),
        CropStrategy::Right => (right, centre_y),
        CropStrategy::TopLeft => (0, 0),
        CropStrategy::TopRight => (right, 0),
        CropStrategy::BottomLeft => (0, bottom),
        CropStrategy::BottomRight => (right, bottom),
        // Smart is handled before crop_origin is reached (see `crop()`). Falling
        // back rather than panicking: this is unreachable only for as long as
        // `crop()` keeps its early return, and a panic here is a request-driven
        // abort that the worker classifies as retryable -- so the failure mode of
        // being wrong is a redelivery loop, not a clean error. The centre anchor
        // is what `smart_crop` itself falls back to.
        CropStrategy::Smart => {
            debug_assert!(
                false,
                "CropStrategy::Smart must be handled before crop_origin"
            );
            (centre_x, centre_y)
        }
    }
}

/// Pillow's `Image.thumbnail`: fit inside the box, never upscale.
fn thumbnail(
    image: DynamicImage,
    spec: &ImageSpec,
    limits: &DecodeLimits,
) -> Result<DynamicImage, ConversionError> {
    let (width, height) = image.dimensions();

    // Pillow returns the image untouched when the box already contains it.
    if spec.width >= width && spec.height >= height {
        return Ok(image);
    }

    let aspect = f64::from(width) / f64::from(height);
    let (mut target_width, mut target_height) = (spec.width, spec.height);

    if f64::from(spec.width) / f64::from(spec.height) >= aspect {
        let ideal = f64::from(spec.height) * aspect;
        target_width = round_aspect(ideal, |candidate| {
            (aspect - f64::from(candidate) / f64::from(spec.height)).abs()
        });
    } else {
        let ideal = f64::from(spec.width) / aspect;
        target_height = round_aspect(ideal, |candidate| {
            if candidate == 0 {
                0.0
            } else {
                (aspect - f64::from(spec.width) / f64::from(candidate)).abs()
            }
        });
    }

    if (target_width, target_height) == (width, height) {
        return Ok(image);
    }
    resize_checked(
        image,
        u64::from(target_width),
        u64::from(target_height),
        limits,
    )
}

/// Pillow's `round_aspect`: take whichever of floor and ceil sits closer to the
/// source aspect ratio, with floor winning ties, and never return zero.
fn round_aspect(number: f64, distance: impl Fn(u32) -> f64) -> u32 {
    let floor = number.floor().max(0.0) as u32;
    let ceil = number.ceil().max(0.0) as u32;

    // Python's `min` keeps the first argument on a tie, and `floor` is first.
    let closest = if distance(ceil) < distance(floor) {
        ceil
    } else {
        floor
    };
    closest.max(1)
}

#[cfg(test)]
mod tests {
    use super::{crop_origin, region_fits, smart_crop};

    fn region(x: u32, y: u32, width: u32, height: u32) -> smartcrop::Crop {
        smartcrop::Crop {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_region_inside_the_image_fits() {
        assert!(region_fits(&region(0, 0, 100, 100), 100, 100));
        assert!(region_fits(&region(10, 20, 50, 40), 100, 100));
    }

    #[test]
    fn a_region_running_past_an_edge_does_not_fit() {
        assert!(!region_fits(&region(60, 0, 50, 10), 100, 100), "past right");
        assert!(
            !region_fits(&region(0, 60, 10, 50), 100, 100),
            "past bottom"
        );
    }

    /// The analyzer is third-party and `crop_imm` panics rather than erroring,
    /// so an overflowing region has to be caught here — a panic in conversion is
    /// classified retryable and would redeliver the whole pipeline forever.
    #[test]
    fn a_region_whose_extent_overflows_does_not_fit() {
        assert!(!region_fits(&region(u32::MAX, 0, 1, 1), 100, 100));
        assert!(!region_fits(&region(0, 1, u32::MAX, u32::MAX), 100, 100));
    }

    // --- smart_crop fallbacks ---------------------------------------------
    //
    // Both are unreachable through `convert`: spec validation rejects a zero
    // requested dimension before the transform runs, and the decode bounds reject
    // an empty source, which is the only input that makes the analyzer error. So
    // they are exercised here, from inside the module, rather than left uncovered
    // on the grounds that the public path cannot reach them — this path is the
    // shipped default (`CROP` + `SMART`), and the fallbacks are what keep a
    // third-party analyzer from turning into a panic.

    use crate::bounds::DecodeLimits;
    use crate::spec::CropStrategy;
    use image::{DynamicImage, GenericImageView, RgbaImage};

    fn opaque(width: u32, height: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            width,
            height,
            image::Rgba([10, 120, 200, 255]),
        ))
    }

    /// A zero requested dimension falls back to the deterministic centre crop
    /// rather than constructing a `NonZeroU32` that cannot exist.
    #[test]
    fn a_zero_target_dimension_falls_back_to_the_centre_crop() {
        for (width, height) in [(0, 32), (32, 0)] {
            let result = smart_crop(
                opaque(64, 64),
                width,
                height,
                false,
                &DecodeLimits::default(),
            )
            .expect("the fallback must produce an image, not an error");

            // `crop` clamps the target to the image, so a zero dimension yields a
            // zero-extent crop. What matters is that it neither panics nor errors.
            let (out_width, out_height) = result.dimensions();
            assert!(
                out_width == width || out_height == height,
                "the zero dimension should survive the clamp: got {out_width}x{out_height}"
            );
        }
    }

    /// The analyzer's only failure mode is a zero-sized source. It must fall back,
    /// not propagate: the caller asked for a thumbnail and a centre crop is a
    /// valid answer.
    #[test]
    fn an_analyzer_failure_falls_back_to_the_centre_crop() {
        let result = smart_crop(opaque(0, 0), 32, 32, false, &DecodeLimits::default())
            .expect("an analyzer error must fall back rather than fail the conversion");

        assert_eq!(
            result.dimensions(),
            (0, 0),
            "a zero-sized source can only yield a zero-sized crop"
        );
    }

    /// The success path, for contrast: a real image returns the requested size.
    #[test]
    fn a_usable_source_returns_the_requested_dimensions() {
        let result = smart_crop(opaque(128, 96), 48, 32, false, &DecodeLimits::default())
            .expect("content-aware crop of a real image must succeed");

        assert_eq!(result.dimensions(), (48, 32));
    }

    /// And the budget still applies on this path, so the content-aware default
    /// cannot bypass the bound the other transforms respect.
    #[test]
    fn the_fallback_still_respects_the_transform_budget() {
        let tight = DecodeLimits {
            max_transform_peak_bytes: 1_024,
            ..DecodeLimits::default()
        };

        // Zero target -> centre-crop fallback with preserve_aspect_ratio, which
        // pre-scales and therefore resamples.
        let refused = smart_crop(opaque(512, 256), 0, 32, true, &tight);

        assert!(
            matches!(
                refused,
                Err(crate::ConversionError::TransformBudgetExceeded { .. })
            ),
            "the fallback resamples, so it must be bounded too: {refused:?}"
        );
    }

    /// `crop_origin` reaching Smart is a bug, and the two build profiles treat it
    /// differently on purpose.
    ///
    /// In debug the `debug_assert!` fires, which is what this asserts — a
    /// developer who breaks `crop()`'s early return finds out immediately. In
    /// release it returns the centre anchor instead of aborting, because the
    /// alternative in production is a request-driven panic that the worker
    /// classifies as *retryable*: a redelivery loop repeating the whole pipeline,
    /// rather than one clean failure or one slightly-wrong crop.
    #[test]
    #[should_panic(expected = "must be handled before crop_origin")]
    fn crop_origin_asserts_in_debug_when_reached_with_smart() {
        let _ = crop_origin(100, 100, 50, 40, CropStrategy::Smart);
    }
}
