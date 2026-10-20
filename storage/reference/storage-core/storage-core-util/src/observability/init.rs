// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Initialization hooks for framework-provided systems.

use crate::{debug_limited, info_limited, warn_limited};
#[allow(deprecated)] // TODO: migrate to TracingConfig API
use init_tracing_opentelemetry::tracing_subscriber_ext::{
    build_metrics_layer_with_resource, build_tracer_layer_with_resource,
};
use opentelemetry::KeyValue;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use std::env;
use std::sync::OnceLock;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;

use super::logger::{
    azure_credential_log_guard, create_merged_log_filter, make_otlp_logger_layer, make_stderr_logger_layer,
    make_stdout_logger_layer, make_text_subscriber,
};

/// Configure the prefix prepended to every metric name. See [`super::metrics::set_metric_prefix`] for full semantics.
pub use super::metrics::{set_metric_prefix, MetricPrefixError};

/// The namespace for all Omniverse services.
/// Omniverse Metrics SRD 3.1: Set `service.namespace` to `omniverse`.
const SERVICE_NAMESPACE: &str = "omniverse";

// Per the Environment Variable Implementation Guidelines, supporting the
// Environment Variables is "optional."
//
// https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/#implementation-guidelines
//
// So we do our best to backfill here. We should avoid backfilling if future
// versions of `opentelemetry-rust` take care of this for us.
//
// We do cheat around some of the awkward layering here, just to get this
// out the door. This code needs to be cleaned up, but I expect just waiting
// for a non-beta OpenTelemetry, will allow this to plumb through more
// easily without additional effort.

/// Enable or disable the OpenTelemetry SDK, only disable if set to "false".
pub const OTEL_SDK_DISABLED: &str = "OTEL_SDK_DISABLED";
/// Set the service name.
pub const OTEL_SERVICE_NAME: &str = "OTEL_SERVICE_NAME";
/// Set which Traces Exporter To Use.
pub const OTEL_TRACES_EXPORTER: &str = "OTEL_TRACES_EXPORTER";
/// Set which Metrics Exporter To Use.
pub const OTEL_METRICS_EXPORTER: &str = "OTEL_METRICS_EXPORTER";
/// Set which Logs Exporter To Use.
pub const OTEL_LOGS_EXPORTER: &str = "OTEL_LOGS_EXPORTER";
/// Set the logs exporter endpoint.
pub const OTEL_EXPORTER_OTLP_LOGS_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT";
/// Set the traces exporter endpoint.
pub const OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT";
/// Set the propagators.
pub const OTEL_PROPAGATORS: &str = "OTEL_PROPAGATORS";

/// Monitors OTEL_SDK_DISABLED to inform correct wireup behavior.
fn otel_enabled() -> bool {
    let enabled = match env::var(OTEL_SDK_DISABLED) {
        Ok(v) => v.to_lowercase() != "true",
        Err(_) => true,
    };

    if !enabled {
        info_limited!("OpenTelemetry is disabled because {OTEL_SDK_DISABLED} is set to \"true\"");
    }

    enabled
}

/// Check if tracing is enabled by looking at various environment variables. If any of the expected tracing environment
/// variables are set, then tracing is enabled, unless OTEL_TRACES_EXPORTER is set to "none".
///
/// The result is cached after the first call, so subsequent callers get a consistent answer and
/// the resolution diagnostics are only logged once.
///
/// ## NOTE(tgockel)
/// This is a quick workaround that I think is actually wrong. Complaints were made about the Frontend
/// spamming that it could not connect to the tracing service (repeatedly giving a gRPC "not implemented" error). The
/// solution is to not trace in the case where `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, `OTEL_TRACES_EXPORTER` and
/// `OTEL_PROPAGATORS` are not set. However, this behavior seems to clash with the OpenTelemetry SDK specification for
/// [Environment Variables][otel-sdk-env-vars], which has explicit defaults for these, which is not "do not trace."
///
/// The specification does not say what to do if some of these are set and others are not, because it assumes that
/// defaults will be used. So this behavior is just kind of a guess at what seems to work on NVCF.
///
/// [otel-sdk-env-vars]: https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/
pub fn is_tracing_enabled() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(resolve_tracing_enabled)
}

