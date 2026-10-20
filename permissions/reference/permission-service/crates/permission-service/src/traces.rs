// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use crate::access_log::ACCESS_TARGET;
use crate::config::ServiceConfig;
use clap::{ArgAction, Args};
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
use serde_json::{json, Value};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_log::AsLog;
use tracing_subscriber::filter::{filter_fn, FilterExt};
use tracing_subscriber::layer::Context;
use tracing_subscriber::{EnvFilter, Layer};

#[derive(Args, Clone, Debug)]
pub struct TracingArgs {
    /// Enables structured (JSON) logging of trace spans (disabled by default).
    #[arg(long,
        env = "STRUCTURED_LOGGING",
        action = ArgAction::Set,
        default_value_t = false,
        default_missing_value = "false",
        num_args = 0..=1,
        require_equals = false
    )]
    pub structured: bool,

    /// GRPC Exporter for trace spans.
    #[arg(long, env = "TRACES_ENDPOINT", default_value = "")]
    pub endpoint: String,

    /// Defines the sampling ratio for traces sent to the collector
    /// (the percentage of traces that would be sent).
    /// Must be between 0 and 1.
    #[arg(long, env = "TRACE_SAMPLING_RATIO", default_value = "1.0")]
    pub trace_sampling_ratio: f64,
}

pub fn configure_tracing(config: &ServiceConfig) -> Tracing {
    let mut layers = vec![];
    layers.push(Layer::boxed(EnvFilter::from_default_env()));

    let fmt_layer = tracing_subscriber::fmt::layer();
    let formatter = if config.tracing.structured {
        // The JSON formatter would stringify the nested access envelope, so the
        // dedicated `AccessLogLayer` renders those events instead and the fmt
        // layer skips target `access` to avoid a second stringified line.
        fmt_layer
            .json()
            .with_filter(
                EnvFilter::from_default_env().and(filter_fn(|meta| meta.target() != ACCESS_TARGET)),
            )
            .boxed()
    } else {
        // Pretty mode renders access events through the fmt formatter like any
        // other `INFO` line (the emit helper writes them as a plain message), so
        // they share the same timestamp, coloring, and `at` location.
        fmt_layer
            .pretty()
            .with_filter(EnvFilter::from_default_env())
            .boxed()
    };
    layers.push(formatter);

    if config.tracing.structured && config.access_log.enabled {
        let access_layer = AccessLogLayer
            .with_filter(
                EnvFilter::from_default_env().and(filter_fn(|meta| meta.target() == ACCESS_TARGET)),
            )
            .boxed();
        layers.push(access_layer);
    }

    let mut tracer_provider: Option<SdkTracerProvider> = None;
    if !config.tracing.endpoint.is_empty() {
        let layer = {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(&config.tracing.endpoint)
                .build()
                .unwrap();

            opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
            let sampler = Sampler::TraceIdRatioBased(config.tracing.trace_sampling_ratio);

            let provider = SdkTracerProvider::builder()
                .with_resource(config.opentelemetry_resource())
                .with_sampler(sampler)
                .with_batch_exporter(exporter)
                .build();
            let tracer = provider.tracer("permission_service");

            tracer_provider = Some(provider);
            tracing_opentelemetry::layer()
                .with_tracer(tracer)
                .with_filter(filter_fn(|meta| meta.target() != ACCESS_TARGET))
                .boxed()
        };
        layers.push(layer);
    }

    use tracing_subscriber::layer::SubscriberExt as _;
    let subscriber = tracing_subscriber::Registry::default().with(layers);
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let max_level = tracing_core::LevelFilter::current().as_log();
    tracing_log::LogTracer::builder()
        // Note that we must call this *after* setting the global default
        // subscriber, so that we get its max level hint.
        .with_max_level(max_level)
        .init()
        .expect("Failed to set global tracing logger.");

    Tracing {
        provider: tracer_provider,
    }
}

/// Renders access-log events as the nested `{ "type": "access", "access": { ... } }`
/// envelope on stdout. Installed only when structured logging and access logs are
/// both enabled; in pretty mode the fmt layer records these events, which the emit
/// helper writes as a plain message so it renders like any other `INFO` line.
struct AccessLogLayer;

impl<S: Subscriber> Layer<S> for AccessLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() != ACCESS_TARGET {
            return;
        }

        let mut visitor = AccessFieldVisitor::default();
        event.record(&mut visitor);
        let Some(access) = visitor.access else {
            return;
        };
        let access: Value = serde_json::from_str(&access).unwrap_or(Value::Null);

        let timestamp = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();

        let envelope = json!({
            "timestamp": timestamp,
            "level": event.metadata().level().as_str(),
            "type": "access",
            "access": access,
        });

        if let Ok(line) = serde_json::to_string(&envelope) {
            println!("{}", line);
        }
    }
}

/// Captures the single `access` field carrying the compact JSON envelope emitted
/// by the access-log helper.
#[derive(Default)]
struct AccessFieldVisitor {
    access: Option<String>,
}

impl Visit for AccessFieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "access" {
            self.access = Some(value.to_string());
        }
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn std::fmt::Debug) {}
}

pub struct Tracing {
    provider: Option<SdkTracerProvider>,
}

impl Drop for Tracing {
    fn drop(&mut self) {
        if let Some(provider) = &self.provider {
            provider
                .shutdown()
                .expect("Failed to shutdown the tracer provider.");
        }
    }
}
