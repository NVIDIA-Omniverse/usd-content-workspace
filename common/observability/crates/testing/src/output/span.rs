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

use super::{Output, attrs};
use opentelemetry::SpanId;
use opentelemetry::trace::SpanContext;
use otel_asserts::semconv;
use otel_asserts::types::{common, resource, trace};
use std::collections::HashMap;
use std::fmt;

impl<T> Output<T> {
    pub fn spans<'a>(&'a self, resource: &otel_tools::resource::Resource) -> OutputSpans<'a> {
        let expected_resource = Some(resource::Resource::new(attrs(resource)));
        let expected_scope = Some(common::InstrumentationScope::new("test", "", Vec::new()));
        let expected_trace_id = self.root.trace_id();

        let mut root = None;
        let mut server = None;
        let mut client = None;
        let mut seen = HashMap::new();
        let mut spans = Vec::new();

        for resource_span in &self.spans.resource_spans {
            assert_eq!(
                resource_span.resource, expected_resource,
                "resource_span.resource"
            );
            for scope_span in &resource_span.scope_spans {
                assert_eq!(scope_span.scope, expected_scope, "scope_span.scope");

                for span in &scope_span.spans {
                    assert_eq!(span.context.trace_id(), expected_trace_id, "span.trace_id");

                    let span_id = span.context.span_id();
                    if let Some(prev) = seen.insert(span_id, span) {
                        panic!(
                            "duplicate span ID `{}`: `{}` and `{}`",
                            span_id, prev.name, span.name
                        );
                    }

                    if span.context == self.root {
                        if let Some(existing) = root {
                            panic!("duplicate root span:\n{span:#?}\n{existing:#?}");
                        }
                        root = Some(span);
                    }

                    if span.kind == Some(trace::SpanKind::Client) {
                        if let Some(existing) = client {
                            panic!("duplicate client span:\n{span:#?}\n{existing:#?}");
                        }
                        client = Some(span);
                    }

                    if span.kind == Some(trace::SpanKind::Server) {
                        if let Some(existing) = server {
                            panic!("duplicate server span:\n{span:#?}\n{existing:#?}");
                        }
                        server = Some(span);
                    }

                    spans.push(span);
                }
            }
        }

        let root = root.expect("no root span");
        let client = client.expect("no client span");
        // server span is optional; some tests don't have server instrumentation

        let hierarchy = SpanRef::parent(spans);

        OutputSpans {
            hierarchy,
            root,
            server,
            client,
        }
    }
}

#[derive(Debug)]
pub struct OutputSpans<'a> {
    pub hierarchy: SpanRef<'a>,
    pub root: &'a trace::Span,
    pub server: Option<&'a trace::Span>,
    pub client: &'a trace::Span,
}

#[derive(PartialEq)]
struct Kv<'a> {
    k: &'a str,
    v: Option<&'a common::Value>,
}

impl fmt::Debug for Kv<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.v {
            Some(v) => write!(f, "{:?}: {:?}", self.k, v),
            None => write!(f, "{:?}: <none>", self.k),
        }
    }
}

impl<'a> From<&'a common::KeyValue> for Kv<'a> {
    fn from(kv: &'a common::KeyValue) -> Self {
        Self {
            k: kv.key(),
            v: kv.value.as_ref(),
        }
    }
}

