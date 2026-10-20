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

//! Tests for storage association operations

use serde_json::json;
use uuid::Uuid;

#[test]
fn test_thumbnail_key_generation_format() {
    let width = "256";
    let height = "256";
    let format = "PNG";
    let uuid = Uuid::new_v4().to_string();

    let thumbnail_key = format!("thumbnail_{}x{}_{}_{}", width, height, format, uuid);

    assert!(thumbnail_key.starts_with("thumbnail_"));
    assert!(thumbnail_key.contains("256x256"));
    assert!(thumbnail_key.contains("PNG"));
    assert!(thumbnail_key.contains(&uuid));

    let parts: Vec<&str> = thumbnail_key.split('_').collect();
    assert_eq!(parts[0], "thumbnail");
    assert_eq!(parts[1], "256x256");
    assert_eq!(parts[2], "PNG");
    assert_eq!(parts[3], uuid);
}

#[test]
fn test_thumbnail_key_with_different_dimensions() {
    let test_cases = vec![
        ("128", "128", "JPEG"),
        ("256", "256", "PNG"),
        ("512", "512", "WEBP"),
        ("1024", "768", "PNG"),
        ("1920", "1080", "JPEG"),
    ];

    for (width, height, format) in test_cases {
        let uuid = Uuid::new_v4().to_string();
        let key = format!("thumbnail_{}x{}_{}_{}", width, height, format, uuid);

        assert!(key.contains(&format!("{}x{}", width, height)));
        assert!(key.contains(format));
        assert!(key.contains(&uuid));
    }
}

#[test]
fn test_thumbnail_key_uuid_uniqueness() {
    let mut keys = Vec::new();

    for _ in 0..10 {
        let uuid = Uuid::new_v4().to_string();
        let key = format!("thumbnail_256x256_PNG_{}", uuid);
        keys.push(key);
    }

    let unique_keys: std::collections::HashSet<_> = keys.iter().collect();
    assert_eq!(unique_keys.len(), keys.len());
}

