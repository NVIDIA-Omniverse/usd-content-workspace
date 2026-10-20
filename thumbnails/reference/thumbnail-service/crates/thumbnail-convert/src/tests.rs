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

//! Conversion parity tests.
//!
//! Fixtures are generated rather than checked in: a gradient of known dimensions
//! makes the geometry assertions readable, and an asymmetric one makes crop
//! anchoring observable — a centre crop and a top-left crop of a symmetric image
//! are indistinguishable, which is exactly the bug these tests exist to catch.

use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};

use super::*;

fn spec(width: u32, height: u32, transform: Transform) -> ImageSpec {
    ImageSpec {
        width,
        height,
        format: OutputFormat::Png,
        transform,
        crop: CropStrategy::Center,
        quality: 85,
        preserve_aspect_ratio: true,
    }
}

/// An image whose every quadrant is a different flat colour, so which region a
/// crop kept is readable from a single pixel.
fn quadrant_image(width: u32, height: u32) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = match (x < width / 2, y < height / 2) {
            (true, true) => Rgba([255, 0, 0, 255]),     // top-left
            (false, true) => Rgba([0, 255, 0, 255]),    // top-right
            (true, false) => Rgba([0, 0, 255, 255]),    // bottom-left
            (false, false) => Rgba([255, 255, 0, 255]), // bottom-right
        };
    }
    image
}

fn encoded(image: &RgbaImage, format: ImageFormat) -> Vec<u8> {
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut std::io::Cursor::new(&mut bytes), format)
        .expect("fixture must encode");
    bytes
}

fn png_fixture(width: u32, height: u32) -> Vec<u8> {
    encoded(&quadrant_image(width, height), ImageFormat::Png)
}

fn decode(bytes: &[u8]) -> DynamicImage {
    image::load_from_memory(bytes).expect("converted output must decode")
}

// --- Transform geometry ---------------------------------------------------

#[test]
fn resize_produces_exactly_the_requested_dimensions() {
    let output = convert(
        png_fixture(400, 200),
        &spec(64, 128, Transform::Resize),
        &DecodeLimits::default(),
    )
    .expect("resize should succeed");

    assert_eq!(decode(&output).dimensions(), (64, 128));
}

#[test]
fn scale_by_width_derives_height_from_the_aspect_ratio() {
    let output = convert(
        png_fixture(400, 200),
        &spec(100, 999, Transform::ScaleByWidth),
        &DecodeLimits::default(),
    )
    .expect("scale-by-width should succeed");

    assert_eq!(
        decode(&output).dimensions(),
        (100, 50),
        "height follows the 2:1 source ratio, ignoring the requested height"
    );
}

#[test]
fn scale_by_height_derives_width_from_the_aspect_ratio() {
    let output = convert(
        png_fixture(400, 200),
        &spec(999, 100, Transform::ScaleByHeight),
        &DecodeLimits::default(),
    )
    .expect("scale-by-height should succeed");

    assert_eq!(decode(&output).dimensions(), (200, 100));
}

#[test]
fn square_crops_to_the_smaller_dimension_then_resizes_square() {
    let output = convert(
        png_fixture(400, 200),
        &spec(64, 128, Transform::Square),
        &DecodeLimits::default(),
    )
    .expect("square should succeed");

    assert_eq!(
        decode(&output).dimensions(),
        (64, 64),
        "the result stays square at min(width, height)"
    );
}

#[test]
fn square_is_centered_even_when_the_spec_requests_another_crop_anchor() {
    let mut image = RgbaImage::new(300, 100);
    for (x, _, pixel) in image.enumerate_pixels_mut() {
        *pixel = match x {
            0..=99 => Rgba([255, 0, 0, 255]),
            100..=199 => Rgba([0, 255, 0, 255]),
            _ => Rgba([0, 0, 255, 255]),
        };
    }
    let mut square_spec = spec(50, 50, Transform::Square);
    square_spec.crop = CropStrategy::TopLeft;

    let output = convert(
        encoded(&image, ImageFormat::Png),
        &square_spec,
        &DecodeLimits::default(),
    )
    .expect("square should succeed");

    assert_eq!(
        decode(&output).get_pixel(25, 25),
        Rgba([0, 255, 0, 255]),
        "Square always takes the centered source square"
    );
}

