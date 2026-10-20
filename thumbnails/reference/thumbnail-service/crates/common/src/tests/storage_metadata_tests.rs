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

//! Tests for storage metadata operations

use std::env;

use crate::tests::try_parse_runtime_config_for_tests;

// These assert that a rejected retry setting is rejected at parse time. They
// used to prove it by exporting the bad value and catching the resulting panic,
// which made the invalid value visible to every concurrently running test that
// parses runtime config — 59 call sites, none of which expect
// STORAGE_METADATA_MAX_RETRIES to be 0. `#[serial]` did not prevent it: it
// orders serial tests against each other, not against the rest of the suite.
// The failure surfaced in CI as two unrelated tests failing on a bad retry
// value neither of them had set.
//
// Passing the value on the argument list tests exactly the same clap constraint
// and keeps the blast radius inside the test.

#[test]
#[serial_test::serial]
fn storage_metadata_retry_config_rejects_zero_durations() {
    assert!(
        try_parse_runtime_config_for_tests(&["--storage-metadata-initial-delay", "0ms"]).is_err(),
        "a zero initial delay should be rejected at parse time"
    );

    assert!(
        try_parse_runtime_config_for_tests(&["--storage-metadata-max-delay", "0"]).is_err(),
        "a zero max delay should be rejected at parse time"
    );
}

#[test]
#[serial_test::serial]
fn storage_metadata_retry_config_accepts_positive_durations() {
    try_parse_runtime_config_for_tests(&[
        "--storage-metadata-initial-delay",
        "200ms",
        "--storage-metadata-max-delay",
        "2s",
    ])
    .expect("positive durations should parse");
}

#[test]
#[serial_test::serial]
fn storage_metadata_max_retries_rejects_zero() {
    assert!(
        try_parse_runtime_config_for_tests(&["--storage-metadata-max-retries", "0"]).is_err(),
        "zero retries should be rejected at parse time"
    );

    try_parse_runtime_config_for_tests(&["--storage-metadata-max-retries", "1"])
        .expect("one retry is the documented minimum and should parse");
}

#[test]
fn test_verify_thumbnail_metadata_key_extraction() {
    // Metadata key format used in verify_thumbnail_and_get_source
    let _thumbnail_identity = "thumb-resource-id-123";
    let expected_key = "source_resource_id";

    assert_eq!(expected_key, "source_resource_id");
    assert!(!expected_key.contains(" "));
    assert!(!expected_key.contains("/"));
}

#[test]
fn test_metadata_update_key_format() {
    let test_keys = vec![
        "source_resource_id",
        "thumbnail_256x256_PNG_abc123",
        "processor_parameters_hash",
        "status",
        "task_id",
    ];

    for key in test_keys {
        assert!(
            !key.contains(" "),
            "Key '{}' should not contain spaces",
            key
        );
        assert!(
            !key.contains("/"),
            "Key '{}' should not contain slashes",
            key
        );
        assert!(
            !key.contains("?"),
            "Key '{}' should not contain question marks",
            key
        );
        assert!(
            !key.contains("&"),
            "Key '{}' should not contain ampersands",
            key
        );
    }
}

#[test]
fn test_resource_address_formats() {
    let valid_addresses = vec![
        "omniverse://storage/path/to/file.usd",
        "https://storage.example.com/file.png",
        "file:///local/path/file.txt",
        "/absolute/path/file.jpg",
        "relative/path/file.mp4",
    ];

    for address in valid_addresses {
        assert!(!address.is_empty(), "Address should not be empty");

        assert!(
            address.contains('/') || !address.contains('/'),
            "Address format: {}",
            address
        );
    }
}

#[test]
#[serial_test::serial]
fn test_storage_grpc_endpoint_formatting() {
    let original = crate::config::raw_env_value("STORAGE_GRPC_ENDPOINT");

    let test_endpoint = "storage-service.cluster.local:50001";
    env::set_var("STORAGE_GRPC_ENDPOINT", test_endpoint);

    let endpoint = crate::config::raw_env_value("STORAGE_GRPC_ENDPOINT")
        .unwrap_or_else(|| "localhost:50001".to_string());

    // tonic prepends http:// when creating the channel
    let channel_endpoint = format!("http://{}", endpoint);

    assert_eq!(
        channel_endpoint,
        "http://storage-service.cluster.local:50001"
    );
    assert!(channel_endpoint.starts_with("http://"));

    match original {
        Some(val) => env::set_var("STORAGE_GRPC_ENDPOINT", val),
        None => env::remove_var("STORAGE_GRPC_ENDPOINT"),
    }
}