#[test]
fn test_processor_parameters_hash_generation() {
    let spec = json!({
        "width": "512",
        "height": "512",
        "encoding_format": "JPEG",
        "transform_type": "SQUARE",
        "crop_strategy": "CENTER",
        "quality": "90",
        "preserve_aspect_ratio": true,
        "processor_identity": "IMAGE_TO_IMAGE",
        "upload_preference": "storage"
    });

    let hash = format!("{:x}", md5::compute(spec.to_string().as_bytes()));

    assert_eq!(hash.len(), 32); // MD5 produces 32 hex characters
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_processor_parameters_hash_consistency() {
    let spec1 = json!({"width": "256", "height": "256", "format": "PNG"});
    let spec2 = json!({"width": "256", "height": "256", "format": "PNG"});

    let hash1 = format!("{:x}", md5::compute(spec1.to_string().as_bytes()));
    let hash2 = format!("{:x}", md5::compute(spec2.to_string().as_bytes()));

    assert_eq!(hash1, hash2);
}

#[test]
fn test_processor_parameters_hash_different_for_different_params() {
    let spec1 = json!({"width": "256", "height": "256", "format": "PNG"});
    let spec2 = json!({"width": "512", "height": "512", "format": "JPEG"});

    let hash1 = format!("{:x}", md5::compute(spec1.to_string().as_bytes()));
    let hash2 = format!("{:x}", md5::compute(spec2.to_string().as_bytes()));

    assert_ne!(hash1, hash2);
}

#[test]
fn test_thumbnail_metadata_structure() {
    let thumbnail_identity = "thumb-resource-id-123";
    let task_id = "task-456";
    let _uuid = Uuid::new_v4().to_string();

    let spec = json!({
        "width": "256",
        "height": "256",
        "encoding_format": "PNG"
    });

    let processor_hash = format!("{:x}", md5::compute(spec.to_string().as_bytes()));

    let metadata = json!({
        "thumbnail_resource_id": thumbnail_identity,
        "status": "completed",
        "task_id": task_id,
        "processor_parameters_hash": processor_hash,
        "specification": spec
    });

    assert_eq!(metadata["thumbnail_resource_id"], thumbnail_identity);
    assert_eq!(metadata["status"], "completed");
    assert_eq!(metadata["task_id"], task_id);
    assert_eq!(metadata["processor_parameters_hash"], processor_hash);
    assert_eq!(metadata["specification"]["width"], "256");
}

#[test]
fn test_thumbnail_metadata_with_null_task_id() {
    // Event-driven creation may omit a task id
    let metadata = json!({
        "thumbnail_resource_id": "thumb-123",
        "status": "completed",
        "task_id": null,
        "processor_parameters_hash": "abc123"
    });

    assert!(metadata["task_id"].is_null());
    assert_eq!(metadata["status"], "completed");
}

#[test]
fn test_source_metadata_structure() {
    let source_metadata = json!({
        "source_resource_id": "source-resource-id-789"
    });

    assert_eq!(
        source_metadata["source_resource_id"],
        "source-resource-id-789"
    );

    let thumbnail_key = "thumbnail_256x256_PNG_abc-def-123";
    let full_metadata = json!({
        thumbnail_key: source_metadata
    });

    assert!(full_metadata[thumbnail_key].is_object());
}

#[test]
fn test_association_metadata_key_format() {
    let uuid = Uuid::new_v4().to_string();
    let thumbnail_key = format!("thumbnail_256x256_PNG_{}", uuid);

    let source_key = "source_resource_id";

    assert_eq!(source_key, "source_resource_id");
    assert!(thumbnail_key.starts_with("thumbnail_"));
}

#[test]
fn test_disassociation_removes_source_reference() {
    let thumbnail_metadata_before = json!({
        "thumbnail_resource_id": "thumb-123",
        "source_resource_id": "source-456",
        "status": "completed"
    });

    let mut thumbnail_metadata_after = thumbnail_metadata_before.clone();
    if let Some(obj) = thumbnail_metadata_after.as_object_mut() {
        obj.remove("source_resource_id");
    }

    assert!(thumbnail_metadata_before["source_resource_id"].is_string());
    assert!(thumbnail_metadata_after.get("source_resource_id").is_none());
}

#[test]
fn test_specification_structure() {
    let spec = json!({
        "type": "ImageToImageSpecification",
        "width": "512",
        "height": "512",
        "encoding_format": "JPEG",
        "transform_type": "SQUARE",
        "crop_strategy": "CENTER",
        "quality": "90",
        "preserve_aspect_ratio": true,
        "processor_identity": "IMAGE_TO_IMAGE",
        "destination_resource_address": null,
        "upload_preference": "storage"
    });

    assert_eq!(spec["type"], "ImageToImageSpecification");
    assert_eq!(spec["width"], "512");
    assert_eq!(spec["height"], "512");
    assert_eq!(spec["encoding_format"], "JPEG");

    assert!(spec["destination_resource_address"].is_null());
    assert_eq!(spec["upload_preference"], "storage");
}

#[test]
fn test_specification_field_types() {
    let spec = json!({
        "width": "256",
        "height": "256",
        "encoding_format": "PNG",
        "quality": "90",
        "preserve_aspect_ratio": true,
    });

    assert!(spec["width"].is_string());
    assert!(spec["height"].is_string());
    assert!(spec["encoding_format"].is_string());
    assert!(spec["quality"].is_string());
    assert!(spec["preserve_aspect_ratio"].is_boolean());
}

#[test]
fn test_metadata_json_serialization_for_grpc() {
    let metadata = json!({
        "thumbnail_resource_id": "thumb-123",
        "status": "completed",
        "specification": {
            "width": "256",
            "height": "256"
        }
    });

    let serialized = metadata.to_string();

    assert!(!serialized.is_empty());
    assert!(serialized.contains("thumbnail_resource_id"));
    assert!(serialized.contains("specification"));

    let deserialized: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized["thumbnail_resource_id"], "thumb-123");
    assert_eq!(deserialized["specification"]["width"], "256");
}

#[test]
fn test_thumbnail_status_values() {
    let valid_statuses = vec!["pending", "processing", "completed", "failed"];

    for status in valid_statuses {
        let metadata = json!({
            "status": status
        });

        assert_eq!(metadata["status"], status);
        assert!(metadata["status"].is_string());
    }
}