#[test]
fn fit_letterboxes_when_preserving_aspect_ratio() {
    let output = convert(
        png_fixture(400, 200),
        &spec(200, 200, Transform::Fit),
        &DecodeLimits::default(),
    )
    .expect("fit should succeed");

    let fitted = decode(&output);
    assert_eq!(fitted.dimensions(), (200, 200));
    // The source is 2:1, so it occupies the middle 100 rows and the rest is the
    // transparent white canvas.
    assert_eq!(
        fitted.get_pixel(100, 2),
        Rgba([255, 255, 255, 0]),
        "the letterbox band is the canvas, not image content"
    );
    assert_ne!(
        fitted.get_pixel(100, 100)[3],
        0,
        "the centre is image content"
    );
}

#[test]
fn fit_without_aspect_preservation_is_an_exact_resize() {
    let mut fit_spec = spec(200, 200, Transform::Fit);
    fit_spec.preserve_aspect_ratio = false;

    let output = convert(png_fixture(400, 200), &fit_spec, &DecodeLimits::default())
        .expect("fit should succeed");

    let fitted = decode(&output);
    assert_eq!(fitted.dimensions(), (200, 200));
    assert_ne!(
        fitted.get_pixel(100, 2)[3],
        0,
        "with no letterbox the whole canvas is image content"
    );
}

#[test]
fn scale_and_crop_fills_the_box_without_letterboxing() {
    let output = convert(
        png_fixture(400, 200),
        &spec(200, 200, Transform::ScaleAndCrop),
        &DecodeLimits::default(),
    )
    .expect("scale-and-crop should succeed");

    let filled = decode(&output);
    assert_eq!(filled.dimensions(), (200, 200));
    for (x, y) in [(0, 0), (199, 0), (0, 199), (199, 199)] {
        assert_eq!(
            filled.get_pixel(x, y)[3],
            255,
            "every corner is image content, not canvas"
        );
    }
}

/// Pillow's `thumbnail` never enlarges. A box bigger than the source returns the
/// source untouched.
#[test]
fn thumbnail_never_upscales() {
    let output = convert(
        png_fixture(100, 50),
        &spec(400, 400, Transform::Thumbnail),
        &DecodeLimits::default(),
    )
    .expect("thumbnail should succeed");

    assert_eq!(decode(&output).dimensions(), (100, 50));
}

#[test]
fn thumbnail_fits_inside_the_box_preserving_aspect_ratio() {
    let output = convert(
        png_fixture(400, 200),
        &spec(100, 100, Transform::Thumbnail),
        &DecodeLimits::default(),
    )
    .expect("thumbnail should succeed");

    assert_eq!(decode(&output).dimensions(), (100, 50));
}

// --- Crop anchoring -------------------------------------------------------

