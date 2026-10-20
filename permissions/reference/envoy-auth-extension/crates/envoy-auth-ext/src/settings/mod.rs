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

mod parse;

pub use parse::parse_config;
use permission_auth::config::AuthConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TonicSettings {
    pub initial_connection_window_size: u32,
    pub initial_stream_window_size: u32,
    pub max_decoding_message_size: u32,
}

impl Default for TonicSettings {
    fn default() -> Self {
        Self {
            initial_connection_window_size: 65_535,
            initial_stream_window_size: 65_535,
            max_decoding_message_size: 4 * 1024 * 1024,
        }
    }
}

const DEFAULT_IP: &str = "0.0.0.0";
const DEFAULT_PORT: u16 = 50051;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ServerSettings {
    pub ip_address: String,
    pub port: u16,
    pub tonic: TonicSettings,
    /// The claims in the token payload tried in order to forward upstream as the
    /// username. The first claim present in the token wins.
    /// Defaults to ["preferred_username"].
    pub username_claims: Vec<String>,
    /// The header forwarded upstream carrying the user identifier (`sub` claim).
    /// Defaults to "user-id".
    pub user_id_header: String,
    /// The header forwarded upstream carrying the resolved username claim.
    /// Defaults to "username".
    pub username_header: String,
    /// The header forwarded upstream carrying the name of the claim that
    /// supplied the username. Defaults to "username-source-claim".
    pub username_source_claim_header: String,
}

impl Default for ServerSettings {
    fn default() -> Self {
        ServerSettings {
            ip_address: DEFAULT_IP.to_string(),
            port: DEFAULT_PORT,
            tonic: TonicSettings::default(),
            username_claims: vec!["preferred_username".to_string()],
            user_id_header: "user-id".to_string(),
            username_header: "username".to_string(),
            username_source_claim_header: "username-source-claim".to_string(),
        }
    }
}

/// Logging settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LoggingSettings {
    /// The log level filter, using `tracing_subscriber::EnvFilter` directive
    /// syntax, e.g. "info" or "info,envoy_auth_ext=debug".
    pub level: String,
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
        }
    }
}

