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

//! Unit tests for the `THUMBNAIL_SIZES` parser.
//!
//! These mirror the scenarios in
//! `openspec/changes/configure-thumbnail-sizes/specs/thumbnail-sizes/spec.md`
//! under the "comma-separated list" requirement.

use crate::thumbnail_sizes::{
    ThumbnailSizes, ThumbnailSizesError, DIM_MAX, DIM_MIN, ENV_VAR, MAX_ENTRIES,
};
use serial_test::serial;
use std::env;

// ---------------------------------------------------------------------------
// Successful parses — scenarios listed in spec.md
// ---------------------------------------------------------------------------

#[test]
fn parses_single_square_size() {
    let parsed: ThumbnailSizes = "256x256".parse().expect("single square size must parse");
    assert_eq!(parsed.as_slice(), &[(256, 256)]);
}

#[test]
fn parses_multiple_square_sizes_preserving_order() {
    let parsed: ThumbnailSizes = "64x64,128x128,256x256"
        .parse()
        .expect("multi-square size list must parse");
    assert_eq!(parsed.as_slice(), &[(64, 64), (128, 128), (256, 256)]);
}

#[test]
fn tolerates_whitespace_between_and_within_entries() {
    let parsed: ThumbnailSizes = "  64x64 , 128 x 128 ,256x256  "
        .parse()
        .expect("whitespace-laden list must parse");
    assert_eq!(parsed.as_slice(), &[(64, 64), (128, 128), (256, 256)]);
}

#[test]
fn accepts_non_square_pair() {
    let parsed: ThumbnailSizes = "128x64".parse().expect("non-square pair must parse");
    assert_eq!(parsed.as_slice(), &[(128, 64)]);
}

#[test]
fn accepts_leading_zeros() {
    let parsed: ThumbnailSizes = "064x064".parse().expect("leading zeros must parse");
    assert_eq!(parsed.as_slice(), &[(64, 64)]);
}

#[test]
fn accepts_max_entries_exactly() {
    // 10 distinct entries — at the cap, must succeed.
    let raw = (1..=MAX_ENTRIES as u32)
        .map(|i| format!("{}x{}", i * 8, i * 8))
        .collect::<Vec<_>>()
        .join(",");
    let parsed: ThumbnailSizes = raw.parse().expect("exactly MAX_ENTRIES entries must parse");
    assert_eq!(parsed.len(), MAX_ENTRIES);
}

#[test]
fn accepts_boundary_dimensions() {
    let parsed: ThumbnailSizes = format!("{}x{}", DIM_MIN, DIM_MAX).parse().unwrap();
    assert_eq!(parsed.as_slice(), &[(DIM_MIN, DIM_MAX)]);

    let parsed: ThumbnailSizes = format!("{}x{}", DIM_MAX, DIM_MIN).parse().unwrap();
    assert_eq!(parsed.as_slice(), &[(DIM_MAX, DIM_MIN)]);
}

// ---------------------------------------------------------------------------
// Rejected inputs — scenarios listed in spec.md
// ---------------------------------------------------------------------------

#[test]
fn rejects_duplicate_pairs() {
    let err = "256x256,256x256".parse::<ThumbnailSizes>().unwrap_err();
    match err {
        ThumbnailSizesError::Duplicate { ref entry } if entry == "256x256" => {}
        other => panic!("expected Duplicate{{entry:\"256x256\"}}, got {:?}", other),
    }
    let msg = err.to_string();
    assert!(msg.contains("THUMBNAIL_SIZES"));
    assert!(msg.contains("256x256"));
}

#[test]
fn rejects_zero_dimension() {
    let err = "0x0".parse::<ThumbnailSizes>().unwrap_err();
    assert!(matches!(err, ThumbnailSizesError::OutOfRange { .. }));
    let msg = err.to_string();
    assert!(msg.contains("THUMBNAIL_SIZES"));
    assert!(msg.contains("0x0"));
    assert!(msg.contains(&DIM_MIN.to_string()));
    assert!(msg.contains(&DIM_MAX.to_string()));
}