fn resolve_tracing_enabled() -> bool {
    // If OTEL_TRACES_EXPORTER is explicitly set to "none", tracing is disabled
    if env::var(OTEL_TRACES_EXPORTER).is_ok_and(|v| v.eq_ignore_ascii_case("none")) {
        info_limited!("OpenTelemetry Tracing is disabled because OTEL_TRACES_EXPORTER is set to \"none\"");
        return false;
    }

    let vars = [
        OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
        OTEL_TRACES_EXPORTER,
        OTEL_PROPAGATORS,
    ];
    let (present, missing) = vars.iter().partition::<Vec<&str>, _>(|&var| env::var_os(var).is_some());

    if !present.is_empty() && !missing.is_empty() {
        // See function documentation
        info_limited!(
            "OpenTelemetry environment variables are set inconsistently. It is expected that all of {vars:?} will be set, or none of them will be set. However, the variables {present:?} are set, but {missing:?} are not."
        );
    }

    let enabled = !present.is_empty();
    if enabled {
        info_limited!("OpenTelemetry Tracing is enabled because environment variables {present:?} are set");
    } else {
        info_limited!("OpenTelemetry Tracing is disabled because none of {vars:?} are set");
    }

    enabled
}

/// Check if logging is enabled by looking at various environment variables. If any of the expected logging environment
/// variables are set, then logging is enabled, unless OTEL_LOGS_EXPORTER is set to "none".
///
/// This has the same caveats as `is_tracing_enabled`.
fn is_logging_enabled() -> bool {
    // If OTEL_LOGS_EXPORTER is explicitly set to "none", logging is disabled
    if env::var(OTEL_LOGS_EXPORTER).is_ok_and(|v| v.eq_ignore_ascii_case("none")) {
        info_limited!("OpenTelemetry Logging is disabled because OTEL_LOGS_EXPORTER is set to \"none\"");
        return false;
    }

    let vars = [OTEL_EXPORTER_OTLP_LOGS_ENDPOINT, OTEL_LOGS_EXPORTER];
    let (present, missing) = vars.iter().partition::<Vec<&str>, _>(|&var| env::var_os(var).is_some());

    if !present.is_empty() && !missing.is_empty() {
        // See function documentation
        info_limited!(
            "OpenTelemetry environment variables are set inconsistently. It is expected that all of {vars:?} will be set, or none of them will be set. However, the variables {present:?} are set, but {missing:?} are not."
        );
    }

    let enabled = !present.is_empty();
    if enabled {
        info_limited!("OpenTelemetry Logging is enabled because environment variables {present:?} are set");
    } else {
        info_limited!("OpenTelemetry Logging is disabled because none of {vars:?} are set");
    }

    enabled
}

/// Check if metrics are enabled. Metrics are disabled if `OTEL_METRICS_EXPORTER` is set to `none`.
fn is_metrics_enabled() -> bool {
    let enabled = env::var(OTEL_METRICS_EXPORTER)
        .map(|v| !v.eq_ignore_ascii_case("none"))
        .unwrap_or(true);

    if enabled {
        info_limited!("OpenTelemetry Metrics are enabled because OTEL_METRICS_EXPORTER is not set to \"none\"");
    } else {
        info_limited!("OpenTelemetry Metrics are disabled because OTEL_METRICS_EXPORTER is set to \"none\"");
    }

    enabled
}

/// Builds an OpenTelemetry Resource with standard service attributes.
///
/// This creates a Resource with the following attributes:
/// - `service.name`: The name of the service (can be overridden by `OTEL_SERVICE_NAME` env var)
/// - `service.version`: The semantic version of the service (from CARGO_PKG_VERSION)
/// - `service.namespace`: Set to `"omniverse"` for all Omniverse services
///
/// These attributes follow the [OpenTelemetry Service semantic conventions](https://opentelemetry.io/docs/specs/semconv/resource/#service).
pub fn build_resource(service_name: &str, service_version: &str) -> Resource {
    // OTEL_SERVICE_NAME env var takes precedence over the provided service name
    let effective_service_name = env::var(OTEL_SERVICE_NAME)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| service_name.to_string());

    Resource::builder()
        .with_service_name(effective_service_name)
        .with_attribute(KeyValue::new("service.version", service_version.to_string()))
        .with_attribute(KeyValue::new("service.namespace", SERVICE_NAMESPACE))
        .build()
}

