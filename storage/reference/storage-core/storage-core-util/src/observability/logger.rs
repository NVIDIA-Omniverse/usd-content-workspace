// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{error_limited, info_limited, warn_limited};
use opentelemetry::logs::{AnyValue, LogRecord, Logger, LoggerProvider};
use opentelemetry::trace::{SpanId, TraceId};
use opentelemetry_otlp::WithTonicConfig;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::Resource;
use std::env;
use tracing::{Level, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_opentelemetry::OtelData;
use tracing_subscriber::filter::{filter_fn, Directive, FilterFn, LevelFilter};
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, FormattedFields};
use tracing_subscriber::{layer::SubscriberExt, registry::LookupSpan, EnvFilter, Layer};

/// Creates an EnvFilter that merges RUST_LOG with opentelemetry-specific directives.
///
/// This function:
/// 1. Reads the RUST_LOG environment variable (via `EnvFilter::from_default_env()`)
/// 2. Conditionally adds opentelemetry-specific directives based on whether trace-level debugging is enabled
///
/// The opentelemetry directives (opentelemetry-otlp, opentelemetry_sdk, h2) are only enabled
/// if the user is debugging at trace level. Otherwise, they are suppressed to prevent noise.
/// Azure SDK targets that can emit the Authorization header (RUSTSEC-2026-0275).
///
/// `azure_core` is the one that logs the header; the others share its `Headers`
/// `Debug` impl, whose redaction predicate is inverted (it prints `authorization`
/// and redacts everything else), so they are capped together.
const AZURE_SDK_TARGETS: &[&str] = &["azure_core", "azure_identity", "azure_storage", "azure_storage_blobs"];

/// Targets named by a well-formed directive in `rust_log`.
///
/// Only directives that `parse()` count: `EnvFilter::from_default_env` silently drops
/// malformed ones, so a typo must not be read as an opt-in for a directive that
/// contributes nothing to the filter.
fn azure_targets_named_in(rust_log: &str) -> Vec<&str> {
    rust_log
        .split(',')
        .map(str::trim)
        .filter(|directive| directive.parse::<Directive>().is_ok())
        // A directive may omit the level (`azure_core` means that target at trace), in
        // which case the whole directive is the target. A bare level such as `debug`
        // lands here too, but never matches one of the SDK target names.
        .map(|directive| directive.split_once('=').map_or(directive, |(target, _)| target.trim()))
        .filter(|target| !target.is_empty())
        .collect()
}

/// Whether the global (target-less) level in `rust_log` is more verbose than `info`.
///
/// Only bare level directives count as global; `azure_core=debug` sets a target, not
/// the global level. An unset or level-less `RUST_LOG` leaves `EnvFilter` at its ERROR
/// default, which is already quieter than the cap.
fn global_level_is_more_verbose_than_info(rust_log: &str) -> bool {
    rust_log
        .split(',')
        .map(str::trim)
        .filter(|directive| !directive.contains('='))
        .filter_map(|directive| directive.parse::<LevelFilter>().ok())
        .any(|level| level > LevelFilter::INFO)
}

/// The enforcing half of the RUSTSEC-2026-0275 compensating control.
///
/// Rejects DEBUG/TRACE events from the Azure SDK targets outright, regardless of how
/// they were enabled and with no opt-out. Two distinct ways exist to get around a mere
/// directive, and this closes both:
///
/// - `EnvFilter` checks dynamic span directives before target directives, so
///   `RUST_LOG='info,[Get Properties]=debug'` enables
///   `azure_core::policies::transport` debug events without naming an Azure target.
/// - `EnvFilter` picks the most specific matching directive, and `add_directive` only
///   replaces one whose target matches exactly, so
///   `RUST_LOG=azure_core::policies::transport=debug` outranks `azure_core=info`.
///
/// A target directive cannot veto either; a layer filter can, because subscriber layer
/// filters are ANDed together. It also covers the OTLP exporter, whose own per-layer
/// filter (`create_otel_log_filter`) is evaluated separately -- otherwise a credential
/// could be suppressed on stdout and still shipped off-box.
///
/// There is deliberately **no opt-in**. `azure_core` 0.21's `Debug for Headers` has an
/// inverted redaction predicate: it prints `authorization` verbatim and redacts every
/// other header. So "let me debug the Azure SDK" and "log the credential" are the same
/// request, and the credential lands in a retained, searchable log store. `info` and
/// above still pass, so the SDK is not silenced entirely.
///
/// Delete alongside the deny.toml ignore when the azure_core >= 1.0 migration lands.
pub(crate) fn azure_credential_log_guard() -> FilterFn<impl Fn(&tracing::Metadata<'_>) -> bool + Clone> {
    filter_fn(move |metadata: &tracing::Metadata<'_>| {
        if *metadata.level() < Level::DEBUG {
            return true;
        }
        let target = metadata.target();
        !AZURE_SDK_TARGETS
            .iter()
            .any(|sdk| target == *sdk || target.starts_with(&format!("{sdk}::")))
    })
}