/// Every anchor keeps a different region, and the quadrant fixture makes which
/// one observable. A test that only asserted dimensions would pass for all nine.
#[test]
fn every_crop_anchor_keeps_its_own_region() {
    const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);
    const GREEN: Rgba<u8> = Rgba([0, 255, 0, 255]);
    const BLUE: Rgba<u8> = Rgba([0, 0, 255, 255]);
    const YELLOW: Rgba<u8> = Rgba([255, 255, 0, 255]);

    // A 200x200 source cropped to 100x100 lands entirely inside one quadrant for
    // the four corners, and straddles two for the edges.
    let cases = [
        (CropStrategy::TopLeft, RED),
        (CropStrategy::TopRight, GREEN),
        (CropStrategy::BottomLeft, BLUE),
        (CropStrategy::BottomRight, YELLOW),
    ];

    for (anchor, expected) in cases {
        let mut crop_spec = spec(100, 100, Transform::Crop);
        crop_spec.crop = anchor;
        crop_spec.preserve_aspect_ratio = false;

        let output = convert(png_fixture(200, 200), &crop_spec, &DecodeLimits::default())
            .expect("crop should succeed");
        let cropped = decode(&output);

        assert_eq!(cropped.dimensions(), (100, 100));
        assert_eq!(
            cropped.get_pixel(50, 50),
            expected,
            "{anchor:?} should keep its own quadrant"
        );
    }

    // The edge anchors straddle two quadrants, so check the split rather than a
    // single colour.
    let edge_cases = [
        (CropStrategy::Top, (RED, GREEN), true),
        (CropStrategy::Bottom, (BLUE, YELLOW), true),
        (CropStrategy::Left, (RED, BLUE), false),
        (CropStrategy::Right, (GREEN, YELLOW), false),
    ];

    for (anchor, (first, second), horizontal_split) in edge_cases {
        let mut crop_spec = spec(100, 100, Transform::Crop);
        crop_spec.crop = anchor;
        crop_spec.preserve_aspect_ratio = false;

        let output = convert(png_fixture(200, 200), &crop_spec, &DecodeLimits::default())
            .expect("crop should succeed");
        let cropped = decode(&output);

        let (a, b) = if horizontal_split {
            (cropped.get_pixel(10, 50), cropped.get_pixel(90, 50))
        } else {
            (cropped.get_pixel(50, 10), cropped.get_pixel(50, 90))
        };
        assert_eq!(a, first, "{anchor:?} first half");
        assert_eq!(b, second, "{anchor:?} second half");
    }

    // Centre sits on the meeting point of all four.
    let mut centre = spec(100, 100, Transform::Crop);
    centre.crop = CropStrategy::Center;
    centre.preserve_aspect_ratio = false;
    let output = convert(png_fixture(200, 200), &centre, &DecodeLimits::default()).expect("crop");
    let cropped = decode(&output);
    assert_eq!(cropped.get_pixel(10, 10), RED);
    assert_eq!(cropped.get_pixel(90, 10), GREEN);
    assert_eq!(cropped.get_pixel(10, 90), BLUE);
    assert_eq!(cropped.get_pixel(90, 90), YELLOW);
}

// --- Encoding -------------------------------------------------------------

#[test]
fn each_output_encoding_is_written_in_its_own_format() {
    for (format, expected) in [
        (OutputFormat::Png, ImageFormat::Png),
        (OutputFormat::Jpeg, ImageFormat::Jpeg),
        (OutputFormat::Gif, ImageFormat::Gif),
        (OutputFormat::Webp, ImageFormat::WebP),
    ] {
        let mut encode_spec = spec(32, 32, Transform::Resize);
        encode_spec.format = format;

        let output = convert(png_fixture(64, 64), &encode_spec, &DecodeLimits::default())
            .unwrap_or_else(|e| panic!("{format:?} encode failed: {e}"));

        assert_eq!(
            image::guess_format(&output).expect("output must be a recognizable image"),
            expected,
            "{format:?} should be written as {expected:?}"
        );
    }
}

/// Alpha becomes white for JPEG, not black — the failure mode of simply dropping
/// the channel.
#[test]
fn transparent_pixels_are_flattened_onto_white_for_jpeg() {
    let mut transparent = RgbaImage::new(64, 64);
    for pixel in transparent.pixels_mut() {
        *pixel = Rgba([0, 0, 0, 0]);
    }

    let mut jpeg_spec = spec(16, 16, Transform::Resize);
    jpeg_spec.format = OutputFormat::Jpeg;

    let output = convert(
        encoded(&transparent, ImageFormat::Png),
        &jpeg_spec,
        &DecodeLimits::default(),
    )
    .expect("jpeg encode should succeed");

    let pixel = decode(&output).get_pixel(8, 8);
    assert!(
        pixel[0] > 240 && pixel[1] > 240 && pixel[2] > 240,
        "fully transparent input should flatten to white, got {pixel:?}"
    );
}

