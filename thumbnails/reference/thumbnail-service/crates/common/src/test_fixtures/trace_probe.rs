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

//! A subscriber that gives spans real OpenTelemetry span contexts, for tests
//! that assert on trace propagation.
//!
//! Without an OpenTelemetry layer in the subscriber, `tracing::info_span!`
//! produces a span with no OTel span context at all: `set_parent` fails and the
//! propagator injects nothing. Every assertion about trace context would then
//! pass vacuously against `None`, which is the failure mode this exists to
//! prevent — a propagation test that cannot tell "not propagated" from "nothing
//! to propagate" is not a test.
//!
//! Spans are exported to memory rather than to a collector, so a test can read
//! back a span's own context *and its parent* — which is the only way to tell a
//! continued trace from a coincidentally-matching trace id.

use opentelemetry::trace::{SpanId, TraceId, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use tracing_subscriber::layer::{Layer as _, SubscriberExt as _};

/// Holds the thread-local subscriber, the provider, and the exported spans.
///
/// Dropping it restores the previous subscriber. Keep it bound for the whole
/// test — `let _ = probe()` drops it immediately and silently un-instruments
/// everything after that line.
pub struct TraceProbe {
    provider: SdkTracerProvider,
    exporter: InMemorySpanExporter,
    _guard: tracing::subscriber::DefaultGuard,
}

impl TraceProbe {
    /// Every span that has closed, flushed first.
    ///
    /// A span still open at the call site is absent rather than incomplete, which
    /// is usually a sign the future under test was not driven to completion.
    pub fn exported_spans(&self) -> Vec<SpanData> {
        let _ = self.provider.force_flush();
        self.exporter
            .get_finished_spans()
            .expect("the in-memory exporter cannot fail")
    }

    /// Wait for a span with this name to close and export, then return it.
    ///
    /// Detached work closes its span after the call that spawned it has already
    /// returned, so a test reading `exported_spans()` straight away races it.
    /// Polling is what keeps such a test an assertion about parentage rather than
    /// an assertion about scheduling.
    pub async fn await_span_named(&self, name: &str, timeout: std::time::Duration) -> SpanData {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(span) = self
                .exported_spans()
                .into_iter()
                .find(|span| span.name == name)
            {
                return span;
            }
            if std::time::Instant::now() >= deadline {
                panic!("no span named '{name}' exported within {timeout:?}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    /// Every exported span with this name, for asserting on a fan-out.
    pub fn spans_named(&self, name: &str) -> Vec<SpanData> {
        self.exported_spans()
            .into_iter()
            .filter(|span| span.name == name)
            .collect()
    }

    /// Wait for `count` spans with this name to close and export.
    ///
    /// The fan-out counterpart of [`Self::await_span_named`], and bounded for the
    /// same reason: a detached fan-out closes its spans after the call returned,
    /// so the wait is unavoidable, but an unbounded one turns a naming regression
    /// or a stalled export into a test that hangs until the suite timeout instead
    /// of a failure that names what it was waiting for.
    pub async fn await_spans_named(
        &self,
        name: &str,
        count: usize,
        timeout: std::time::Duration,
    ) -> Vec<SpanData> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let spans = self.spans_named(name);
            if spans.len() >= count {
                return spans;
            }
            if std::time::Instant::now() >= deadline {
                panic!(
                    "expected {count} span(s) named '{name}' within {timeout:?}, got {}",
                    spans.len()
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    /// The single exported span with this name.
    ///
    /// Panics when there is not exactly one, because both "missing" and
    /// "duplicated" break the assertion the caller is about to make and the
    /// distinction matters when diagnosing it.
    pub fn span_named(&self, name: &str) -> SpanData {
        let mut matching: Vec<SpanData> = self
            .exported_spans()
            .into_iter()
            .filter(|span| span.name == name)
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "expected exactly one span named '{name}', found {}",
            matching.len()
        );
        matching.remove(0)
    }
}

/// The trace and span ids a child span should report to have continued a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedParent {
    pub trace_id: TraceId,
    pub span_id: SpanId,
}

impl ExpectedParent {
    /// The parentage a direct child of `parent` should report.
    pub fn of(parent: &SpanData) -> Self {
        Self {
            trace_id: parent.span_context.trace_id(),
            span_id: parent.span_context.span_id(),
        }
    }

    /// Assert `child` continued this trace *under its own span id*.
    ///
    /// All three assertions earn their place. A matching trace id alone is also
    /// what forwarding a `traceparent` verbatim produces, which leaves a hole
    /// where the parent's own work happened; naming the parent is what closes it;
    /// and a child that reused the parent's span id would satisfy both of the
    /// first two while being the same span rather than a descendant.
    pub fn assert_parents(&self, child: &SpanData) {
        assert_eq!(
            child.span_context.trace_id(),
            self.trace_id,
            "child span '{}' must report the parent's trace id",
            child.name
        );
        assert_eq!(
            child.parent_span_id, self.span_id,
            "child span '{}' must name the parent's span id as its parent",
            child.name
        );
        assert_ne!(
            child.span_context.span_id(),
            self.span_id,
            "child span '{}' must carry its own span id, not the parent's",
            child.name
        );
    }
}

/// A span's exported wall-clock duration, which is what a latency panel reads.
///
/// Reads the exported `SpanData` rather than timing the call, because the defect
/// this exists to catch is a span whose *reported* end time is not the point the
/// operation finished.
pub fn exported_duration(span: &SpanData) -> std::time::Duration {
    span.end_time
        .duration_since(span.start_time)
        .unwrap_or_default()
}

/// Install a sampling OTel-backed subscriber on the current thread.
///
/// Thread-local by `set_default` rather than global: the workspace runs tests in
/// parallel, and a global subscriber can only be set once per process. The
/// documented flake shape in this repo involves a thread-local subscriber and a
/// handler running on *another* thread; that does not apply here, because a span
/// context is captured synchronously on the thread that opened the span, and
/// `#[tokio::test]` drives its future on that same thread.
pub fn probe() -> TraceProbe {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("thumbnail-trace-probe");

    // Filtered to INFO, which is what the deployed default is, and not merely for
    // noise: `tracing_subscriber::Registry` keeps a span alive while any
    // descendant is open, and h2 opens connection-scoped spans at TRACE that live
    // as long as the pooled connection. Unfiltered, those descendants hold every
    // ancestor open, so the root span of the operation under test never closes and
    // never exports — the span tree looks truncated at exactly the span the test
    // wants to read.
    let subscriber = tracing_subscriber::registry().with(
        tracing_opentelemetry::layer()
            .with_tracer(tracer)
            .with_tracked_inactivity(false)
            .with_filter(tracing_subscriber::filter::LevelFilter::INFO),
    );

    let guard = tracing::subscriber::set_default(subscriber);

    TraceProbe {
        provider,
        exporter,
        _guard: guard,
    }
}

/// The trace id in a W3C `traceparent`, for comparing two ends of a hop.
pub fn trace_id_of(traceparent: &str) -> Option<&str> {
    traceparent.split('-').nth(1)
}

/// The span id in a W3C `traceparent`.
pub fn span_id_of(traceparent: &str) -> Option<&str> {
    traceparent.split('-').nth(2)
}
