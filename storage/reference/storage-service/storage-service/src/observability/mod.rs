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

use std::net::{Ipv4Addr, SocketAddrV4};

use init_tracing_opentelemetry::tracing_subscriber_ext::{
    build_metrics_layer_with_resource, build_tracer_layer_with_resource,
};
use opentelemetry::KeyValue;
use opentelemetry_prometheus_text_exporter::PrometheusExporter;
use opentelemetry_sdk::{metrics::SdkMeterProvider, trace::SdkTracerProvider, Resource};
use serde::Deserialize;

use error::Error;
use tracing::{debug, info, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, registry::LookupSpan, EnvFilter, Layer};

mod logger;
mod prometheus;

pub mod error;

/// Observability background worker guard.
#[derive(Debug)]
pub struct Guard {
    _meter_provider: Option<SdkMeterProvider>,
    _tracer_provider: Option<SdkTracerProvider>,
    _prometheus_handle: Option<prometheus::Handle>,
    _logs_guard: Option<WorkerGuard>,
}

/// Initialize tracing from env.
pub async fn init_observability(service_name: &'static str, service_version: &'static str) -> Result<Guard, Error> {
    // Override the default logger to catch wireup errors.
    let (default_logger_layer, default_logger_write_guard) = create_stdout_logger()?;
    let default_logger_guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(default_logger_layer));

    let config: OtelConfig = envy::from_env().map_err(Error::environment_parse)?;
    debug!(?config, "otel configuration parsed");

    let resource = Resource::builder()
        .with_service_name(service_name)
        .with_attribute(KeyValue::new("service.version", service_version))
        .with_attribute(KeyValue::new("service.namespace", "omniverse"))
        .build();
    let init_result = config.init_with_resource(resource)?;

    drop(default_logger_write_guard);
    drop(default_logger_guard);

    let prometheus_handle = if let Some(exporter) = init_result.prometheus_exporter {
        let handle = prometheus::start_prometheus_service(
            exporter,
            SocketAddrV4::new(
                config
                    .otel_exporter_prometheus_host
                    .unwrap_or(Ipv4Addr::new(0, 0, 0, 0)),
                config.otel_exporter_prometheus_port.unwrap_or(9464),
            ),
        )
        .await?;
        Some(handle)
    } else {
        None
    };

    Ok(Guard {
        _meter_provider: init_result.meter_provider,
        _tracer_provider: init_result.tracer_provider,
        _prometheus_handle: prometheus_handle,
        _logs_guard: init_result.logs_guard,
    })
}

/// Trace exporter kinds.
#[derive(Default, Deserialize, Debug, Copy, Clone)]
enum TracesExporter {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "otlp")]
    Otlp,
}

/// Metric exporter kinds.
#[derive(Default, Deserialize, Debug, Copy, Clone)]
enum MetricsExporter {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "otlp")]
    Otlp,
    #[serde(rename = "prometheus")]
    Prometheus,
}

/// Logs exporter kinds.
#[derive(Default, Deserialize, Debug, Copy, Clone)]
enum LogsExporter {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "otlp")]
    Otlp,
}

/// Open Telemetry settings.
#[derive(Default, Deserialize, Debug)]
#[serde(default)]
struct OtelConfig {
    otel_sdk_disabled: bool,
    otel_traces_exporter: TracesExporter,
    otel_metrics_exporter: MetricsExporter,
    otel_logs_exporter: LogsExporter,
    otel_exporter_prometheus_host: Option<Ipv4Addr>,
    otel_exporter_prometheus_port: Option<u16>,
}

impl OtelConfig {
    fn init_with_resource(&self, resource: Resource) -> Result<OtelInitResult, Error> {
        let mut result = Default::default();
        self.continue_with_metrics_exporter(&mut result, resource, tracing_subscriber::registry())?;

        Ok(result)
    }

    fn continue_with_metrics_exporter<S>(
        &self,
        result: &mut OtelInitResult,
        resource: Resource,
        subscriber: S,
    ) -> Result<(), Error>
    where
        S: 'static + Send + Sync + Subscriber + for<'a> LookupSpan<'a>,
    {
        let exporter = if self.otel_sdk_disabled {
            MetricsExporter::None
        } else {
            self.otel_metrics_exporter
        };

        match exporter {
            MetricsExporter::None => {
                info!("no metrics exporter selected");

                self.continue_with_logs_exporter(result, resource, subscriber)
            },
            MetricsExporter::Otlp => {
                info!("configuring otlp metrics exporter");

                let (layer, layer_guard) =
                    build_metrics_layer_with_resource(resource.clone()).map_err(Error::metrics_exporter_init)?;
                result.meter_provider = Some(layer_guard);

                self.continue_with_logs_exporter(result, resource, subscriber.with(layer))
            },
            MetricsExporter::Prometheus => {
                info!("configuring prometheus metrics exporter");

                let exporter = PrometheusExporter::new();
                let provider = SdkMeterProvider::builder()
                    .with_resource(resource.clone())
                    .with_reader(exporter.clone())
                    .build();

                result.prometheus_exporter = Some(exporter);

                opentelemetry::global::set_meter_provider(provider.clone());

                self.continue_with_logs_exporter(result, resource, subscriber)
            },
        }
    }