#[test]
fn quality_is_rejected_out_of_range_for_jpeg_and_ignored_elsewhere() {
    let mut jpeg_spec = spec(16, 16, Transform::Resize);
    jpeg_spec.format = OutputFormat::Jpeg;
    jpeg_spec.quality = 0;

    let error = convert(png_fixture(32, 32), &jpeg_spec, &DecodeLimits::default())
        .expect_err("quality 0 is not a JPEG quality");
    assert_eq!(error.failure_class(), ConversionFailureClass::Specification);

    // PNG, GIF, and WebP do not read quality at all, so an out-of-range value is
    // simply irrelevant rather than an error.
    for format in [OutputFormat::Png, OutputFormat::Gif, OutputFormat::Webp] {
        let mut lossless = spec(16, 16, Transform::Resize);
        lossless.format = format;
        lossless.quality = 0;

        convert(png_fixture(32, 32), &lossless, &DecodeLimits::default())
            .unwrap_or_else(|e| panic!("{format:?} should ignore quality entirely: {e}"));
    }
}

/// JPEG quality has to actually reach the encoder: a low setting must produce
/// visibly fewer bytes than a high one.
#[test]
fn jpeg_quality_reaches_the_encoder() {
    let mut low = spec(256, 256, Transform::Resize);
    low.format = OutputFormat::Jpeg;
    low.quality = 10;
    let mut high = low;
    high.quality = 95;

    let source = png_fixture(512, 512);
    let low_bytes =
        convert(source.clone(), &low, &DecodeLimits::default()).expect("low-quality jpeg");
    let high_bytes = convert(source, &high, &DecodeLimits::default()).expect("high-quality jpeg");

    assert!(
        low_bytes.len() < high_bytes.len(),
        "quality 10 produced {} bytes and quality 95 produced {}",
        low_bytes.len(),
        high_bytes.len()
    );
}

// --- Source formats -------------------------------------------------------

#[test]
fn every_advertised_input_format_decodes() {
    for format in [
        ImageFormat::Png,
        ImageFormat::Jpeg,
        ImageFormat::Gif,
        ImageFormat::Bmp,
        ImageFormat::Tiff,
        ImageFormat::WebP,
        // On the allowlist, and so reachable — an entry no decoder covers
        // becomes generate work that can only be permanently rejected.
        ImageFormat::Ico,
    ] {
        let fixture = encoded(&quadrant_image(64, 64), format);
        convert(
            fixture,
            &spec(16, 16, Transform::Resize),
            &DecodeLimits::default(),
        )
        .unwrap_or_else(|e| panic!("{format:?} should decode: {e}"));
    }
}

/// Pillow converted the first frame of an animated source and ignored the rest.
/// `image` decodes a `DynamicImage` from the first frame too, so parity here is
/// inherited rather than implemented — which is exactly why it needs a test: the
/// behavior would change silently if the decode path ever moved to an animation
/// decoder.
#[test]
fn an_animated_source_converts_its_first_frame() {
    use image::codecs::gif::{GifDecoder, GifEncoder};
    use image::{AnimationDecoder, Delay, Frame};

    // Two frames of flat colour, so which one survived is readable from a pixel.
    let red = RgbaImage::from_pixel(64, 64, Rgba([255, 0, 0, 255]));
    let blue = RgbaImage::from_pixel(64, 64, Rgba([0, 0, 255, 255]));

    let mut animated = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut animated);
        encoder
            .encode_frames(vec![
                Frame::from_parts(red, 0, 0, Delay::from_numer_denom_ms(100, 1)),
                Frame::from_parts(blue, 0, 0, Delay::from_numer_denom_ms(100, 1)),
            ])
            .expect("two-frame fixture must encode");
    }

    // The fixture really is animated, or the assertion below proves nothing.
    let frame_count = GifDecoder::new(std::io::Cursor::new(&animated))
        .expect("fixture must decode as GIF")
        .into_frames()
        .count();
    assert_eq!(frame_count, 2, "the fixture should have two frames");

    let mut gif_spec = spec(16, 16, Transform::Resize);
    gif_spec.format = OutputFormat::Png;

    let output = convert(animated, &gif_spec, &DecodeLimits::default())
        .expect("an animated source must convert, not fail on the extra frames");

    let pixel = decode(&output).get_pixel(8, 8);
    assert!(
        pixel[0] > 200 && pixel[2] < 60,
        "the first (red) frame should be what was converted, got {pixel:?}"
    );
}

