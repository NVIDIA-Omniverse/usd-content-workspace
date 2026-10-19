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

use super::common::{
    OtelExporterOtlpProtocol, OtelExporterOtlpType, debug_transparent, otel_exporter_otlp_endpoint,
};
use super::{OtelEnvVal, Var, env_vars, var_parse};
use http::uri::Uri;

env_vars! {
    /// https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/#exporter-selection
    OTEL_LOGS_EXPORTER,
    /// https://opentelemetry.io/docs/languages/sdk-configuration/otlp-exporter/#otel_exporter_otlp_logs_protocol
    OTEL_EXPORTER_OTLP_LOGS_PROTOCOL,
    /// https://opentelemetry.io/docs/languages/sdk-configuration/otlp-exporter/#otel_exporter_otlp_logs_endpoint
    OTEL_EXPORTER_OTLP_LOGS_ENDPOINT,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OtelLogsExporter {
    None,
    Otlp,
    Console,
    #[default]
    OtlpAndConsole,
}

impl OtelEnvVal for OtelLogsExporter {
    type Err = &'static str;

    fn from_env(s: &str) -> Result<Self, Self::Err> {
        match s {
            "otlp" => Ok(Self::Otlp),
            "console" => Ok(Self::Console),
            "otlp,console" => Ok(Self::OtlpAndConsole),
            "console,otlp" => Ok(Self::OtlpAndConsole),
            "none" => Ok(Self::None),
            _ => Err("expected `otlp`, `console`, `otlp,console` or `none`"),
        }
    }
}

impl OtelLogsExporter {
    pub(crate) fn get() -> Option<Self> {
        var_parse!(OTEL_LOGS_EXPORTER)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub(crate) struct OtelExporterOtlpLogsProtocol(pub(crate) OtelExporterOtlpProtocol);

debug_transparent!(OtelExporterOtlpLogsProtocol);

impl OtelExporterOtlpLogsProtocol {
    pub(crate) fn get() -> Option<Self> {
        var_parse!(OTEL_EXPORTER_OTLP_LOGS_PROTOCOL)
            .or_else(OtelExporterOtlpProtocol::get)
            .map(Self)
    }
}

#[derive(Clone, PartialEq, Eq)]
#[repr(transparent)]
pub(crate) struct OtelExporterOtlpLogsEndpoint(pub(crate) String);

debug_transparent!(OtelExporterOtlpLogsEndpoint);

fn otel_exporter_otlp_logs_endpoint_uri(
    var: Var,
    protocol: OtelExporterOtlpProtocol,
) -> Option<String> {
    match protocol {
        OtelExporterOtlpProtocol::Grpc => {
            // opentelemetry-otlp doesn't parse this as a Uri
            // https://github.com/open-telemetry/opentelemetry-rust/blob/opentelemetry-otlp-0.27.0/opentelemetry-otlp/src/exporter/tonic/mod.rs#L221
            Some(var.into_inner())
        }
        OtelExporterOtlpProtocol::HttpJson | OtelExporterOtlpProtocol::HttpProtobuf => {
            // https://github.com/open-telemetry/opentelemetry-rust/blob/opentelemetry-otlp-0.27.0/opentelemetry-otlp/src/exporter/http/mod.rs#L365
            var.parse().map(|uri: Uri| uri.to_string())
        }
    }
}

impl OtelExporterOtlpLogsEndpoint {
    pub(crate) fn get(protocol: OtelExporterOtlpProtocol) -> Option<Self> {
        Var::get(OTEL_EXPORTER_OTLP_LOGS_ENDPOINT)
            .and_then(|var| otel_exporter_otlp_logs_endpoint_uri(var, protocol))
            .or_else(|| otel_exporter_otlp_endpoint(protocol, OtelExporterOtlpType::Logs))
            .map(Self)
    }
}

#[cfg(test)]
mod tests;
