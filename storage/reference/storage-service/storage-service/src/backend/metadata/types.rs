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

use storage_service_protos::google::protobuf::Value;
use storage_service_protos::nvidia::omniverse::storage::metadata::v1alpha::UserMetadataValue;

/// A metadata key-value pair.
#[derive(Debug, Clone, get_size2::GetSize)]
pub struct NamedKeyValue {
    pub key: String,
    pub value: KeyValue,
}

/// The value of a metadata key.
#[derive(Debug, Clone, get_size2::GetSize)]
pub struct KeyValue {
    pub value: Value,
    pub etag: String,
}

impl From<KeyValue> for UserMetadataValue {
    fn from(value: KeyValue) -> Self {
        Self {
            value: Some(value.value),
            etag: value.etag,
        }
    }
}