#[test]
fn an_unrecognizable_source_is_a_permanent_format_failure() {
    let error = convert(
        b"this is not an image".to_vec(),
        &spec(16, 16, Transform::Resize),
        &DecodeLimits::default(),
    )
    .expect_err("arbitrary bytes are not an image");

    assert_eq!(
        error.failure_class(),
        ConversionFailureClass::UnsupportedFormat
    );
}

#[test]
fn an_empty_source_is_rejected_before_any_decode() {
    let error = convert(
        vec![],
        &spec(16, 16, Transform::Resize),
        &DecodeLimits::default(),
    )
    .expect_err("empty bytes cannot be an image");

    assert!(matches!(error, ConversionError::EmptySource));
}

/// SVG and HEIC have no decoder in this build, which is why they are off the
/// eligibility allowlist. If one arrives anyway it must be a clean rejection
/// rather than a panic.
#[test]
fn a_source_with_no_decoder_is_rejected_cleanly() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>"#;

    let error = convert(
        svg.to_vec(),
        &spec(16, 16, Transform::Resize),
        &DecodeLimits::default(),
    )
    .expect_err("SVG has no decoder in this build");

    assert_eq!(
        error.failure_class(),
        ConversionFailureClass::UnsupportedFormat
    );
}

// --- Decode bounds --------------------------------------------------------

#[test]
fn an_oversized_source_is_rejected_before_decoding() {
    let limits = DecodeLimits {
        max_source_bytes: 16,
        max_decoded_pixels: DecodeLimits::DEFAULT_MAX_DECODED_PIXELS,
        max_transform_peak_bytes: DecodeLimits::DEFAULT_MAX_TRANSFORM_PEAK_BYTES,
    };

    let error = convert(
        png_fixture(64, 64),
        &spec(16, 16, Transform::Resize),
        &limits,
    )
    .expect_err("a source over the byte bound must be refused");

    assert!(matches!(error, ConversionError::SourceTooLarge { .. }));
    assert_eq!(error.failure_class(), ConversionFailureClass::Bounds);
}

/// The bomb case: the process must stay up and the failure must be permanent, so
/// the delivery is acknowledged rather than crash-looping on redelivery.
#[test]
fn a_source_over_the_pixel_bound_is_rejected_not_fatal() {
    let limits = DecodeLimits {
        max_source_bytes: DecodeLimits::DEFAULT_MAX_SOURCE_BYTES,
        max_decoded_pixels: 1_000,
        max_transform_peak_bytes: DecodeLimits::DEFAULT_MAX_TRANSFORM_PEAK_BYTES,
    };

    let error = convert(
        png_fixture(200, 200),
        &spec(16, 16, Transform::Resize),
        &limits,
    )
    .expect_err("40,000 pixels is over a 1,000-pixel bound");

    match error {
        ConversionError::PixelBudgetExceeded { pixels, limit, .. } => {
            assert_eq!(pixels, 40_000);
            assert_eq!(limit, 1_000);
        }
        other => panic!("expected a pixel-budget rejection, got {other}"),
    }
}

#[test]
fn a_source_inside_both_bounds_converts() {
    let limits = DecodeLimits {
        max_source_bytes: 10 * 1024 * 1024,
        max_decoded_pixels: 100_000,
        max_transform_peak_bytes: DecodeLimits::DEFAULT_MAX_TRANSFORM_PEAK_BYTES,
    };

    convert(
        png_fixture(200, 200),
        &spec(16, 16, Transform::Resize),
        &limits,
    )
    .expect("40,000 pixels is inside a 100,000-pixel bound");
}

