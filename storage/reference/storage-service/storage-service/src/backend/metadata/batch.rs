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

use crate::backend::metadata::*;

pub trait BatchRequestEntry {
    fn resource_location(&self) -> Option<&ResourceLocation>;

    fn resource_location_mut(&mut self) -> Option<&mut ResourceLocation>;

    fn user_metadata_key(&self) -> &str;

    fn namespace(&self) -> Option<&str>;
}

pub trait BatchResponseEntry {
    fn from_error_entry(request_index: u32, error: BatchErrorEntry) -> Self;

    fn request_index(&self) -> u32;
}

pub trait BatchResponse {
    type ResponseEntry;

    fn from_entries(entries: impl IntoIterator<Item = Self::ResponseEntry>) -> Self;

    fn extend(&mut self, entries: impl IntoIterator<Item = Self::ResponseEntry>);

    fn merge(&mut self, other: Self);

    fn update_request_indexes(&mut self, map: &[u32]);
}

impl BatchRequestEntry for GetMetadataBatchRequestEntry {
    fn resource_location(&self) -> Option<&ResourceLocation> {
        self.resource_location.as_ref()
    }

    fn resource_location_mut(&mut self) -> Option<&mut ResourceLocation> {
        self.resource_location.as_mut()
    }

    fn user_metadata_key(&self) -> &str {
        self.user_metadata_key.as_str()
    }

    fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }
}

impl BatchRequestEntry for UpdateMetadataBatchRequestEntry {
    fn resource_location(&self) -> Option<&ResourceLocation> {
        self.resource_location.as_ref()
    }

    fn resource_location_mut(&mut self) -> Option<&mut ResourceLocation> {
        self.resource_location.as_mut()
    }

    fn user_metadata_key(&self) -> &str {
        self.user_metadata_key.as_str()
    }

    fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }
}

impl BatchRequestEntry for DeleteMetadataBatchRequestEntry {
    fn resource_location(&self) -> Option<&ResourceLocation> {
        self.resource_location.as_ref()
    }

    fn resource_location_mut(&mut self) -> Option<&mut ResourceLocation> {
        self.resource_location.as_mut()
    }

    fn user_metadata_key(&self) -> &str {
        self.user_metadata_key.as_str()
    }

    fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }
}

impl BatchResponseEntry for GetMetadataBatchResponseEntry {
    fn from_error_entry(request_index: u32, error: BatchErrorEntry) -> Self {
        Self {
            request_index,
            outcome: Some(get_metadata_batch_response_entry::Outcome::Error(error)),
        }
    }

    fn request_index(&self) -> u32 {
        self.request_index
    }
}

impl BatchResponseEntry for UpdateMetadataBatchResponseEntry {
    fn from_error_entry(request_index: u32, error: BatchErrorEntry) -> Self {
        Self {
            request_index,
            outcome: Some(update_metadata_batch_response_entry::Outcome::Error(error)),
        }
    }

    fn request_index(&self) -> u32 {
        self.request_index
    }
}

impl BatchResponseEntry for DeleteMetadataBatchResponseEntry {
    fn from_error_entry(request_index: u32, error: BatchErrorEntry) -> Self {
        Self {
            request_index,
            outcome: Some(delete_metadata_batch_response_entry::Outcome::Error(error)),
        }
    }

    fn request_index(&self) -> u32 {
        self.request_index
    }
}

impl BatchResponse for GetMetadataBatchResponse {
    type ResponseEntry = GetMetadataBatchResponseEntry;

    fn from_entries(entries: impl IntoIterator<Item = Self::ResponseEntry>) -> Self {
        Self {
            results: entries.into_iter().collect(),
        }
    }

    fn update_request_indexes(&mut self, map: &[u32]) {
        for entry in self.results.iter_mut() {
            entry.request_index = map[entry.request_index as usize];
        }
    }

    fn extend(&mut self, entries: impl IntoIterator<Item = Self::ResponseEntry>) {
        self.results.extend(entries);
    }

    fn merge(&mut self, mut other: Self) {
        self.results.append(&mut other.results);
    }
}

impl BatchResponse for UpdateMetadataBatchResponse {
    type ResponseEntry = UpdateMetadataBatchResponseEntry;

    fn from_entries(entries: impl IntoIterator<Item = Self::ResponseEntry>) -> Self {
        Self {
            results: entries.into_iter().collect(),
        }
    }

    fn update_request_indexes(&mut self, map: &[u32]) {
        for entry in self.results.iter_mut() {
            entry.request_index = map[entry.request_index as usize];
        }
    }

    fn extend(&mut self, entries: impl IntoIterator<Item = Self::ResponseEntry>) {
        self.results.extend(entries);
    }

    fn merge(&mut self, mut other: Self) {
        self.results.append(&mut other.results);
    }
}

impl BatchResponse for DeleteMetadataBatchResponse {
    type ResponseEntry = DeleteMetadataBatchResponseEntry;

    fn from_entries(entries: impl IntoIterator<Item = Self::ResponseEntry>) -> Self {
        Self {
            results: entries.into_iter().collect(),
        }
    }

    fn update_request_indexes(&mut self, map: &[u32]) {
        for entry in self.results.iter_mut() {
            entry.request_index = map[entry.request_index as usize];
        }
    }

    fn extend(&mut self, entries: impl IntoIterator<Item = Self::ResponseEntry>) {
        self.results.extend(entries);
    }

    fn merge(&mut self, mut other: Self) {
        self.results.append(&mut other.results);
    }
}

pub fn create_batch_error_entry_v1alpha(code: batch_error_entry::Code, message: impl Into<String>) -> BatchErrorEntry {
    BatchErrorEntry {
        code: code as i32,
        message: message.into(),
    }
}