impl OutputSpans<'_> {
    #[track_caller]
    pub fn assert(&self, expected: SpanName) {
        let actual = self.hierarchy.names();
        pretty_assertions::assert_eq!(expected, actual);
    }

    #[track_caller]
    fn assert_attrs(span: &trace::Span, expected: Vec<(&str, common::Value)>, name: &str) {
        const IGNORED: &[&str] = &[
            semconv::BUSY_NS,
            semconv::IDLE_NS,
            semconv::CODE_LINE_NUMBER,
            semconv::THREAD_ID,
            semconv::THREAD_NAME,
        ];

        let mut actual: Vec<Kv<'_>> = span
            .attributes
            .iter()
            .filter_map(|kv| {
                if IGNORED.contains(&kv.key()) {
                    None
                } else {
                    Some(kv.into())
                }
            })
            .collect();

        let status_code = span
            .status
            .as_ref()
            .and_then(|status| status.code)
            .map(|status_code| {
                let name = match status_code {
                    trace::StatusCode::Unset => "Unset",
                    trace::StatusCode::Ok => "Ok",
                    trace::StatusCode::Error => "Error",
                };
                common::Value::from(name)
            });

        if let Some(status_code) = status_code.as_ref() {
            actual.push(Kv {
                k: "otel.status_code",
                v: Some(status_code),
            });
        }

        let status_desc = span
            .status
            .as_ref()
            .filter(|status| !status.message.is_empty())
            .map(|status| common::Value::from(status.message.clone()));

        if let Some(status_desc) = status_desc.as_ref() {
            actual.push(Kv {
                k: "otel.status_description",
                v: Some(status_desc),
            });
        }

        actual.sort_by(|a, b| a.k.cmp(b.k));

        let mut expected: Vec<Kv<'_>> =
            expected.iter().map(|(k, v)| Kv { k, v: Some(v) }).collect();
        expected.sort_by(|a, b| a.k.cmp(b.k));

        pretty_assertions::assert_eq!(expected, actual, "{name} attributes");
    }

    #[track_caller]
    pub fn assert_client_attrs(&self, expected: Vec<(&str, common::Value)>) {
        Self::assert_attrs(self.client, expected, "client");
    }

    #[track_caller]
    pub fn assert_no_server_span(&self) {
        match self.server {
            None => {}
            Some(server) => {
                eprintln!("unexpected server span");
                Self::assert_attrs(server, vec![], "server");
            }
        }
    }

    #[track_caller]
    pub fn assert_server_attrs(&self, expected: Vec<(&str, common::Value)>) {
        let server = self.server.expect("no server span");
        Self::assert_attrs(server, expected, "server");
    }

    #[track_caller]
    pub fn assert_client_events(&self, expected: Vec<Event>) {
        Self::assert_events(&self.client.events, expected, "client");
    }

    #[track_caller]
    pub fn assert_no_client_events(&self) {
        Self::assert_events(&self.client.events, vec![], "client");
    }

    #[track_caller]
    pub fn assert_server_events(&self, expected: Vec<Event>) {
        let server = self.server.expect("no server span");
        Self::assert_events(&server.events, expected, "server");
    }

    #[track_caller]
    pub fn assert_no_server_events(&self) {
        let server = self.server.expect("no server span");
        Self::assert_events(&server.events, vec![], "server");
    }

    #[track_caller]
    pub fn assert_events(actual: &[trace::Event], expected: Vec<Event>, label: &str) {
        const IGNORED: &[&str] = &[
            semconv::CODE_FILE_PATH,
            semconv::CODE_LINE_NUMBER,
            semconv::CODE_MODULE_NAME,
            semconv::LEVEL,
            semconv::TARGET,
        ];

        let actual: Vec<Evt<'_>> = actual
            .iter()
            .map(|event| {
                let mut attrs: Vec<Kv<'_>> = event
                    .attributes
                    .iter()
                    .filter_map(|kv| {
                        if IGNORED.contains(&kv.key()) {
                            None
                        } else {
                            Some(kv.into())
                        }
                    })
                    .collect();

                attrs.sort_by(|a, b| a.k.cmp(b.k));

                Evt {
                    name: &event.name,
                    attrs,
                }
            })
            .collect();

        let expected: Vec<Evt<'_>> = expected
            .iter()
            .map(|event| {
                let mut attrs: Vec<Kv<'_>> = event
                    .attrs
                    .iter()
                    .map(|(k, v)| Kv { k, v: Some(v) })
                    .collect();

                attrs.sort_by(|a, b| a.k.cmp(b.k));
                Evt {
                    name: &event.name,
                    attrs,
                }
            })
            .collect();

        pretty_assertions::assert_eq!(expected, actual, "{label} events");
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub name: String,
    pub attrs: Vec<(&'static str, common::Value)>,
}

#[derive(PartialEq)]
struct Evt<'a> {
    name: &'a str,
    attrs: Vec<Kv<'a>>,
}

impl fmt::Debug for Evt<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // format output matches macro syntax, so it's easier to review
        // the test condition
        write!(f, "{:?}", self.name)?;
        if !self.attrs.is_empty() {
            f.write_str(" => ")?;
            f.debug_list().entries(&self.attrs).finish()?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct SpanRef<'a> {
    pub span: &'a trace::Span,
    pub children: Vec<SpanRef<'a>>,
}