// --- Ownership contract ---------------------------------------------------

/// `convert()` takes ownership of the source bytes so it can release them
/// after decode and before transform/encode. This test documents that contract
/// explicitly: the caller moves the `Vec<u8>` in and cannot use it after the
/// call (the compiler enforces this; the test proves the function compiles and
/// works when called that way).
#[test]
fn convert_accepts_and_consumes_an_owned_vec() {
    let source: Vec<u8> = png_fixture(64, 64);
    // Move `source` in directly — no borrow, no clone.
    let output = convert(
        source,
        &spec(32, 32, Transform::Resize),
        &DecodeLimits::default(),
    )
    .expect("conversion of an owned Vec<u8> must succeed");
    assert_eq!(decode(&output).dimensions(), (32, 32));
}

// --- Specification validation ---------------------------------------------

#[test]
fn zero_dimensions_are_rejected() {
    for (width, height) in [(0, 16), (16, 0)] {
        let error = convert(
            png_fixture(32, 32),
            &spec(width, height, Transform::Resize),
            &DecodeLimits::default(),
        )
        .expect_err("a zero dimension cannot produce an image");

        assert_eq!(error.failure_class(), ConversionFailureClass::Specification);
    }
}

// --- Transform bounds -----------------------------------------------------
//
// The decode bounds above cap the source. These cap what converting it costs,
// which is a different and larger number: `imageops::resize` allocates a
// working buffer of source_width × new_height at 16 B/px, so a source that
// passes every decode check can still peak in gigabytes.

fn limits_with_peak(max_transform_peak_bytes: u64) -> DecodeLimits {
    DecodeLimits {
        max_transform_peak_bytes,
        ..DecodeLimits::default()
    }
}

/// The case an output-pixel bound cannot catch, and the reason this bound is
/// denominated in bytes rather than output pixels.
///
/// A 4000×10 source is 40,000 pixels — nothing. Asked for an 8×8 thumbnail it
/// produces 64 pixels — nothing. The working buffer in between is
/// 4000 × 8 × 16 B, two orders of magnitude larger than either, and it is what
/// the process has to allocate.
#[test]
fn a_tiny_result_does_not_make_a_conversion_small() {
    let source_width = 4_000;
    let target = 8;
    let working_bytes = u64::from(source_width) * u64::from(target) * 16;

    let error = convert(
        png_fixture(source_width, 10),
        &spec(target, target, Transform::Resize),
        &limits_with_peak(working_bytes / 2),
    )
    .expect_err("the working buffer is over the budget even though the result is 64 pixels");

    match error {
        ConversionError::TransformBudgetExceeded { peak, limit, .. } => {
            assert!(
                peak > limit,
                "the rejection must report the peak it refused ({peak}) against the budget ({limit})"
            );
            assert!(
                peak > u64::from(target) * u64::from(target) * 4,
                "a bound on output pixels alone would have let this through"
            );
        }
        other => panic!("expected a transform-budget rejection, got {other}"),
    }
    assert_eq!(
        ConversionError::TransformBudgetExceeded {
            source_width: 0,
            source_height: 0,
            new_width: 0,
            new_height: 0,
            peak: 0,
            limit: 0,
        }
        .failure_class(),
        ConversionFailureClass::Bounds,
        "a budget rejection is a bounds failure, so the worker ACKs rather than retrying"
    );
}

