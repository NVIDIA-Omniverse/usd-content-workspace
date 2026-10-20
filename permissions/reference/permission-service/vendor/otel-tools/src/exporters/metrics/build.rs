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

use super::config::MetricsConfig;
use super::guard::MetricsGuard;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::periodic_reader_with_async_runtime::PeriodicReader;

/// A builder for a [`MetricsGuard`].
///
/// This cannot be constructed directly, see [`MetricsConfig::guard`].
#[derive(Debug, Clone)]
pub struct MetricsGuardBuilder<'a> {
    config: &'a MetricsConfig,
    resource: Option<&'a Resource>,
}

impl<'a> MetricsGuardBuilder<'a> {
    pub(crate) fn new(config: &'a MetricsConfig) -> Self {
        Self {
            config,
            resource: None,
        }
    }

    /// Resource attributes to use for the exporter.
    #[must_use]
    pub fn with_resource(mut self, resource: &'a Resource) -> Self {
        self.resource = Some(resource);
        self
    }

    /// Initialize an OTLP metrics exporter.
    ///
    /// Metrics export is shut down when the returned guard is dropped. You must
    /// hold the guard for metrics export to remain active.
    ///
    /// This is a periodic reader for multi-threaded tokio runtimes (rt), and
    /// must be called in an asynchronous context.
    ///
    /// No global state is set.
    #[must_use = "metric exporter is shut down when dropped"]
    pub fn build(self) -> MetricsGuard {
        if self.config.is_disabled() {
            return MetricsGuard(None);
        }

        let metric_exporter = exporter(self.config).expect("failed to build OTLP metric exporter");
        let periodic_reader = reader(metric_exporter);

        let empty_resource = Resource::builder_empty().build();
        let resource = self.resource.unwrap_or(&empty_resource);
        let meter_provider = meter_provider(periodic_reader, resource);

        MetricsGuard(Some(meter_provider))
    }
}

pub(super) fn exporter(
    config: &MetricsConfig,
) -> Result<opentelemetry_otlp::MetricExporter, opentelemetry_otlp::ExporterBuildError> {
    let protocol = config
        .protocol()
        .unwrap_or(opentelemetry_otlp::Protocol::Grpc);

    // build the exporter depending on protocol
    use opentelemetry_otlp::{HasExportConfig as _, HasHttpConfig as _, HasTonicConfig as _};
    match protocol {
        opentelemetry_otlp::Protocol::Grpc => {
            // this is so convoluted... the important type is
            // `TonicExporterBuilder`, which (currently) has two fields:
            // `TonicConfig` and `ExportConfig`
            // as of v0.29.0, the builder does not read and env vars
            let mut builder = opentelemetry_otlp::MetricExporter::builder().with_tonic();
            // we don't currently customize the tonic config, but you could here
            // (types exposed to allow checking for changes in behavior)
            let _tonic_config = builder.tonic_config();
            let export_config = builder.export_config();
            export_config.protocol = protocol;
            // env vars respected in build
            //
            // compression:
            // 1. `tonic_config.compression`
            // 2. OTEL_EXPORTER_OTLP_METRICS_COMPRESSION
            // 3. OTEL_EXPORTER_OTLP_COMPRESSION
            // 4. None
            //
            // headers:
            // 1. OTEL_EXPORTER_OTLP_METRICS_HEADERS or OTEL_EXPORTER_OTLP_HEADERS
            // 2. env headers merged with `tonic_config.metadata` (env > metadata)
            //
            // endpoint (assuming a custom channel wasn't provided):
            // 1. OTEL_EXPORTER_OTLP_METRICS_ENDPOINT
            // 2. OTEL_EXPORTER_OTLP_ENDPOINT
            // 3. `export_config.endpoint`
            // 4. Default ("http://localhost:4317")
            //
            // timeout:
            // 1. OTEL_EXPORTER_OTLP_METRICS_TIMEOUT
            // 2. OTEL_EXPORTER_OTLP_TIMEOUT
            // 3. `export_config.timeout`
            // 4. Default (10 seconds)
            builder.build()
        }
        opentelemetry_otlp::Protocol::HttpJson | opentelemetry_otlp::Protocol::HttpBinary => {
            // this is so convoluted... the important type is
            // `HttpExporterBuilder`, which (currently) has two fields:
            // `HttpConfig` and `ExportConfig`
            let mut builder = opentelemetry_otlp::MetricExporter::builder().with_http();
            // we don't currently customize the http config, but you could here
            // (types exposed to allow checking for changes in behavior)
            let _http_config = builder.http_client_config();
            let export_config = builder.export_config();
            export_config.protocol = protocol;
            // env vars respected in build
            //
            // endpoint:
            // 1. OTEL_EXPORTER_OTLP_METRICS_ENDPOINT (unchanged)
            // 2. OTEL_EXPORTER_OTLP_ENDPOINT (has "/v1/metrics" appended)
            // 3. `export_config.endpoint` (unchanged)
            // 4. Default ("http://localhost:4318/v1/metrics")
            //
            // timeout:
            // 1. OTEL_EXPORTER_OTLP_METRICS_TIMEOUT
            // 2. OTEL_EXPORTER_OTLP_TIMEOUT
            // 3. `export_config.timeout`
            // 4. Default (10 seconds)
            //
            // headers:
            // 1. OTEL_EXPORTER_OTLP_METRICS_HEADERS or OTEL_EXPORTER_OTLP_HEADERS
            // 2. env headers merged with `http_config.metadata` (env > metadata)
            builder.build()
        }
    }
}

pub(super) fn reader(
    metric_exporter: opentelemetry_otlp::MetricExporter,
) -> PeriodicReader<opentelemetry_otlp::MetricExporter> {
    // Note: the `experimental_metrics_periodicreader_with_async_runtime` reader
    // is used because it has the same behavior as v0.27.1. In v0.29.0, the
    // "standard" reader spawns a thread and uses blocking clients (!). As a
    // result, it also doesn't support `OTEL_METRIC_EXPORT_TIMEOUT`.

    // not as bad as tracing...
    //
    // `PeriodicReader::builder()` calls `PeriodicReaderBuilder::new()`
    //
    // * OTEL_METRIC_EXPORT_INTERVAL
    // * OTEL_METRIC_EXPORT_TIMEOUT (requires `experimental_metrics_periodicreader_with_async_runtime`)
    let builder = PeriodicReader::builder(metric_exporter, opentelemetry_sdk::runtime::Tokio);

    // `PeriodicReaderBuilder::build()` just makes a `PeriodicReader` directly
    builder.build()
}

pub(super) fn meter_provider(
    reader: PeriodicReader<opentelemetry_otlp::MetricExporter>,
    resource: &opentelemetry_sdk::Resource,
) -> opentelemetry_sdk::metrics::SdkMeterProvider {
    // `SdkMeterProvider::builder()` calls `MeterProviderBuilder::default()`
    //
    // currently, this is:
    // ```rust
    // #[derive(Default)]
    // pub struct MeterProviderBuilder {
    //     resource: Option<Resource>,
    //     readers: Vec<Box<dyn MetricReader>>,
    //     views: Vec<Arc<dyn View>>,
    // }
    // ```
    //
    // so nothing is read here
    let builder = opentelemetry_sdk::metrics::SdkMeterProvider::builder();
    builder
        .with_reader(reader)
        .with_resource(resource.clone())
        .build()
}