/// Logs the effective observability configuration after the subscriber is installed.
fn log_effective_config() {
    let rust_log = env::var("RUST_LOG").unwrap_or_else(|_| "<not set>".to_string());
    let logs_level = env::var(super::logger::LOGS_LEVEL).unwrap_or_else(|_| "<not set>".to_string());
    let logs_modules = env::var(super::logger::LOGS_MODULES).unwrap_or_else(|_| "<not set>".to_string());
    let traces_endpoint = env::var(OTEL_EXPORTER_OTLP_TRACES_ENDPOINT).unwrap_or_else(|_| "<not set>".to_string());
    let logs_endpoint = env::var(OTEL_EXPORTER_OTLP_LOGS_ENDPOINT).unwrap_or_else(|_| "<not set>".to_string());
    let compression = env::var("OTEL_EXPORTER_OTLP_COMPRESSION").unwrap_or_else(|_| "gzip (default)".to_string());
    let logs_protocol = super::logger::resolve_otlp_logs_protocol();

    info_limited!(
        RUST_LOG = %rust_log,
        LOGS_LEVEL = %logs_level,
        LOGS_MODULES = %logs_modules,
        traces_endpoint = %traces_endpoint,
        logs_endpoint = %logs_endpoint,
        logs_protocol = %logs_protocol,
        compression = %compression,
        "Observability configuration summary"
    );
}

