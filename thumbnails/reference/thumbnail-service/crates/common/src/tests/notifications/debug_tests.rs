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

//! Tests for debug functionality and logging

use crate::notifications::*;

#[test]
fn test_debug_message_structure() {
    println!("[TEST] Testing debug message structures...");

    let event = StorageEvent {
        event_type: StorageEventType::Created.as_str().to_string(),
        principal_identity: "debug-test".to_string(),
        occurred_at: "2024-01-01T00:00:00Z".to_string(),
        published_at: "2024-01-01T00:00:00Z".to_string(),
        message: StorageEventMessage {
            resource_identity: Some("test-resource".to_string()),
            resource_address: "https://example.com/test.png".to_string(),
            mime_type: None,
            generated_by_thumbnail_pipeline: None,
            traceparent: None,
            tracestate: None,
        },
    };

    // Test event serialization for debugging
    let serialized = serde_json::to_string(&event).expect("Should serialize");
    assert!(serialized.contains(StorageEventType::Created.as_str()));
    assert!(serialized.contains("debug-test"));

    println!("[OK] Debug structure test passed!");
}

// More tests will be extracted from notifications.rs
