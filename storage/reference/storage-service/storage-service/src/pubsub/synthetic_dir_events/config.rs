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

use serde::{de::Error as DeError, Deserialize, Deserializer};

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct SyntheticDirEventsConfig {
    pub enabled: bool,
    pub flush_interval: NonZeroDuration,
}

impl Default for SyntheticDirEventsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            flush_interval: NonZeroDuration::new(Duration::from_secs(6)).unwrap(),
        }
    }
}

/// A `Duration` that is rejected at parse time if it equals zero. Mirrors `std::num::NonZero<T>`
/// for the `Duration` use cases where a zero value would silently break the feature
/// (`tokio::time::interval` panics on zero; zero backoff degenerates to a busy loop).
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct NonZeroDuration(Duration);

impl NonZeroDuration {
    pub fn new(value: Duration) -> Option<Self> {
        if value.is_zero() {
            None
        } else {
            Some(Self(value))
        }
    }

    pub fn get(self) -> Duration {
        self.0
    }
}

impl From<NonZeroDuration> for Duration {
    fn from(value: NonZeroDuration) -> Self {
        value.0
    }
}

impl<'de> Deserialize<'de> for NonZeroDuration {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let duration = humantime_serde::deserialize(deserializer)?;
        Self::new(duration).ok_or_else(|| D::Error::custom("must be non-zero"))
    }
}
