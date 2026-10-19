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

//! Which source formats the thumbnail pipeline accepts.
//!
//! This is the eligibility gate on the storage-CREATE path, and it is kept in
//! step with what `crates/thumbnail-convert` can actually decode. Publishing
//! generate work for a format no decoder handles costs a download and a
//! permanent rejection to reach the same outcome as skipping it here.

use std::sync::LazyLock;

/// Source extensions eligible for thumbnail generation (O(1) lookup).
///
/// Exactly the set the `image` crate decodes on its default features. `svg`,
/// `heic`, `heif`, and `avif` were listed here while conversion lived in the
/// Python worker, but Pillow could not decode them either — they produced failed
/// tasks, not thumbnails. Each needs a decoder `image` does not ship (`resvg`,
/// `libheif`, `dav1d`). Re-add an extension here in the same change that adds
/// its decoder.
pub static ALLOWED_EXTENSIONS: LazyLock<std::collections::HashSet<&'static str>> =
    LazyLock::new(|| {
        [
            "jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp", "ico",
        ]
        .into_iter()
        .collect()
    });

/// Returns the file extension from the path portion of `address` (query string stripped).
///
/// Returns `None` if the path contains no `.` or if the segment after the last `.` contains
/// a `/` (meaning the dot was in a directory component, not the filename).
pub fn extract_extension(address: &str) -> Option<&str> {
    // Both delimiters, not just the query. Splitting on `?` alone left a fragment
    // attached to the extension, so `…/asset.png#frag` yielded `png#frag`, missed
    // ALLOWED_EXTENSIONS, and the CREATE event was skipped and acknowledged — no
    // thumbnail and no error, indistinguishable from a genuinely ineligible
    // source. `thumbnail_path::matches_rendered_output` already strips both via a
    // real URL parse; this is the consistency it was missing.
    let path = address.split(['?', '#']).next().unwrap_or(address);
    path.rsplit('.').next().filter(|ext| !ext.contains('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_comes_from_the_path_not_the_query() {
        assert_eq!(
            extract_extension("https://storage.example/a/image.png?etag=abc"),
            Some("png")
        );
    }

    /// A fragment must not ride along on the extension. It used to, which made
    /// the source silently ineligible: the CREATE event was skipped and ACKed, so
    /// there was no thumbnail and no error to notice.
    #[test]
    fn extension_comes_from_the_path_not_the_fragment() {
        assert_eq!(
            extract_extension("https://storage.example/a/image.png#preview"),
            Some("png")
        );
        assert_eq!(
            extract_extension("https://storage.example/a/image.png?etag=abc#preview"),
            Some("png")
        );
        // Fragment before query, which is malformed as a URL but must still not
        // produce a bogus extension.
        assert_eq!(
            extract_extension("https://storage.example/a/image.png#frag?etag=abc"),
            Some("png")
        );
    }

    #[test]
    fn a_dot_in_a_directory_is_not_an_extension() {
        assert_eq!(
            extract_extension("https://storage.example/a.b/README"),
            None
        );
    }

    #[test]
    fn a_file_with_no_dot_has_no_extension() {
        assert_eq!(extract_extension("https://storage.example/a/README"), None);
    }

    /// The allowlist is the decoder set. Compared against
    /// `thumbnail-convert`'s own published list rather than a copy of it: a
    /// third hand-maintained list would be one more thing to drift.
    ///
    /// `thumbnail-convert` is a dev-dependency only. It must not become a real
    /// one — `thumbnail-common` is linked by `server`, and depending on it in
    /// either direction would put image codecs on the API deployment, which is
    /// the whole reason that crate is a leaf.
    #[test]
    fn the_allowlist_is_exactly_the_decodable_set() {
        let mut allowed: Vec<&str> = ALLOWED_EXTENSIONS.iter().copied().collect();
        allowed.sort_unstable();

        let mut decodable = thumbnail_convert::SUPPORTED_INPUT_EXTENSIONS.to_vec();
        decodable.sort_unstable();

        assert_eq!(
            allowed, decodable,
            "an eligible extension no decoder handles becomes work that can only \
             be permanently rejected; a decodable one left out silently produces \
             no thumbnail"
        );
    }
}
