// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use super::common::KeyValue;
use super::{proto, vec_into};

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct Resource {
    pub attributes: Vec<KeyValue>,
    pub dropped_attributes_count: u32,
}

impl Resource {
    pub fn new(attrs: Vec<KeyValue>) -> Self {
        Self {
            attributes: attrs,
            dropped_attributes_count: 0,
        }
    }
}

impl From<proto::resource::Resource> for Resource {
    fn from(value: proto::resource::Resource) -> Self {
        let proto::resource::Resource {
            attributes,
            dropped_attributes_count,
            entity_refs: _,
        } = value;
        let attributes = vec_into!(attributes);
        Self {
            attributes,
            dropped_attributes_count,
        }
    }
}
