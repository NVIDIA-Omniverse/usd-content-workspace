// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub display_name: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub email: Option<String>,
    pub username: String,
    pub active: Option<bool>,
    pub job_title: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub display_name: String,
    pub description: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserQuery {
    pub email: Option<String>,
    pub display_name: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroupQuery {
    pub display_name: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
    pub cursor: Option<String>,
}

pub enum DisplayNameFilter<'a> {
    Search(&'a str),
    Prefix(&'a str),
}

pub fn parse_display_name_filter(value: &str) -> DisplayNameFilter<'_> {
    match value.strip_suffix('*') {
        Some(prefix) => DisplayNameFilter::Prefix(prefix),
        None => DisplayNameFilter::Search(value),
    }
}

pub const MAX_PAGE_SIZE: u32 = 100;

fn default_limit() -> u32 {
    20
}

impl UserQuery {
    pub fn effective_limit(&self) -> u32 {
        self.limit.min(MAX_PAGE_SIZE)
    }
}

impl GroupQuery {
    pub fn effective_limit(&self) -> u32 {
        self.limit.min(MAX_PAGE_SIZE)
    }
}
