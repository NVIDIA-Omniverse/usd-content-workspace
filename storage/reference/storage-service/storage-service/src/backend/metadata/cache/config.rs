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

use std::time::Duration;

use bytesize::ByteSize;
use serde::Deserialize;

pub const DEFAULT_CACHE_TIME_TO_LIVE: Duration = Duration::from_hours(1);
pub const DEFAULT_CACHE_CAPACITY: ByteSize = ByteSize::gib(1);
pub const DEFAULT_PAGE_SIZE_LIMIT: ByteSize = ByteSize::mib(3);
pub const DEFAULT_L2_CLAIM_TIME_TO_LIVE: Duration = Duration::from_secs(30);

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub cache_capacity: Option<ByteSize>,
    #[serde(with = "humantime_serde::option")]
    pub cache_time_to_live: Option<Duration>,
    pub page_size_limit: Option<ByteSize>,
    pub l2_enabled: bool,
    #[serde(with = "humantime_serde::option")]
    pub l2_claim_time_to_live: Option<Duration>,
}
