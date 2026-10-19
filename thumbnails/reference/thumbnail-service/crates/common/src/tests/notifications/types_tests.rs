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

//! Tests for notification types and basic configurations

use crate::notifications::*;

#[test]
fn test_storage_event_type_conversion() {
    assert_eq!(StorageEventType::Created.as_str(), "omni.storage.created");
    assert_eq!(
        StorageEventType::from_str("omni.storage.created"),
        Some(StorageEventType::Created)
    );
    assert_eq!(StorageEventType::from_str("unknown"), None);
}
