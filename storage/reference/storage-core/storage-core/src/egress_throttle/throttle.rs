// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use figment::Figment;
use serde::{Deserialize, Serialize};
use storage_core_util::semaphore::Semaphore;
use storage_core_util::warn_limited;

use storage_core_builder::layer::Layer;

/// Configuration for the egress throttle.
///
/// The configuration key is `egress_throttle` when extracting from a Figment struct.
///
/// Example configuration:
/// ```toml
/// [egress_throttle]
/// enabled = true
/// max_concurrency = 256
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Enable the egress throttle.
    /// Default is true
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Maximum number of concurrent requests to the backend to allow.
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,
    /// Log WARN when this percentage of permits are in use (0-100, 0 to disable). Default: 80.
    #[serde(default = "default_warn_utilization_pct")]
    pub warn_utilization_pct: u8,
    /// Log ERROR when this percentage of permits are in use (0-100, 0 to disable). Default: 95.
    #[serde(default = "default_error_utilization_pct")]
    pub error_utilization_pct: u8,
}

fn default_enabled() -> bool {
    true
}

fn default_max_concurrency() -> usize {
    256
}

fn default_warn_utilization_pct() -> u8 {
    Semaphore::DEFAULT_WARN_UTILIZATION_PCT
}

fn default_error_utilization_pct() -> u8 {
    Semaphore::DEFAULT_ERROR_UTILIZATION_PCT
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            max_concurrency: default_max_concurrency(),
            warn_utilization_pct: default_warn_utilization_pct(),
            error_utilization_pct: default_error_utilization_pct(),
        }
    }
}

impl Config {
    pub const CONFIG_KEY: &str = "egress_throttle";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> std::result::Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

/// Layer for the egress throttle.
///
/// This layer is used to limit the number of concurrent requests to the backend.
///
/// Supports the following traits:
/// - `v1alpha::FileObject`
/// - `v1alpha::FileFolder`
/// - `v1alpha::Capabilities`
/// - `v1alpha::Versioning`
/// - `v1beta::FileObject`
/// - `v1beta::FileFolder`
/// - `v1beta::Capabilities`
/// - `v1beta::Versioning`
/// - `v1alpha::Metadata`
pub struct EgressThrottleLayer {
    pub semaphore: Arc<Semaphore>,
}

impl EgressThrottleLayer {
    pub fn new(config: Config) -> Option<Self> {
        if config.enabled {
            if config.error_utilization_pct > 0
                && config.warn_utilization_pct > 0
                && config.error_utilization_pct < config.warn_utilization_pct
            {
                warn_limited!(
                    warn_utilization_pct = config.warn_utilization_pct,
                    error_utilization_pct = config.error_utilization_pct,
                    "error_utilization_pct should be >= warn_utilization_pct"
                );
            }
            Some(Self {
                semaphore: Arc::new(Semaphore::with_thresholds(
                    config.max_concurrency,
                    "egress_throttle",
                    config.warn_utilization_pct,
                    config.error_utilization_pct,
                )),
            })
        } else {
            None
        }
    }

    /// Creates a new EgressThrottleLayer from a Figment struct.
    ///
    /// The config will be extracted from the Figment struct, looking for the `egress_throttle` key.
    pub fn from_figment(figment: &Figment) -> std::result::Result<Option<Self>, figment::Error> {
        let config = Config::try_from(figment)?;
        Ok(Self::new(config))
    }
}

impl<S> Layer<S> for EgressThrottleLayer {
    type Service = EgressThrottle<S>;

    fn layer(&self, service: S) -> Self::Service {
        EgressThrottle::new(service, self.semaphore.clone())
    }
}

pub struct EgressThrottle<S> {
    pub inner: S,
    pub semaphore: Arc<Semaphore>,
}

impl<S> EgressThrottle<S> {
    pub fn new(inner: S, semaphore: Arc<Semaphore>) -> Self {
        Self { inner, semaphore }
    }
}
