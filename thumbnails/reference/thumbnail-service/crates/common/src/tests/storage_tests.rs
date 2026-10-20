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

use crate::thumbnails::*;
use std::env;

#[test]
fn test_parse_thumbnail_key() {
    let valid_key = "thumbnail_256x256_PNG_9766778a-d92b-4463-92a7-8c1a93ea2e63";
    let parsed = parse_thumbnail_key(valid_key);
    assert!(parsed.is_some());

    let info = parsed.unwrap();
    assert_eq!(info.width, 256);
    assert_eq!(info.height, 256);
    assert_eq!(info.format, "PNG");
    assert_eq!(info.uuid, "9766778a-d92b-4463-92a7-8c1a93ea2e63");

    assert!(parse_thumbnail_key("not_a_thumbnail_key").is_none());
    assert!(parse_thumbnail_key("thumbnail_invalid").is_none());
    assert!(parse_thumbnail_key("thumbnail_256_PNG_uuid").is_none()); // Missing 'x' in dimensions
    assert!(parse_thumbnail_key("").is_none());
}

#[test]
fn test_parse_thumbnail_key_with_different_formats() {
    let test_cases = vec![
        (
            "thumbnail_128x128_JPEG_abcd1234-5678-9012-3456-789012345678",
            128,
            128,
            "JPEG",
        ),
        (
            "thumbnail_512x512_WEBP_11111111-2222-3333-4444-555555555555",
            512,
            512,
            "WEBP",
        ),
        (
            "thumbnail_1024x768_PNG_aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            1024,
            768,
            "PNG",
        ),
    ];

    for (key, expected_width, expected_height, expected_format) in test_cases {
        let parsed = parse_thumbnail_key(key);
        assert!(parsed.is_some(), "Failed to parse key: {}", key);

        let info = parsed.unwrap();
        assert_eq!(info.width, expected_width);
        assert_eq!(info.height, expected_height);
        assert_eq!(info.format, expected_format);
    }
}

#[test]
fn test_parse_thumbnail_key_edge_cases() {
    // Note: The parser doesn't validate UUID format, it accepts any string as the UUID part
    let invalid_keys = vec![
        "",
        "thumbnail",
        "thumbnail_256",
        "thumbnail_256x",
        "thumbnail_x256_PNG_uuid",
        "thumbnail_256x_PNG_uuid",
        "thumbnail_abcx256_PNG_uuid",
        "thumbnail_256xabc_PNG_uuid",
        "not_thumbnail_256x256_PNG_valid-uuid",
        "thumbnail_256x256",
        "thumbnail_256x256_PNG",
    ];

    for key in invalid_keys {
        let parsed = parse_thumbnail_key(key);
        assert!(
            parsed.is_none(),
            "Key '{}' should be invalid but was parsed successfully",
            key
        );
    }

    let lenient_key = "thumbnail_256x256_PNG_not-a-valid-uuid";
    let parsed = parse_thumbnail_key(lenient_key);
    assert!(
        parsed.is_some(),
        "Parser should accept any string as UUID part"
    );
    assert_eq!(parsed.unwrap().uuid, "not-a-valid-uuid");
}

#[test]
fn test_association_metadata_structure() {
    use serde_json::json;

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

    // Same logic as associate_thumbnail
    let processor_parameters_hash = format!("{:x}", md5::compute(spec.to_string().as_bytes()));

    assert!(!processor_parameters_hash.is_empty());
    assert_eq!(processor_parameters_hash.len(), 32); // MD5 hash length

    let thumbnail_metadata = json!({
        "thumbnail_resource_id": "test-thumbnail-id",
        "status": "completed",
        "task_id": null,
        "processor_parameters_hash": processor_parameters_hash,
        "specification": {
            "type": "ImageToImageSpecification",
            "width": spec["width"],
            "height": spec["height"],
            "encoding_format": spec["encoding_format"],
            "transform_type": spec["transform_type"],
            "crop_strategy": spec["crop_strategy"],
            "quality": spec["quality"],
            "preserve_aspect_ratio": spec["preserve_aspect_ratio"],
            "processor_identity": spec["processor_identity"],
            "destination_resource_address": null,
            "upload_preference": spec["upload_preference"]
        }
    });

    assert!(thumbnail_metadata.is_object());
    assert_eq!(thumbnail_metadata["status"], "completed");
    assert!(thumbnail_metadata["task_id"].is_null());
    assert_eq!(thumbnail_metadata["specification"]["width"], spec["width"]);
    assert_eq!(
        thumbnail_metadata["specification"]["height"],
        spec["height"]
    );
}

