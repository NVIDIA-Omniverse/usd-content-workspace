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

use std::num::NonZero;

use bytesize::ByteSize;
use deadpool_postgres::SslMode;
use serde::{Deserialize, Deserializer};

/// Postgres metadata backend configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(flatten)]
    pub pool_config: PoolConfig,
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub query_batch_size: Option<NonZero<u32>>,
    #[serde(default)]
    pub query_all_threshold: Option<NonZero<usize>>,
    #[serde(default)]
    pub query_byte_size_budget: Option<ByteSize>,
    #[serde(default)]
    pub max_metadata_value_size: Option<ByteSize>,
}

/// Postgres connection pool config.
#[derive(Debug, Deserialize, Clone)]
pub struct PoolConfig {
    pub host: String,
    pub dbname: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default, deserialize_with = "deserialize_option_ssl_mode")]
    pub ssl_mode: Option<SslMode>,
    #[serde(default)]
    pub connection_pool_size: Option<NonZero<usize>>,
}

fn deserialize_option_ssl_mode<'de, D>(deserialize: D) -> Result<Option<SslMode>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<SslModeLower>::deserialize(deserialize).map(|maybe_mode| {
        maybe_mode.map(|mode| match mode {
            SslModeLower::Disable => SslMode::Disable,
            SslModeLower::Prefer => SslMode::Prefer,
            SslModeLower::Require => SslMode::Require,
        })
    })
}

#[derive(Debug, Deserialize)]
enum SslModeLower {
    #[serde(rename = "disable")]
    Disable,
    #[serde(rename = "prefer")]
    Prefer,
    #[serde(rename = "require")]
    Require,
}
