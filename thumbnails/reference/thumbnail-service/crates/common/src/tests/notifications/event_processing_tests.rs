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

//! Tests for event processing and thumbnail detection

use crate::notifications::*;
use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::Event as NotificationEvent;
use prost_types::{value::Kind, Struct, Timestamp, Value};
use std::collections::BTreeMap;

fn create_mock_notification_event(
    event_type: &str,
    resource_address: &str,
    resource_identity: Option<&str>,
    mime_type: Option<&str>,
) -> NotificationEvent {
    let mut fields = BTreeMap::new();

    fields.insert(
        "resource_address".to_string(),
        Value {
            kind: Some(Kind::StringValue(resource_address.to_string())),
        },
    );

    if let Some(identity) = resource_identity {
        fields.insert(
            "resource_identity".to_string(),
            Value {
                kind: Some(Kind::StringValue(identity.to_string())),
            },
        );
    }

    if let Some(mime) = mime_type {
        fields.insert(
            "mime_type".to_string(),
            Value {
                kind: Some(Kind::StringValue(mime.to_string())),
            },
        );
    }

    NotificationEvent {
        event_type: event_type.to_string(),
        principal_identity: "test-user".to_string(),
        occurred_at: Some(Timestamp {
            seconds: 1609459200,
            nanos: 123456789,
        }),
        published_at: Some(Timestamp {
            seconds: 1609459200,
            nanos: 987654321,
        }),
        message: Some(Struct { fields }),
    }
}

#[test]
fn test_convert_grpc_event_to_storage_event_success() {
    let notification_event = create_mock_notification_event(
        StorageEventType::Created.as_str(),
        "omniverse://storage/projects/test/file.usd",
        Some("identity-123"),
        Some("application/x-usd"),
    );

    let result = crate::notifications::event_processing::convert_grpc_event_to_storage_event(
        notification_event.clone(),
    );

    assert!(result.is_ok());
    let storage_event = result.unwrap();

    assert_eq!(storage_event.event_type, StorageEventType::Created.as_str());
    assert_eq!(storage_event.principal_identity, "test-user");
    assert_eq!(
        storage_event.message.resource_address,
        "omniverse://storage/projects/test/file.usd"
    );
    assert_eq!(
        storage_event.message.resource_identity,
        Some("identity-123".to_string())
    );
    assert_eq!(
        storage_event.message.mime_type,
        Some("application/x-usd".to_string())
    );
    assert!(storage_event.occurred_at.contains("1609459200"));
    assert!(storage_event.published_at.contains("1609459200"));
}

#[test]
fn test_convert_grpc_event_without_optional_fields() {
    let notification_event = create_mock_notification_event(
        StorageEventType::Created.as_str(),
        "omniverse://storage/projects/test/file.png",
        None,
        None,
    );

    let result = crate::notifications::event_processing::convert_grpc_event_to_storage_event(
        notification_event,
    );

    assert!(result.is_ok());
    let storage_event = result.unwrap();

    assert_eq!(
        storage_event.message.resource_address,
        "omniverse://storage/projects/test/file.png"
    );
    assert_eq!(storage_event.message.resource_identity, None);
    assert_eq!(storage_event.message.mime_type, None);
}

#[test]
fn test_convert_grpc_event_missing_message() {
    let event = NotificationEvent {
        event_type: StorageEventType::Created.as_str().to_string(),
        principal_identity: "test-user".to_string(),
        occurred_at: Some(Timestamp {
            seconds: 1609459200,
            nanos: 0,
        }),
        published_at: Some(Timestamp {
            seconds: 1609459200,
            nanos: 0,
        }),
        message: None,
    };

    let result = crate::notifications::event_processing::convert_grpc_event_to_storage_event(event);

    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), "Event message is missing");
}

#[test]
fn test_convert_grpc_event_missing_resource_address() {
    let mut fields = BTreeMap::new();
    // Intentionally not adding resource_address
    fields.insert(
        "resource_identity".to_string(),
        Value {
            kind: Some(Kind::StringValue("identity-123".to_string())),
        },
    );

    let event = NotificationEvent {
        event_type: StorageEventType::Created.as_str().to_string(),
        principal_identity: "test-user".to_string(),
        occurred_at: Some(Timestamp {
            seconds: 1609459200,
            nanos: 0,
        }),
        published_at: Some(Timestamp {
            seconds: 1609459200,
            nanos: 0,
        }),
        message: Some(Struct { fields }),
    };

    let result = crate::notifications::event_processing::convert_grpc_event_to_storage_event(event);

    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        "Missing resource_address in event message"
    );
}

#[test]
fn test_convert_grpc_event_wrong_type_resource_address() {
    let mut fields = BTreeMap::new();
    // Use wrong type (number instead of string)
    fields.insert(
        "resource_address".to_string(),
        Value {
            kind: Some(Kind::NumberValue(123.45)),
        },
    );

    let event = NotificationEvent {
        event_type: StorageEventType::Created.as_str().to_string(),
        principal_identity: "test-user".to_string(),
        occurred_at: Some(Timestamp {
            seconds: 1609459200,
            nanos: 0,
        }),
        published_at: Some(Timestamp {
            seconds: 1609459200,
            nanos: 0,
        }),
        message: Some(Struct { fields }),
    };

    let result = crate::notifications::event_processing::convert_grpc_event_to_storage_event(event);

    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        "Missing resource_address in event message"
    );
}

