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

use opentelemetry::propagation::{Extractor, Injector};
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::Layer;
use tracing_subscriber::Registry;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;

use crate::config::Config;

pub struct OtelProviders {
    pub tracer: Option<SdkTracerProvider>,
    pub meter: Option<SdkMeterProvider>,
}

/// Initializes the tracing subscriber and, when enabled, the OTLP trace and/or
/// metric exporters. Traces and metrics can be toggled independently. Returns
/// the providers (if created) so the caller can shut them down on exit.
pub fn init_telemetry(config: &Config) -> Option<OtelProviders> {
    let otel = &config.otel;
    let fmt_layers = fmt_layers(config.structured_logging);

    if !otel.traces_enabled && !otel.metrics_enabled {
        tracing_subscriber::registry()
            .with(fmt_layers)
            .with(tracing_subscriber::EnvFilter::new(&config.rust_log))
            .init();
        return None;
    }

    let resource = opentelemetry_sdk::Resource::builder_empty()
        .with_attributes([opentelemetry::KeyValue::new(
            "service.name",
            otel.service_name.clone(),
        )])
        .build();

    let tracer_provider = if otel.traces_enabled {
        let span_exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&otel.traces_endpoint)
            .build()
            .expect("failed to create OTLP span exporter");

        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(span_exporter)
            .with_resource(resource.clone())
            .build();

        opentelemetry::global::set_tracer_provider(provider.clone());
        opentelemetry::global::set_text_map_propagator(
            opentelemetry_sdk::propagation::TraceContextPropagator::new(),
        );
        Some(provider)
    } else {
        None
    };

    let meter_provider = if otel.metrics_enabled {
        let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_endpoint(&otel.metrics_endpoint)
            .build()
            .expect("failed to create OTLP metric exporter");

        let metric_reader =
            opentelemetry_sdk::metrics::PeriodicReader::builder(metric_exporter).build();

        let provider = SdkMeterProvider::builder()
            .with_resource(resource)
            .with_reader(metric_reader)
            .build();

        opentelemetry::global::set_meter_provider(provider.clone());
        Some(provider)
    } else {
        None
    };

    let otel_layer = tracer_provider.as_ref().map(|tp| {
        tracing_opentelemetry::layer().with_tracer(tp.tracer("microsoft-graph-userinfo-service"))
    });

    tracing_subscriber::registry()
        .with(fmt_layers)
        .with(tracing_subscriber::EnvFilter::new(&config.rust_log))
        .with(otel_layer)
        .init();

    Some(OtelProviders {
        tracer: tracer_provider,
        meter: meter_provider,
    })
}

const ACCESS_TARGET: &str = "access";

/// Builds the stdout formatting layers. Operational logs use the text or JSON
/// formatter selected by `structured`. Access-log events are split onto a
/// dedicated formatter: in JSON mode the `access` field is rendered as a nested
/// object with `type`/`access` at the root (the standard JSON layer would
/// serialize it as an escaped string); in text mode the standard event header
/// is kept for identical coloring while the `access` object is flattened into
/// one indented `key.path: value` line per leaf.
fn fmt_layers(structured: bool) -> Vec<Box<dyn Layer<Registry> + Send + Sync>> {
    if structured {
        vec![
            Box::new(
                tracing_subscriber::fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_filter(filter_fn(|meta| meta.target() != ACCESS_TARGET)),
            ),
            Box::new(
                tracing_subscriber::fmt::layer()
                    .event_format(AccessLogFormat)
                    .with_filter(filter_fn(|meta| meta.target() == ACCESS_TARGET)),
            ),
        ]
    } else {
        vec![
            Box::new(
                tracing_subscriber::fmt::layer()
                    .with_filter(filter_fn(|meta| meta.target() != ACCESS_TARGET)),
            ),
            Box::new(
                tracing_subscriber::fmt::layer()
                    .fmt_fields(AccessTextFields)
                    .with_filter(filter_fn(|meta| meta.target() == ACCESS_TARGET)),
            ),
        ]
    }
}