#[test]
fn test_association_with_multiple_thumbnails() {
    let source_identity = "source-123";

    let thumb_key1 = "thumbnail_256x256_PNG_abc123";
    let thumb_key2 = "thumbnail_512x512_JPEG_xyz789";

    let source_metadata = json!({
        thumb_key1: {
            "source_resource_id": source_identity
        },
        thumb_key2: {
            "source_resource_id": source_identity
        }
    });

    assert_eq!(
        source_metadata[thumb_key1]["source_resource_id"],
        source_identity
    );
    assert_eq!(
        source_metadata[thumb_key2]["source_resource_id"],
        source_identity
    );
}

#[test]
fn test_thumbnail_key_extraction_from_resource_address() {
    let resource_address = "omniverse://storage/thumbnails/thumbnail_256x256_PNG_abc123.png";

    let filename = resource_address.split('/').next_back().unwrap();
    assert!(filename.starts_with("thumbnail_"));

    let key = filename.split('.').next().unwrap();
    assert_eq!(key, "thumbnail_256x256_PNG_abc123");
}

#[test]
fn test_metadata_update_operation_structure() {
    let resource_identity = "resource-id-123";
    let metadata_key = "thumbnail_256x256_PNG_abc";
    let metadata_value = json!({
        "source_resource_id": "source-id-456"
    });

    assert!(!resource_identity.is_empty());
    assert!(!metadata_key.is_empty());
    assert!(metadata_value.is_object());
}

#[test]
fn test_metadata_removal_operation_structure() {
    let resource_identity = "resource-id-123";
    let metadata_key = "thumbnail_256x256_PNG_abc";

    assert!(!resource_identity.is_empty());
    assert!(!metadata_key.is_empty());
}

#[test]
fn test_encoding_format_values() {
    let valid_formats = vec!["PNG", "JPEG", "WEBP", "GIF", "BMP"];

    for format in valid_formats {
        let spec = json!({
            "encoding_format": format
        });

        assert_eq!(spec["encoding_format"], format);
        assert!(format.chars().all(|c| c.is_uppercase() || c.is_numeric()));
    }
}

#[test]
fn test_transform_type_values() {
    let valid_transforms = vec!["SQUARE", "FIT", "SCALE_BY_WIDTH", "SCALE_AND_CROP"];

    for transform in valid_transforms {
        let spec = json!({
            "transform_type": transform
        });

        assert_eq!(spec["transform_type"], transform);
    }
}

#[test]
fn test_crop_strategy_values() {
    let valid_directions = vec!["CENTER", "TOP", "BOTTOM", "LEFT", "RIGHT"];

    for direction in valid_directions {
        let spec = json!({
            "crop_strategy": direction
        });

        assert_eq!(spec["crop_strategy"], direction);
    }
}

#[test]
fn test_quality_parameter_range() {
    let quality_values = vec!["1", "50", "75", "90", "100"];

    for quality in quality_values {
        let spec = json!({
            "quality": quality
        });

        assert_eq!(spec["quality"], quality);

        let quality_num: u8 = quality.parse().unwrap();
        assert!((1..=100).contains(&quality_num));
    }
}

#[test]
fn test_processor_identity_values() {
    let valid_processors = vec!["IMAGE_TO_IMAGE", "VIDEO_TO_IMAGE", "USD_TO_IMAGE"];

    for processor in valid_processors {
        let spec = json!({
            "processor_identity": processor
        });

        assert_eq!(spec["processor_identity"], processor);
    }
}

#[test]
fn test_upload_preference_values() {
    let valid_preferences = vec!["storage", "direct", "none"];

    for preference in valid_preferences {
        let spec = json!({
            "upload_preference": preference
        });

        assert_eq!(spec["upload_preference"], preference);
    }
}

#[test]
fn test_metadata_key_naming_conventions() {
    let metadata_keys = vec![
        "source_resource_id",
        "thumbnail_resource_id",
        "processor_parameters_hash",
        "task_id",
        "status",
        "specification",
    ];

    for key in metadata_keys {
        assert!(key
            .chars()
            .all(|c| c.is_lowercase() || c.is_numeric() || c == '_'));

        assert!(!key.starts_with('_'));
        assert!(!key.ends_with('_'));
    }
}
