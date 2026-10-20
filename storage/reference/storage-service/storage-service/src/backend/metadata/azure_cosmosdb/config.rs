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

use serde::Deserialize;

/// CosmosDB metadata backend configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub account: String,
    pub database_id: String,
    pub container_id: String,
    pub query_batch_size: Option<NonZero<usize>>,
    pub max_concurrency: Option<NonZero<usize>>,
    pub max_idle_connections_per_host: Option<usize>,
    #[serde(default)]
    pub consistency_levels: ConsistencyLevelsConfig,
}

#[derive(Debug, Default, Deserialize, Clone, Copy)]
pub enum ConsistencyLevel {
    #[default]
    #[serde(rename = "strong")]
    Strong,
    #[serde(rename = "bounded")]
    Bounded,
    #[serde(rename = "consistent_prefix")]
    ConsistentPrefix,
    #[serde(rename = "eventual")]
    Eventual,
}

impl From<ConsistencyLevel> for azure_data_cosmos::ConsistencyLevel {
    fn from(value: ConsistencyLevel) -> Self {
        match value {
            ConsistencyLevel::Strong => azure_data_cosmos::ConsistencyLevel::Strong,
            ConsistencyLevel::Bounded => azure_data_cosmos::ConsistencyLevel::Bounded,
            ConsistencyLevel::ConsistentPrefix => azure_data_cosmos::ConsistencyLevel::ConsistentPrefix,
            ConsistencyLevel::Eventual => azure_data_cosmos::ConsistencyLevel::Eventual,
        }
    }
}

/// Consistency level configuration for individual operations.
#[derive(Debug, Default, Deserialize, Clone, Copy)]
#[serde(default)]
pub struct ConsistencyLevelsConfig {
    pub query: ConsistencyLevel,
    pub create: ConsistencyLevel,
    pub delete: ConsistencyLevel,
}
