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

use super::config::TracingConfig;
use crate::exporters::logs::LogsGuard;
use crate::exporters::traces::TracesGuard;
use crate::layers::LoggingLayer;
use crate::layers::logging::Stdout;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::Resource;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

fn env_filter_opt(src: &str, dirs: &str) -> Option<EnvFilter> {
    if dirs.is_empty() {
        eprintln!("{src} filter directives is empty (did you mean `off`?)");
        return None;
    }

    match EnvFilter::builder().parse(dirs) {
        Ok(filter) => Some(filter),
        Err(e) => {
            eprintln!("{src} filter directives `{dirs}` are invalid: {e}");
            None
        }
    }
}

fn env_filter_req(src: &str, dirs: &str) -> EnvFilter {
    if dirs.is_empty() {
        panic!("{src} filter directives is empty (did you mean `off`?)");
    }

    EnvFilter::builder().parse(dirs).unwrap_or_else(|e| {
        panic!("{src} filter directives `{dirs}` are invalid: {e}");
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EnvFilterSource<'a> {
    pub(crate) dirs: Option<&'a str>,
    pub(crate) src: &'static str,
}

impl EnvFilterSource<'_> {
    fn parse(self, default_dirs: &str) -> EnvFilter {
        self.dirs
            .and_then(|dirs| env_filter_opt(self.src, dirs))
            .unwrap_or_else(|| env_filter_req("default", default_dirs))
    }
}

/// A builder for a tracing subscriber.
///
/// This cannot be constructed directly, see [`TracingConfig::subscriber`].
#[derive(Debug, Clone)]
pub struct TracingSubscriberBuilder<'a, M = Stdout> {
    tracing_config: &'a TracingConfig,
    traces_guard: Option<&'a TracesGuard>,
    logs_guard: Option<&'a LogsGuard>,
    resource: Option<&'a Resource>,
    default_dirs: &'a str,
    with_target: bool,
    writer: M,
}

impl<'a> TracingSubscriberBuilder<'a, Stdout> {
    pub(crate) fn new(tracing_config: &'a TracingConfig) -> Self {
        Self {
            tracing_config,
            traces_guard: None,
            logs_guard: None,
            resource: None,
            default_dirs: "info",
            with_target: false,
            writer: Stdout,
        }
    }

    /// Set the default env filter directives for all layers. These directives
    /// are used for all layers, although override directives may be specified
    /// via env vars.
    ///
    /// If no default env filter directives are provided, the default is `info`.
    ///
    /// You must ensure that the directives are valid, otherwise the code may
    /// panic when constructing an env filter at some later stage. It's
    /// recommended that you have a test to ensure this:
    ///
    /// ```rust
    /// use tracing_subscriber::filter::EnvFilter;
    /// const DIRS: &str = "my_module=info";
    /// #[test]
    /// fn default_dirs_valid() {
    ///     EnvFilter::builder().parse(DIRS).unwrap();
    /// }
    /// ```
    #[expect(clippy::test_attr_in_doctest)]
    #[must_use]
    pub fn with_default_env_filter(mut self, dirs: &'a str) -> Self {
        self.default_dirs = dirs;
        self
    }

    /// Enable OTLP trace export, i.e. forwarding tracing spans to an OTLP
    /// trace exporter.
    ///
    /// If no guard is provided, tracing spans are not forwarded!
    #[must_use]
    pub fn with_traces(mut self, traces_guard: &'a TracesGuard) -> Self {
        self.traces_guard = Some(traces_guard);
        self
    }

    /// Enable OTLP logs export, i.e. forwarding tracing events to an OTLP
    /// logs exporter.
    ///
    /// If no guard is provided, tracing events are not forwarded!
    #[must_use]
    pub fn with_logs(mut self, logs_guard: &'a LogsGuard) -> Self {
        self.logs_guard = Some(logs_guard);
        self
    }

    /// Resource attributes to use for console logging only.
    #[must_use]
    pub fn with_resource(mut self, resource: &'a Resource) -> Self {
        self.resource = Some(resource);
        self
    }

    /// Whether to emit the `tracing` span target as an attribute in exported
    /// OpenTelemetry spans (default: false).
    #[must_use]
    pub fn with_span_target(mut self, target: bool) -> Self {
        self.with_target = target;
        self
    }

    /// Redirect logging output from stdout to the provided writer.
    ///
    /// This breaks the usual convention, and is only recommended for testing.
    #[doc(hidden)]
    #[must_use]
    pub fn with_make_writer<M>(self, make_writer: M) -> TracingSubscriberBuilder<'a, M>
    where
        M: for<'b> MakeWriter<'b> + Send + Sync + 'static,
    {
        TracingSubscriberBuilder {
            tracing_config: self.tracing_config,
            traces_guard: self.traces_guard,
            logs_guard: self.logs_guard,
            resource: self.resource,
            default_dirs: self.default_dirs,
            with_target: self.with_target,
            writer: make_writer,
        }
    }
}

impl<M> TracingSubscriberBuilder<'_, M>
where
    M: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    /// Build a tracing subscriber. It's recommended to use
    /// [`build`](Self::build) instead.
    ///
    /// The `name` should be the application name or the name of the library
    /// providing instrumentation. This is used to initialize the tracer for
    /// OTLP trace exports.
    #[must_use]
    pub fn build_subscriber(
        self,
        name: &'static str,
    ) -> impl tracing::Subscriber + Send + Sync + 'static + use<M> {
        use tracing_subscriber::layer::{Layer as _, SubscriberExt as _};

        let console_layer = self.tracing_config.console_enabled().then(|| {
            let mut builder = LoggingLayer::builder().with_make_writer(self.writer);

            if let Some(resource) = self.resource {
                builder = builder.with_resource(resource);
            }

            let layer = builder.build();

            let filter = self
                .tracing_config
                .console_filter()
                .parse(self.default_dirs);

            layer.with_filter(filter)
        });

        let traces_layer = self
            .traces_guard
            .and_then(|guard| guard.tracer(name))
            .map(|tracer| {
                let layer = OpenTelemetryLayer::new(tracer).with_target(self.with_target);

                let filter = self
                    .tracing_config
                    .otlp_traces_filter()
                    .parse(self.default_dirs);

                layer.with_filter(filter)
            });

        let logs_layer = self
            .logs_guard
            .and_then(LogsGuard::provider)
            .map(|provider| {
                let layer = OpenTelemetryTracingBridge::new(provider);

                let filter = self
                    .tracing_config
                    .otlp_logs_filter()
                    .parse(self.default_dirs);

                layer.with_filter(filter)
            });

        tracing_subscriber::registry()
            .with(traces_layer)
            .with(logs_layer)
            .with(console_layer)
    }

    /// Build a tracing subscriber and set as the global default.
    ///
    /// The `name` should be the application name or the name of the library
    /// providing instrumentation. This is used to initialize the tracer for
    /// OTLP trace exports.
    ///
    /// # Panics
    ///
    /// This method panics if [`tracing::subscriber::set_global_default`]
    /// returns an error. This occurs if the global default has already been
    /// set.
    pub fn build(self, name: &'static str) {
        let subscriber = self.build_subscriber(name);
        tracing::subscriber::set_global_default(subscriber)
            .expect("failed to set global tracing subscriber")
    }
}

#[cfg(test)]
mod tests;
