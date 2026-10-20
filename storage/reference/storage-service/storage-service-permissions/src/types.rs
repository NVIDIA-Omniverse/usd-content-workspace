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

use std::hash::Hash;

use indexmap::Equivalent;

/// A batch evaluation condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    None,
    And,
    Or,
}

/// A decision response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionResponse {
    Allow,
    Deny,
    Skip,
    Missing,
}

/// A final decision whether to give or not to give access to a specific resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
}

/// An action over a resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceAction {
    pub resource: String,
    pub resource_type: String,
    pub action: String,
}

impl Hash for ResourceAction {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.resource.hash(state);
        self.resource_type.hash(state);
        self.action.hash(state);
    }
}

/// A non-owning ResourceAction. Used as an alternative to the ResourceAction key.
#[derive(Debug, Clone)]
pub struct ResourceActionRef<'a> {
    pub resource: &'a str,
    pub resource_type: &'a str,
    pub action: &'a str,
}

impl Hash for ResourceActionRef<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.resource.hash(state);
        self.resource_type.hash(state);
        self.action.hash(state);
    }
}

impl Equivalent<ResourceAction> for ResourceActionRef<'_> {
    fn equivalent(&self, key: &ResourceAction) -> bool {
        self.resource == key.resource && self.resource_type == key.resource_type && self.action == key.action
    }
}

/// Any resource type.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ResourceType {
    Object,
    Folder,
}