pub fn try_init_observability(
    service_name: &str,
    service_version: &str,
) -> Result<Guard, Box<dyn std::error::Error + Send + Sync + 'static>> {
    // Keep this first so panics get handled
    super::install_panic_handler();

    // Validate config before anything else (SRD 3.11: terminate on invalid config)
    super::validate_observability_config()?;

    // Capture wireup errors.
    let _guard = tracing::subscriber::set_default(make_text_subscriber()?);

    info_limited!("Initializing observability for service {service_name} v{service_version}");

    let mut result = if otel_enabled() {
        let resource = build_resource(service_name, service_version);

        // Conditionally add metrics layer if OTEL_METRICS_EXPORTER != "none"
        let subscriber = tracing_subscriber::registry();
        let (metrics_layer, metrics_guard) = if is_metrics_enabled() {
            #[allow(deprecated)] // TODO: migrate to TracingConfig API
            let (layer, guard) = build_metrics_layer_with_resource(resource.clone())?;
            (Some(layer), Some(guard))
        } else {
            (None, None)
        };
        let subscriber = subscriber.with(metrics_layer);

        if is_tracing_enabled() {
            // We call register_otel_layers_with_resource on a plain registry first,
            // then create and add our layers on top of the resulting subscriber.
            #[allow(deprecated)] // TODO: migrate to TracingConfig API
            let (tracing_layer, tracing_guard) = build_tracer_layer_with_resource(resource.clone())?;
            let subscriber = subscriber.with(tracing_layer);

            // Create layers for this specific subscriber type
            let filter_layer = create_merged_log_filter();
            // ANDed with filter_layer: the enforcing half of the RUSTSEC-2026-0275
            // control, which a target directive alone cannot provide.
            let azure_guard = azure_credential_log_guard();
            let (stdout_layer, stdout_guard) = make_stdout_logger_layer();
            let (stderr_layer, stderr_guard) = make_stderr_logger_layer();

            if is_logging_enabled() {
                // Logging + Tracing + Metrics (if enabled)
                let (otel_layer, log_provider) = make_otlp_logger_layer(resource)?;
                let subscriber = subscriber
                    .with(filter_layer)
                    .with(azure_guard)
                    .with(stdout_layer)
                    .with(stderr_layer)
                    .with(otel_layer);
                tracing::subscriber::set_global_default(subscriber)?;
                Ok(Guard::new(
                    Some(tracing_guard),
                    metrics_guard,
                    Some(log_provider),
                    stdout_guard,
                    stderr_guard,
                ))
            } else {
                // Tracing + Metrics (if enabled)
                let subscriber = subscriber
                    .with(filter_layer)
                    .with(azure_guard)
                    .with(stdout_layer)
                    .with(stderr_layer);
                tracing::subscriber::set_global_default(subscriber)?;
                Ok(Guard::new(
                    Some(tracing_guard),
                    metrics_guard,
                    None,
                    stdout_guard,
                    stderr_guard,
                ))
            }
        } else {
            // No tracing - create layers for stdout logging and metrics
            let filter_layer = create_merged_log_filter();
            // ANDed with filter_layer: the enforcing half of the RUSTSEC-2026-0275
            // control, which a target directive alone cannot provide.
            let azure_guard = azure_credential_log_guard();
            let (stdout_layer, stdout_guard) = make_stdout_logger_layer();
            let (stderr_layer, stderr_guard) = make_stderr_logger_layer();

            if is_logging_enabled() {
                // Logging + Metrics (if enabled), no tracing
                let (otel_layer, log_provider) = make_otlp_logger_layer(resource)?;
                let subscriber = subscriber
                    .with(filter_layer)
                    .with(azure_guard)
                    .with(stdout_layer)
                    .with(stderr_layer)
                    .with(otel_layer);
                tracing::subscriber::set_global_default(subscriber)?;
                Ok(Guard::new(
                    None,
                    metrics_guard,
                    Some(log_provider),
                    stdout_guard,
                    stderr_guard,
                ))
            } else {
                // Metrics only (if enabled), no tracing, no logging
                let subscriber = subscriber
                    .with(filter_layer)
                    .with(azure_guard)
                    .with(stdout_layer)
                    .with(stderr_layer);
                tracing::subscriber::set_global_default(subscriber)?;
                Ok(Guard::new(None, metrics_guard, None, stdout_guard, stderr_guard))
            }
        }
    } else {
        // No OpenTelemetry
        let filter_layer = create_merged_log_filter();
        // ANDed with filter_layer: the enforcing half of the RUSTSEC-2026-0275
        // control, which a target directive alone cannot provide.
        let azure_guard = azure_credential_log_guard();
        let (stdout_layer, stdout_guard) = make_stdout_logger_layer();
        let (stderr_layer, stderr_guard) = make_stderr_logger_layer();
        let subscriber = tracing_subscriber::registry()
            .with(filter_layer)
            .with(azure_guard)
            .with(stdout_layer)
            .with(stderr_layer);
        tracing::subscriber::set_global_default(subscriber)?;
        Ok(Guard::new(None, None, None, stdout_guard, stderr_guard))
    };

    if let Ok(ref mut guard) = result {
        log_effective_config();
        guard.config_relog_handle = spawn_config_relog_task();
    }

    result
}

/// Spawn a background task that periodically re-logs the effective config (SRD 3.11).
///
/// Returns `None` if no tokio runtime is available or if the interval is zero (disabled).
fn spawn_config_relog_task() -> Option<tokio::task::JoinHandle<()>> {
    let interval = get_config_relog_interval();
    if interval.is_zero() {
        info_limited!("Periodic config re-logging is disabled (LOGS_CONFIG_RELOG_INTERVAL=0)");
        return None;
    }

    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        debug_limited!("No tokio runtime available -- skipping periodic config re-log task");
        return None;
    };

    Some(handle.spawn(async move {
        let mut timer = tokio::time::interval(interval);
        timer.tick().await; // first tick is immediate; skip it (we already logged at startup)
        loop {
            timer.tick().await;
            log_effective_config();
        }
    }))
}

/// Read the periodic config re-log interval from `LOGS_CONFIG_RELOG_INTERVAL`.
/// Default: 60 minutes. Set to `0s` to disable.
fn get_config_relog_interval() -> std::time::Duration {
    let default = std::time::Duration::from_secs(60 * 60);
    match std::env::var("LOGS_CONFIG_RELOG_INTERVAL") {
        Ok(s) => match s.parse::<humantime::Duration>() {
            Ok(d) => std::time::Duration::from(d),
            Err(e) => {
                warn_limited!(
                    value = %s,
                    error = %e,
                    "Invalid LOGS_CONFIG_RELOG_INTERVAL, using default of 60m"
                );
                default
            },
        },
        Err(_) => default,
    }
}

