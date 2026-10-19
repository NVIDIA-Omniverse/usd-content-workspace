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

use envoy_auth_ext::settings::{parse_config, AppSettings};
use envoy_auth_ext::{build_idp_client, AppState, AuthServer};
use permission_auth::AuthCache;
use std::error::Error;
use std::sync::Arc;

const DEFAULT_SERVICE_NAME: &str = "envoy-auth-ext-service";
const DEFAULT_SERVICE_SCOPE: &str = "envoy_auth_ext";

async fn run(
    app_settings: AppSettings,
    meter: opentelemetry::metrics::Meter,
    state: AppState,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    AuthServer::new(app_settings, meter, state)
        .await?
        .start()
        .await
}

/// Main entry-point into starting the auth extension service
fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    match DefaultCliArgs::try_parse() {
        Ok(cli_args) => {
            let mut app_settings =
                parse_config(cli_args.config.as_deref(), cli_args.generate.as_deref())?;

            if let Ok(level) = std::env::var("RUST_LOG") {
                app_settings.logging.level = level;
            }

            let structured_logging = std::env::var("STRUCTURED_LOGGING")
                .map(|value| value.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let service_name =
                std::env::var("SERVICE_NAME").unwrap_or_else(|_| DEFAULT_SERVICE_NAME.to_string());
            let service_scope: &'static str = Box::leak(
                std::env::var("SERVICE_SCOPE")
                    .unwrap_or_else(|_| DEFAULT_SERVICE_SCOPE.to_string())
                    .into_boxed_str(),
            );

            // Build the Otel resource attributes
            let resource = otel_tools::resource::ResourceBuilder::new(service_name)
                .with_service_namespace("envoy-auth-extension-service")
                .with_service_version(env!("CARGO_PKG_VERSION"))
                .build();

            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build runtime");

            rt.block_on(init(
                resource,
                app_settings,
                structured_logging,
                service_scope,
            ))
        }
        Err(clap_err) => {
            clap_err.exit();
        }
    }
}

/// Asynchronous setup code.
async fn init(
    resource: otel_tools::resource::Resource,
    mut app_settings: AppSettings,
    structured_logging: bool,
    service_scope: &'static str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Trace/span exporter setup
    let traces_config = otel_tools::exporters::traces::TracesConfig::from_env();
    // optional, for debugging
    // eprintln!("{:#?}", traces_config);
    let traces_guard = traces_config.guard().with_resource(&resource).build();

    // Set up `tracing`/`tracing_subscriber`
    // This is what makes `tracing::info!` work, and where it goes
    // (We are working on a way to automate this)
    let env_filter_layer = tracing_subscriber::filter::EnvFilter::builder()
        .parse(&app_settings.logging.level)
        .expect("failed to create env filter");

    use tracing_subscriber::layer::SubscriberExt as _;
    if structured_logging {
        let logging_stdout_layer = otel_tools::layers::LoggingLayer::builder()
            .with_resource(&resource)
            .build();
        let tracer = traces_guard.tracer(service_scope);
        let otel_trace_layer = tracer.map(otel_tools::layers::OpenTelemetryLayer::new);
        let subscriber = tracing_subscriber::registry()
            .with(env_filter_layer)
            .with(logging_stdout_layer)
            .with(otel_trace_layer);
        tracing::subscriber::set_global_default(subscriber)
            .expect("failed to set global subscriber");
    } else {
        let tracer = traces_guard.tracer(service_scope);
        let otel_trace_layer = tracer.map(otel_tools::layers::OpenTelemetryLayer::new);
        let subscriber = tracing_subscriber::registry()
            .with(env_filter_layer)
            .with(tracing_subscriber::fmt::layer().pretty().with_ansi(false))
            .with(otel_trace_layer);
        tracing::subscriber::set_global_default(subscriber)
            .expect("failed to set global subscriber");
    }

    tracing::info!("Starting service on port {}", app_settings.server.port);

    // Metric exporter setup
    let metrics_config = otel_tools::exporters::metrics::MetricsConfig::from_env();
    // optional, for debugging
    // eprintln!("{:#?}", metrics_config);
    let metrics_guard = metrics_config.guard().with_resource(&resource).build();

    // An example of how to do process-level metrics
    let meter = metrics_guard.meter_or_noop(service_scope);
    let process_metrics = ProcessMetrics::new(&meter);

    let jwt_cache = AuthCache::new(
        app_settings.auth.token_cache_size,
        app_settings.auth.token_ttl,
        app_settings.auth.invalid_token_cache_size,
        app_settings.auth.invalid_token_cache_ttl,
    );
    let client = Arc::new(build_idp_client(&app_settings.idp_client)?);
    let state = AppState {
        cache: Arc::new(jwt_cache),
        client: Arc::clone(&client),
    };

    if let Err(err) = app_settings.auth.load(&client).await {
        tracing::error!("Failed to initialize authentication: {}", err);
        std::process::exit(1);
    }

    // Run the server
    // let res = server::run(app_settings, meter).await;
    let res = run(app_settings, meter, state).await;

    // Tear down process-level metrics (equivalent to drop)
    let _ = process_metrics;
    // Tear down trace export
    drop(traces_guard);
    // Tear down metrics export
    drop(metrics_guard);

    res
}

#[derive(Debug, Clone, Default, clap::Parser)]
#[command(arg_required_else_help(true))]
struct DefaultCliArgs {
    /// Config file to read.
    #[arg(long, short)]
    config: Option<String>,
    /// Config file to write.
    #[arg(long, short)]
    generate: Option<String>,
}

impl DefaultCliArgs {
    /// Try to parse from [`std::env::args_os()`].
    fn try_parse() -> Result<Self, clap::Error> {
        <Self as clap::Parser>::try_parse()
    }
}

#[expect(
    dead_code,
    reason = "observable metrics must be kept in scope for callback"
)]
struct ProcessMetrics {
    up: opentelemetry::metrics::ObservableGauge<i64>,
    uptime: opentelemetry::metrics::ObservableUpDownCounter<i64>,
}

impl ProcessMetrics {
    fn new(meter: &opentelemetry::metrics::Meter) -> Self {
        // up metric
        // CDEVS-85
        // see also Python scheduler impl
        let up = meter
            .i64_observable_gauge("up")
            .with_unit("{count}")
            .with_callback(|instrument| {
                instrument.observe(1, &[]);
            })
            .build();

        // process.uptime metric
        // https://github.com/open-telemetry/opentelemetry-specification/pull/2824#issuecomment-1258386816
        // this metric is kind of controversial; we can revisit this later. for
        // now, this metric exists to check metric exports work.
        let start = std::time::Instant::now();
        let uptime = meter
            .i64_observable_up_down_counter("process.uptime")
            .with_unit("s")
            .with_callback(move |instrument| {
                let measurement = start.elapsed().as_secs() as _;
                instrument.observe(measurement, &[]);
            })
            .build();

        Self { up, uptime }
    }
}