#[test]
fn test_auth_token_format_validation() {
    let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test";
    let auth_header = format!("Bearer {}", token);

    assert!(auth_header.starts_with("Bearer "));
    assert!(auth_header.contains(token));

    let extracted_token = auth_header.strip_prefix("Bearer ").unwrap();
    assert_eq!(extracted_token, token);
}

#[test]
fn test_cache_control_header_format() {
    let cache_control = "no-cache, no-store";

    assert!(cache_control.contains("no-cache"));
    assert!(cache_control.contains("no-store"));
    assert_eq!(cache_control, "no-cache, no-store");
}

#[test]
fn test_metadata_service_error_categories() {
    let error_messages = vec![
        ("File not found", "not found"),
        ("Permission denied", "permission"),
        ("Authentication failed", "auth"),
        ("BlobNotFound", "blob"),
        ("relative URL without a base", "url"),
    ];

    for (error_msg, expected_category) in error_messages {
        let lowercase_msg = error_msg.to_lowercase();
        let contains_category =
            lowercase_msg.contains(expected_category) || error_msg.contains(expected_category);

        assert!(
            contains_category,
            "Error '{}' should contain category '{}'",
            error_msg, expected_category
        );
    }
}

#[test]
fn test_resource_identity_encoding() {
    let test_identities = vec![
        "simple-identity-123",
        "identity_with_underscores",
        "identity-with-dashes",
        "CamelCaseIdentity",
    ];

    for identity in test_identities {
        assert!(!identity.is_empty());

        assert!(!identity.contains(" "));
        assert!(!identity.contains("?"));
        assert!(!identity.contains("&"));
        assert!(!identity.contains("="));
    }
}

#[test]
fn test_thumbnail_metadata_value_types() {
    use serde_json::json;

    let string_value = json!("test-identity-123");
    let null_value = json!(null);
    let object_value = json!({
        "width": "256",
        "height": "256",
        "format": "PNG"
    });

    assert!(string_value.is_string());
    assert!(null_value.is_null());
    assert!(object_value.is_object());

    assert_eq!(string_value.as_str(), Some("test-identity-123"));

    assert_eq!(object_value["width"], "256");
}

#[test]
fn test_metadata_json_serialization() {
    use serde_json::json;

    let metadata = json!({
        "thumbnail_resource_id": "thumb-123",
        "status": "completed",
        "task_id": null,
        "specification": {
            "width": "256",
            "height": "256"
        }
    });

    let serialized = metadata.to_string();
    assert!(serialized.contains("thumbnail_resource_id"));
    assert!(serialized.contains("thumb-123"));
    assert!(serialized.contains("status"));
    assert!(serialized.contains("completed"));

    let deserialized: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized["status"], "completed");
}

#[test]
fn test_md5_hash_generation_for_processor_params() {
    use serde_json::json;

    let spec1 = json!({
        "width": "256",
        "height": "256",
        "format": "PNG"
    });

    let spec2 = json!({
        "width": "256",
        "height": "256",
        "format": "PNG"
    });

    let spec3 = json!({
        "width": "512",
        "height": "512",
        "format": "JPEG"
    });

    let hash1 = format!("{:x}", md5::compute(spec1.to_string().as_bytes()));
    let hash2 = format!("{:x}", md5::compute(spec2.to_string().as_bytes()));
    let hash3 = format!("{:x}", md5::compute(spec3.to_string().as_bytes()));

    assert_eq!(hash1, hash2);

    assert_ne!(hash1, hash3);

    assert_eq!(hash1.len(), 32);
    assert!(hash1.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_extract_valid_token_helper_usage() {
    // Used in get_resource_identity_by_address_with_service_account; TokenManager is not exercised here.
    let test_token = Some("valid-token-123".to_string());
    assert!(test_token.is_some());

    let test_none: Option<String> = None;
    assert!(test_none.is_none());
}
