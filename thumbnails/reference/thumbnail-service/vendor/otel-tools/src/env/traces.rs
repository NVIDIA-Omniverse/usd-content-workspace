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
    OTEL_TRACES_EXPORTER,
    /// https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/#general-sdk-configuration
    OTEL_PROPAGATORS,
    /// https://opentelemetry.io/docs/languages/sdk-configuration/otlp-exporter/#otel_exporter_otlp_traces_protocol
    OTEL_EXPORTER_OTLP_TRACES_PROTOCOL,
    /// https://opentelemetry.io/docs/languages/sdk-configuration/otlp-exporter/#otel_exporter_otlp_traces_endpoint
    OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
    ///https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/#span-limits
    OTEL_EVENT_ATTRIBUTE_COUNT_LIMIT,
    ///https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/#span-limits
    OTEL_LINK_ATTRIBUTE_COUNT_LIMIT,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OtelTracesExporter {
    None,
    #[default]
    Otlp,
}

impl OtelEnvVal for OtelTracesExporter {
    type Err = &'static str;

    fn from_env(s: &str) -> Result<Self, Self::Err> {
        match s {
            "otlp" => Ok(Self::Otlp),
            "none" => Ok(Self::None),
            _ => Err("expected `otlp` or `none`"),
        }
    }
}

impl OtelTracesExporter {
    pub(crate) fn get() -> Option<Self> {
        var_parse!(OTEL_TRACES_EXPORTER)
    }
}

/// A dummy structure to validate `OTEL_PROPAGATORS`.
///
/// This is not an enum, there could be multiple propagators enabled. (Only
/// W3C trace context is supported.)
///
/// TODO: does this belong in the traces OTLP exporter setup?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct OtelPropagators {
    trace_context: bool,
}

impl OtelEnvVal for OtelPropagators {
    type Err = &'static str;

    fn from_env(s: &str) -> Result<Self, Self::Err> {
        if s == "tracecontext" {
            Ok(OtelPropagators {
                trace_context: true,
            })
        } else {
            Err("expected `tracecontext` only")
        }
    }
}

impl OtelPropagators {
    pub(crate) fn get() -> Option<Self> {
        var_parse!(OTEL_PROPAGATORS)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub(crate) struct OtelExporterOtlpTracesProtocol(pub(crate) OtelExporterOtlpProtocol);

debug_transparent!(OtelExporterOtlpTracesProtocol);

impl OtelExporterOtlpTracesProtocol {
    pub(crate) fn get() -> Option<Self> {
        var_parse!(OTEL_EXPORTER_OTLP_TRACES_PROTOCOL)
            .or_else(OtelExporterOtlpProtocol::get)
            .map(Self)
    }
}

#[derive(Clone, PartialEq, Eq)]
#[repr(transparent)]
pub(crate) struct OtelExporterOtlpTracesEndpoint(pub(crate) String);

debug_transparent!(OtelExporterOtlpTracesEndpoint);

fn otel_exporter_otlp_traces_endpoint_uri(
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

impl OtelExporterOtlpTracesEndpoint {
    pub(crate) fn get(protocol: OtelExporterOtlpProtocol) -> Option<Self> {
        Var::get(OTEL_EXPORTER_OTLP_TRACES_ENDPOINT)
            .and_then(|var| otel_exporter_otlp_traces_endpoint_uri(var, protocol))
            .or_else(|| otel_exporter_otlp_endpoint(protocol, OtelExporterOtlpType::Traces))
            .map(Self)
    }
}

pub(crate) fn otel_event_attribute_count_limit() -> Option<u32> {
    var_parse!(OTEL_EVENT_ATTRIBUTE_COUNT_LIMIT)
}

pub(crate) fn otel_link_attribute_count_limit() -> Option<u32> {
    var_parse!(OTEL_LINK_ATTRIBUTE_COUNT_LIMIT)
}

#[cfg(test)]
mod tests;