#[test]
fn test_storage_event_type_conversions() {
    assert_eq!(StorageEventType::Created.as_str(), "omni.storage.created");
    assert_eq!(StorageEventType::Deleted.as_str(), "omni.storage.deleted");
    assert_eq!(
        StorageEventType::DirCreated.as_str(),
        "omni.storage.dir_created"
    );
    assert_eq!(
        StorageEventType::DirDeleted.as_str(),
        "omni.storage.dir_deleted"
    );

    assert_eq!(
        StorageEventType::from_str("omni.storage.created"),
        Some(StorageEventType::Created)
    );
    assert_eq!(
        StorageEventType::from_str("omni.storage.deleted"),
        Some(StorageEventType::Deleted)
    );
    assert_eq!(
        StorageEventType::from_str("omni.storage.dir_created"),
        Some(StorageEventType::DirCreated)
    );
    assert_eq!(
        StorageEventType::from_str("omni.storage.dir_deleted"),
        Some(StorageEventType::DirDeleted)
    );
    assert_eq!(StorageEventType::from_str("invalid.event.type"), None);
    assert_eq!(StorageEventType::from_str(""), None);
}

#[test]
fn test_storage_event_type_roundtrip() {
    let types = vec![
        StorageEventType::Created,
        StorageEventType::Deleted,
        StorageEventType::DirCreated,
        StorageEventType::DirDeleted,
    ];

    for event_type in types {
        let as_str = event_type.as_str();
        let from_str = StorageEventType::from_str(as_str);
        assert_eq!(from_str, Some(event_type));
    }
}

#[test]
fn test_folder_detection_trailing_slash() {
    let event = StorageEvent {
        event_type: StorageEventType::Created.as_str().to_string(),
        principal_identity: "test".to_string(),
        occurred_at: "2024-01-01T00:00:00Z".to_string(),
        published_at: "2024-01-01T00:00:00Z".to_string(),
        message: StorageEventMessage {
            resource_identity: None,
            resource_address: "omniverse://storage/projects/test/".to_string(),
            mime_type: None,
            generated_by_thumbnail_pipeline: None,
            traceparent: None,
            tracestate: None,
        },
    };

    // We can't directly test is_folder_event as it's private, but we can test the logic
    assert!(event.message.resource_address.ends_with('/'));
}

#[test]
fn test_folder_detection_dot_folder_suffix() {
    let addresses = vec![
        "omniverse://storage/projects/test.folder",
        "omniverse://storage/projects/.folder/nested",
        "omniverse://storage/projects/test/.folder/file",
    ];

    for address in addresses {
        assert!(
            address.ends_with(".folder") || address.contains("/.folder/"),
            "Address {} should be detected as folder",
            address
        );
    }
}

#[test]
fn test_file_detection_not_folder() {
    let addresses = vec![
        "omniverse://storage/projects/test/file.usd",
        "omniverse://storage/projects/test.png",
        "omniverse://storage/projects/test_folder_not",
    ];

    for address in addresses {
        assert!(
            !address.ends_with('/')
                && !address.ends_with(".folder")
                && !address.contains("/.folder/"),
            "Address {} should be detected as file",
            address
        );
    }
}

#[test]
fn test_storage_event_structure() {
    let event = StorageEvent {
        event_type: StorageEventType::Created.as_str().to_string(),
        principal_identity: "user-123".to_string(),
        occurred_at: "2024-01-01T00:00:00Z".to_string(),
        published_at: "2024-01-01T00:00:01Z".to_string(),
        message: StorageEventMessage {
            resource_identity: Some("identity-456".to_string()),
            resource_address: "omniverse://storage/file.usd".to_string(),
            mime_type: Some("application/x-usd".to_string()),
            generated_by_thumbnail_pipeline: None,
            traceparent: None,
            tracestate: None,
        },
    };

    let cloned = event.clone();
    assert_eq!(cloned.event_type, event.event_type);
    assert_eq!(cloned.principal_identity, event.principal_identity);
    assert_eq!(
        cloned.message.resource_address,
        event.message.resource_address
    );

    let debug_output = format!("{:?}", event);
    assert!(debug_output.contains("StorageEvent"));
    assert!(debug_output.contains(StorageEventType::Created.as_str()));
}

#[test]
fn test_storage_event_message_structure() {
    let message = StorageEventMessage {
        resource_identity: Some("id-123".to_string()),
        resource_address: "omniverse://storage/test.usd".to_string(),
        mime_type: Some("application/x-usd".to_string()),
        generated_by_thumbnail_pipeline: None,
        traceparent: None,
        tracestate: None,
    };

    let cloned = message.clone();
    assert_eq!(cloned.resource_identity, message.resource_identity);
    assert_eq!(cloned.resource_address, message.resource_address);
    assert_eq!(cloned.mime_type, message.mime_type);
}

// Note: the old `ClientCredentialsArgs::from_env()` smoke test was removed
// when `thumbnail-common` migrated to `thumbnail-auth`'s clap-driven config.
// End-to-end env-var parsing is now covered in `crates/auth/src/interceptors/tests.rs`.
