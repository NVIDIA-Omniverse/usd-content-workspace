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

use super::config::LogsConfig;
use super::guard::LogsGuard;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::log_processor_with_async_runtime::BatchLogProcessor;

/// A builder for a [`LogsGuard`].
///
/// This cannot be constructed directly, see [`LogsConfig::guard`].
#[derive(Debug, Clone)]
pub struct LogsGuardBuilder<'a> {
    config: &'a LogsConfig,
    resource: Option<&'a Resource>,
}

impl<'a> LogsGuardBuilder<'a> {
    pub(crate) fn new(config: &'a LogsConfig) -> Self {
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

    /// Initialize an OTLP logs exporter.
    ///
    /// Logs export is shut down when the returned guard is dropped. You must
    /// hold the guard for logs export to remain active.
    ///
    /// This is a batch log processor for multi-threaded tokio runtimes (rt),
    /// and must be called in an asynchronous context.
    ///
    /// No global state is set.
    #[must_use = "log exporter is shut down when dropped"]
    pub fn build(self) -> LogsGuard {
        if self.config.is_disabled() {
            return LogsGuard(None);
        }

        let log_exporter = exporter(self.config).expect("failed to build OTLP log exporter");
        let log_processor = processor(log_exporter);

        let empty_resource = Resource::builder_empty().build();
        let resource = self.resource.unwrap_or(&empty_resource);
        let logger_provider = logger_provider(log_processor, resource);

        LogsGuard(Some(logger_provider))
    }
}

pub(super) fn exporter(
    config: &LogsConfig,
) -> Result<opentelemetry_otlp::LogExporter, opentelemetry_otlp::ExporterBuildError> {
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
            let mut builder = opentelemetry_otlp::LogExporter::builder().with_tonic();
            // we don't currently customize the tonic config, but you could here
            // (types exposed to allow checking for changes in behavior)
            let _tonic_config = builder.tonic_config();
            let export_config = builder.export_config();
            export_config.protocol = protocol;
            // env vars respected in build
            //
            // compression:
            // 1. `tonic_config.compression`
            // 2. OTEL_EXPORTER_OTLP_LOGS_COMPRESSION
            // 3. OTEL_EXPORTER_OTLP_COMPRESSION
            // 4. None
            //
            // headers:
            // 1. OTEL_EXPORTER_OTLP_LOGS_HEADERS or OTEL_EXPORTER_OTLP_HEADERS
            // 2. env headers merged with `tonic_config.metadata` (env > metadata)
            //
            // endpoint (assuming a custom channel wasn't provided):
            // 1. OTEL_EXPORTER_OTLP_LOGS_ENDPOINT
            // 2. OTEL_EXPORTER_OTLP_ENDPOINT
            // 3. `export_config.endpoint`
            // 4. Default ("http://localhost:4317")
            //
            // timeout:
            // 1. OTEL_EXPORTER_OTLP_LOGS_TIMEOUT
            // 2. OTEL_EXPORTER_OTLP_TIMEOUT
            // 3. `export_config.timeout`
            // 4. Default (10 seconds)
            builder.build()
        }
        opentelemetry_otlp::Protocol::HttpJson | opentelemetry_otlp::Protocol::HttpBinary => {
            // this is so convoluted... the important type is
            // `HttpExporterBuilder`, which (currently) has two fields:
            // `HttpConfig` and `ExportConfig`
            // as of v0.29.0, the builder does not read and env vars
            let mut builder = opentelemetry_otlp::LogExporter::builder().with_http();
            // we don't currently customize the http config, but you could here
            // (types exposed to allow checking for changes in behavior)
            let _http_config = builder.http_client_config();
            let export_config = builder.export_config();
            export_config.protocol = protocol;
            // env vars respected in build
            //
            // endpoint:
            // 1. OTEL_EXPORTER_OTLP_LOGS_ENDPOINT (unchanged)
            // 2. OTEL_EXPORTER_OTLP_ENDPOINT (has "/v1/logs" appended)
            // 3. `export_config.endpoint` (unchanged)
            // 4. Default ("http://localhost:4318/v1/logs")
            //
            // timeout:
            // 1. OTEL_EXPORTER_OTLP_LOGS_TIMEOUT
            // 2. OTEL_EXPORTER_OTLP_TIMEOUT
            // 3. `export_config.timeout`
            // 4. Default (10 seconds)
            //
            // headers:
            // 1. OTEL_EXPORTER_OTLP_LOGS_HEADERS or OTEL_EXPORTER_OTLP_HEADERS
            // 2. env headers merged with `http_config.metadata` (env > metadata)
            builder.build()
        }
    }
}

pub(super) fn processor(
    log_exporter: opentelemetry_otlp::LogExporter,
) -> BatchLogProcessor<opentelemetry_sdk::runtime::Tokio> {
    // Note: the `experimental_logs_batch_log_processor_with_async_runtime` BLP
    // is used because it has the same behavior as v0.27.1. In v0.29.0, the
    // "standard" BLP spawns a thread and uses blocking clients (!). As a
    // result, it also doesn't support `OTEL_BLRP_EXPORT_TIMEOUT`.

    // not as bad as tracing...
    //
    // `BatchLogProcessor::builder()` is `BatchLogProcessorBuilder { ... }`
    //
    // `BatchLogProcessorBuilder` doesn't implement `Default` or read any env
    // vars, but does call `BatchConfig::default()` - we override this for
    // clarity, to make it easier to jump to the relevant code.
    let builder = BatchLogProcessor::builder(log_exporter, opentelemetry_sdk::runtime::Tokio);

    // `BatchConfig::default()` is `BatchConfigBuilder::default().build()`
    // `BatchConfigBuilder::default()` calls `BatchConfigBuilder::init_from_env_vars()`
    //
    // * OTEL_BLRP_MAX_QUEUE_SIZE
    // * OTEL_BLRP_MAX_EXPORT_BATCH_SIZE
    // * OTEL_BLRP_SCHEDULE_DELAY
    // * OTEL_BLRP_EXPORT_TIMEOUT (requires `experimental_logs_batch_log_processor_with_async_runtime`)
    let config = opentelemetry_sdk::logs::BatchConfigBuilder::default().build();
    let builder = builder.with_batch_config(config);

    // `BatchLogProcessorBuilder::build()` calls `BatchLogProcessor::new()`
    builder.build()
}

pub(super) fn logger_provider(
    processor: BatchLogProcessor<opentelemetry_sdk::runtime::Tokio>,
    resource: &opentelemetry_sdk::Resource,
) -> opentelemetry_sdk::logs::SdkLoggerProvider {
    // `SdkLoggerProvider::builder()` calls `LoggerProviderBuilder::default()`
    //
    // currently, this is:
    // ```rust
    // #[derive(Debug, Default)]
    // /// Builder for provider attributes.
    // pub struct LoggerProviderBuilder {
    //     processors: Vec<Box<dyn LogProcessor>>,
    //     resource: Option<Resource>,
    // }
    // ```
    //
    // so nothing is read here
    let builder = opentelemetry_sdk::logs::SdkLoggerProvider::builder();
    builder
        .with_log_processor(processor)
        .with_resource(resource.clone())
        .build()
}