pub(crate) fn create_merged_log_filter() -> EnvFilter {
    use std::env;

    // Start with RUST_LOG environment variable
    let mut filter = EnvFilter::from_default_env();

    // CRITICAL: Always allow otel::tracing spans through at trace level.
    // The tonic-tracing-opentelemetry middleware creates spans with target "otel::tracing"
    // at TRACE level. If these are filtered out, the OpenTelemetry layer never sees them
    // and trace propagation breaks completely.
    filter = filter.add_directive("otel::tracing=trace".parse().unwrap());

    let rust_log = env::var("RUST_LOG").unwrap_or_default();

    // Part of the compensating control for RUSTSEC-2026-0275 (see deny.toml):
    // azure_core 0.21 writes the Authorization header value into its request logs at
    // debug level. This caps the Azure SDK targets at `info` so a broad
    // `RUST_LOG=debug` cannot surface the header.
    //
    // A directive alone is NOT sufficient — see `azure_credential_log_guard`, which is
    // the part that actually holds. `EnvFilter` evaluates dynamic span directives before
    // target directives, so `RUST_LOG='info,[Get Properties]=debug'` would otherwise
    // re-enable these events without naming an Azure target at all. The directive here
    // keeps the common case cheap (rejected at callsite registration); the guard layer
    // is the backstop.
    //
    // Only added when the global level is more verbose than `info`. `EnvFilter`
    // directives are absolute, not maxima: the most specific matching directive wins
    // outright, so `azure_core=info` is a floor as well as a ceiling. Adding it under
    // `RUST_LOG=warn` — or unset, where `EnvFilter` defaults to ERROR — would *raise*
    // these targets and surface INFO/WARN events that were previously filtered out.
    // Below `debug` the targets are already quieter than the cap, so it is not needed.
    //
    // Lifted only when the operator names that exact SDK target in a directive that
    // parses. `EnvFilter::from_default_env` silently drops malformed directives, so a
    // typo like `azure_core=dbug` must not count as an opt-in. A narrower
    // `azure_core::policies=trace` does not lift the SDK-wide cap either: `EnvFilter`
    // specificity already honours the child directive with the parent in place.
    if global_level_is_more_verbose_than_info(&rust_log) {
        let named = azure_targets_named_in(&rust_log);
        for target in AZURE_SDK_TARGETS {
            if !named.contains(target) {
                filter = filter.add_directive(format!("{target}=info").parse().unwrap());
            }
        }
    }

    // Check if trace-level debugging is enabled globally by checking if the default/global level is trace
    // We only enable opentelemetry targets if the user explicitly sets the global level to trace
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
        filter = filter.add_directive(
            "axum_tracing_opentelemetry::middleware::trace_extractor=trace"
                .parse()
                .unwrap(),
        );
    } else {
        // User is not debugging at trace level, suppress opentelemetry targets
        // If these directives are allowed at the debug level, they are so verbose that they overwhelm the logs
        filter = filter.add_directive("opentelemetry-otlp=error".parse().unwrap());
        filter = filter.add_directive("opentelemetry_sdk=error".parse().unwrap());
        filter = filter.add_directive("h2=error".parse().unwrap());
        filter = filter.add_directive("tower=error".parse().unwrap());
        filter = filter.add_directive("hyper_util=error".parse().unwrap());
        filter = filter.add_directive("tonic_tracing_opentelemetry::middleware::server=error".parse().unwrap());
        filter = filter.add_directive(
            "axum_tracing_opentelemetry::middleware::trace_extractor=error"
                .parse()
                .unwrap(),
        );
    }

    filter
}

/// Create a basic subscriber that logs output as text.
///
/// This is used as a temporary subscriber in startup to capture logging outputs that occur before the real handler gets
/// installed.
pub(crate) fn make_text_subscriber(
) -> Result<impl tracing::Subscriber + Send + Sync + 'static, Box<dyn std::error::Error + Send + Sync + 'static>> {
    let text_layer = tracing_subscriber::fmt::layer()
        .with_ansi(cfg!(debug_assertions))
        .event_format(OtelFormat {
            timer: tracing_subscriber::fmt::time::ChronoLocal::new("%Y-%m-%dT%H:%M:%S%.6f%z".to_string()),
        });
    let filter = create_merged_log_filter();
    Ok(tracing_subscriber::registry()
        .with(text_layer)
        .with(filter)
        .with(azure_credential_log_guard()))
}

/// Custom log format that integrates OpenTelemetry trace and span IDs into the log output.
///
/// Produces output matching the built-in `Full` format, but with trace context inserted after
/// the log level:
///
/// ```text
/// {timestamp} {LEVEL} [{trace_id}/{span_id}] {span_chain}: {file}:{line}: {message}
/// ```
///
/// When no OTel context is available (e.g. during startup), the `[trace_id/span_id]` section
/// is omitted and the output is identical to the standard `Full` format.
struct OtelFormat<T> {
    timer: T,
}

