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

use super::{common, opt_into, proto, resource, span_context, vec_into};
use opentelemetry::SpanId;
use opentelemetry::trace::SpanContext;
pub use proto::trace::span::SpanKind;
pub use proto::trace::status::StatusCode;

#[derive(Debug, Clone, PartialEq)]
pub struct OutputSpans {
    pub resource_spans: Vec<ResourceSpans>,
}

impl From<Vec<proto::trace::ResourceSpans>> for OutputSpans {
    fn from(value: Vec<proto::trace::ResourceSpans>) -> Self {
        let resource_spans = vec_into!(value);
        Self { resource_spans }
    }
}

impl OutputSpans {
    pub fn assert_timestamps(&mut self) {
        let timestamp = common::TimestampRange::now();
        for resource_span in &mut self.resource_spans {
            for scope_span in &mut resource_span.scope_spans {
                for span in &mut scope_span.spans {
                    assert_eq!(
                        timestamp, span.start_time_unix_nano,
                        "span start time {} < {} < {}: {:#?}",
                        timestamp.start, span.start_time_unix_nano, timestamp.end, span
                    );
                    assert_eq!(
                        timestamp, span.end_time_unix_nano,
                        "span end time {} < {} < {}: {:#?}",
                        timestamp.start, span.end_time_unix_nano, timestamp.end, span
                    );
                    for event in &span.events {
                        assert_eq!(
                            timestamp, event.time_unix_nano,
                            "span event `{}` time {} < {} < {}: {:#?}",
                            event.name, timestamp.start, event.time_unix_nano, timestamp.end, span
                        );
                    }
                    span.start_time_unix_nano = 0;
                    span.end_time_unix_nano = 0;
                    for event in &mut span.events {
                        event.time_unix_nano = 0;
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceSpans {
    pub resource: Option<resource::Resource>,
    pub schema_url: String,
    pub scope_spans: Vec<ScopeSpans>,
}

impl From<proto::trace::ResourceSpans> for ResourceSpans {
    fn from(value: proto::trace::ResourceSpans) -> Self {
        let proto::trace::ResourceSpans {
            resource,
            scope_spans,
            schema_url,
        } = value;
        let resource = opt_into!(resource);
        let scope_spans = vec_into!(scope_spans);
        Self {
            resource,
            schema_url,
            scope_spans,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScopeSpans {
    pub scope: Option<common::InstrumentationScope>,
    pub schema_url: String,
    pub spans: Vec<Span>,
}

impl From<proto::trace::ScopeSpans> for ScopeSpans {
    fn from(value: proto::trace::ScopeSpans) -> Self {
        let proto::trace::ScopeSpans {
            scope,
            spans,
            schema_url,
        } = value;
        let scope = opt_into!(scope);
        let spans = vec_into!(spans);
        Self {
            scope,
            schema_url,
            spans,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub name: String,
    pub kind: Option<SpanKind>,
    pub attributes: Vec<common::KeyValue>,
    pub dropped_attributes_count: u32,
    pub events: Vec<Event>,
    pub dropped_events_count: u32,
    pub links: Vec<Link>,
    pub dropped_links_count: u32,
    pub status: Option<Status>,
    pub context: SpanContext,
    pub parent: SpanId,
    pub start_time_unix_nano: u64,
    pub end_time_unix_nano: u64,
}

impl Span {
    pub fn new(
        name: &str,
        kind: SpanKind,
        attrs: Vec<common::KeyValue>,
        events: Vec<Event>,
        status: Status,
        context: SpanContext,
        parent: SpanId,
    ) -> Self {
        Self {
            name: name.to_string(),
            kind: Some(kind),
            attributes: attrs,
            dropped_attributes_count: 0,
            events,
            dropped_events_count: 0,
            links: Vec::new(),
            dropped_links_count: 0,
            status: Some(status),
            context,
            parent,
            start_time_unix_nano: 0,
            end_time_unix_nano: 0,
        }
    }

    /// Whether the span's parent is in another process, i.e. the span context
    /// was propagated to this span (e.g. a server span whose parent is the
    /// client span that sent the request).
    ///
    /// OTLP records this on the span itself (`SPAN_FLAGS_CONTEXT_IS_REMOTE`),
    /// and it is decoded into [`SpanContext::is_remote`](opentelemetry::trace::SpanContext::is_remote).
    pub fn has_remote_parent(&self) -> bool {
        self.context.is_remote()
    }

    /// Asserts the span starts within this span.
    ///
    /// This is the only timing guaranteed between a span and a child in
    /// another process: the child is started by a request the parent sent, and
    /// the parent cannot have observed the end of that request before the
    /// child started. Each side ends independently though, e.g. a server span
    /// ends when the server is done with the response body, which may be after
    /// the client has received the response or the error and ended its span.
    pub fn assert_starts_within_span(&self, span: &Span) {
        let outside = span.start_time_unix_nano < self.start_time_unix_nano
            || span.start_time_unix_nano > self.end_time_unix_nano;
        if outside {
            panic!(
                "span `{}` ({}..{}) does not contain the start of `{}` ({}..{})",
                self.name,
                self.start_time_unix_nano,
                self.end_time_unix_nano,
                span.name,
                span.start_time_unix_nano,
                span.end_time_unix_nano,
            );
        }
    }

    pub fn assert_contains_span(&self, span: &Span) {
        let outside = span.start_time_unix_nano < self.start_time_unix_nano
            || span.end_time_unix_nano > self.end_time_unix_nano;
        if outside {
            panic!(
                "span `{}` ({}..{}) does not contain `{}` ({}..{})",
                self.name,
                self.start_time_unix_nano,
                self.end_time_unix_nano,
                span.name,
                span.start_time_unix_nano,
                span.end_time_unix_nano,
            );
        }
    }
}

impl From<proto::trace::Span> for Span {
    fn from(value: proto::trace::Span) -> Self {
        let proto::trace::Span {
            trace_id,
            span_id,
            trace_state,
            parent_span_id,
            flags,
            name,
            kind,
            start_time_unix_nano,
            end_time_unix_nano,
            attributes,
            dropped_attributes_count,
            events,
            dropped_events_count,
            links,
            dropped_links_count,
            status,
        } = value;
        let kind = kind.try_into().ok();
        let attributes = vec_into!(attributes);
        let events = vec_into!(events);
        let links = vec_into!(links);
        let status = opt_into!(status);
        let context = span_context::make(&trace_id, &span_id, flags, &trace_state);
        let parent = span_context::make_span_id(&parent_span_id);
        Self {
            name,
            kind,
            attributes,
            dropped_attributes_count,
            events,
            dropped_events_count,
            links,
            dropped_links_count,
            status,
            context,
            parent,
            start_time_unix_nano,
            end_time_unix_nano,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub name: String,
    pub attributes: Vec<common::KeyValue>,
    pub dropped_attributes_count: u32,
    pub time_unix_nano: u64,
}

impl Event {
    pub fn new(name: &str, attrs: Vec<common::KeyValue>) -> Self {
        Self {
            name: name.to_string(),
            attributes: attrs,
            dropped_attributes_count: 0,
            time_unix_nano: 0,
        }
    }
}

impl From<proto::trace::span::Event> for Event {
    fn from(value: proto::trace::span::Event) -> Self {
        let proto::trace::span::Event {
            time_unix_nano,
            name,
            attributes,
            dropped_attributes_count,
        } = value;
        let attributes = vec_into!(attributes);
        Self {
            name,
            attributes,
            dropped_attributes_count,
            time_unix_nano,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub context: SpanContext,
    pub attributes: Vec<common::KeyValue>,
    pub dropped_attributes_count: u32,
}

impl From<proto::trace::span::Link> for Link {
    fn from(value: proto::trace::span::Link) -> Self {
        let proto::trace::span::Link {
            trace_id,
            span_id,
            trace_state,
            attributes,
            dropped_attributes_count,
            flags,
        } = value;
        let context = span_context::make(&trace_id, &span_id, flags, &trace_state);
        let attributes = vec_into!(attributes);
        Self {
            context,
            attributes,
            dropped_attributes_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub message: String,
    pub code: Option<StatusCode>,
}

impl Status {
    pub fn new(message: &str, code: StatusCode) -> Self {
        Self {
            message: message.to_string(),
            code: Some(code),
        }
    }
}

impl From<proto::trace::Status> for Status {
    fn from(value: proto::trace::Status) -> Self {
        let proto::trace::Status { message, code } = value;
        let code = code.try_into().ok();
        Self { message, code }
    }
}