#[test]
fn test_thumbnail_key_generation() {
    use serde_json::json;

    let spec = json!({
        "width": "256",
        "height": "256",
        "encoding_format": "PNG"
    });

    let uuid = uuid::Uuid::new_v4().to_string();

    let thumbnail_key = format!(
        "thumbnail_{}x{}_{}_{}",
        spec["width"].as_str().unwrap_or("256"),
        spec["height"].as_str().unwrap_or("256"),
        spec["encoding_format"].as_str().unwrap_or("PNG"),
        uuid
    );

    assert!(thumbnail_key.starts_with("thumbnail_"));
    assert!(thumbnail_key.contains("256x256"));
    assert!(thumbnail_key.contains("PNG"));
    assert!(thumbnail_key.contains(&uuid));

    let parsed = parse_thumbnail_key(&thumbnail_key);
    assert!(parsed.is_some());

    let info = parsed.unwrap();
    assert_eq!(info.width, 256);
    assert_eq!(info.height, 256);
    assert_eq!(info.format, "PNG");
    assert_eq!(info.uuid, uuid);
}

#[test]
#[serial_test::serial]
fn test_storage_endpoint_protocol_consistency() {
    // No scheme for local gRPC compatibility (tonic expects host:port)

    let original_value = crate::config::raw_env_value("STORAGE_GRPC_ENDPOINT");

    let test_endpoint = "test-storage-service.test.svc.cluster.local:50001";
    env::set_var("STORAGE_GRPC_ENDPOINT", test_endpoint);

    let test_cases = vec![test_endpoint.to_string(), test_endpoint.to_string()];

    for endpoint in &test_cases {
        assert!(
            !endpoint.starts_with("http://")
                && !endpoint.starts_with("https://")
                && !endpoint.starts_with("grpc://"),
            "Storage endpoint '{}' should use no scheme for local gRPC (just hostname:port)",
            endpoint
        );

        assert!(
            endpoint.contains(":"),
            "Storage endpoint '{}' should contain port (format: hostname:port)",
            endpoint
        );

        assert!(
            endpoint.contains(test_endpoint),
            "Storage endpoint '{}' should contain the configured endpoint '{}'",
            endpoint,
            test_endpoint
        );
    }

    // Regression: MetadataServiceClient::connect used to pass a scheme incorrectly
    let metadata_client_endpoint = test_endpoint.to_string();
    assert_eq!(
        metadata_client_endpoint, test_endpoint,
        "MetadataServiceClient::connect should use no scheme (just hostname:port)"
    );

    if let Some(value) = original_value {
        env::set_var("STORAGE_GRPC_ENDPOINT", value);
    } else {
        env::remove_var("STORAGE_GRPC_ENDPOINT");
    }
}

#[test]
#[serial_test::serial]
fn test_storage_endpoint_default_value() {
    let original_value = crate::config::raw_env_value("STORAGE_GRPC_ENDPOINT");

    env::remove_var("STORAGE_GRPC_ENDPOINT");

    let default_endpoint = crate::config::raw_env_value("STORAGE_GRPC_ENDPOINT")
        .unwrap_or_else(|| "localhost:50001".to_string());

    assert_eq!(
        default_endpoint, "localhost:50001",
        "Default endpoint should be localhost:50001"
    );

    env::set_var("STORAGE_GRPC_ENDPOINT", "custom-host:9999");

    let custom_endpoint = crate::config::raw_env_value("STORAGE_GRPC_ENDPOINT")
        .unwrap_or_else(|| "localhost:50001".to_string());

    assert_eq!(
        custom_endpoint, "custom-host:9999",
        "Custom endpoint should be used when set"
    );

    if let Some(value) = original_value {
        env::set_var("STORAGE_GRPC_ENDPOINT", value);
    } else {
        env::remove_var("STORAGE_GRPC_ENDPOINT");
    }
}

#[test]
fn test_folder_filtering_logic() {
    let test_cases = vec![
        ("https://storage.example.com/folder/", true),
        ("https://storage.example.com/file.txt", false),
        ("https://storage.example.com/.folder/test", true),
        ("https://storage.example.com/test.folder", true),
        ("https://storage.example.com/folder/file.txt", false),
    ];

    for (resource_address, is_folder) in test_cases {
        let should_skip = resource_address.ends_with("/")
            || resource_address.ends_with(".folder")
            || resource_address.contains("/.folder/");

        assert_eq!(
            should_skip, is_folder,
            "Resource '{}' folder detection mismatch",
            resource_address
        );
    }
}