impl<S, N, T> FormatEvent<S, N> for OtelFormat<T>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    T: FormatTime,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: tracing_subscriber::fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let meta = event.metadata();
        let ansi = writer.has_ansi_escapes();

        // Timestamp
        write_timestamp(&self.timer, ansi, &mut writer)?;

        // Level
        write_level(meta.level(), ansi, &mut writer)?;
        writer.write_char(' ')?;

        // Thread ID
        let thread_id = std::thread::current().id();
        if ansi {
            write!(writer, "\x1b[2m{thread_id:?}\x1b[0m ")?;
        } else {
            write!(writer, "{thread_id:?} ")?;
        }

        // Trace/Span IDs (only when OTel context is available)
        if let Some((trace_id, span_id)) = find_otel_ids(ctx) {
            if ansi {
                write!(writer, "\x1b[2m{{trace_id={trace_id}, span_id={span_id}}}\x1b[0m ")?;
            } else {
                write!(writer, "{{trace_id={trace_id}, span_id={span_id}}} ")?;
            }
        }

        // Span chain (root to leaf)
        if let Some(scope) = ctx.event_scope() {
            let mut seen = false;
            for span in scope.from_root() {
                // Skip OTel middleware spans entirely — their name (e.g. "GRPC request")
                // is redundant with the trace context we already display, and their fields
                // are verbose semantic convention attributes already in the OTel trace.
                if span.metadata().target() == "otel::tracing" {
                    continue;
                }
                seen = true;
                if ansi {
                    write!(writer, "\x1b[1m{}\x1b[0m", span.metadata().name())?;
                } else {
                    write!(writer, "{}", span.metadata().name())?;
                }
                let ext = span.extensions();
                if let Some(fields) = &ext.get::<FormattedFields<N>>() {
                    if !fields.is_empty() {
                        if ansi {
                            write!(writer, "\x1b[1m{{\x1b[0m{fields}\x1b[1m}}\x1b[0m")?;
                        } else {
                            write!(writer, "{{{fields}}}")?;
                        }
                    }
                }
                if ansi {
                    write!(writer, "\x1b[2m:\x1b[0m")?;
                } else {
                    writer.write_char(':')?;
                }
            }
            if seen {
                writer.write_char(' ')?;
            }
        }

        // File:line
        let line_number = meta.line();
        if let Some(filename) = meta.file() {
            if ansi {
                write!(writer, "\x1b[2m{filename}")?;
            } else {
                write!(writer, "{filename}")?;
            }
            if let Some(line) = line_number {
                write!(writer, ":{line}")?;
            }
            if ansi {
                write!(writer, ":\x1b[0m ")?;
            } else {
                write!(writer, ": ")?;
            }
        } else if let Some(line) = line_number {
            if ansi {
                write!(writer, "\x1b[2m{line}:\x1b[0m ")?;
            } else {
                write!(writer, "{line}: ")?;
            }
        }

        // Event fields (message, key=value pairs)
        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// Walks the span ancestry of the given event looking for OpenTelemetry trace and span IDs.
///
/// Returns the first valid (non-zero) trace ID and span ID found in the span's [`OtelData`]
/// extensions, starting from the event's immediate span and walking towards the root.
fn find_otel_ids<S, N>(ctx: &FmtContext<'_, S, N>) -> Option<(TraceId, SpanId)>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    for span in ctx.event_scope()? {
        let ext = span.extensions();
        if let Some(otel_data) = ext.get::<OtelData>() {
            if let (Some(trace_id), Some(span_id)) = (otel_data.trace_id(), otel_data.span_id()) {
                if trace_id != TraceId::INVALID && span_id != SpanId::INVALID {
                    return Some((trace_id, span_id));
                }
            }
        }
    }
    None
}

fn write_timestamp<T: FormatTime>(
    timer: &T,
    ansi: bool,
    writer: &mut tracing_subscriber::fmt::format::Writer<'_>,
) -> std::fmt::Result {
    if ansi {
        writer.write_str("\x1b[2m")?;
        if timer.format_time(writer).is_err() {
            writer.write_str("<unknown time>")?;
        }
        write!(writer, "\x1b[0m ")
    } else {
        if timer.format_time(writer).is_err() {
            writer.write_str("<unknown time>")?;
        }
        writer.write_char(' ')
    }
}

fn write_level(
    level: &tracing::Level,
    ansi: bool,
    writer: &mut tracing_subscriber::fmt::format::Writer<'_>,
) -> std::fmt::Result {
    if ansi {
        let (color, name) = match *level {
            Level::TRACE => ("\x1b[35m", "TRACE"),
            Level::DEBUG => ("\x1b[34m", "DEBUG"),
            Level::INFO => ("\x1b[32m", " INFO"),
            Level::WARN => ("\x1b[33m", " WARN"),
            Level::ERROR => ("\x1b[31m", "ERROR"),
        };
        write!(writer, "{color}{name}\x1b[0m")
    } else {
        let name = match *level {
            Level::TRACE => "TRACE",
            Level::DEBUG => "DEBUG",
            Level::INFO => " INFO",
            Level::WARN => " WARN",
            Level::ERROR => "ERROR",
        };
        writer.write_str(name)
    }
}

