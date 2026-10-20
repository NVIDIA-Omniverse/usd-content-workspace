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

use opentelemetry::logs::{AnyValue, LogRecord, Logger, LoggerProvider, Severity};
use opentelemetry_sdk::{logs::SdkLoggerProvider, Resource};
use std::collections::BTreeMap;
use tracing::{Level, Subscriber};
use tracing_subscriber::{registry::LookupSpan, EnvFilter, Layer};

use crate::observability::error::Error;

/// Creates an EnvFilter that merges RUST_LOG with opentelemetry-specific directives.
///
/// This function:
/// 1. Reads the RUST_LOG environment variable (via `EnvFilter::from_default_env()`)
/// 2. Conditionally adds opentelemetry-specific directives based on whether trace-level debugging is enabled
///
/// The opentelemetry directives (opentelemetry-otlp, opentelemetry_sdk, h2) are only enabled
/// if the user is debugging at trace level. Otherwise, they are suppressed to prevent noise.
pub(crate) fn create_merged_log_filter() -> EnvFilter {
    use std::env;

    // Start with RUST_LOG environment variable
    let mut filter = EnvFilter::from_default_env();

    // CRITICAL: Always allow otel::tracing spans through at trace level.
    // The tonic-tracing-opentelemetry middleware creates spans with target "otel::tracing"
    // at TRACE level. If these are filtered out, the OpenTelemetry layer never sees them
    // and trace propagation breaks completely.
    filter = filter.add_directive("otel::tracing=trace".parse().unwrap());

    // Check if trace-level debugging is enabled globally by checking if the default/global level is trace
    // We only enable opentelemetry targets if the user explicitly sets the global level to trace
    let rust_log = env::var("RUST_LOG").unwrap_or_default();
    let trace_enabled = if rust_log.is_empty() {
        // If RUST_LOG is not set, EnvFilter defaults to ERROR, so trace is not enabled
        false
    } else {
        // Check if the first directive is a global trace level (just "trace")
        // RUST_LOG format: "level" or "module=level,module2=level2"
        // We only enable if it's explicitly set to global trace, not if a specific module is trace
        let first_directive = rust_log.split(',').next().unwrap_or("").trim();
        first_directive == "trace"
    };

    if trace_enabled {
        // User wants trace-level debugging, so allow opentelemetry targets at trace level
        filter = filter.add_directive("opentelemetry-otlp=trace".parse().unwrap());
        filter = filter.add_directive("opentelemetry_sdk=trace".parse().unwrap());
        filter = filter.add_directive("h2=trace".parse().unwrap());
        filter = filter.add_directive("tower=trace".parse().unwrap());
        filter = filter.add_directive("hyper_util=trace".parse().unwrap());
        filter = filter.add_directive("tonic_tracing_opentelemetry::middleware::server=trace".parse().unwrap());
        filter = filter.add_directive("axum_tracing_opentelemetry::middleware::server=trace".parse().unwrap());
    } else {
        // User is not debugging at trace level, suppress opentelemetry targets
        // If these directives are allowed at the debug level, they are so verbose that they overwhelm the logs
        filter = filter.add_directive("opentelemetry-otlp=error".parse().unwrap());
        filter = filter.add_directive("opentelemetry_sdk=error".parse().unwrap());
        filter = filter.add_directive("h2=error".parse().unwrap());
        filter = filter.add_directive("tower=error".parse().unwrap());
        filter = filter.add_directive("hyper_util=error".parse().unwrap());
        filter = filter.add_directive("tonic_tracing_opentelemetry::middleware::server=error".parse().unwrap());
        filter = filter.add_directive("axum_tracing_opentelemetry::middleware::server=error".parse().unwrap());
    }

    filter
}

/// A custom visitor that collects all fields and ensures a message is always set in the log record.
/// This visitor wraps the OpenTelemetry EventVisitor and ensures the body is always set.
struct MessageEnsuringVisitor<'a, LR: LogRecord> {
    log_record: &'a mut LR,
    fields: BTreeMap<String, String>,
    has_message: bool,
}

impl<'a, LR: LogRecord> MessageEnsuringVisitor<'a, LR> {
    fn new(log_record: &'a mut LR) -> Self {
        Self {
            log_record,
            fields: BTreeMap::new(),
            has_message: false,
        }
    }