/// Same as `try_init_observability`, but panic if setup does not work.
pub fn init_observability(service_name: &str, service_version: &str) -> Guard {
    match try_init_observability(service_name, service_version) {
        Ok(guard) => guard,
        Err(error) => panic!("Got error initializing the observability: {error:?}"),
    }
}

/// Initialize observability using the caller's `CARGO_BIN_NAME` and `CARGO_PKG_VERSION`.
///
/// This macro captures both values at the call site, ensuring the correct application
/// name and version are used.
///
/// # Example
///
/// ```ignore
/// use storage_core_util::init_observability;
///
/// // Uses CARGO_BIN_NAME as service name and CARGO_PKG_VERSION as version
/// let _guard = init_observability!();
///
/// // Or specify a custom service name (still uses CARGO_PKG_VERSION)
/// let _guard = init_observability!("my-custom-service-name");
///
/// // Or specify both service name and version explicitly
/// let _guard = init_observability!("my-custom-service-name", "1.2.3");
/// ```
#[macro_export]
macro_rules! init_observability {
    () => {
        $crate::observability::init_observability(env!("CARGO_BIN_NAME"), env!("CARGO_PKG_VERSION"))
    };
    ($service_name:expr) => {
        $crate::observability::init_observability($service_name, env!("CARGO_PKG_VERSION"))
    };
    ($service_name:expr, $service_version:expr) => {
        $crate::observability::init_observability($service_name, $service_version)
    };
}

/// Initialize observability using the caller's `CARGO_BIN_NAME` and `CARGO_PKG_VERSION`.
///
/// This macro captures both values at the call site, ensuring the correct application
/// name and version are used. Returns a `Result` instead of panicking on error.
///
/// # Example
///
/// ```ignore
/// use storage_core_util::try_init_observability;
///
/// // Uses CARGO_BIN_NAME as service name and CARGO_PKG_VERSION as version
/// let _guard = try_init_observability!()?;
///
/// // Or specify a custom service name (still uses CARGO_PKG_VERSION)
/// let _guard = try_init_observability!("my-custom-service-name")?;
///
/// // Or specify both service name and version explicitly
/// let _guard = try_init_observability!("my-custom-service-name", "1.2.3")?;
/// ```
#[macro_export]
macro_rules! try_init_observability {
    () => {
        $crate::observability::try_init_observability(env!("CARGO_BIN_NAME"), env!("CARGO_PKG_VERSION"))
    };
    ($service_name:expr) => {
        $crate::observability::try_init_observability($service_name, env!("CARGO_PKG_VERSION"))
    };
    ($service_name:expr, $service_version:expr) => {
        $crate::observability::try_init_observability($service_name, $service_version)
    };
}

pub struct Guard {
    pub tracing_guard: Option<SdkTracerProvider>,
    pub metrics_guard: Option<SdkMeterProvider>,
    pub log_provider: Option<SdkLoggerProvider>,
    pub stdout_guard: Option<WorkerGuard>,
    pub stderr_guard: Option<WorkerGuard>,
    /// Background task for periodic config re-logging (SRD 3.11).
    config_relog_handle: Option<tokio::task::JoinHandle<()>>,
}

impl Guard {
    pub fn new(
        tracing_guard: Option<SdkTracerProvider>,
        metrics_guard: Option<SdkMeterProvider>,
        log_provider: Option<SdkLoggerProvider>,
        stdout_guard: Option<WorkerGuard>,
        stderr_guard: Option<WorkerGuard>,
    ) -> Self {
        Self {
            tracing_guard,
            metrics_guard,
            log_provider,
            stdout_guard,
            stderr_guard,
            config_relog_handle: None,
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(handle) = self.config_relog_handle.take() {
            handle.abort();
        }
        // Flush pending log exports on shutdown
        if let Some(ref provider) = self.log_provider {
            let _ = provider.force_flush();
        }
    }
}