#[test]
fn rejects_dimension_above_dim_max() {
    let too_big = DIM_MAX + 1;
    let raw = format!("{}x{}", too_big, too_big);
    let err = raw.parse::<ThumbnailSizes>().unwrap_err();
    assert!(matches!(err, ThumbnailSizesError::OutOfRange { .. }));
    let msg = err.to_string();
    assert!(msg.contains(&raw));
}

#[test]
fn rejects_malformed_substrings() {
    // Cases each from the spec's "malformed entries are rejected" scenario.
    // We only assert that parse fails with an `Invalid` variant; the exact
    // wording of the offending substring inside `reason` is not part of the
    // contract (and varies based on which token the parser tripped on).
    for bad in &["abc", "64-64", "64x", "x64", "64x64x64", "64,x,64"] {
        let err = bad.parse::<ThumbnailSizes>().unwrap_err();
        assert!(
            matches!(err, ThumbnailSizesError::Invalid { .. }),
            "expected Invalid for input `{}`, got {:?}",
            bad,
            err
        );
    }
}

#[test]
fn rejects_malformed_inside_otherwise_valid_list() {
    // Spec scenario: malformed entry sandwiched between valid ones.
    let err = "64x64,abc,128x128".parse::<ThumbnailSizes>().unwrap_err();
    let msg = err.to_string();
    assert!(matches!(err, ThumbnailSizesError::Invalid { .. }));
    assert!(msg.contains("abc") || msg.contains("`abc`"));
}

#[test]
fn rejects_trailing_comma() {
    let err = "256x256,".parse::<ThumbnailSizes>().unwrap_err();
    assert!(matches!(err, ThumbnailSizesError::Invalid { .. }));
}

#[test]
fn rejects_more_than_max_entries() {
    let raw = (1..=(MAX_ENTRIES as u32 + 1))
        .map(|i| format!("{}x{}", i * 8, i * 8))
        .collect::<Vec<_>>()
        .join(",");
    let err = raw.parse::<ThumbnailSizes>().unwrap_err();
    match err {
        ThumbnailSizesError::TooManyEntries { count, max } => {
            assert_eq!(max, MAX_ENTRIES);
            assert!(count > MAX_ENTRIES);
        }
        other => panic!("expected TooManyEntries, got {:?}", other),
    }
    let msg = ThumbnailSizesError::TooManyEntries {
        count: MAX_ENTRIES + 1,
        max: MAX_ENTRIES,
    }
    .to_string();
    assert!(msg.contains(&MAX_ENTRIES.to_string()));
}

#[test]
fn rejects_empty_string_with_dedicated_variant() {
    assert_eq!(
        "".parse::<ThumbnailSizes>().unwrap_err(),
        ThumbnailSizesError::Empty
    );
    assert_eq!(
        "   ".parse::<ThumbnailSizes>().unwrap_err(),
        ThumbnailSizesError::Empty
    );
    let msg = ThumbnailSizesError::Empty.to_string();
    assert!(msg.contains("THUMBNAIL_SIZES"));
    assert!(msg.contains("64x64,128x128,256x256"));
}

// ---------------------------------------------------------------------------
// `RuntimeConfigArgs`-level behavior: `THUMBNAIL_SIZES` is now parsed
// directly by clap (via `parse_thumbnail_sizes`) as part of `RuntimeConfigArgs`,
// with no separate `ThumbnailSizes::from_env()`/global cache. The parsing
// contract itself (valid/invalid inputs) is already covered by the `FromStr`
// tests above; these confirm the value survives the full clap parse path and
// is reachable through the installed-runtime-config accessors.
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn thumbnail_sizes_parses_through_runtime_config_args() {
    let saved = crate::config::raw_env_value(ENV_VAR);
    env::set_var(ENV_VAR, "64x64,256x256");

    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    assert_eq!(
        runtime.thumbnail_defaults.thumbnail_sizes.as_slice(),
        &[(64, 64), (256, 256)]
    );

    match saved {
        Some(v) => env::set_var(ENV_VAR, v),
        None => env::remove_var(ENV_VAR),
    }
}