fn span_parent<'a>(
    span_ref: &mut SpanRef<'a>,
    parent_id: SpanId,
    spans: &mut Vec<&'a trace::Span>,
) {
    while let Some(child_index) = spans
        .iter()
        .copied()
        .position(|span| span.parent == parent_id)
    {
        let span = spans.swap_remove(child_index);
        let mut child_span_ref = SpanRef {
            span,
            children: Vec::new(),
        };
        span_parent(&mut child_span_ref, span.context.span_id(), spans);
        span_ref.children.push(child_span_ref);
    }
    // sort children by start time
    span_ref
        .children
        .sort_by_key(|span_ref| span_ref.span.start_time_unix_nano);
}

impl<'a> SpanRef<'a> {
    const NULL_SPAN: &'static trace::Span = &trace::Span {
        name: String::new(),
        kind: None,
        attributes: Vec::new(),
        dropped_attributes_count: 0,
        events: Vec::new(),
        dropped_events_count: 0,
        links: Vec::new(),
        dropped_links_count: 0,
        status: None,
        context: SpanContext::NONE,
        parent: SpanId::INVALID,
        start_time_unix_nano: 0,
        end_time_unix_nano: 0,
    };

    fn parent(mut spans: Vec<&'a trace::Span>) -> Self {
        let mut hierarchy = SpanRef {
            span: Self::NULL_SPAN,
            children: Vec::new(),
        };
        span_parent(&mut hierarchy, SpanId::INVALID, &mut spans);
        if !spans.is_empty() {
            panic!("remaining spans after parenting: {spans:#?}");
        }
        let SpanRef {
            span: _,
            mut children,
        } = hierarchy;
        if children.len() != 1 {
            panic!("multiple root spans: {children:#?}");
        }
        children.pop().unwrap()
    }

    fn names(&self) -> SpanName {
        let children = self.children.iter().map(Self::names).collect();
        let name = match self.span.kind {
            Some(trace::SpanKind::Client) => format!("{} (client)", self.span.name),
            Some(trace::SpanKind::Server) => format!("{} (server)", self.span.name),
            _ => self.span.name.clone(),
        };
        SpanName { name, children }
    }

    /// Asserts every span contains its children, recursively.
    ///
    /// A child in the same process must be fully contained. A child in another
    /// process (i.e. the server span under the client span that sent the
    /// request) only has to start within its parent: the two ends are not
    /// ordered. The server span ends when the server drops the response body,
    /// which happens on the server's task after the last frame or the body
    /// error has been handed to the connection, so the client can receive the
    /// response (or the reset) and end its span first. That is most visible
    /// when the client span only covers the response head, or the response
    /// body fails, but it is possible for any response.
    #[track_caller]
    pub fn assert_contains_children(&self) {
        for child in &self.children {
            if child.span.has_remote_parent() {
                self.span.assert_starts_within_span(child.span);
            } else {
                self.span.assert_contains_span(child.span);
            }
            child.assert_contains_children();
        }
    }

    #[track_caller]
    pub fn assert_server_streaming(&self) {
        // the server span outlives the client span due to streaming
        for child_span in &self.children {
            self.span.assert_contains_span(child_span.span);
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct SpanName {
    pub name: String,
    pub children: Vec<SpanName>,
}

impl fmt::Debug for SpanName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // format output matches macro syntax, so it's easier to review
        // the test condition
        write!(f, "{:?}", self.name)?;
        if !self.children.is_empty() {
            f.write_str(" => ")?;
            f.debug_set().entries(&self.children).finish()?;
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! expected_hierarchy {
    ($name:literal => {
        $(
            $child:literal $(=> { $($inner:tt)* })?
        ),* $(,)?
    } $(,)?) => {
        $crate::output::SpanName {
            name: $name.to_string(),
            children: vec![
                $( expected_hierarchy!($child $(=> { $($inner)* })?) ,)*
            ],
        }
    };
    ($name:literal) => {
        $crate::output::SpanName {
            name: $name.to_string(),
            children: vec![],
        }
    };
}

#[macro_export]
macro_rules! expected_attrs {
    ($($k:expr => $v:expr),* $(,)?) => {
        vec![
            $(($k, otel_asserts::types::common::Value::from($v)),)*
        ]
    };
}

#[macro_export]
macro_rules! expected_events {
    ($($name:literal { $($k:expr => $v:expr),* $(,)? }),* $(,)?) => {
        vec![
            $($crate::output::Event {
                name: $name.to_string(),
                attrs: vec![
                    $(($k, otel_asserts::types::common::Value::from($v)),)*
                ],
            },)*
        ]
    };
}