pub(crate) fn make_stdout_logger_layer<S>() -> (Box<dyn Layer<S> + Send + Sync + 'static>, Option<WorkerGuard>)
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let (non_blocking, stdout_guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .buffered_lines_limit(1_048_576)
        .lossy(false)
        .finish(std::io::stdout());

    let stdout_layer = Box::new(
        tracing_subscriber::fmt::layer()
            .with_writer(non_blocking.clone())
            .event_format(OtelFormat {
                timer: tracing_subscriber::fmt::time::ChronoLocal::new("%Y-%m-%dT%H:%M:%S%.6f%z".to_string()),
            })
            .with_ansi(cfg!(debug_assertions)),
    );

    (stdout_layer, Some(stdout_guard))
}

/// Creates a non-blocking stderr logger layer filtered to ERROR and above.
///
/// This allows log scrapers and operators to monitor stderr for errors separately
/// from the full log stream on stdout (SRD section 3.11: "Ideally a filtered subset
/// of logs can be redirected to stderr").
pub(crate) fn make_stderr_logger_layer<S>() -> (Box<dyn Layer<S> + Send + Sync + 'static>, Option<WorkerGuard>)
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let (non_blocking, stderr_guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .buffered_lines_limit(1_048_576)
        .lossy(false)
        .finish(std::io::stderr());

    let stderr_filter = EnvFilter::new("error");

    let stderr_layer = Box::new(
        tracing_subscriber::fmt::layer()
            .with_writer(non_blocking)
            .event_format(OtelFormat {
                timer: tracing_subscriber::fmt::time::ChronoLocal::new("%Y-%m-%dT%H:%M:%S%.6f%z".to_string()),
            })
            .with_ansi(cfg!(debug_assertions))
            .with_filter(stderr_filter),
    );

    (stderr_layer, Some(stderr_guard))
}

/// A custom visitor that collects all fields and ensures a message is always set in the log record.
/// This visitor wraps the OpenTelemetry EventVisitor and ensures the body is always set.
struct MessageEnsuringVisitor<'a, LR: LogRecord> {
    log_record: &'a mut LR,
    fields: Vec<(String, String)>,
    has_message: bool,
}

impl<'a, LR: LogRecord> MessageEnsuringVisitor<'a, LR> {
    fn new(log_record: &'a mut LR) -> Self {
        Self {
            log_record,
            fields: Vec::new(),
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
            if let Some((_, msg)) = self.fields.iter().find(|(k, _)| k == "message") {
                parts.push(msg.clone());
            }
            // Append other fields
            for (key, value) in &self.fields {
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
        self.fields.push((field.name().to_string(), formatted.clone()));
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
        self.fields.push((field.name().to_string(), value_str.clone()));
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
        self.fields.push((field.name().to_string(), formatted.clone()));
        // Also add as attribute to preserve all fields
        use opentelemetry::Key;
        self.log_record
            .add_attribute(Key::new("exception.message"), AnyValue::from(formatted));
    }

    fn record_bytes(&mut self, field: &tracing::field::Field, value: &[u8]) {
        let formatted = format!("{:?}", value);
        self.fields.push((field.name().to_string(), formatted.clone()));
        use opentelemetry::Key;
        self.log_record
            .add_attribute(Key::new(field.name()), AnyValue::from(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields.push((field.name().to_string(), value.to_string()));
        use opentelemetry::Key;
        self.log_record
            .add_attribute(Key::new(field.name()), AnyValue::from(value));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.fields.push((field.name().to_string(), value.to_string()));
        use opentelemetry::Key;
        self.log_record
            .add_attribute(Key::new(field.name()), AnyValue::from(value));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.push((field.name().to_string(), value.to_string()));
        use opentelemetry::Key;
        self.log_record
            .add_attribute(Key::new(field.name()), AnyValue::from(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.push((field.name().to_string(), value.to_string()));
        use opentelemetry::Key;
        let key = Key::new(field.name());
        if let Ok(signed) = i64::try_from(value) {
            self.log_record.add_attribute(key, AnyValue::from(signed));
        } else {
            self.log_record.add_attribute(key, AnyValue::from(format!("{value:?}")));
        }
    }

    fn record_i128(&mut self, field: &tracing::field::Field, value: i128) {
        self.fields.push((field.name().to_string(), value.to_string()));
        use opentelemetry::Key;
        let key = Key::new(field.name());
        if let Ok(signed) = i64::try_from(value) {
            self.log_record.add_attribute(key, AnyValue::from(signed));
        } else {
            self.log_record.add_attribute(key, AnyValue::from(format!("{value:?}")));
        }
    }

    fn record_u128(&mut self, field: &tracing::field::Field, value: u128) {
        self.fields.push((field.name().to_string(), value.to_string()));
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

impl<S, P, L> Layer<S> for MessageFormatterOpenTelemetryLayer<P, L>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    P: LoggerProvider<Logger = L> + Send + Sync + 'static,
    L: Logger + Send + Sync + 'static,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let metadata = event.metadata();
        let target = metadata.target();
        let name = metadata.name();

        let mut log_record = self.logger.create_log_record();

        log_record.set_target(target);
        log_record.set_event_name(name);
        // SRD: "Set SeverityText field. Don't set SeverityNumber as it is redundant."
        log_record.set_severity_text(metadata.level().as_str());

        // Use our custom visitor that ensures the message is always set
        let mut visitor = MessageEnsuringVisitor::new(&mut log_record);
        event.record(&mut visitor);
        visitor.ensure_message();

        // Trace context (TraceId, SpanId) is automatically injected by the
        // SDK's Logger::emit() when the `trace` feature is enabled (default).
        // It reads the active span from the current OpenTelemetry Context,
        // which is maintained by the tracing-opentelemetry OpenTelemetryLayer.
        // No manual extraction is needed here.
        self.logger.emit(log_record);
    }
}

/// Environment variable for setting the global OTLP log severity filter.
///
/// Values: `ERROR`, `WARN`/`WARNING`, `INFO`, `DEBUG`, `TRACE` (case-insensitive).
/// OpenTelemetry doesn't define a standard client-side log level filter, so this provides
/// a simple way to set a global override for OTLP log export.
pub(crate) const LOGS_LEVEL: &str = "LOGS_LEVEL";

/// Environment variable for per-module OTLP log severity overrides.
///
/// Format: comma-separated `module=LEVEL` pairs, e.g. `"storage_core=DEBUG,hyper=ERROR"`.
/// Overrides `LOGS_LEVEL` for individual modules. Useful for ad-hoc debugging in production.
pub(crate) const LOGS_MODULES: &str = "LOGS_MODULES";

/// Normalize SRD-style level names to tracing directive level names.
///
/// Accepts level names (ERROR, WARN, WARNING, INFO, DEBUG, TRACE — case-insensitive)
/// or OTEL severity numbers (1-4 = TRACE, 5-8 = DEBUG, 9-12 = INFO, 13-16 = WARN, 17+ = ERROR).
/// Returns None for unrecognized values.
pub(super) fn normalize_level_str(s: &str) -> Option<&'static str> {
    match s.to_uppercase().as_str() {
        "ERROR" => Some("error"),
        "WARN" | "WARNING" => Some("warn"),
        "INFO" => Some("info"),
        "DEBUG" => Some("debug"),
        "TRACE" => Some("trace"),
        _ => {
            // SRD 3.11.1: "Values can be a severity number or a short name"
            // OTEL severity numbers: 1-4=TRACE, 5-8=DEBUG, 9-12=INFO, 13-16=WARN, 17+=ERROR
            if let Ok(n) = s.parse::<u32>() {
                match n {
                    1..=4 => Some("trace"),
                    5..=8 => Some("debug"),
                    9..=12 => Some("info"),
                    13..=16 => Some("warn"),
                    17..=24 => Some("error"),
                    _ => None,
                }
            } else {
                None
            }
        },
    }
}

/// Creates an EnvFilter for the OTLP log exporter.
///
/// Priority order for the base log level:
/// 1. `LOGS_LEVEL` — SRD-defined global severity filter for OTLP export
/// 2. `RUST_LOG` — standard Rust logging env var
/// 3. `OTEL_LOG_LEVEL` — OpenTelemetry fallback
/// 4. Default (ERROR)
///
/// `LOGS_MODULES` provides per-module overrides on top of the base level,
/// e.g. `"storage_core=DEBUG,hyper=ERROR"`.
///
/// Telemetry-related libraries (hyper, tonic, h2, reqwest) are always suppressed
/// to prevent telemetry-induced-telemetry loops. See:
/// <https://github.com/open-telemetry/opentelemetry-rust/issues/2877>
fn create_otel_log_filter() -> EnvFilter {
    use std::env;

    // Determine the base filter from environment variables (priority order)
    let mut filter = if let Ok(logs_level) = env::var(LOGS_LEVEL) {
        if let Some(level) = normalize_level_str(&logs_level) {
            info_limited!("OTLP log filter: using {LOGS_LEVEL}={logs_level} (normalized to {level})");
            EnvFilter::new(level)
        } else {
            error_limited!("Invalid {LOGS_LEVEL} value: {logs_level:?}. Expected ERROR, WARN, INFO, DEBUG, or TRACE. Falling back to defaults.");
            EnvFilter::from_default_env()
        }
    } else if env::var("RUST_LOG").is_ok() {
        EnvFilter::from_default_env()
    } else if let Ok(otel_log_level) = env::var("OTEL_LOG_LEVEL") {
        EnvFilter::new(&otel_log_level)
    } else {
        EnvFilter::from_default_env()
    };

    // Apply per-module overrides from LOGS_MODULES
    if let Ok(logs_modules) = env::var(LOGS_MODULES) {
        info_limited!("OTLP log filter: applying per-module overrides from {LOGS_MODULES}={logs_modules}");
        for entry in logs_modules.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            if let Some((module, level_str)) = entry.split_once('=') {
                let module = module.trim();
                let level_str = level_str.trim();
                if let Some(level) = normalize_level_str(level_str) {
                    let directive = format!("{module}={level}");
                    match directive.parse() {
                        Ok(d) => filter = filter.add_directive(d),
                        Err(e) => error_limited!("Invalid {LOGS_MODULES} directive {entry:?}: {e}"),
                    }
                } else {
                    error_limited!(
                        "Invalid level in {LOGS_MODULES} entry {entry:?}. Expected ERROR, WARN, INFO, DEBUG, or TRACE."
                    );
                }
            } else {
                error_limited!("Invalid {LOGS_MODULES} entry {entry:?}. Expected format: module=LEVEL");
            }
        }
    }

    // Suppress telemetry-related libraries to prevent telemetry-induced-telemetry loops.
    filter
        .add_directive("hyper=off".parse().unwrap())
        .add_directive("tonic=off".parse().unwrap())
        .add_directive("h2=off".parse().unwrap())
        .add_directive("reqwest=off".parse().unwrap())
}

/// Resolve the OTLP log export protocol from environment variables.
///
/// Checks (in order):
/// 1. `OTEL_EXPORTER_OTLP_LOGS_PROTOCOL` (signal-specific override)
/// 2. `OTEL_EXPORTER_OTLP_PROTOCOL` (generic fallback)
/// 3. Infer from `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT` or `OTEL_EXPORTER_OTLP_ENDPOINT` port
/// 4. Default to `"grpc"`
///
/// Valid values: `"grpc"`, `"http/protobuf"`.
pub(crate) fn resolve_otlp_logs_protocol() -> String {
    // 1. Signal-specific, then generic env var
    if let Ok(protocol) = env::var("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL") {
        return protocol;
    }
    if let Ok(protocol) = env::var("OTEL_EXPORTER_OTLP_PROTOCOL") {
        return protocol;
    }
    // 2. Infer from endpoint port (4317 = grpc, anything else = http/protobuf)
    let endpoint = env::var("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT").or_else(|_| env::var("OTEL_EXPORTER_OTLP_ENDPOINT"));
    if let Ok(endpoint) = endpoint {
        if endpoint.contains(":4317") {
            return "grpc".to_string();
        }
        return "http/protobuf".to_string();
    }
    // 3. Default — our deployments use the gRPC collector sidecar
    "grpc".to_string()
}

type OtlpLayerResult<S> = Result<
    (Box<dyn Layer<S> + Send + Sync + 'static>, SdkLoggerProvider),
    Box<dyn std::error::Error + Send + Sync + 'static>,
>;

pub(crate) fn make_otlp_logger_layer<S>(resource: Resource) -> OtlpLayerResult<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let protocol = resolve_otlp_logs_protocol();

    let exporter = match protocol.as_str() {
        "http/protobuf" => opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .build()
            .inspect_err(|e| error_limited!("failed to create HTTP log exporter: {e:?}"))?,
        // "grpc" or unrecognized — default to gRPC (with gzip compression)
        other => {
            if other != "grpc" {
                warn_limited!(protocol = other, "unrecognized OTLP logs protocol, defaulting to grpc");
            }
            opentelemetry_otlp::LogExporter::builder()
                .with_tonic()
                .with_compression(opentelemetry_otlp::Compression::Gzip)
                .build()
                .inspect_err(|e| error_limited!("failed to create gRPC log exporter: {e:?}"))?
        },
    };

    let provider: SdkLoggerProvider = SdkLoggerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter)
        .build();

    // Create a filter that merges RUST_LOG with telemetry suppression directives
    let filter_otel = create_otel_log_filter();

    // Use our custom OpenTelemetry layer that ensures messages are always formatted
    let otel_layer = MessageFormatterOpenTelemetryLayer::new(&provider);
    Ok((Box::new(otel_layer.with_filter(filter_otel)), provider))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    /// Target of the WARN emitted by `axum-tracing-opentelemetry` when `OpenTelemetryLayer`
    /// is not registered on the current subscriber (see `trace_extractor.rs:165`).
    const AXUM_TARGET: &str = "axum_tracing_opentelemetry::middleware::trace_extractor";
    /// Same misconfiguration, gRPC side (see `tonic-tracing-opentelemetry` `server.rs:85`).
    const TONIC_TARGET: &str = "tonic_tracing_opentelemetry::middleware::server";

    #[derive(Default, Clone)]
    struct CapturingLayer {
        targets: Arc<Mutex<Vec<String>>>,
    }

    impl<S: tracing::Subscriber> Layer<S> for CapturingLayer {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
            self.targets.lock().unwrap().push(event.metadata().target().to_string());
        }
    }

    /// Runs `f` with RUST_LOG temporarily set, guaranteeing the var is removed
    /// afterward even on panic — otherwise state leaks to later tests that read
    /// the env. Tests using this must be annotated with `#[serial]` to serialize
    /// against each other and against other tests that mutate RUST_LOG.
    fn run_with_filter<F: FnOnce()>(rust_log: &str, f: F) -> Vec<String> {
        unsafe {
            env::set_var("RUST_LOG", rust_log);
        }
        let layer = CapturingLayer::default();
        let result = {
            let filter = create_merged_log_filter();
            let subscriber = tracing_subscriber::registry()
                .with(layer.clone())
                .with(filter)
                .with(azure_credential_log_guard());
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tracing::subscriber::with_default(subscriber, f);
            }))
        };
        unsafe {
            env::remove_var("RUST_LOG");
        }
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
        let out = layer.targets.lock().unwrap().clone();
        out
    }

    fn emit_layer_not_found_warnings() {
        // `set_parent` returns `LayerNotFound` on every request when OpenTelemetryLayer
        // is absent from the subscriber — this is the exact call site we're suppressing.
        tracing::warn!(
            target: AXUM_TARGET,
            error = "LayerNotFound",
            "can not set parent trace_id to span",
        );
        tracing::warn!(
            target: TONIC_TARGET,
            error = "LayerNotFound",
            "can not set parent trace_id to span",
        );
    }

    /// The LayerNotFound condition is a static misconfiguration that the middleware
    /// re-emits on every request. The filter must suppress it at every RUST_LOG level
    /// an operator might reasonably run in production, AND the debug escape hatch at
    /// RUST_LOG=trace must still let it through.
    #[test]
    #[serial]
    fn layer_not_found_warning_is_suppressed_at_production_levels() {
        for rust_log in ["error", "warn", "info", "debug"] {
            let captured = run_with_filter(rust_log, emit_layer_not_found_warnings);
            let leaked: Vec<&String> = captured
                .iter()
                .filter(|t| *t == AXUM_TARGET || *t == TONIC_TARGET)
                .collect();
            assert!(
                leaked.is_empty(),
                "RUST_LOG={rust_log:?}: middleware LayerNotFound WARN should be filtered, but got {leaked:?}",
            );
        }
    }

    #[test]
    #[serial]
    fn layer_not_found_warning_is_visible_at_rust_log_trace() {
        let captured = run_with_filter("trace", emit_layer_not_found_warnings);
        assert!(
            captured.iter().any(|t| t == AXUM_TARGET),
            "RUST_LOG=trace should let axum middleware warning through for debugging; got {captured:?}",
        );
        assert!(
            captured.iter().any(|t| t == TONIC_TARGET),
            "RUST_LOG=trace should let tonic middleware warning through for debugging; got {captured:?}",
        );
    }

    /// Target used by the azure_core request logger that RUSTSEC-2026-0275 warns
    /// can carry the Authorization header value.
    const AZURE_CORE_TARGET: &str = "azure_core";

    fn emit_azure_core_debug() {
        tracing::debug!(target: AZURE_CORE_TARGET, "azure_core request log (may contain Authorization header)");
    }

    /// Compensating control for RUSTSEC-2026-0275: a broad `RUST_LOG=debug` must
    /// not surface azure_core debug logs (which can include the auth header).
    #[test]
    #[serial]
    fn azure_core_debug_is_suppressed_under_global_debug() {
        let captured = run_with_filter("debug", emit_azure_core_debug);
        assert!(
            !captured.iter().any(|t| t == AZURE_CORE_TARGET),
            "RUST_LOG=debug must not surface azure_core debug logs; got {captured:?}",
        );
    }

    /// Naming the whole crate no longer opts in: the guard has no opt-out, because
    /// azure_core 0.21 prints the Authorization header verbatim, so "debug the SDK" and
    /// "log the credential" are the same request. INFO and above still pass.
    #[test]
    #[serial]
    fn explicit_azure_core_directive_does_not_surface_debug() {
        let captured = run_with_filter("azure_core=debug", emit_azure_core_debug);
        assert!(
            !captured.iter().any(|t| t == AZURE_CORE_TARGET),
            "an explicit azure_core=debug directive must not surface the credential log; got {captured:?}",
        );
    }

    /// The guard is a DEBUG/TRACE floor, not a blanket silence: azure_core INFO still
    /// reaches the subscriber when the level allows it.
    #[test]
    #[serial]
    fn azure_core_info_still_passes() {
        let captured = run_with_filter("debug", || {
            tracing::info!(target: AZURE_CORE_TARGET, "azure_core info line");
        });
        assert!(
            captured.iter().any(|t| t == AZURE_CORE_TARGET),
            "azure_core INFO must still pass; got {captured:?}",
        );
    }

    /// The precedence bypass: `EnvFilter` picks the most specific matching directive and
    /// `add_directive` only replaces an exactly-matching target, so a sub-path directive
    /// outranks `azure_core=info`. These are the two forms that reach the transport
    /// policy's request dump, which is what prints the header.
    #[test]
    #[serial]
    fn sub_path_directives_cannot_surface_the_credential_log() {
        for rust_log in [
            "azure_core::policies::transport=debug",
            "azure_core::policies=debug",
            "debug,azure_core::policies::transport=trace",
        ] {
            let captured = run_with_filter(rust_log, || {
                tracing::debug!(
                    target: "azure_core::policies::transport",
                    "the following request will be passed to the transport policy",
                );
            });
            assert!(
                !captured.iter().any(|t| t == "azure_core::policies::transport"),
                "RUST_LOG={rust_log:?} must not surface the transport request dump; got {captured:?}",
            );
        }
    }

    /// The cap keys off the parsed directive targets, not a raw substring: an unrelated
    /// directive that merely contains "azure" (e.g. our own `storage_core::azure` module)
    /// must not lift the `azure_core=info` cap under a broad `debug` level.
    #[test]
    #[serial]
    fn azure_core_debug_is_suppressed_despite_unrelated_azure_directive() {
        let captured = run_with_filter("debug,storage_core::azure=trace", emit_azure_core_debug);
        assert!(
            !captured.iter().any(|t| t == AZURE_CORE_TARGET),
            "an unrelated storage_core::azure directive must not surface azure_core debug logs; got {captured:?}",
        );
    }

    /// A sub-path directive is honored for its own target by `EnvFilter` specificity,
    /// so it does not need to — and must not — lift the SDK-wide cap. Otherwise a narrow
    /// `azure_core::policies` directive would leave every other `azure_core` module at
    /// the global level, where the Authorization header can be logged.
    #[test]
    #[serial]
    fn azure_core_sub_path_directive_does_not_lift_the_sdk_wide_cap() {
        let captured = run_with_filter("debug,azure_core::policies=trace", emit_azure_core_debug);
        assert!(
            !captured.iter().any(|t| t == AZURE_CORE_TARGET),
            "a narrow azure_core:: sub-path directive must not surface other azure_core targets; got {captured:?}",
        );
    }

    /// A directive may omit the level (`azure_core` means that target at trace). That
    /// form must not become a bypass either.
    #[test]
    #[serial]
    fn bare_azure_core_target_directive_does_not_surface_debug() {
        let captured = run_with_filter("debug,azure_core", emit_azure_core_debug);
        assert!(
            !captured.iter().any(|t| t == AZURE_CORE_TARGET),
            "a bare azure_core directive must not surface the credential log; got {captured:?}",
        );
    }

    /// The case the target directive alone cannot stop: `EnvFilter` evaluates dynamic
    /// span directives before target directives, so a span filter enables the event
    /// without naming an Azure target anywhere. Azure requests run inside spans such as
    /// `Get Properties`, so this is a realistic incident-debugging setting — and it
    /// would log the live Authorization header. The guard layer is what rejects it.
    #[test]
    #[serial]
    fn span_directive_cannot_surface_azure_core_debug() {
        let captured = run_with_filter("info,[Get Properties]=debug", || {
            let span = tracing::info_span!("Get Properties");
            let _entered = span.enter();
            emit_azure_core_debug();
        });
        assert!(
            !captured.iter().any(|t| t == AZURE_CORE_TARGET),
            "a span directive must not surface azure_core debug logs; got {captured:?}",
        );
    }

    /// The OTLP exporter attaches its own per-layer filter
    /// (`otel_layer.with_filter(create_otel_log_filter())`), which is evaluated
    /// separately from the subscriber-level filters. The guard must still veto first,
    /// or a credential-bearing event would be exported off-box even though it never
    /// reached stdout. This models that shape: a capturing layer with a permissive
    /// per-layer filter, behind the guard.
    #[test]
    #[serial]
    fn guard_vetoes_before_a_per_layer_filter_can_export() {
        // The span directive is used deliberately: it is the case `EnvFilter` *allows*,
        // so only the guard can reject the event. Under a plain `RUST_LOG=debug` the
        // `azure_core=info` directive already rejects it and the test would prove
        // nothing about the guard.
        unsafe {
            env::set_var("RUST_LOG", "info,[Get Properties]=debug");
        }
        let layer = CapturingLayer::default();
        let captured = {
            // `LevelFilter::TRACE` as a per-layer filter would admit everything.
            let permissive = layer.clone().with_filter(LevelFilter::TRACE);
            let subscriber = tracing_subscriber::registry()
                .with(permissive)
                .with(create_merged_log_filter())
                .with(azure_credential_log_guard());
            tracing::subscriber::with_default(subscriber, || {
                let span = tracing::info_span!("Get Properties");
                let _entered = span.enter();
                emit_azure_core_debug();
            });
            let out = layer.targets.lock().unwrap().clone();
            out
        };
        unsafe {
            env::remove_var("RUST_LOG");
        }
        assert!(
            !captured.iter().any(|t| t == AZURE_CORE_TARGET),
            "a permissive per-layer filter must not export azure_core debug; got {captured:?}",
        );
    }

    /// The same span directive must still work for everything that is not an Azure SDK
    /// target — the guard is narrow, not a blanket debug suppressor.
    #[test]
    #[serial]
    fn span_directive_still_works_for_non_azure_targets() {
        let captured = run_with_filter("info,[Get Properties]=debug", || {
            let span = tracing::info_span!("Get Properties");
            let _entered = span.enter();
            tracing::debug!(target: "storage_core::s3", "unrelated debug line");
        });
        assert!(
            captured.iter().any(|t| t == "storage_core::s3"),
            "the span directive must still enable non-Azure debug logs; got {captured:?}",
        );
    }

    /// `EnvFilter` directives are absolute, not maxima, so `azure_core=info` is a floor
    /// as well as a ceiling. Below `debug` the cap must not be added at all, or it would
    /// *raise* these targets and surface events that were previously filtered out.
    #[test]
    #[serial]
    fn cap_does_not_raise_azure_level_below_debug() {
        for rust_log in ["warn", "error", ""] {
            let captured = run_with_filter(rust_log, || {
                tracing::info!(target: AZURE_CORE_TARGET, "azure_core info line");
            });
            assert!(
                !captured.iter().any(|t| t == AZURE_CORE_TARGET),
                "RUST_LOG={rust_log:?} must not newly enable azure_core INFO; got {captured:?}",
            );
        }
    }

    /// `EnvFilter::from_default_env` silently drops a malformed directive. If a typo
    /// counted as an opt-in, the cap would be skipped while the directive contributed
    /// nothing — leaving azure_core at the global level.
    #[test]
    #[serial]
    fn malformed_azure_core_directive_does_not_lift_the_cap() {
        let captured = run_with_filter("debug,azure_core=dbug", emit_azure_core_debug);
        assert!(
            !captured.iter().any(|t| t == AZURE_CORE_TARGET),
            "a malformed azure_core directive must not surface azure_core debug logs; got {captured:?}",
        );
    }
}
