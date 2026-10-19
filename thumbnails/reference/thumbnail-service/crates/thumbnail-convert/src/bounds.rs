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

//! Explicit bounds on conversion work — decode and resample both.
//!
//! Neither Pillow nor `image` caps this by default, and the cost of not capping
//! it is specific: a decompression bomb OOM-kills the replica, which leaves the
//! delivery unacknowledged, which redelivers the same bytes to the replacement
//! pod. Rejecting the input keeps the process up and the failure permanent.
//!
//! Bounding the decode is not enough on its own, because the resample allocates
//! more than the image it produces. See [`DecodeLimits::max_transform_peak_bytes`].
//!
//! The bounds live here rather than in the worker so a second caller inherits
//! them by construction instead of reimplementing them.

use crate::ConversionError;

/// Bytes per pixel of the working buffer `image::imageops::resize` allocates in
/// its vertical pass: `Rgba32F`, four `f32` channels.
///
/// This is not the source layout and not the output layout. It is wider than
/// both, which is why the peak cannot be expressed as a pixel count.
const RESAMPLE_WORKING_BYTES_PER_PIXEL: u64 = 16;

/// How much work a single conversion may cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    /// Largest source payload accepted, in bytes.
    pub max_source_bytes: u64,
    /// Largest decoded image accepted, in pixels (width × height).
    pub max_decoded_pixels: u64,
    /// Largest peak resident allocation a single resample may reach, in bytes.
    ///
    /// Bounding the source and the requested output does not bound the
    /// conversion. `image::imageops::resize` is two passes:
    ///
    /// ```text
    /// let tmp: Rgba32FImage = vertical_sample(image, nheight);  // source_width × new_height @ 16 B/px
    /// horizontal_sample(&tmp, nwidth)                           // new_width × new_height @ source B/px
    /// ```
    ///
    /// `tmp`'s width comes from the *source* and never shrinks with the
    /// request, so it exceeds the result by the horizontal downscale factor —
    /// which, when thumbnailing, is always greater than one. The source is
    /// borrowed throughout, so all three buffers are resident at the peak.
    ///
    /// The consequence that makes an output bound insufficient: a 1,000,000×64
    /// source is exactly 64 MP and passes [`Self::max_decoded_pixels`], yet a
    /// 100×100 thumbnail of it allocates 1,000,000 × 100 × 16 B = 1.6 GB. No
    /// transform is safe merely because its output dimensions are fixed.
    ///
    /// # What this counts
    ///
    /// Every allocation that is resident at the same time, at each point a
    /// transform allocates:
    ///
    /// - the resample's source, working buffer and result
    ///   ([`Self::check_resample`]);
    /// - buffers a transform holds *across* a resample and passes as
    ///   `extra_resident_bytes` — the content-aware crop's RGB copy is one;
    /// - a transform's own allocation outside a resample, checked at its site
    ///   ([`Self::check_allocation`]) — the FIT letterbox canvas is one.
    ///
    /// It deliberately does not add the alpha-flatten transient. Flatten
    /// completes and drops its original before `transform::apply` runs, so the
    /// flatten peak and the transform peak are consecutive rather than
    /// simultaneous: what a replica must hold is the larger of the two, not their
    /// sum. Flatten is bounded separately, by
    /// [`Self::max_decoded_pixels`] and its own documented invariant.
    pub max_transform_peak_bytes: u64,
}

impl DecodeLimits {
    /// Bounds sized for the thumbnail workload rather than for arbitrary images.
    ///
    /// 64 MiB (64 × 1024 × 1024 bytes) is the source-payload cap — binary,
    /// like every other byte quantity in this crate.
    /// 64 megapixels is 64 × 1_000_000 pixels, about 8000×8000. That is a
    /// decimal million, not 64 mebipixels (8192×8192). The two 64s are
    /// different units on purpose.
    /// `max_alloc` below is a **byte** budget of pixels × 4, so a 64 MP
    /// 16-bit RGBA decode (8 B/px) is refused, while 64 MP 16-bit gray+alpha
    /// (4 B/px) is not. PNG carries both of those layouts. Flattening the
    /// `La16` one for an output encoding without alpha (JPEG; chart-default
    /// PNG skips flatten) allocates a 4 B/px RGBA8 buffer while it is still
    /// alive, which is why worker memory limits use 8 B/px at this bound.
    pub const DEFAULT_MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
    pub const DEFAULT_MAX_DECODED_PIXELS: u64 = 64 * 1_000_000;

    /// 1 GiB, sized against the workload rather than the contract ceiling.
    ///
    /// The configured `THUMBNAIL_SIZES` are 256×256 and 512×512, so the
    /// realistic worst case is a 64 MP source reduced to 512 with an output
    /// encoding that forces the flatten overlap:
    ///
    /// ```text
    /// source   64e6 px × 8 B/px         =  488 MiB   (8 B/px is the La16-flatten overlap)
    /// working  8000 × 512 × 16 B        =   63 MiB
    /// result   512 × 512 × 8 B          =    2 MiB
    ///                                      --------
    ///                                       553 MiB
    /// ```
    ///
    /// (64e6 is decimal megapixels, so 64e6 × 8 B is 512 *MB* — 488 MiB, not
    /// 512 MiB. Mixing the two units is what an earlier revision of this
    /// comment did, overstating the total as 576 MiB.)
    ///
    /// 1 GiB leaves that 1.85× headroom while still refusing the shapes that
    /// motivated the bound — a 1,000,000×64 source reduced to 100×100 peaks at
    /// 1770 MiB, and the 8192×8192 ceiling the API permits peaks at 2000 MiB.
    ///
    /// Neither of those reliably worked before this bound existed: at the
    /// previous 2560Mi limit they fit only on an otherwise-idle replica and
    /// OOM-killed it under any concurrency. Refusing them deterministically is
    /// the change, not the loss.
    ///
    /// The deployed memory limit is this figure times the conversion
    /// concurrency plus process overhead, and `worker.validate` in the chart
    /// refuses to render a combination the limit cannot cover.
    pub const DEFAULT_MAX_TRANSFORM_PEAK_BYTES: u64 = 1024 * 1024 * 1024;