/// Every transform that resamples is covered, including the four whose output
/// dimensions are fixed by the request. Those are bounded in what they *produce*
/// and unbounded in what they allocate to produce it, which is why "safe by
/// construction" was the wrong reading.
#[test]
fn every_resampling_transform_is_bounded() {
    let resampling = [
        Transform::Resize,
        Transform::ScaleByWidth,
        Transform::ScaleByHeight,
        Transform::Square,
        Transform::Fit,
        Transform::ScaleAndCrop,
        Transform::Thumbnail,
    ];

    for transform in resampling {
        let result = convert(
            png_fixture(512, 256),
            &spec(32, 32, transform),
            &limits_with_peak(1_024),
        );

        match result {
            Err(ConversionError::TransformBudgetExceeded { .. }) => {}
            Err(other) => panic!("{transform:?} was refused for the wrong reason: {other}"),
            Ok(_) => panic!("{transform:?} resamples and must respect the budget"),
        }
    }
}

/// A derived dimension past `u32` must be a rejection carrying its real value,
/// not a silent saturation to 4,294,967,295 — which is still unallocatable and
/// would defeat any check placed after the cast.
#[test]
fn a_saturating_dimension_is_rejected_not_clamped() {
    // A 1-pixel-wide source scaled to 8192 wide multiplies its height by 8192.
    let error = convert(
        png_fixture(1, 600_000),
        &spec(8_192, 8_192, Transform::ScaleByWidth),
        &DecodeLimits::default(),
    )
    .expect_err("4,915,200,000 rows cannot be allocated");

    match error {
        ConversionError::TransformBudgetExceeded { new_height, .. } => {
            assert_eq!(
                new_height, 4_915_200_000,
                "the real derived height must survive to the rejection"
            );
            assert!(
                new_height > u64::from(u32::MAX),
                "this is the case that would have saturated"
            );
        }
        other => panic!("expected a transform-budget rejection, got {other}"),
    }
}

#[test]
fn an_ordinary_thumbnail_is_well_inside_the_default_budget() {
    for transform in [Transform::Resize, Transform::ScaleByWidth, Transform::Crop] {
        convert(
            png_fixture(4_000, 3_000),
            &spec(512, 512, transform),
            &DecodeLimits::default(),
        )
        .unwrap_or_else(|error| {
            panic!("a 4000x3000 source to a 512 thumbnail must convert, {transform:?} gave {error}")
        });
    }
}

// --- Smart crop -----------------------------------------------------------

#[test]
fn smart_crop_produces_the_requested_dimensions() {
    let mut smart = spec(64, 48, Transform::Crop);
    smart.crop = CropStrategy::Smart;

    let output = convert(png_fixture(256, 256), &smart, &DecodeLimits::default())
        .expect("content-aware crop is the shipped default and must convert");

    assert_eq!(decode(&output).dimensions(), (64, 48));
}

#[test]
fn smart_crop_is_bounded_like_every_other_resample() {
    let mut smart = spec(64, 48, Transform::Crop);
    smart.crop = CropStrategy::Smart;

    convert(png_fixture(512, 256), &smart, &limits_with_peak(1_024))
        .expect_err("the content-aware path must respect the budget too");
}

/// A source whose header is valid but whose pixel data is cut short must be a
/// clean `Err`, not a panic.
///
/// This is the gap the arbitrary-bytes test above does not cover: those bytes are
/// rejected at format detection, before any decoder runs. A truncated PNG gets
/// past detection and past the header-derived bounds check, then fails inside the
/// decoder — which is the path where a panic would surface as a retryable
/// conversion failure and redeliver the same broken bytes indefinitely.
#[test]
fn a_truncated_source_is_a_clean_decode_failure() {
    let complete = png_fixture(64, 64);

    // Several truncation points, because where the cut lands decides which part of
    // the decoder meets it: just past the header, mid-IDAT, and one byte short.
    for keep in [40, complete.len() / 2, complete.len() - 1] {
        let truncated = complete[..keep].to_vec();

        let result = convert(
            truncated,
            &spec(16, 16, Transform::Resize),
            &DecodeLimits::default(),
        );

        // An `Ok` is not impossible: a PNG cut on an IDAT boundary can still decode
        // to a partial image. That is the decoder's call, and it is not a panic,
        // which is what this test is about.
        if let Err(error) = result {
            assert!(
                matches!(
                    error.failure_class(),
                    ConversionFailureClass::Decode | ConversionFailureClass::UnsupportedFormat
                ),
                "truncating to {keep} bytes should be a decode or format failure, got {error:?}"
            );
        }
    }
}

