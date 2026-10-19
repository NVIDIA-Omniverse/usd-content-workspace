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

use super::config::TracesConfig;
use super::guard::TracesGuard;
use crate::env;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::span_processor_with_async_runtime::BatchSpanProcessor;

/// A builder for a [`TracesGuard`].
///
/// This cannot be constructed directly, see [`TracesConfig::guard`].
#[derive(Debug, Clone)]
pub struct TracesGuardBuilder<'a> {
    config: &'a TracesConfig,
    resource: Option<&'a Resource>,
}

impl<'a> TracesGuardBuilder<'a> {
    pub(crate) fn new(config: &'a TracesConfig) -> Self {
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

    /// Initialize an OTLP traces exporter.
    ///
    /// Traces export is shut down when the returned guard is dropped. You must
    /// hold the guard for traces export to remain active.
    ///
    /// This is a batch exporter for multi-threaded tokio runtimes (rt), and
    /// must be called in an asynchronous context.
    ///
    /// Note: This method does *not* configure integration between
    /// `opentelemetry` and `tracing`. That must be done afterwards.
    ///
    /// No global state is set.
    #[must_use = "trace exporter is shut down when dropped"]
    pub fn build(self) -> TracesGuard {
        if self.config.is_disabled() {
            return TracesGuard(None);
        }

        let span_exporter = exporter(self.config).expect("failed to build OTLP span exporter");
        let span_processor = processor(span_exporter);

        let empty_resource = Resource::builder_empty().build();
        let resource = self.resource.unwrap_or(&empty_resource);
        let tracer_provider = tracer_provider(span_processor, resource);

        TracesGuard(Some(tracer_provider))
    }
}

pub(super) fn exporter(
    config: &TracesConfig,
) -> Result<opentelemetry_otlp::SpanExporter, opentelemetry_otlp::ExporterBuildError> {
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
            let mut builder = opentelemetry_otlp::SpanExporter::builder().with_tonic();
            // we don't currently customize the tonic config, but you could here
            // (types exposed to allow checking for changes in behavior)
            let _tonic_config = builder.tonic_config();
            let export_config = builder.export_config();
            export_config.protocol = protocol;
            // env vars respected in build
            //
            // compression:
            // 1. `tonic_config.compression`
            // 2. OTEL_EXPORTER_OTLP_TRACES_COMPRESSION
            // 3. OTEL_EXPORTER_OTLP_COMPRESSION
            // 4. None
            //
            // headers:
            // 1. OTEL_EXPORTER_OTLP_TRACES_HEADERS or OTEL_EXPORTER_OTLP_HEADERS
            // 2. env headers merged with `tonic_config.metadata` (env > metadata)
            //
            // endpoint (assuming a custom channel wasn't provided):
            // 1. OTEL_EXPORTER_OTLP_TRACES_ENDPOINT
            // 2. OTEL_EXPORTER_OTLP_ENDPOINT
            // 3. `export_config.endpoint`
            // 4. Default ("http://localhost:4317")
            //
            // timeout:
            // 1. OTEL_EXPORTER_OTLP_TRACES_TIMEOUT
            // 2. OTEL_EXPORTER_OTLP_TIMEOUT
            // 3. `export_config.timeout`
            // 4. Default (10 seconds)
            builder.build()
        }
        opentelemetry_otlp::Protocol::HttpJson | opentelemetry_otlp::Protocol::HttpBinary => {
            // this is so convoluted... the important type is
            // `HttpExporterBuilder`, which (currently) has two fields:
            // `HttpConfig` and `ExportConfig`
            let mut builder = opentelemetry_otlp::SpanExporter::builder().with_http();
            // we don't currently customize the http config, but you could here
            // (types exposed to allow checking for changes in behavior)
            let _http_config = builder.http_client_config();
            let export_config = builder.export_config();
            export_config.protocol = protocol;
            // env vars respected in build
            //
            // endpoint:
            // 1. OTEL_EXPORTER_OTLP_TRACES_ENDPOINT (unchanged)
            // 2. OTEL_EXPORTER_OTLP_ENDPOINT (has "/v1/traces" appended)
            // 3. `export_config.endpoint` (unchanged)
            // 4. Default ("http://localhost:4318/v1/traces")
            //
            // timeout:
            // 1. OTEL_EXPORTER_OTLP_TRACES_TIMEOUT
            // 2. OTEL_EXPORTER_OTLP_TIMEOUT
            // 3. `export_config.timeout`
            // 4. Default (10 seconds)
            //
            // headers:
            // 1. OTEL_EXPORTER_OTLP_TRACES_HEADERS or OTEL_EXPORTER_OTLP_HEADERS
            // 2. env headers merged with `http_config.headers` (env > headers)
            builder.build()
        }
    }
}

