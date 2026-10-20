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

use super::{OtelEnvVal, env_vars, var_parse};
use tracing_subscriber::filter::EnvFilter;

env_vars! {
    OTEL_RUST_OTLP_LOGS_FILTER,
    OTEL_RUST_OTLP_TRACES_FILTER,
}

pub(crate) const RUST_LOG: &str = EnvFilter::DEFAULT_ENV;

/// [`EnvFilter`] doesn't implement [`Clone`] or
/// [`PartialEq`]: <https://github.com/tokio-rs/tracing/issues/2360>. And the
/// [`Debug`](std::fmt::Debug) implementation is also questionable.
///
/// However, if we defer parsing, from the config it isn't obvious which filter
/// directives are being used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilterDirectives {
    dirs: String,
    valid: bool,
}

impl FilterDirectives {
    pub(crate) fn new(s: &str) -> Self {
        let dirs = s.to_string();
        let valid = if dirs.is_empty() {
            false
        } else {
            EnvFilter::builder().parse(&dirs).is_ok()
        };
        Self { dirs, valid }
    }

    pub(crate) fn dirs(&self) -> &str {
        &self.dirs
    }
}

impl OtelEnvVal for FilterDirectives {
    type Err = std::convert::Infallible;

    fn from_env(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

pub(crate) fn rust_log() -> Option<FilterDirectives> {
    var_parse!(RUST_LOG)
}

pub(crate) fn otlp_logs_env_filter() -> Option<FilterDirectives> {
    var_parse!(OTEL_RUST_OTLP_LOGS_FILTER)
}

pub(crate) fn otlp_traces_env_filter() -> Option<FilterDirectives> {
    var_parse!(OTEL_RUST_OTLP_TRACES_FILTER)
}

#[cfg(test)]
mod tests;
