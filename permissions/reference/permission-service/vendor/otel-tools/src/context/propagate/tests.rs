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

use super::{
    TRACE_PARENT, TRACE_RESPONSE, TRACE_STATE, TraceParent, extract, get_span_context, http,
    inject, response,
};
use crate::testing::{FixedTracer, otel};
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState, Tracer};
use tracing::Span;

const TEST_PARENT_HEADER: http::HeaderValue =
    http::HeaderValue::from_static("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01");

const TEST_TRACE_ID: TraceId = TraceId::from_bytes([
    0x0a, 0xf7, 0x65, 0x19, 0x16, 0xcd, 0x43, 0xdd, 0x84, 0x48, 0xeb, 0x21, 0x1c, 0x80, 0x31, 0x9c,
]);
const TEST_PARENT_ID: SpanId = SpanId::from_bytes([0xb7, 0xad, 0x6b, 0x71, 0x69, 0x20, 0x33, 0x31]);
const TEST_FLAGS: TraceFlags = TraceFlags::SAMPLED;
const TEST_PARENT: TraceParent = TraceParent {
    trace_id: TEST_TRACE_ID,
    parent_id: TEST_PARENT_ID,
    trace_flags: TEST_FLAGS,
};

const TEST_STATE_STR: &str = "congo=t61rcWkgMzE";
const TEST_STATE_HEADER: http::HeaderValue = http::HeaderValue::from_static(TEST_STATE_STR);

#[track_caller]
fn assert_span_context(span: &Span, parent: &TraceParent, is_remote: bool, state: &TraceState) {
    get_span_context!(span_context = span);
    assert_eq!(span_context.trace_id(), parent.trace_id, "trace ID");
    assert_eq!(span_context.span_id(), parent.parent_id, "parent ID");
    assert_eq!(span_context.trace_flags(), parent.trace_flags, "flags");
    assert_eq!(span_context.is_remote(), is_remote, "is remote");
    assert_eq!(span_context.trace_state(), state, "state");
}

macro_rules! headers {
    ($($key:ident => $val:expr),* $(,)?) => {{
        let capacity = <[()]>::len(&[ $(headers!(@replace $key)),* ]);
        let mut headers = http::HeaderMap::with_capacity(capacity);
        $(
            if headers.insert($key, $val).is_some() {
                panic!("duplicate `{}`", $key);
            }
        )*
        headers
    }};
    (@replace $_key:ident) => { () };
}

#[test]
fn trace_context_propagated_both() {
    let headers_expected = headers! {
        TRACE_PARENT => TEST_PARENT_HEADER,
        TRACE_STATE => TEST_STATE_HEADER,
    };
    let state_expected: TraceState = std::str::FromStr::from_str(TEST_STATE_STR).unwrap();

    let mut headers_actual = http::HeaderMap::with_capacity(2);
    otel(|| {
        let span = tracing::info_span!("test span");
        // if we access the span in any way, including cloning it, it is built,
        // and the parent can't be set any more...
        let copy = tracing::info_span!("test span");
        assert_span_context(&copy, &TraceParent::NONE, false, &TraceState::NONE);
        assert!(extract(&span, &headers_expected));
        assert_span_context(&span, &TEST_PARENT, true, &state_expected);

        inject(&span, &mut headers_actual);
    });
    assert_eq!(headers_expected, headers_actual);
}

#[test]
fn trace_context_propagated_trace_parent_only() {
    let headers_expected = headers! {
        TRACE_PARENT => TEST_PARENT_HEADER,
    };

    let mut headers_actual = http::HeaderMap::with_capacity(2);
    otel(|| {
        let span = tracing::info_span!("test span");
        // if we access the span in any way, including cloning it, it is built,
        // and the parent can't be set any more...
        let copy = tracing::info_span!("test span");
        assert_span_context(&copy, &TraceParent::NONE, false, &TraceState::NONE);
        assert!(extract(&span, &headers_expected));
        assert_span_context(&span, &TEST_PARENT, true, &TraceState::NONE);
        inject(&span, &mut headers_actual);
    });
    assert_eq!(headers_expected, headers_actual);
}

#[test]
fn inject_no_context_noop() {
    let headers_expected = http::HeaderMap::new();

    let mut headers_actual = http::HeaderMap::new();
    otel(|| {
        let span = tracing::info_span!("test span");
        assert_span_context(&span, &TraceParent::NONE, false, &TraceState::NONE);
        inject(&span, &mut headers_actual);
        assert_span_context(&span, &TraceParent::NONE, false, &TraceState::NONE);
    });
    assert_eq!(headers_expected, headers_actual);
}

#[test]
fn extract_no_context_noop() {
    otel(|| {
        let span = tracing::info_span!("test span");
        // if we access the span in any way, including cloning it, it is built,
        // and the parent can't be set any more...
        let copy = tracing::info_span!("test span");
        assert_span_context(&copy, &TraceParent::NONE, false, &TraceState::NONE);
        let headers = http::HeaderMap::new();
        assert!(!extract(&span, &headers));
        assert_span_context(&span, &TraceParent::NONE, false, &TraceState::NONE);
    })
}

#[test]
fn extract_invalid_noop() {
    otel(|| {
        let span = tracing::info_span!("test span");
        // if we access the span in any way, including cloning it, it is built,
        // and the parent can't be set any more...
        let copy = tracing::info_span!("test span");
        assert_span_context(&copy, &TraceParent::NONE, false, &TraceState::NONE);
        let mut headers = http::HeaderMap::new();
        let val = http::HeaderValue::from_static("invalid");
        headers.insert(TRACE_PARENT, val);
        assert!(!extract(&span, &headers));
        assert_span_context(&span, &TraceParent::NONE, false, &TraceState::NONE);
    })
}

#[test]
fn response_no_context_noop() {
    let headers_expected = http::HeaderMap::new();
    let mut headers_actual = http::HeaderMap::new();
    otel(|| {
        let span = tracing::info_span!("test span");
        let copy = tracing::info_span!("test span");
        assert_span_context(&copy, &TraceParent::NONE, false, &TraceState::NONE);
        response(&span, &mut headers_actual);
        assert_span_context(&span, &TraceParent::NONE, false, &TraceState::NONE);
    });
    assert_eq!(headers_expected, headers_actual);
}

#[test]
fn response_has_context() {
    let headers_expected = headers! {
        TRACE_RESPONSE => TEST_PARENT_HEADER,
    };
    let mut headers_actual = http::HeaderMap::with_capacity(1);
    // fake a context that might be provided if OpenTelemetry is set up with
    // a `RandomIdGenerator` (the default `IdGenerator`).
    let tracer = FixedTracer::new(SpanContext::new(
        TEST_TRACE_ID,
        TEST_PARENT_ID,
        TEST_FLAGS,
        false,
        TraceState::NONE,
    ));
    tracer.in_span("fake", |_context| {
        otel(|| {
            // the (tracing) span inherits the context...
            let span = tracing::info_span!("test span");
            // ...which should then be injected
            response(&span, &mut headers_actual);
        });
    });
    assert_eq!(headers_expected, headers_actual);
}