//FIXME: These OTEL settings are mostly ignored due to use of new telemetry crate

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct OtelSettings {
    /// Custom Otel resource attributes.
    ///
    /// Precedence (lower precedence keys will be overwritten!)
    /// * Settings (this)
    /// * `OTEL_RESOURCE_ATTRIBUTES` env var
    /// * Key: `telemetry.sdk.name`, `telemetry.sdk.language`, and
    ///   `telemetry.sdk.version` from SDK
    /// * Key: `service.name` from `OTEL_SERVICE_NAME` env var
    pub resource: Option<HashMap<String, String>>,
    /// Otel tracing settings.
    pub tracing: TracingSettings,
    /// Otel metrics settings.
    pub metrics: MetricsSettings,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct TracingSettings {
    /// The tracing exporter endpoint.
    ///
    /// Precedence:
    /// * `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` env var
    /// * `OTEL_EXPORTER_OTLP_ENDPOINT` env var
    /// * Settings (this)
    /// * Default from SDK (depends on protocol)
    ///   * gRPC: "http://localhost:4317"
    ///   * HTTP: "http://localhost:4318"
    pub endpoint: Option<String>,
    /// The tracing exporter protocol.
    ///
    /// Precedence:
    /// * Settings (this)
    /// * `OTEL_EXPORTER_OTLP_TRACES_PROTOCOL` env var
    /// * `OTEL_EXPORTER_OTLP_PROTOCOL` env var
    /// * Default from setup (gRPC)
    pub protocol: Option<OtlpProtocol>,
    /// The tracing exporter timeout in milliseconds.
    ///
    /// Precedence:
    /// * Settings (this)
    /// * `OTEL_EXPORTER_OTLP_TRACES_TIMEOUT` env var
    /// * `OTEL_EXPORTER_OTLP_TIMEOUT` env var
    /// * Default from SDK (10 seconds)
    pub exporter_timeout_ms: Option<u64>,
    /// The tracing sampler.
    ///
    /// Precedence:
    /// * Settings (this)
    /// * `OTEL_TRACES_SAMPLER` and `OTEL_TRACES_SAMPLER_ARG` env var
    /// * Default from SDK (ParentBased AlwaysOn)
    pub sampler: Option<TracingSampler>,
    /// Span limit configuration.
    pub span_limits: Option<SpanLimits>,
}

/// See [opentelemetry_sdk::trace::Sampler].
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub enum TracingSampler {
    /// See [opentelemetry_sdk::trace::Sampler::AlwaysOn].
    AlwaysOn,
    /// See [opentelemetry_sdk::trace::Sampler::AlwaysOff].
    AlwaysOff,
    /// See [opentelemetry_sdk::trace::Sampler::TraceIdRatioBased].
    TraceIdRatio(f64),
    /// See [opentelemetry_sdk::trace::Sampler::ParentBased] and [opentelemetry_sdk::trace::Sampler::AlwaysOn].
    ParentBasedAlwaysOn,
    /// See [opentelemetry_sdk::trace::Sampler::ParentBased] and [opentelemetry_sdk::trace::Sampler::AlwaysOff].
    ParentBasedAlwaysOff,
    /// See [opentelemetry_sdk::trace::Sampler::ParentBased] and [opentelemetry_sdk::trace::Sampler::TraceIdRatioBased].
    ParentBasedTraceIdRatio(f64),
}

impl TracingSampler {
    pub fn sdk(self) -> opentelemetry_sdk::trace::Sampler {
        match self {
            Self::AlwaysOn => opentelemetry_sdk::trace::Sampler::AlwaysOn,
            Self::AlwaysOff => opentelemetry_sdk::trace::Sampler::AlwaysOff,
            Self::TraceIdRatio(v) => opentelemetry_sdk::trace::Sampler::TraceIdRatioBased(v),
            Self::ParentBasedAlwaysOn => opentelemetry_sdk::trace::Sampler::ParentBased(Box::new(
                opentelemetry_sdk::trace::Sampler::AlwaysOn,
            )),
            Self::ParentBasedAlwaysOff => opentelemetry_sdk::trace::Sampler::ParentBased(Box::new(
                opentelemetry_sdk::trace::Sampler::AlwaysOff,
            )),
            Self::ParentBasedTraceIdRatio(v) => opentelemetry_sdk::trace::Sampler::ParentBased(
                Box::new(opentelemetry_sdk::trace::Sampler::TraceIdRatioBased(v)),
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct SpanLimits {
    /// The max events that can be added to a `Span`.
    ///
    /// Precedence:
    /// * Settings (this)
    /// * `OTEL_SPAN_EVENT_COUNT_LIMIT` env var
    /// * Default from SDK (128)
    pub max_events_per_span: Option<u32>,
    /// The max attributes that can be added to a `Span`.
    ///
    /// Precedence:
    /// * Settings (this)
    /// * `OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT` env var
    /// * Default from SDK (128)
    pub max_attributes_per_span: Option<u32>,
    /// The max links that can be added to a `Span`.
    ///
    /// Precedence:
    /// * Settings (this)
    /// * `OTEL_SPAN_LINK_COUNT_LIMIT` env var
    /// * Default from SDK (128)
    pub max_links_per_span: Option<u32>,
    /// The max attributes that can be added into an `Event`.
    ///
    /// Precedence:
    /// * Settings (this)
    /// * Default from SDK (128)
    pub max_attributes_per_event: Option<u32>,
    /// The max attributes that can be added into a `Link`.
    ///
    /// Precedence:
    /// * Settings (this)
    /// * Default from SDK (128)
    pub max_attributes_per_link: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MetricsSettings {
    /// The metrics exporter endpoint.
    ///
    /// Precedence:
    /// * `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` env var
    /// * `OTEL_EXPORTER_OTLP_ENDPOINT` env var
    /// * Settings (this)
    /// * Default from SDK (depends on protocol)
    ///   * gRPC: "http://localhost:4317"
    ///   * HTTP: "http://localhost:4318"
    pub endpoint: Option<String>,
    /// The metrics exporter protocol.
    ///
    /// Precedence:
    /// * Settings (this)
    /// * `OTEL_EXPORTER_OTLP_METRICS_PROTOCOL` env var
    /// * `OTEL_EXPORTER_OTLP_PROTOCOL` env var
    /// * Default from setup (gRPC)
    pub protocol: Option<OtlpProtocol>,
    /// The metrics exporter temporality.
    ///
    /// Precedence:
    /// * Settings (this)
    /// * Default from SDK (Cumulative)
    pub temporality: Option<MetricsTemporality>,
    /// The metrics exporter timeout in milliseconds.
    ///
    /// Precedence:
    /// * Settings (this)
    /// * `OTEL_EXPORTER_OTLP_METRICS_TIMEOUT` env var
    /// * `OTEL_EXPORTER_OTLP_TIMEOUT` env var
    /// * Default from SDK (10 seconds)
    pub exporter_timeout_ms: Option<u64>,
    /// The metric periodic reader export timeout.
    ///
    /// Precedence:
    /// * Settings (this)
    /// * `OTEL_METRIC_EXPORT_TIMEOUT` env var
    /// * Default from SDK (30 seconds)
    pub export_timeout_ms: Option<u64>,
    /// The metric periodic reader export interval.
    ///
    /// Precedence:
    /// * Settings (this)
    /// * `OTEL_METRIC_EXPORT_INTERVAL` env var
    /// * Default from SDK (60 seconds)
    pub export_interval_ms: Option<u64>,
}

/// See [opentelemetry_otlp::Protocol].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum OtlpProtocol {
    /// See [opentelemetry_otlp::Protocol::Grpc].
    Grpc,
    /// See [opentelemetry_otlp::Protocol::HttpJson].
    HttpJson,
    /// See [opentelemetry_otlp::Protocol::HttpBinary].
    HttpBinary,
}

impl OtlpProtocol {
    pub fn sdk(self) -> opentelemetry_otlp::Protocol {
        match self {
            Self::Grpc => opentelemetry_otlp::Protocol::Grpc,
            Self::HttpJson => opentelemetry_otlp::Protocol::HttpJson,
            Self::HttpBinary => opentelemetry_otlp::Protocol::HttpBinary,
        }
    }
}

/// See [opentelemetry_sdk::metrics::Temporality].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum MetricsTemporality {
    /// See [opentelemetry_sdk::metrics::Temporality::Cumulative].
    Cumulative,
    /// See [opentelemetry_sdk::metrics::Temporality::Delta].
    Delta,
    /// See [opentelemetry_sdk::metrics::Temporality::LowMemory].
    LowMemory,
}

impl MetricsTemporality {
    pub fn sdk(self) -> opentelemetry_sdk::metrics::Temporality {
        match self {
            Self::Cumulative => opentelemetry_sdk::metrics::Temporality::Cumulative,
            Self::Delta => opentelemetry_sdk::metrics::Temporality::Delta,
            Self::LowMemory => opentelemetry_sdk::metrics::Temporality::LowMemory,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct IdpClientSettings {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub pool_idle_timeout: Duration,
}

impl Default for IdpClientSettings {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(15),
            pool_idle_timeout: Duration::from_secs(90),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    /// Basic server settings
    pub server: ServerSettings,
    /// Logging settings
    pub logging: LoggingSettings,
    /// OpenTelemetry settings
    pub otel: OtelSettings,
    /// Authentication settings
    pub auth: AuthConfig,
    pub idp_client: IdpClientSettings,
}
