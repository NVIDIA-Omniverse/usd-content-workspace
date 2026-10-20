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

//! Header propagation functions.
use super::{TraceParent, get_span_context, set_span_context, trace_parent};
use opentelemetry::trace::TraceState;
// imported from tonic to ensure the correct `http` crate version is used.
use tonic::codegen::http;
use tracing::Span;

/// Traceparent header.
///
/// See <https://www.w3.org/TR/trace-context/#traceparent-header>.
pub const TRACE_PARENT: http::HeaderName = http::HeaderName::from_static("traceparent");
/// Tracestate header.
///
/// See <https://www.w3.org/TR/trace-context/#tracestate-header>.
pub const TRACE_STATE: http::HeaderName = http::HeaderName::from_static("tracestate");
/// Traceresponse header.
///
/// The `traceresponse` header is a draft proposal, see
/// <https://w3c.github.io/trace-context/#trace-context-http-response-headers-format>.
pub const TRACE_RESPONSE: http::HeaderName = http::HeaderName::from_static("traceresponse");

/// Get the W3C trace context from the span and inject it into the headers (if
/// the trace context is valid).
pub fn inject(span: &Span, headers: &mut http::HeaderMap) {
    let current_span = span.clone().or_current();

    get_span_context!(span_context = current_span);

    // do not inject the trace context if it is invalid
    use trace_parent::SpanContextToHeader as _;
    let Some(header) = span_context.header() else {
        return;
    };

    // always inject the trace parent
    if let Ok(val) = http::HeaderValue::from_str(&header) {
        headers.insert(TRACE_PARENT, val);
    }

    // do not inject the trace state if it is empty
    let trace_state = span_context.trace_state();
    if trace_state != &TraceState::NONE {
        let state = trace_state.header();
        if let Ok(val) = http::HeaderValue::from_str(&state) {
            headers.insert(TRACE_STATE, val);
        }
    }
}

fn parse_trace_state(v: &http::HeaderValue) -> Option<TraceState> {
    let s = v.to_str().ok()?;
    use std::str::FromStr as _;
    TraceState::from_str(s).ok()
}

/// Extract the W3C trace context from the headers and set it on the span (if
/// the trace context is valid).
pub fn extract(span: &Span, headers: &http::HeaderMap) -> bool {
    let Some(header) = headers.get(TRACE_PARENT).map(http::HeaderValue::as_bytes) else {
        return false;
    };
    let Some(trace_parent) = TraceParent::from_header(header).ok() else {
        let header = tracing::field::display(header.escape_ascii());
        tracing::error!(%header, "invalid traceparent header");
        return false;
    };

    let trace_state = headers
        .get(TRACE_STATE)
        .and_then(parse_trace_state)
        .unwrap_or_default();

    let span_context = trace_parent.remote_span_context(trace_state);
    set_span_context(span, span_context)
}

/// Draft proposal: Get the W3C trace context from the span and inject a
/// `traceresponse` header (if the trace context is valid).
///
/// The `traceresponse` header is a draft proposal, see [`TRACE_RESPONSE`]. For
/// normal W3C trace context propagation, see [`inject`].
pub fn response(span: &Span, headers: &mut http::HeaderMap) {
    let current_span = span.clone().or_current();

    get_span_context!(span_context = current_span);

    // do not inject the trace response if it is invalid
    use trace_parent::SpanContextToHeader as _;
    let Some(header) = span_context.header() else {
        return;
    };

    if let Ok(val) = http::HeaderValue::from_str(&header) {
        headers.insert(TRACE_RESPONSE, val);
    }
}

#[cfg(test)]
mod tests;
