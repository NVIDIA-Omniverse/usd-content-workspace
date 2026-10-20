// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Configuration for the retry policy.
///
/// The configuration key is `retry` when extracting from a Figment struct.
///
/// Example configuration:
/// ```toml
/// [retry]
/// attempts = 15
/// initial_backoff = "1s"
/// max_backoff = "30s"
/// max_total_elapsed = "60s"
/// ```
///
/// Note that not all backends support all of these options.
/// For example, the S3 backend does not support the max_total_elapsed option.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Sets the max number of retry attempts. This value must be greater than zero.
    #[serde(default = "default_attempts")]
    pub attempts: u32,

    /// Sets the initial backoff duration. This value must be greater than zero.
    #[serde(default = "default_initial_backoff", with = "humantime_serde")]
    pub initial_backoff: Duration,

    /// Sets the maximum backoff duration. This value must be greater than zero.
    #[serde(default = "default_max_backoff", with = "humantime_serde")]
    pub max_backoff: Duration,

    /// The maximum permissible elapsed time since starting to retry before giving up. This value must be greater than zero.
    #[serde(default = "default_max_total_elapsed", with = "humantime_serde")]
    pub max_total_elapsed: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            attempts: default_attempts(),
            initial_backoff: default_initial_backoff(),
            max_backoff: default_max_backoff(),
            max_total_elapsed: default_max_total_elapsed(),
        }
    }
}

fn default_attempts() -> u32 {
    5
}

fn default_initial_backoff() -> Duration {
    Duration::from_secs(1)
}

fn default_max_backoff() -> Duration {
    Duration::from_secs(20)
}

fn default_max_total_elapsed() -> Duration {
    Duration::from_secs(120)
}