pub(super) fn processor(
    span_exporter: opentelemetry_otlp::SpanExporter,
) -> BatchSpanProcessor<opentelemetry_sdk::runtime::Tokio> {
    // Note: the `experimental_trace_batch_span_processor_with_async_runtime` BSP
    // is used because it has the same behavior as v0.27.1. In v0.29.0, the
    // "standard" BSP spawns a thread and uses blocking clients (!). As a
    // result, it also doesn't support `OTEL_BSP_EXPORT_TIMEOUT`.

    // `BatchSpanProcessor::builder()` is `BatchSpanProcessorBuilder { ... }`
    //
    // `BatchSpanProcessorBuilder` doesn't implement `Default` or read any env
    // vars, but does call `BatchConfig::default()` - we override this for
    // clarity, to make it easier to jump to the relevant code.
    let builder = BatchSpanProcessor::builder(span_exporter, opentelemetry_sdk::runtime::Tokio);

    // `BatchConfig::default()` is `BatchConfigBuilder::default().build()`
    // `BatchConfigBuilder::default()` calls `BatchConfigBuilder::init_from_env_vars()`
    //
    // * OTEL_BSP_MAX_CONCURRENT_EXPORTS
    // * OTEL_BSP_MAX_QUEUE_SIZE
    // * OTEL_BSP_SCHEDULE_DELAY
    // * OTEL_BSP_MAX_EXPORT_BATCH_SIZE
    // * OTEL_BSP_EXPORT_TIMEOUT (requires `experimental_trace_batch_span_processor_with_async_runtime`)
    let config = opentelemetry_sdk::trace::BatchConfigBuilder::default().build();
    let builder = builder.with_batch_config(config);

    // `BatchSpanProcessorBuilder::build()` calls `BatchSpanProcessor::new()`
    builder.build()
}

pub(super) fn tracer_provider(
    span_processor: BatchSpanProcessor<opentelemetry_sdk::runtime::Tokio>,
    resource: &opentelemetry_sdk::Resource,
) -> opentelemetry_sdk::trace::SdkTracerProvider {
    // this is so convoluted...
    //
    // `opentelemetry_sdk::trace::TracerProvider::builder()`
    // https://github.com/open-telemetry/opentelemetry-rust/blob/opentelemetry_sdk-0.27.1/opentelemetry-sdk/src/trace/provider.rs#L170-L173
    // `opentelemetry_sdk::trace::provider::Builder::default()`
    // https://github.com/open-telemetry/opentelemetry-rust/blob/opentelemetry_sdk-0.27.1/opentelemetry-sdk/src/trace/provider.rs#L284-L289
    // `opentelemetry_sdk::trace::config::Config::default()`
    // https://github.com/open-telemetry/opentelemetry-rust/blob/opentelemetry_sdk-0.27.1/opentelemetry-sdk/src/trace/config.rs#L137-L231
    //
    // * OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT
    // * OTEL_SPAN_EVENT_COUNT_LIMIT
    // * OTEL_SPAN_LINK_COUNT_LIMIT
    // * OTEL_TRACES_SAMPLER
    // * OTEL_TRACES_SAMPLER_ARG
    //
    //
    // we cannot retrieve the config, and so have no access to the resource,
    // sampler, id_generator, or span limits...

    // `SdkTracerProvider::builder()` calls `TracerProviderBuilder::default()`
    // this implicitly calls `opentelemetry_sdk::trace::Config::default()`.
    // unfortunately, the builder has no `with_config()` method, so the below
    // line is only there to help inspect the code.
    let _config = opentelemetry_sdk::trace::Config::default();
    // `Config::default()` calls `SpanLimits::default()`, which currently
    // doesn't read any env vars
    //
    // however, `Config::default()` reads the following vars:
    //
    // * OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT
    // * OTEL_SPAN_EVENT_COUNT_LIMIT
    // * OTEL_SPAN_LINK_COUNT_LIMIT
    // * OTEL_TRACES_SAMPLER / OTEL_TRACES_SAMPLER_ARG
    //
    // it doesn't read:
    //
    // * OTEL_EVENT_ATTRIBUTE_COUNT_LIMIT
    // * OTEL_LINK_ATTRIBUTE_COUNT_LIMIT
    // * OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT (unsupported/no way to set that)
    let mut builder = opentelemetry_sdk::trace::SdkTracerProvider::builder();

    if let Some(max_attributes) = env::traces::otel_event_attribute_count_limit() {
        builder = builder.with_max_attributes_per_event(max_attributes);
    }
    if let Some(max_attributes) = env::traces::otel_link_attribute_count_limit() {
        builder = builder.with_max_attributes_per_link(max_attributes);
    }

    builder
        .with_resource(resource.clone())
        .with_span_processor(span_processor)
        .build()
}
