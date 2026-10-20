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

use super::field_visitor::FieldVisitor;
use super::message_visitor::MessageVisitor;
use super::otel2json::otel_resource;
use opentelemetry::trace::{SpanId, TraceId};
use serde::ser::{SerializeMap as _, Serializer};
use std::io::Write as _;
use time::format_description::well_known::Rfc3339;
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_opentelemetry::OtelData;
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::SpanRef;

const MESSAGE: &str = "message";
const LEVEL: &str = "level";
const TIMESTAMP: &str = "timestamp";
const TARGET: &str = "target";
const LINE: &str = "line";
const FILE: &str = "file";
const TRACE_ID: &str = "trace_id";
const SPAN_ID: &str = "span_id";

const RESERVED: &[&str] = &[
    MESSAGE, LEVEL, TIMESTAMP, TARGET, LINE, FILE, TRACE_ID, SPAN_ID,
];

fn level(level: &Level) -> &'static str {
    match *level {
        Level::TRACE => "trace",
        Level::DEBUG => "debug",
        Level::INFO => "info",
        Level::WARN => "warn",
        Level::ERROR => "error",
    }
}

fn format_event_message<S>(current_span: &Option<SpanRef<S>>, visitor: MessageVisitor) -> String
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    let message = visitor.message.unwrap_or_else(|| "<empty>".to_string());
    // if the event is in the context of a span, prepend the span name to the message
    match current_span {
        Some(span) => format!("[{}] {}", span.metadata().name(), message),
        None => message,
    }
}

fn span_ext_get_trace_context(
    extensions: tracing_subscriber::registry::Extensions<'_>,
) -> Option<(TraceId, SpanId)> {
    let otel_data = extensions.get::<OtelData>()?;
    let trace_id = otel_data
        .trace_id()
        .filter(|trace_id| *trace_id != TraceId::INVALID)?;

    let span_id = otel_data
        .span_id()
        .filter(|span_id| *span_id != SpanId::INVALID)?;

    Some((trace_id, span_id))
}

/// The builder for [`LoggingLayer`].
#[derive(Debug)]
pub struct LoggingLayerBuilder<W: for<'a> MakeWriter<'a> + 'static> {
    make_writer: W,
    /// Otel resource attributes, converted to JSON values.
    resource: Vec<(String, serde_json::Value)>,
}

impl<W: for<'a> MakeWriter<'a> + 'static> LoggingLayerBuilder<W> {
    /// Replace the current [`MakeWriter`] with a different one.
    ///
    /// Note that this changes the type signature.
    pub fn with_make_writer<M: for<'a> MakeWriter<'a> + 'static>(
        self,
        make_writer: M,
    ) -> LoggingLayerBuilder<M> {
        LoggingLayerBuilder {
            make_writer,
            resource: self.resource,
        }
    }

    /// Set the resource attributes for logging.
    pub fn with_resource(mut self, resource: &opentelemetry_sdk::Resource) -> Self {
        self.resource = otel_resource(resource, RESERVED);
        self
    }

    /// Build the logging layer.
    pub fn build(self) -> LoggingLayer<W> {
        let Self {
            make_writer,
            resource,
        } = self;
        LoggingLayer {
            make_writer,
            resource,
        }
    }
}

/// Dummy structure as [`std::io::stdout`] can't be named.
#[derive(Debug)]
pub struct Stdout;

impl MakeWriter<'_> for Stdout {
    type Writer = std::io::Stdout;

    #[inline]
    fn make_writer(&'_ self) -> Self::Writer {
        std::io::stdout()
    }
}

/// A structured JSON logging layer.
///
/// This layer emits tracing events to a writer, by default stdout.
///
/// ```rust
/// use otel_tools::layers::LoggingLayer;
/// let resource = opentelemetry_sdk::Resource::builder_empty().build();
/// let layer = LoggingLayer::builder()
///     .with_resource(&resource)
///     .build();
/// ```
#[derive(Debug)]
pub struct LoggingLayer<W> {
    make_writer: W,
    /// Otel resource attributes, converted to JSON values.
    resource: Vec<(String, serde_json::Value)>,
}

impl LoggingLayer<Stdout> {
    /// Create a builder for the logging layer using stdout by default.
    pub fn builder() -> LoggingLayerBuilder<Stdout> {
        LoggingLayerBuilder {
            make_writer: Stdout,
            resource: Vec::new(),
        }
    }
}

impl<W: for<'a> MakeWriter<'a> + 'static> LoggingLayer<W> {
    fn serialize_event<S>(
        &self,
        event: &Event,
        current_span: Option<SpanRef<S>>,
    ) -> Result<Vec<u8>, std::io::Error>
    where
        S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let mut buffer = Vec::new();
        let mut serializer = serde_json::Serializer::new(&mut buffer);
        let mut map_serializer = serializer.serialize_map(None)?;

        // first: message, level, type, and timestamp
        // this keeps the logs easily readable for debugging
        let message = format_event_message(&current_span, visitor);
        map_serializer.serialize_entry(MESSAGE, &message)?;
        let level = level(event.metadata().level());
        map_serializer.serialize_entry(LEVEL, &level)?;
        if let Ok(time) = time::OffsetDateTime::now_utc().format(&Rfc3339) {
            map_serializer.serialize_entry(TIMESTAMP, &time)?;
        } // cov-exclude [branch]: not taken if the timestamp is invalid

        // next: the call-site
        map_serializer.serialize_entry(TARGET, event.metadata().target())?;
        map_serializer.serialize_entry(LINE, &event.metadata().line())?;
        map_serializer.serialize_entry(FILE, &event.metadata().file())?;

        // next: the event's fields
        let mut visitor = FieldVisitor {
            serializer: &mut map_serializer,
            reserved: RESERVED,
        };
        event.record(&mut visitor);

        // next: the otel resource attributes
        for (key, value) in self.resource.iter() {
            map_serializer.serialize_entry(key, value)?;
        }

        // last: trace context information
        if let Some(span) = current_span
            && let Some((trace_id, span_id)) = span_ext_get_trace_context(span.extensions())
        {
            let value = format!("{trace_id}");
            map_serializer.serialize_entry(TRACE_ID, &value)?;
            // serialize the span ID regardless
            let value = format!("{span_id}");
            map_serializer.serialize_entry(SPAN_ID, &value)?;
        }

        map_serializer.end()?;
        // add a trailing new line
        buffer.push(b'\n');

        Ok(buffer)
    }

    /// Write a complete JSON line to the writer.
    ///
    /// Writing the entire line at once avoids "tearing" or interleaving output.
    fn emit(&self, buffer: &[u8], meta: &Metadata<'_>) -> Result<(), std::io::Error> {
        self.make_writer.make_writer_for(meta).write_all(buffer)
    }
}

impl<S, W> Layer<S> for LoggingLayer<W>
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    W: for<'a> MakeWriter<'a> + 'static,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let current_span = ctx.event_span(event);
        if let Ok(buffer) = self.serialize_event(event, current_span) {
            let _ = self.emit(&buffer, event.metadata());
        }
    }
}

#[cfg(test)]
mod tests;
