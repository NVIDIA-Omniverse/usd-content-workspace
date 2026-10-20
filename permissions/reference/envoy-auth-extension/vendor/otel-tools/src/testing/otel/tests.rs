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

use opentelemetry::trace::{self, SpanBuilder, SpanContext, Tracer, noop};
use opentelemetry::{Context, KeyValue};
use std::borrow::Cow;

/// For OpenTelemetry contexts to even be settable, a subscriber and tracer is
/// required.
///
/// This one sets an empty context by default (noop tracer).
///
/// I don't know why or care to know, but it's probably because the
/// opentelemetry crate is a Lovecraftian horror. My sanity can only take so
/// many not-global but kinda global state hacks. Learn Rust they said. It'll be
/// fun they said.
pub(crate) fn otel<T>(f: impl FnOnce() -> T) -> T {
    let layer = tracing_opentelemetry::layer().with_tracer(noop::NoopTracer::new());
    use tracing_subscriber::layer::SubscriberExt as _;
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, f)
}

/// A [`Tracer`] that always returns OpenTelemetry spans with a set [`SpanContext`].
#[derive(Debug, Clone)]
pub(crate) struct FixedTracer(SpanContext);

impl FixedTracer {
    pub(crate) fn new(span_context: SpanContext) -> Self {
        Self(span_context)
    }
}

impl Tracer for FixedTracer {
    type Span = FixedSpan;

    fn build_with_context(&self, _builder: SpanBuilder, _parent_cx: &Context) -> Self::Span {
        let span_context = self.0.clone();
        FixedSpan { span_context }
    }
}

/// An OpenTelemetry span from [`FixedTracer`].
#[derive(Debug)]
pub(crate) struct FixedSpan {
    span_context: SpanContext,
}

impl trace::Span for FixedSpan {
    fn add_event<T>(&mut self, _name: T, _attributes: Vec<KeyValue>)
    where
        T: Into<Cow<'static, str>>,
    {
    }

    fn add_event_with_timestamp<T>(
        &mut self,
        _name: T,
        _timestamp: std::time::SystemTime,
        _attributes: Vec<KeyValue>,
    ) where
        T: Into<Cow<'static, str>>,
    {
    }

    fn span_context(&self) -> &SpanContext {
        &self.span_context
    }

    fn is_recording(&self) -> bool {
        true
    }

    fn set_attribute(&mut self, _attribute: KeyValue) {}

    fn set_status(&mut self, _status: trace::Status) {}

    fn update_name<T>(&mut self, _new_name: T)
    where
        T: Into<Cow<'static, str>>,
    {
    }

    fn add_link(&mut self, _span_context: SpanContext, _attributes: Vec<KeyValue>) {}

    fn end_with_timestamp(&mut self, _timestamp: std::time::SystemTime) {}
}
