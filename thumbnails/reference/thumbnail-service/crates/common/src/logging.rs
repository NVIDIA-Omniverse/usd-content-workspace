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

//! Shared `tracing` subscriber init for `server` and `task-worker`.
//!
//! Both binaries call [`init_tracing`] so log format and level cannot drift
//! between processes. Logging is handled by [`otel_tools`]'s `LoggingLayer`,
//! which writes structured JSON to stdout and embeds `trace_id`/`span_id` from
//! the active OTel span — enabling log-to-trace correlation in Grafana without
//! any OTLP log export or secondary delivery path.
//!
//! Expected config: `OTEL_LOGS_EXPORTER=console` (or unset, which defaults to
//! console). This keeps logs on stdout where the filelog DaemonSet collects
//! them; `OTEL_LOGS_EXPORTER=otlp` would push logs via OTLP instead and is
//! not the intended deployment mode for this service.
//!
//! OTLP span and metric export is opt-in via the standard `OTEL_*` environment
//! variables. `OTEL_TRACES_EXPORTER=none` / `OTEL_METRICS_EXPORTER=none`
//! disables OTLP for those signals without touching console logging.
//! `OTEL_SDK_DISABLED=true` is a full kill switch that also suppresses stdout
//! output — avoid it when disabling only OTLP.

use otel_tools::{
    exporters::{logs::LogsConfig, metrics::MetricsConfig, traces::TracesConfig},
    resource::ResourceBuilder,
    tracing::TracingConfig,
};
use tracing_subscriber::EnvFilter;

/// `info` as a *fallback* rather than an added directive. A bare `info` passed
/// to `add_directive` sets the global default, which is the same slot
/// `RUST_LOG=debug` populates, so adding it would discard the operator's level.
pub fn env_log_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

/// Keeps the OTLP exporters alive for the lifetime of the process.
///
/// The guards flush and shut down their exporters on drop, so this value
/// must be bound to a variable that lives until the end of `main`.
#[must_use = "OTel guards must be held until process exit to flush telemetry"]
pub struct OtelGuards {
    _traces: otel_tools::exporters::traces::TracesGuard,
    _metrics: otel_tools::exporters::metrics::MetricsGuard,
    _logs: otel_tools::exporters::logs::LogsGuard,
}

/// Install the process-global tracing subscriber and initialize OTLP export.
///
/// Call once near process start, after dotenv (if any) so chart-injected and
/// local `.env` knobs are visible. Bind the returned [`OtelGuards`] to a
/// variable that lives until the end of `main` — dropping it early flushes
/// and shuts down the exporters.
///
/// Log format: always JSON via [`otel_tools`]'s `LoggingLayer`. Each log line
/// includes `trace_id` and `span_id` when an OTel span is active, enabling
/// log-to-trace correlation in Grafana (Loki derived field → Tempo).
///
/// OTLP export is controlled by the standard `OTEL_*` env vars:
/// - `OTEL_EXPORTER_OTLP_ENDPOINT` — enables OTLP for spans and metrics
/// - `OTEL_LOGS_EXPORTER=console` (or unset) — logs stay on stdout
/// - `RUST_LOG` — level filter; defaults to `info`
pub fn init_tracing(service_name: &'static str) -> OtelGuards {
    let resource = ResourceBuilder::new(service_name)
        .with_service_namespace("omniverse")
        .with_service_version(env!("CARGO_PKG_VERSION"))
        .build();

    let traces_guard = TracesConfig::from_env()
        .guard()
        .with_resource(&resource)
        .build();

    let logs_guard = LogsConfig::from_env()
        .guard()
        .with_resource(&resource)
        .build();

    let metrics_guard = MetricsConfig::from_env()
        .guard()
        .with_resource(&resource)
        .build();

    // Expose the SDK meter provider globally so any crate can call
    // `opentelemetry::global::meter("name")` to obtain instruments.
    if let Some(provider) = metrics_guard.provider() {
        opentelemetry::global::set_meter_provider(provider.clone());
    }

    // Register the W3C TraceContext propagator so incoming `traceparent`
    // headers are honoured by all propagation helpers (gRPC, HTTP).
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    // `TracingConfig` reads `RUST_LOG` for the console filter and
    // `OTEL_LOGS_EXPORTER` to decide whether to enable console (stdout)
    // logging. With `OTEL_LOGS_EXPORTER=console` (or unset), the
    // `LoggingLayer` writes JSON to stdout; the `OpenTelemetryTracingBridge`
    // (OTLP log export) is NOT activated — avoiding duplication with the
    // filelog DaemonSet. `trace_id`/`span_id` are embedded by `LoggingLayer`
    // reading the OTel span context set by the traces layer.
    TracingConfig::from_env()
        .subscriber()
        .with_default_env_filter("info")
        .with_resource(&resource)
        .with_traces(&traces_guard)
        .with_logs(&logs_guard)
        .build(service_name);

    OtelGuards {
        _traces: traces_guard,
        _metrics: metrics_guard,
        _logs: logs_guard,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    /// Restores `RUST_LOG` when the test ends, including on panic.
    struct ScopedEnv {
        rust_log: Option<OsString>,
    }

    impl ScopedEnv {
        fn set_rust_log(value: Option<&str>) -> Self {
            let previous = std::env::var_os("RUST_LOG");
            match value {
                Some(v) => std::env::set_var("RUST_LOG", v),
                None => std::env::remove_var("RUST_LOG"),
            }
            Self { rust_log: previous }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            match self.rust_log.take() {
                Some(prev) => std::env::set_var("RUST_LOG", prev),
                None => std::env::remove_var("RUST_LOG"),
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn invalid_rust_log_falls_back_to_info_without_aborting() {
        let _restore = ScopedEnv::set_rust_log(Some("=$%not a filter"));
        let directives = env_log_filter().to_string();
        assert!(
            directives == "info" || directives.contains("info"),
            "invalid RUST_LOG must fall back to info; got {directives}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn env_filter_honors_rust_log_debug_directive() {
        let _restore = ScopedEnv::set_rust_log(Some("debug"));
        let directives = env_log_filter().to_string();
        assert!(
            directives.contains("debug"),
            "filter should reflect RUST_LOG=debug; got directives: {directives}"
        );
    }
}