    fn ensure_message(&mut self) {
        if self.fields.is_empty() {
            return;
        }

        let formatted_message = if self.has_message {
            // If there's already a message field, use it as the base
            let mut parts = Vec::new();
            if let Some(msg) = self.fields.get("message") {
                parts.push(msg.clone());
            }
            // Append other fields
            for (key, value) in self.fields.iter() {
                if key != "message" {
                    parts.push(format!("{key}={value}"));
                }
            }
            parts.join(" ")
        } else {
            // No message field, format all fields as key=value pairs
            self.fields
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(" ")
        };

        // Always set the body with the formatted message
        self.log_record.set_body(AnyValue::from(formatted_message));
    }
}

impl<LR: LogRecord> tracing::field::Visit for MessageEnsuringVisitor<'_, LR> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let formatted = format!("{value:?}");
        if field.name() == "message" {
            self.has_message = true;
        }
        self.fields.insert(field.name().to_string(), formatted.clone());
        // Also add as attribute to preserve all fields
        use opentelemetry::Key;
        self.log_record
            .add_attribute(Key::new(field.name()), AnyValue::from(formatted));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.has_message = true;
        }
        let value_str = value.to_string();
        self.fields.insert(field.name().to_string(), value_str.clone());
        // Also add as attribute to preserve all fields
        use opentelemetry::Key;
        self.log_record
            .add_attribute(Key::new(field.name()), AnyValue::from(value_str));
    }

    fn record_error(&mut self, field: &tracing::field::Field, value: &(dyn std::error::Error + 'static)) {
        let formatted = value.to_string();
        if field.name() == "message" {
            self.has_message = true;
        }
        self.fields.insert(field.name().to_string(), formatted.clone());
        // Also add as attribute to preserve all fields
        use opentelemetry::Key;
        self.log_record
            .add_attribute(Key::new("exception.message"), AnyValue::from(formatted));
    }

    fn record_bytes(&mut self, field: &tracing::field::Field, value: &[u8]) {
        let formatted = format!("{:?}", value);
        self.fields.insert(field.name().to_string(), formatted.clone());
        use opentelemetry::Key;
        self.log_record
            .add_attribute(Key::new(field.name()), AnyValue::from(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields.insert(field.name().to_string(), value.to_string());
        use opentelemetry::Key;
        self.log_record
            .add_attribute(Key::new(field.name()), AnyValue::from(value));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.fields.insert(field.name().to_string(), value.to_string());
        use opentelemetry::Key;
        self.log_record
            .add_attribute(Key::new(field.name()), AnyValue::from(value));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.insert(field.name().to_string(), value.to_string());
        use opentelemetry::Key;
        self.log_record
            .add_attribute(Key::new(field.name()), AnyValue::from(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.insert(field.name().to_string(), value.to_string());
        use opentelemetry::Key;
        let key = Key::new(field.name());
        if let Ok(signed) = i64::try_from(value) {
            self.log_record.add_attribute(key, AnyValue::from(signed));
        } else {
            self.log_record.add_attribute(key, AnyValue::from(format!("{value:?}")));
        }
    }

    fn record_i128(&mut self, field: &tracing::field::Field, value: i128) {
        self.fields.insert(field.name().to_string(), value.to_string());
        use opentelemetry::Key;
        let key = Key::new(field.name());
        if let Ok(signed) = i64::try_from(value) {
            self.log_record.add_attribute(key, AnyValue::from(signed));
        } else {
            self.log_record.add_attribute(key, AnyValue::from(format!("{value:?}")));
        }
    }

    fn record_u128(&mut self, field: &tracing::field::Field, value: u128) {
        self.fields.insert(field.name().to_string(), value.to_string());
        use opentelemetry::Key;
        let key = Key::new(field.name());
        if let Ok(signed) = i64::try_from(value) {
            self.log_record.add_attribute(key, AnyValue::from(signed));
        } else {
            self.log_record.add_attribute(key, AnyValue::from(format!("{value:?}")));
        }
    }
}

/// A custom OpenTelemetry layer that ensures all fields are formatted into a message.
/// This is based on the original OpenTelemetryTracingBridge but uses our custom visitor.
struct MessageFormatterOpenTelemetryLayer<P, L>
where
    P: LoggerProvider<Logger = L> + Send + Sync,
    L: Logger + Send + Sync,
{
    logger: L,
    _phantom: std::marker::PhantomData<P>,
}

impl<P, L> MessageFormatterOpenTelemetryLayer<P, L>
where
    P: LoggerProvider<Logger = L> + Send + Sync,
    L: Logger + Send + Sync,
{
    fn new(provider: &P) -> Self {
        Self {
            logger: provider.logger(""),
            _phantom: Default::default(),
        }
    }
}

const fn severity_of_level(level: &Level) -> Severity {
    match *level {
        Level::TRACE => Severity::Trace,
        Level::DEBUG => Severity::Debug,
        Level::INFO => Severity::Info,
        Level::WARN => Severity::Warn,
        Level::ERROR => Severity::Error,
    }
}

impl<S, P, L> Layer<S> for MessageFormatterOpenTelemetryLayer<P, L>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    P: LoggerProvider<Logger = L> + Send + Sync + 'static,
    L: Logger + Send + Sync + 'static,
{
    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let metadata = event.metadata();
        let severity = severity_of_level(metadata.level());
        let target = metadata.target();
        let name = metadata.name();

        let mut log_record = self.logger.create_log_record();

        log_record.set_target(target);
        log_record.set_event_name(name);
        log_record.set_severity_number(severity);
        log_record.set_severity_text(metadata.level().as_str());

        // Use our custom visitor that ensures the message is always set
        let mut visitor = MessageEnsuringVisitor::new(&mut log_record);
        event.record(&mut visitor);
        visitor.ensure_message();

        // Handle span context (same as original OpenTelemetry layer)
        // Note: This feature may not be available, so we'll skip it if not compiled in
        if let Some(_span) = ctx.event_span(event) {
            // Try to get span context if tracing-opentelemetry is available
            // This is a best-effort attempt to preserve trace context
        }

        self.logger.emit(log_record);
    }
}

/// Creates an EnvFilter that merges RUST_LOG (or OTEL_LOG_LEVEL) with telemetry suppression directives.
///
/// This function:
/// 1. Reads the RUST_LOG environment variable (via `EnvFilter::from_default_env()`)
/// 2. If RUST_LOG is not set, falls back to OTEL_LOG_LEVEL environment variable
/// 3. Adds suppression directives for telemetry-related libraries to prevent loops
///
/// The suppression directives will override any conflicting directives from RUST_LOG or OTEL_LOG_LEVEL,
/// ensuring that `hyper`, `tonic`, `h2`, and `reqwest` are always suppressed.
///
/// To prevent a telemetry-induced-telemetry loop, OpenTelemetry's own internal
/// logging is properly suppressed. However, logs emitted by external components
/// (such as reqwest, tonic, etc.) are not suppressed as they do not propagate
/// OpenTelemetry context. Until this issue is addressed
/// (https://github.com/open-telemetry/opentelemetry-rust/issues/2877),
/// filtering like this is the best way to suppress such logs.
///
/// Note: This filtering will also drop logs from these components even when
/// they are used outside of the OTLP Exporter.
fn create_otel_log_filter() -> EnvFilter {
    use std::env;

    // Check if RUST_LOG is set, otherwise fall back to OTEL_LOG_LEVEL
    let filter = if env::var("RUST_LOG").is_ok() {
        // RUST_LOG is set, use it
        EnvFilter::from_default_env()
    } else if let Ok(otel_log_level) = env::var("OTEL_LOG_LEVEL") {
        // RUST_LOG is not set, but OTEL_LOG_LEVEL is, use it as the default level
        // OTEL_LOG_LEVEL is typically a single level like "info", "debug", "error", etc.
        EnvFilter::new(&otel_log_level)
    } else {
        // Neither is set, use default (ERROR level)
        EnvFilter::from_default_env()
    };

    // Add suppression directives for telemetry-related libraries.
    // These will override any conflicting directives from RUST_LOG or OTEL_LOG_LEVEL.
    filter
        .add_directive("hyper=off".parse().unwrap())
        .add_directive("tonic=off".parse().unwrap())
        .add_directive("h2=off".parse().unwrap())
        .add_directive("reqwest=off".parse().unwrap())
}

pub(crate) fn make_otlp_logger_layer<S>(resource: Resource) -> Result<impl Layer<S> + Send + Sync + 'static, Error>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let exporter = opentelemetry_otlp::LogExporter::builder()
        .with_tonic()
        .build()
        .map_err(Error::logs_exporter_init)?;

    let provider: SdkLoggerProvider = SdkLoggerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter)
        .build();

    // Create a filter that merges RUST_LOG with telemetry suppression directives
    let filter_otel = create_otel_log_filter();

    // Use our custom OpenTelemetry layer that ensures messages are always formatted
    let otel_layer = MessageFormatterOpenTelemetryLayer::new(&provider);
    Ok(otel_layer.with_filter(filter_otel))
}