    pub(crate) fn check_source_bytes(&self, actual: u64) -> Result<(), ConversionError> {
        if actual > self.max_source_bytes {
            return Err(ConversionError::SourceTooLarge {
                actual,
                limit: self.max_source_bytes,
            });
        }
        Ok(())
    }

    pub(crate) fn check_decoded_pixels(
        &self,
        width: u32,
        height: u32,
    ) -> Result<(), ConversionError> {
        let pixels = u64::from(width) * u64::from(height);
        if pixels > self.max_decoded_pixels {
            return Err(ConversionError::PixelBudgetExceeded {
                width,
                height,
                pixels,
                limit: self.max_decoded_pixels,
            });
        }
        Ok(())
    }

    /// Check a resample against the peak budget, narrowing the target to `u32`
    /// only once it is known to fit.
    ///
    /// Targets arrive as `u64` on purpose. A dimension derived from the source
    /// aspect ratio can exceed `u32`, and `as u32` saturates at 4,294,967,295 —
    /// which would convert an absurd request into a merely catastrophic one and
    /// hide the overflow from a check placed after the cast. Here the overflow
    /// is the rejection.
    pub(crate) fn check_resample(
        &self,
        source_width: u32,
        source_height: u32,
        source_bytes_per_pixel: u64,
        new_width: u64,
        new_height: u64,
        extra_resident_bytes: u64,
    ) -> Result<(u32, u32), ConversionError> {
        let source_bytes = u64::from(source_width)
            .saturating_mul(u64::from(source_height))
            .saturating_mul(source_bytes_per_pixel);
        // The vertical pass keeps the source's width and takes the target's
        // height, at 16 B/px regardless of either image's layout.
        let working_bytes = u64::from(source_width)
            .saturating_mul(new_height)
            .saturating_mul(RESAMPLE_WORKING_BYTES_PER_PIXEL);
        // The horizontal pass writes the result in the source's layout.
        let result_bytes = new_width
            .saturating_mul(new_height)
            .saturating_mul(source_bytes_per_pixel);

        let peak = source_bytes
            .saturating_add(working_bytes)
            .saturating_add(result_bytes)
            // Buffers the caller is holding across this resample. The
            // content-aware crop keeps a full RGB copy of the source alive while
            // it resizes the region it chose, which is resident alongside all
            // three terms above and was previously uncounted.
            .saturating_add(extra_resident_bytes);

        if peak > self.max_transform_peak_bytes
            || new_width > u64::from(u32::MAX)
            || new_height > u64::from(u32::MAX)
        {
            return Err(ConversionError::TransformBudgetExceeded {
                source_width,
                source_height,
                new_width,
                new_height,
                peak,
                limit: self.max_transform_peak_bytes,
            });
        }

        Ok((new_width as u32, new_height as u32))
    }

    /// Check one allocation a transform is about to make outside a resample.
    ///
    /// The resample check cannot cover these: it is called before the resample and
    /// knows nothing about what the transform does afterwards. FIT allocates its
    /// letterbox canvas after resizing, and both are resident together, so the
    /// canvas has to be checked where it happens.
    pub(crate) fn check_allocation(
        &self,
        resident_bytes: u64,
        allocating_bytes: u64,
        what: &'static str,
    ) -> Result<(), ConversionError> {
        let peak = resident_bytes.saturating_add(allocating_bytes);
        if peak > self.max_transform_peak_bytes {
            return Err(ConversionError::AllocationBudgetExceeded {
                what,
                peak,
                limit: self.max_transform_peak_bytes,
            });
        }
        Ok(())
    }

    /// The same bound expressed for the decoder itself, as a second line of
    /// defense against a header that understates the real dimensions.
    pub(crate) fn to_image_limits(self) -> image::Limits {
        let mut limits = image::Limits::default();
        // Byte budget, not a pixel budget: pixels × 4. Widest decode layout
        // this crate can produce is wider (Rgba32F is 16 B/px via TIFF); this
        // cap still holds because it is denominated in bytes. It is also why
        // 64 MP 16-bit RGBA is refused while 64 MP La16 (4 B/px) is not.
        limits.max_alloc = Some(self.max_decoded_pixels.saturating_mul(4));
        limits
    }
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: Self::DEFAULT_MAX_SOURCE_BYTES,
            max_decoded_pixels: Self::DEFAULT_MAX_DECODED_PIXELS,
            max_transform_peak_bytes: Self::DEFAULT_MAX_TRANSFORM_PEAK_BYTES,
        }
    }
}
