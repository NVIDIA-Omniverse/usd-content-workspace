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

//! Which storage CREATE events become thumbnail work.
//!
//! Ported from the retired task-queue tests, with one behavior change: `svg`,
//! `heic`, and `avif` are no longer eligible. No decoder in the workspace handles
//! them, and they produced failed tasks rather than thumbnails under Pillow too,
//! so they are skipped here instead of becoming generate work that can only be
//! permanently rejected.

use crate::notifications::event_processing::{
    check_create_event_preconditions, CreateEventPrecondition,
};
use crate::notifications::{StorageEvent, StorageEventMessage, StorageEventType};

fn is_eligible(resource_address: &str) -> bool {
    let runtime = crate::tests::parse_runtime_config_from_env_for_tests();
    let event = StorageEvent {
        event_type: StorageEventType::Created.as_str().to_string(),
        principal_identity: "test".to_string(),
        occurred_at: String::new(),
        published_at: String::new(),
        message: StorageEventMessage {
            resource_identity: Some("test-identity".to_string()),
            resource_address: resource_address.to_string(),
            mime_type: None,
            generated_by_thumbnail_pipeline: None,
            traceparent: None,
            tracestate: None,
        },
    };
    matches!(
        check_create_event_preconditions(&event, &runtime),
        CreateEventPrecondition::Eligible { .. }
    )
}

#[test]
#[serial_test::serial]
fn eligibility_follows_the_decodable_extension_set() {
    let cases = [
        ("https://storage.com/sourcefiles/image.jpg", true),
        ("https://storage.com/sourcefiles/photo.jpeg", true),
        ("https://storage.com/sourcefiles/image.png", true),
        ("https://storage.com/sourcefiles/image.gif", true),
        ("https://storage.com/sourcefiles/image.bmp", true),
        ("https://storage.com/sourcefiles/image.tiff", true),
        ("https://storage.com/sourcefiles/image.tif", true),
        ("https://storage.com/sourcefiles/image.webp", true),
        ("https://storage.com/sourcefiles/favicon.ico", true),
        // No decoder ships for these; skipping beats publishing work that fails.
        ("https://storage.com/sourcefiles/diagram.svg", false),
        ("https://storage.com/sourcefiles/image.heic", false),
        ("https://storage.com/sourcefiles/image.heif", false),
        ("https://storage.com/sourcefiles/image.avif", false),
        // USD and video need their own processor backends.
        ("https://storage.com/sourcefiles/model.usd", false),
        ("https://storage.com/sourcefiles/model.usda", false),
        ("https://storage.com/sourcefiles/model.usdc", false),
        ("https://storage.com/sourcefiles/model.usdz", false),
        ("https://storage.com/sourcefiles/video.mp4", false),
        ("https://storage.com/sourcefiles/video.mov", false),
        ("https://storage.com/sourcefiles/video.avi", false),
        ("https://storage.com/sourcefiles/video.mkv", false),
        ("https://storage.com/sourcefiles/video.webm", false),
        ("https://storage.com/sourcefiles/document.pdf", false),
        ("https://storage.com/sourcefiles/data.json", false),
        ("https://storage.com/sourcefiles/file.txt", false),
    ];

    for (address, expected) in cases {
        assert_eq!(is_eligible(address), expected, "eligibility for {address}");
    }
}

#[test]
#[serial_test::serial]
fn any_source_directory_is_eligible() {
    for address in [
        "https://storage.com/bucket1/image.jpg",
        "https://storage.com/bucket2/folder/image.png",
        "https://storage.com/custom-source/photo.gif",
        "https://storage.com/any-path/nested/deep/image.webp",
        "https://storage.com/thumbnail-output/image.jpg",
    ] {
        assert!(is_eligible(address), "{address} should be eligible");
    }
}

#[test]
#[serial_test::serial]
fn extension_matching_is_case_insensitive() {
    let cases = [
        ("https://storage.com/forthumbnails/image.JPG", true),
        ("https://storage.com/forthumbnails/image.Jpg", true),
        ("https://storage.com/forthumbnails/image.jpg", true),
        ("https://storage.com/forthumbnails/image.PNG", true),
        ("https://storage.com/forthumbnails/image.WEBP", true),
        ("https://storage.com/forthumbnails/image.GIF", true),
        ("https://storage.com/forthumbnails/video.Mp4", false),
        ("https://storage.com/forthumbnails/video.MP4", false),
    ];

    for (address, expected) in cases {
        assert_eq!(is_eligible(address), expected, "eligibility for {address}");
    }
}

/// A template that renders its own source address is a direct self-overwrite.
/// It is refused at parse time, so the destination guard downstream is
/// defense-in-depth rather than the only protection.
#[test]
fn self_overwrite_template_is_rejected_at_parse_time() {
    let error = crate::ThumbnailPathTemplate::parse("{resource_path}{resource_filename}")
        .expect_err("a template that renders the source address must be refused at startup");

    assert!(
        error.to_string().contains("round") || error.to_string().contains("template"),
        "unexpected error message: {error}"
    );
}