/// Event formatter for access-log records under structured logging. Renders one
/// JSON object per line with `type` and `access` at the root, embedding the
/// `access` field verbatim so it stays a nested object rather than a string.
struct AccessLogFormat;

impl<S, N> FormatEvent<S, N> for AccessLogFormat
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let mut visitor = AccessFieldVisitor::default();
        event.record(&mut visitor);
        let access = visitor
            .access
            .and_then(|json| serde_json::value::RawValue::from_string(json).ok())
            .unwrap_or_else(|| {
                serde_json::value::RawValue::from_string("null".to_owned()).unwrap()
            });

        #[derive(serde::Serialize)]
        struct Record<'a> {
            timestamp: String,
            level: &'a str,
            target: &'a str,
            #[serde(rename = "type")]
            kind: &'a str,
            access: &'a serde_json::value::RawValue,
        }

        let record = Record {
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
            level: event.metadata().level().as_str(),
            target: event.metadata().target(),
            kind: "access",
            access: access.as_ref(),
        };
        let line = serde_json::to_string(&record).map_err(|_| std::fmt::Error)?;
        writeln!(writer, "{line}")
    }
}

#[derive(Default)]
struct AccessFieldVisitor {
    access: Option<String>,
}

impl tracing::field::Visit for AccessFieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "access" {
            self.access = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "access" {
            self.access = Some(format!("{value:?}"));
        }
    }
}

/// Field formatter for access-log records under text logging. Keeps the
/// standard event header (timestamp, level, target) rendered by the default
/// formatter, and renders the `access` object as one indented
/// `key.path: value` line per leaf, honoring the writer's ANSI setting so the
/// output matches the coloring of other text logs.
struct AccessTextFields;

impl<'writer> FormatFields<'writer> for AccessTextFields {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        mut writer: Writer<'writer>,
        fields: R,
    ) -> std::fmt::Result {
        let mut visitor = AccessFieldVisitor::default();
        fields.record(&mut visitor);
        let access = visitor
            .access
            .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
            .unwrap_or(serde_json::Value::Null);

        let mut leaves = Vec::new();
        flatten_access(String::new(), &access, &mut leaves);
        let ansi = writer.has_ansi_escapes();
        for (path, value) in leaves {
            if ansi {
                write!(writer, "\n    \u{1b}[1m{path}\u{1b}[0m: {value}")?;
            } else {
                write!(writer, "\n    {path}: {value}")?;
            }
        }
        Ok(())
    }
}

/// Flattens a JSON value into `(dotted.path, rendered value)` leaves in key
/// order. Scalars render without JSON quoting; nested arrays render as compact
/// JSON.
fn flatten_access(prefix: String, value: &serde_json::Value, leaves: &mut Vec<(String, String)>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_access(path, val, leaves);
            }
        }
        serde_json::Value::String(text) => leaves.push((prefix, text.clone())),
        serde_json::Value::Null => leaves.push((prefix, "null".to_owned())),
        other => leaves.push((prefix, other.to_string())),
    }
}

/// Extracts W3C trace context from incoming HTTP headers.
pub fn extract_context(headers: &http::HeaderMap) -> opentelemetry::Context {
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&HttpHeaderExtractor(headers))
    })
}

/// Extracts W3C trace context from gRPC metadata.
pub fn extract_grpc_context(metadata: &tonic::metadata::MetadataMap) -> opentelemetry::Context {
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&MetadataExtractor(metadata))
    })
}

/// Creates a root span for a gRPC request with propagated parent context.
pub fn grpc_span(metadata: &tonic::metadata::MetadataMap, method: &str) -> tracing::Span {
    let span = tracing::info_span!(
        "grpc.request",
        rpc.method = %method,
        rpc.system = "grpc",
        otel.kind = "server",
    );
    let _ = span.set_parent(extract_grpc_context(metadata));
    span
}

