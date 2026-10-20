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

use super::build::TracesGuardBuilder;
use crate::env::{common, traces};

/// The OTLP traces exporter configuration.
///
/// The configuration may be printed to show basic traces exporter settings
/// for debugging purposes. Note that the endpoint is a best guess.
///
/// Not to be confused with the tracing subscriber
/// [`TracingConfig`](crate::tracing::TracingConfig).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TracesConfig {
    sdk_disabled: Option<bool>,
    propagators: Option<traces::OtelPropagators>,
    exporter: Option<traces::OtelTracesExporter>,
    protocol: Option<traces::OtelExporterOtlpTracesProtocol>,
    endpoint: Option<traces::OtelExporterOtlpTracesEndpoint>,
}

impl TracesConfig {
    /// Construct the traces exporter configuration from environment variables.
    pub fn from_env() -> Self {
        let sdk_disabled = common::otel_sdk_disabled();
        let propagators = traces::OtelPropagators::get();
        let exporter = traces::OtelTracesExporter::get();
        let protocol = traces::OtelExporterOtlpTracesProtocol::get();
        let endpoint =
            traces::OtelExporterOtlpTracesEndpoint::get(protocol.map(|p| p.0).unwrap_or_default());

        Self {
            sdk_disabled,
            propagators,
            exporter,
            protocol,
            endpoint,
        }
    }

    /// Construct the traces exporter configuration with a completely disabled
    /// state. Equivalent to setting `OTEL_SDK_DISABLED`.
    #[inline]
    pub fn disabled_for_test() -> Self {
        Self {
            sdk_disabled: Some(true),
            propagators: None,
            exporter: None,
            protocol: None,
            endpoint: None,
        }
    }

    pub(crate) fn protocol(&self) -> Option<opentelemetry_otlp::Protocol> {
        self.protocol.map(|p| p.0.sdk())
    }

    /// `true` if OTLP traces export is disabled.
    pub(crate) fn is_disabled(&self) -> bool {
        self.sdk_disabled == Some(true) || self.exporter == Some(traces::OtelTracesExporter::None)
    }

    /// Being constructing a [`TracesGuard`](super::guard::TracesGuard) based on
    /// this configuration.
    #[inline]
    pub fn guard(&self) -> TracesGuardBuilder<'_> {
        TracesGuardBuilder::new(self)
    }
}

#[cfg(test)]
mod tests;