    fn continue_with_logs_exporter<S>(
        &self,
        result: &mut OtelInitResult,
        resource: Resource,
        subscriber: S,
    ) -> Result<(), Error>
    where
        S: 'static + Send + Sync + Subscriber + for<'a> LookupSpan<'a>,
    {
        let exporter = if self.otel_sdk_disabled {
            LogsExporter::None
        } else {
            self.otel_logs_exporter
        };

        let (stdout_logger_layer, logs_guard) = create_stdout_logger()?;
        result.logs_guard = Some(logs_guard);

        // todo: support otlp logs exporter.
        match exporter {
            LogsExporter::None => {
                info!("no logs exporter selected");

                self.continue_with_traces_exporter(result, resource, subscriber.with(stdout_logger_layer))
            },
            LogsExporter::Otlp => {
                info!("configuring otlp logs exporter");

                let otel_logger_layer = logger::make_otlp_logger_layer(resource.clone())?;

                self.continue_with_traces_exporter(
                    result,
                    resource,
                    subscriber.with(stdout_logger_layer).with(otel_logger_layer),
                )
            },
        }
    }

    fn continue_with_traces_exporter<S>(
        &self,
        result: &mut OtelInitResult,
        resource: Resource,
        subscriber: S,
    ) -> Result<(), Error>
    where
        S: 'static + Send + Sync + Subscriber + for<'a> LookupSpan<'a>,
    {
        let exporter = if self.otel_sdk_disabled {
            TracesExporter::None
        } else {
            self.otel_traces_exporter
        };

        match exporter {
            TracesExporter::None => {
                info!("no traces exporter selected");

                self.continue_with_log_filter(subscriber)
            },
            TracesExporter::Otlp => {
                info!("configuring otlp traces exporter");

                let (layer, layer_guard) =
                    build_tracer_layer_with_resource(resource).map_err(Error::traces_exporter_init)?;
                result.tracer_provider = Some(layer_guard);

                self.continue_with_log_filter(subscriber.with(layer))
            },
        }
    }

    fn continue_with_log_filter<S>(&self, subscriber: S) -> Result<(), Error>
    where
        S: 'static + Send + Sync + Subscriber + for<'a> LookupSpan<'a>,
    {
        let filter_layer = logger::create_merged_log_filter();
        self.continue_with_global_subscriber(subscriber.with(filter_layer))
    }

    fn continue_with_global_subscriber<S>(&self, subscriber: S) -> Result<(), Error>
    where
        S: 'static + Send + Sync + Subscriber + for<'a> LookupSpan<'a>,
    {
        tracing::subscriber::set_global_default(subscriber).map_err(Error::set_global_subscriber)
    }
}

#[derive(Default)]
struct OtelInitResult {
    meter_provider: Option<SdkMeterProvider>,
    tracer_provider: Option<SdkTracerProvider>,
    prometheus_exporter: Option<PrometheusExporter>,
    logs_guard: Option<WorkerGuard>,
}

fn create_stdout_logger<S>() -> Result<(Box<dyn Layer<S> + Send + Sync + 'static>, WorkerGuard), Error>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    use tracing_appender::non_blocking::NonBlockingBuilder;
    use tracing_subscriber::fmt::{layer, time::ChronoLocal};

    let filter = EnvFilter::from_default_env();
    let (writer, guard) = NonBlockingBuilder::default().lossy(false).finish(std::io::stdout());
    let use_json = std::env::var("STRUCTURED_LOGGING").as_deref() == Ok("true");
    let layer: Box<dyn Layer<S> + Send + Sync + 'static> = if use_json {
        Box::new(
            layer()
                .json()
                .with_writer(writer)
                .with_current_span(true)
                .with_filter(filter),
        )
    } else {
        Box::new(
            layer()
                .with_writer(writer)
                .with_level(true)
                .with_line_number(true)
                .with_thread_ids(false)
                .with_thread_names(false)
                .with_timer(ChronoLocal::new("%Y-%m-%dT%H:%M:%S%.3f%z".into()))
                .with_filter(filter),
        )
    };

    Ok((layer, guard))
}
