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

use super::build::{EnvFilterSource, TracingSubscriberBuilder};
use crate::env::{common, logs, subscriber};

/// The [`tracing`]/[`tracing_subscriber`] configuration.
///
/// Not to be confused with the OTLP traces exporter
/// [`TracesConfig`](crate::exporters::traces::TracesConfig).
///
/// # Environment variables
///
/// * `OTEL_SDK_DISABLED`: If set, disables console/stdout logging regardless of
///   `OTEL_LOGS_EXPORTER`.
/// * `OTEL_LOGS_EXPORTER`: If this value includes `console`, console/stdout
///   logging is enabled.
/// * `RUST_LOG`: If this value is set and is a valid, these filter directives
///   are applied to console/stdout logging.
/// * `OTEL_RUST_OTLP_LOGS_FILTER`: If this value is set and is valid, these
///   filter directives are applied to OTLP logs exports.
/// * `OTEL_RUST_OTLP_TRACES_FILTER`: If this value is set and is valid, these
///   filter directives are applied to OTLP traces exports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TracingConfig {
    sdk_disabled: Option<bool>,
    exporter: Option<logs::OtelLogsExporter>,
    rust_log: Option<subscriber::FilterDirectives>,
    otlp_logs_env_filter: Option<subscriber::FilterDirectives>,
    otlp_traces_env_filter: Option<subscriber::FilterDirectives>,
}

impl TracingConfig {
    /// Construct the tracing subscriber configuration from environment
    /// variables.
    pub fn from_env() -> Self {
        let sdk_disabled = common::otel_sdk_disabled();
        let exporter = logs::OtelLogsExporter::get();
        let rust_log = subscriber::rust_log();
        let otlp_logs_env_filter = subscriber::otlp_logs_env_filter();
        let otlp_traces_env_filter = subscriber::otlp_traces_env_filter();

        Self {
            sdk_disabled,
            exporter,
            rust_log,
            otlp_logs_env_filter,
            otlp_traces_env_filter,
        }
    }

    /// Construct the tracing subscriber configuration with a completely
    /// disabled state. Equivalent to setting `OTEL_SDK_DISABLED`.
    #[inline]
    pub fn disabled_for_test() -> Self {
        Self {
            sdk_disabled: Some(true),
            exporter: None,
            rust_log: None,
            otlp_logs_env_filter: None,
            otlp_traces_env_filter: None,
        }
    }

    /// `true` if console/stdout logs export is enabled.
    pub(crate) fn console_enabled(&self) -> bool {
        if self.sdk_disabled == Some(true) {
            return false;
        }

        self.exporter
            .map(|exporter| match exporter {
                logs::OtelLogsExporter::None => false,
                logs::OtelLogsExporter::Otlp => false,
                logs::OtelLogsExporter::Console => true,
                logs::OtelLogsExporter::OtlpAndConsole => true,
            })
            .unwrap_or(true)
    }

    /// Begin constructing a tracing subscriber based on this configuration.
    #[inline]
    #[must_use = "the builder does nothing if not used"]
    pub fn subscriber(&self) -> TracingSubscriberBuilder<'_> {
        TracingSubscriberBuilder::new(self)
    }

    pub(crate) fn console_filter(&self) -> EnvFilterSource<'_> {
        let dirs = self
            .rust_log
            .as_ref()
            .map(subscriber::FilterDirectives::dirs);

        EnvFilterSource {
            dirs,
            src: subscriber::RUST_LOG,
        }
    }

    pub(crate) fn otlp_traces_filter(&self) -> EnvFilterSource<'_> {
        let dirs = self
            .otlp_traces_env_filter
            .as_ref()
            .map(subscriber::FilterDirectives::dirs);

        EnvFilterSource {
            dirs,
            src: subscriber::OTEL_RUST_OTLP_TRACES_FILTER,
        }
    }

    pub(crate) fn otlp_logs_filter(&self) -> EnvFilterSource<'_> {
        let dirs = self
            .otlp_logs_env_filter
            .as_ref()
            .map(subscriber::FilterDirectives::dirs);

        EnvFilterSource {
            dirs,
            src: subscriber::OTEL_RUST_OTLP_LOGS_FILTER,
        }
    }
}

#[cfg(test)]
mod tests;
