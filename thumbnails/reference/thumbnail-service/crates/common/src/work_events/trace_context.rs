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

//! W3C trace context as it travels on a generate message.
//!
//! The notification event body is a `google.protobuf.Struct` rather than an HTTP
//! request, and the propagator's API is shaped for headers, so this module is the
//! adapter between the two. Everything W3C-specific stays in `otel-tools`: the
//! header names, the `traceparent` grammar, `tracestate` parsing, and the
//! decision to emit nothing when the active span context is invalid.
//!
//! Formatting the traceparent string here would have been fewer lines and is the
//! wrong trade. It drops `tracestate`, duplicates validation that already exists
//! and is tested upstream, and hardcodes a wire format the propagator is allowed
//! to change underneath us.

use otel_tools::context::propagate::{self, TRACE_PARENT, TRACE_STATE};
use tonic::codegen::http::{HeaderMap, HeaderName, HeaderValue};
use tracing::Span;

/// Trace context captured from an active span, ready to serialize onto a message.
///
/// Both fields absent means there was nothing valid to capture, which is the
/// normal state for a publisher running outside a span. It is deliberately not an
/// error: trace context is telemetry, and work publishes without it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapturedTraceContext {
    pub traceparent: Option<String>,
    pub tracestate: Option<String>,
}

impl CapturedTraceContext {
    /// Capture from the span active at the call site.
    pub fn capture() -> Self {
        Self::from_span(&Span::current())
    }

    /// Replace the trace-context keys of a serialized message with this context.
    ///
    /// A replacement rather than an insertion, and both keys are removed first:
    /// whatever the message was carrying is not evidence about the span that is
    /// putting it on the wire now. Leaving a stale value in place when there is
    /// nothing to capture is the failure this is shaped to prevent — it would
    /// hand the consumer a parent span that never published anything.
    pub fn replace_in(&self, fields: &mut std::collections::BTreeMap<String, prost_types::Value>) {
        fields.remove(TRACE_PARENT.as_str());
        fields.remove(TRACE_STATE.as_str());

        for (key, value) in [
            (TRACE_PARENT.as_str(), self.traceparent.as_deref()),
            (TRACE_STATE.as_str(), self.tracestate.as_deref()),
        ] {
            if let Some(value) = value {
                fields.insert(
                    key.to_string(),
                    prost_types::Value {
                        kind: Some(prost_types::value::Kind::StringValue(value.to_string())),
                    },
                );
            }
        }
    }

    /// Capture from a specific span.
    ///
    /// Note that the propagator falls back to the current span when given a
    /// disabled one, so this is a way to name the source explicitly rather than a
    /// way to opt out of the ambient context.
    pub fn from_span(span: &Span) -> Self {
        let mut headers = HeaderMap::new();
        propagate::inject(span, &mut headers);
        Self {
            traceparent: header(&headers, TRACE_PARENT),
            tracestate: header(&headers, TRACE_STATE),
        }
    }
}

/// The longest `traceparent` worth handing to the propagator.
///
/// A valid one is exactly 55 characters — `00-` plus 32 hex, `-` plus 16 hex, `-`
/// plus 2 — so this is nine characters of headroom for a longer future version
/// prefix and nothing more.
const MAX_TRACEPARENT_LEN: usize = 64;

/// How much of an over-long value to show when reporting it.
const REPORTED_PREFIX_LEN: usize = 32;

