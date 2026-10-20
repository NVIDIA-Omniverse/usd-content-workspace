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

use super::{OtelEnvVal, Var, env_vars, var_parse};
use http::uri::Uri;

macro_rules! debug_transparent {
    ($ty:ty) => {
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Debug::fmt(&self.0, f)
            }
        }
    };
}
pub(crate) use debug_transparent;

env_vars! {
    /// https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/#general-sdk-configuration
    OTEL_SDK_DISABLED,
    /// https://opentelemetry.io/docs/languages/sdk-configuration/otlp-exporter/#otel_exporter_otlp_protocol
    OTEL_EXPORTER_OTLP_PROTOCOL,
    /// https://opentelemetry.io/docs/languages/sdk-configuration/otlp-exporter/#otel_exporter_otlp_endpoint
    OTEL_EXPORTER_OTLP_ENDPOINT,
}

pub(crate) fn otel_sdk_disabled() -> Option<bool> {
    var_parse!(OTEL_SDK_DISABLED)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OtelExporterOtlpProtocol {
    /// OTLP/gRPC
    #[default]
    Grpc,
    /// OTLP/HTTP + JSON
    HttpJson,
    /// OTLP/HTTP + protobuf
    HttpProtobuf,
}

impl OtelEnvVal for OtelExporterOtlpProtocol {
    type Err = &'static str;

    fn from_env(s: &str) -> Result<Self, Self::Err> {
        match s {
            "grpc" => Ok(Self::Grpc),
            "http/json" => Ok(Self::HttpJson),
            "http/protobuf" => Ok(Self::HttpProtobuf),
            _ => Err("expected `grpc`, `http/json`, or `http/protobuf`"),
        }
    }
}

impl OtelExporterOtlpProtocol {
    pub(crate) fn get() -> Option<Self> {
        var_parse!(OTEL_EXPORTER_OTLP_PROTOCOL)
    }

    /// Convert the settings type to the SDK type.
    pub(crate) fn sdk(self) -> opentelemetry_otlp::Protocol {
        match self {
            Self::Grpc => opentelemetry_otlp::Protocol::Grpc,
            Self::HttpJson => opentelemetry_otlp::Protocol::HttpJson,
            Self::HttpProtobuf => opentelemetry_otlp::Protocol::HttpBinary,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OtelExporterOtlpType {
    Traces,
    Metrics,
    Logs,
}

/// https://github.com/open-telemetry/opentelemetry-specification/blob/v1.41.0/specification/protocol/exporter.md#endpoint-urls-for-otlphttp
fn http_endpoint_uri(endpoint: &str, ty: OtelExporterOtlpType) -> String {
    let path = match ty {
        OtelExporterOtlpType::Traces => "/v1/traces",
        OtelExporterOtlpType::Metrics => "/v1/metrics",
        OtelExporterOtlpType::Logs => "/v1/logs",
    };

    let path = endpoint
        .ends_with('/')
        .then(|| path.strip_prefix('/'))
        .flatten()
        .unwrap_or(path);
    format!("{endpoint}{path}")
}

pub(crate) fn otel_exporter_otlp_endpoint(
    protocol: OtelExporterOtlpProtocol,
    ty: OtelExporterOtlpType,
) -> Option<String> {
    let endpoint = Var::get(OTEL_EXPORTER_OTLP_ENDPOINT);

    match protocol {
        OtelExporterOtlpProtocol::Grpc => {
            // opentelemetry-otlp doesn't parse this as a Uri
            // https://github.com/open-telemetry/opentelemetry-rust/blob/opentelemetry-otlp-0.27.0/opentelemetry-otlp/src/exporter/tonic/mod.rs#L221
            endpoint.map(Var::into_inner)
        }
        OtelExporterOtlpProtocol::HttpJson | OtelExporterOtlpProtocol::HttpProtobuf => {
            // https://github.com/open-telemetry/opentelemetry-rust/blob/opentelemetry-otlp-0.27.0/opentelemetry-otlp/src/exporter/http/mod.rs#L365
            endpoint
                .map(|var| var.modify(|endpoint| http_endpoint_uri(&endpoint, ty)))
                .and_then(|v| v.parse())
                .map(|uri: Uri| uri.to_string())
        }
    }
}

#[cfg(test)]
mod tests;
