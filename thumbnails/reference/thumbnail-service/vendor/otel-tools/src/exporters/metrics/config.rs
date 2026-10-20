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

use super::build::MetricsGuardBuilder;
use crate::env::{common, metrics};

/// The OTLP metrics exporter configuration.
///
/// The configuration may be printed to show basic metrics exporter settings
/// for debugging purposes. Note that the endpoint is a best guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricsConfig {
    sdk_disabled: Option<bool>,
    exporter: Option<metrics::OtelMetricsExporter>,
    protocol: Option<metrics::OtelExporterOtlpMetricsProtocol>,
    endpoint: Option<metrics::OtelExporterOtlpMetricsEndpoint>,
}

impl MetricsConfig {
    /// Construct the metrics exporter configuration from environment variables.
    pub fn from_env() -> Self {
        let sdk_disabled = common::otel_sdk_disabled();
        let exporter = metrics::OtelMetricsExporter::get();
        let protocol = metrics::OtelExporterOtlpMetricsProtocol::get();
        let endpoint = metrics::OtelExporterOtlpMetricsEndpoint::get(
            protocol.map(|p| p.0).unwrap_or_default(),
        );

        Self {
            sdk_disabled,
            exporter,
            protocol,
            endpoint,
        }
    }

    /// Construct the metrics exporter configuration with a completely disabled
    /// state. Equivalent to setting `OTEL_SDK_DISABLED`.
    #[inline]
    pub fn disabled_for_test() -> Self {
        Self {
            sdk_disabled: Some(true),
            exporter: None,
            protocol: None,
            endpoint: None,
        }
    }

    pub(crate) fn protocol(&self) -> Option<opentelemetry_otlp::Protocol> {
        self.protocol.map(|p| p.0.sdk())
    }

    /// `true` if OTLP metrics export is disabled.
    pub(crate) fn is_disabled(&self) -> bool {
        self.sdk_disabled == Some(true) || self.exporter == Some(metrics::OtelMetricsExporter::None)
    }

    /// Being constructing a [`MetricsGuard`](super::guard::MetricsGuard) based
    /// on this configuration.
    #[inline]
    pub fn guard(&self) -> MetricsGuardBuilder<'_> {
        MetricsGuardBuilder::new(self)
    }
}

#[cfg(test)]
mod tests;