/// Make the trace context from a received message the remote parent of `span`.
///
/// Returns whether a parent was applied, for the caller that wants to record
/// which happened. A missing or unparseable `traceparent` returns `false` and
/// leaves `span` rooting its own trace — a delivery must never fail, or change
/// its acknowledgement, because of a telemetry field.
pub fn set_remote_parent(span: &Span, traceparent: Option<&str>, tracestate: Option<&str>) -> bool {
    let Some(traceparent) = traceparent else {
        return false;
    };

    // Refused here rather than by the propagator, and this is the one case where
    // that is worth doing. `otel-tools` logs a rejected value at ERROR with the
    // value inline and unbounded, once per delivery, so a corrupted or hostile
    // producer sets the size of a log line. That emit site is upstream and cannot
    // be changed from here; the only local lever is not reaching it with a value
    // that could never have been valid.
    if traceparent.len() > MAX_TRACEPARENT_LEN {
        tracing::warn!(
            traceparent_len = traceparent.len(),
            traceparent_prefix = %escaped_prefix(traceparent),
            "refusing an over-long traceparent; the span will root its own trace"
        );
        return false;
    }

    let Ok(traceparent) = HeaderValue::from_str(traceparent) else {
        return false;
    };

    let mut headers = HeaderMap::new();
    headers.insert(TRACE_PARENT, traceparent);
    if let Some(Ok(tracestate)) = tracestate.map(HeaderValue::from_str) {
        headers.insert(TRACE_STATE, tracestate);
    }

    // `extract` logs and returns false on a malformed traceparent, so a bad value
    // is reported once, where it can be read, rather than silently dropped here.
    propagate::extract(span, &headers)
}

/// A bounded, escaped rendering of the start of a value, safe to put in a log.
///
/// Truncating the **escaped** form is the whole point and the easy thing to get
/// wrong: `escape_ascii` expands a non-printable byte to `\xNN`, so truncating
/// the raw value first still permits a line four times as long as the bound.
fn escaped_prefix(value: &str) -> String {
    // Escaped per byte and stopped on a whole escape, rather than taking N
    // characters off the escaped stream: that cuts `\x0A` into `\x0`, which reads
    // as a different byte than the one that arrived. The marker says the value
    // continues, so a bounded line is not mistaken for the whole of a short one.
    let mut out = String::new();
    for byte in value.as_bytes() {
        let escaped = byte.escape_ascii().to_string();
        if out.len() + escaped.len() > REPORTED_PREFIX_LEN {
            out.push('…');
            break;
        }
        out.push_str(&escaped);
    }
    out
}

fn header(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bound applies to the escaped rendering, which is the property that
    /// makes it a bound at all: truncating the raw value first and escaping
    /// afterwards would permit a line four times as long, since every
    /// non-printable byte becomes four characters.
    #[test]
    fn the_reported_prefix_is_bounded_after_escaping_not_before() {
        let control_bytes = "\u{1}\u{2}\u{3}\u{4}\u{5}\u{6}\u{7}\u{8}\t\n\u{b}\u{c}\r".repeat(8);
        let reported = escaped_prefix(&control_bytes);

        assert!(
            reported.trim_end_matches('…').len() <= REPORTED_PREFIX_LEN,
            "the escaped rendering must be bounded, got {} bytes: {reported}",
            reported.len()
        );
        assert!(
            !reported.contains('\n') && !reported.contains('\r'),
            "escaping is what keeps the value inside one log field: {reported}"
        );
        assert!(
            reported.starts_with("\\x01"),
            "a control byte should appear as its escape, got: {reported}"
        );
        assert!(
            reported.ends_with('…'),
            "a truncated value must say so, or a bounded line reads as a whole \
             short one: {reported}"
        );
    }

    /// Truncation lands between escape sequences, never inside one.
    ///
    /// Cutting `\x0A` to `\x0` reports a byte other than the one that arrived,
    /// which is worse than reporting fewer of them.
    #[test]
    fn truncation_never_splits_an_escape_sequence() {
        // Every byte escapes to four characters, so no whole number of them lands
        // exactly on a bound that is not a multiple of four — the case that
        // produced a trailing `\x0` before.
        let reported = escaped_prefix(&"\u{7}".repeat(64));
        let body = reported.trim_end_matches('…');

        assert_eq!(
            body.len() % 4,
            0,
            "truncation must fall on an escape boundary, got: {reported}"
        );
        assert!(
            body.chars().all(|c| "\\x07".contains(c)),
            "no partial escape may survive truncation, got: {reported}"
        );
    }

    /// A printable value is reported as itself, so the ordinary diagnostic case
    /// stays readable rather than arriving as escape sequences.
    #[test]
    fn a_printable_prefix_is_reported_verbatim() {
        assert_eq!(escaped_prefix("00-abc-def-01"), "00-abc-def-01");
    }
}