/// A header that claims dimensions the payload cannot back must not be trusted
/// into an allocation.
#[test]
fn a_header_only_source_does_not_allocate_for_its_claimed_size() {
    // The first 33 bytes of a PNG are signature + IHDR, which is enough to state
    // 64x64 and nothing else.
    let header_only = png_fixture(64, 64)[..33].to_vec();

    let result = convert(
        header_only,
        &spec(16, 16, Transform::Resize),
        &DecodeLimits::default(),
    );

    assert!(
        result.is_err(),
        "a header with no pixel data cannot produce a thumbnail"
    );
}

/// The FIT canvas is allocated after the resample, so it needs its own check.
///
/// The resample check runs before the resize and knows nothing about what the
/// transform does afterwards. FIT then allocates a letterbox canvas of the full
/// requested size, resident alongside the resized image — 256 MiB each at the
/// 8192 ceiling, and previously uncounted.
#[test]
fn the_fit_canvas_is_counted_against_the_budget() {
    fn spec_for_fit(side: u32) -> ImageSpec {
        let mut fit = spec(side, side, Transform::Fit);
        fit.preserve_aspect_ratio = true;
        fit
    }

    // A small source and a large box, which is the shape where the canvas can
    // exceed the resample: the working buffer is only source_width × new_height,
    // while the resized image and the canvas are each the full requested area.
    //
    //   resample peak  8×8×4 + 8×2048×16 + 2048×2048×4   = 16.3 MiB
    //   canvas peak    2048×2048×4 + 2048×2048×4         = 32.0 MiB
    //
    // A 24 MiB budget therefore passes the resample and fails the canvas, which is
    // exactly the window the old check left open.
    let error = convert(
        png_fixture(8, 8),
        &spec_for_fit(2048),
        &limits_with_peak(24 * 1024 * 1024),
    )
    .expect_err("the canvas must be counted");

    assert!(
        matches!(error, ConversionError::AllocationBudgetExceeded { what, .. } if what.contains("canvas")),
        "the rejection must name the canvas, so an operator knows which allocation it was: {error:?}"
    );
    assert_eq!(error.failure_class(), ConversionFailureClass::Bounds);
}

/// The content-aware crop holds a full RGB copy of the source across its resize,
/// so that copy is resident alongside every buffer the resample allocates.
#[test]
fn the_smart_crop_rgb_copy_is_counted_against_the_budget() {
    let mut smart = spec(64, 64, Transform::Crop);
    smart.crop = CropStrategy::Smart;

    let source_bytes = 256_u64 * 256 * 4;
    let error = convert(
        png_fixture(256, 256),
        &smart,
        // Enough for the source alone, not for the source plus its RGB copy.
        &limits_with_peak(source_bytes + source_bytes / 4),
    )
    .expect_err("the RGB copy must be counted");

    assert!(
        matches!(error, ConversionError::AllocationBudgetExceeded { what, .. } if what.contains("RGB copy")),
        "the rejection must name the copy: {error:?}"
    );
}

/// The flatten transient is deliberately *not* added to the transform peak.
///
/// Flatten completes and drops its original before `transform::apply` runs, so the
/// two peaks are consecutive rather than simultaneous — a replica must hold the
/// larger, not the sum. Adding them would refuse conversions that fit.
#[test]
fn a_flatten_then_transform_is_not_charged_for_both_at_once() {
    // JPEG output forces the flatten. A budget that comfortably covers the
    // transform must not be failed by having also flattened beforehand.
    let mut jpeg = spec(64, 64, Transform::Resize);
    jpeg.format = OutputFormat::Jpeg;

    convert(
        png_fixture(256, 256),
        &jpeg,
        &limits_with_peak(64 * 1024 * 1024),
    )
    .expect("sequential phases must not be summed into one budget");
}