/// Injects the current span's trace context into a reqwest `HeaderMap`.
pub fn inject_trace_context(headers: &mut reqwest::header::HeaderMap) {
    let cx = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut ReqwestHeaderInjector(headers));
    });
}

/// Returns the current span's trace ID formatted as a UUID, or a random v4
/// UUID when no trace is active. Suitable for the `client-request-id` header
/// expected by Microsoft Graph.
pub fn client_request_id() -> String {
    use opentelemetry::trace::TraceContextExt;
    let cx = tracing::Span::current().context();
    let bytes = cx.span().span_context().trace_id().to_bytes();
    if bytes != [0u8; 16] {
        uuid::Uuid::from_bytes(bytes).to_string()
    } else {
        uuid::Uuid::new_v4().to_string()
    }
}

struct HttpHeaderExtractor<'a>(&'a http::HeaderMap);

impl Extractor for HttpHeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

struct MetadataExtractor<'a>(&'a tonic::metadata::MetadataMap);

impl Extractor for MetadataExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .filter_map(|k| match k {
                tonic::metadata::KeyRef::Ascii(k) => Some(k.as_str()),
                _ => None,
            })
            .collect()
    }
}

struct ReqwestHeaderInjector<'a>(&'a mut reqwest::header::HeaderMap);

impl Injector for ReqwestHeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let Ok(name) = reqwest::header::HeaderName::from_bytes(key.as_bytes())
            && let Ok(val) = reqwest::header::HeaderValue::from_str(&value)
        {
            self.0.insert(name, val);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedBuffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedBuffer {
        type Writer = SharedBuffer;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn access_event_renders_nested_object_at_root() {
        let buffer = SharedBuffer::default();
        let layer = tracing_subscriber::fmt::layer()
            .event_format(AccessLogFormat)
            .with_writer(buffer.clone())
            .with_filter(filter_fn(|meta| meta.target() == ACCESS_TARGET));
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            let access = serde_json::json!({
                "operation": "list-users",
                "request": { "params": { "limit": "20" } },
                "response": { "status": 200 },
            });
            tracing::info!(target: ACCESS_TARGET, r#type = "access", access = %access);
        });

        let output = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        let line = output.lines().next().expect("one access line");
        let parsed: serde_json::Value = serde_json::from_str(line).expect("valid JSON line");

        assert_eq!(parsed["type"], "access");
        assert_eq!(parsed["level"], "INFO");
        assert_eq!(parsed["target"], "access");
        assert!(
            parsed["access"].is_object(),
            "access must be a nested object"
        );
        assert_eq!(parsed["access"]["operation"], "list-users");
        assert_eq!(parsed["access"]["request"]["params"]["limit"], "20");
        assert_eq!(parsed["access"]["response"]["status"], 200);
    }

    #[test]
    fn access_event_text_mode_flattens_fields_in_order() {
        let buffer = SharedBuffer::default();
        let layer = tracing_subscriber::fmt::layer()
            .fmt_fields(AccessTextFields)
            .with_ansi(false)
            .with_writer(buffer.clone())
            .with_filter(filter_fn(|meta| meta.target() == ACCESS_TARGET));
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            let access = serde_json::json!({
                "transport": "REST",
                "operation": "GET /v1beta/policies/",
                "request": { "params": { "scope": "exact", "page": 1, "limit": 25 } },
                "response": { "status": 200, "detail": { "item_count": 3 } },
            });
            tracing::info!(target: ACCESS_TARGET, r#type = "access", access = %access);
        });

        let output = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();

        assert!(output.contains("INFO access:"), "keeps standard header");
        assert!(!output.contains('{'), "must not dump raw JSON");

        let fields = output.split("access:").nth(1).expect("field section");
        let expected = "\n    transport: REST\
             \n    operation: GET /v1beta/policies/\
             \n    request.params.scope: exact\
             \n    request.params.page: 1\
             \n    request.params.limit: 25\
             \n    response.status: 200\
             \n    response.detail.item_count: 3";
        assert!(
            fields.contains(expected),
            "flattened field block in order, got: {fields:?}"
        );
    }
}
