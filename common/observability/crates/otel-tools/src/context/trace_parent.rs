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

//! W3C Trace Context `traceparent` header parsing and formatting.
//!
//! Why was this re-invented? The `opentelemetry-sdk` traceparent code is poorly
//! documented, and I found some issues:
//!
//! * Whitespace in the header is trimmed. This is not mentioned in the spec.
//! * Invalid lengths are not rejected early.
//! * Handling of unknown flags in version 0x00 is unclear in the spec, and may
//!   be too strict.
//! * Hard to inspect where the parsing failed.
//!
//! See <https://github.com/open-telemetry/opentelemetry-rust/blob/v0.26.1/opentelemetry-sdk/src/propagation/trace_context.rs#L58-L117>
#![allow(clippy::identity_op, reason = "zero indexing makes the code clearer")]
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use std::cmp::Ordering;
use std::fmt;

/// Format the span context information into a `traceparent` header.
///
/// This trait is implemented for [`SpanContext`].
pub trait SpanContextToHeader {
    /// Format the span context information into a `traceparent` header.
    fn header(&self) -> Option<String>;
}

impl SpanContextToHeader for SpanContext {
    fn header(&self) -> Option<String> {
        // do not create header if the span context is invalid
        if !self.is_valid() {
            return None;
        }

        // https://www.w3.org/TR/trace-context/#other-flags
        //
        // > The behavior of other flags, such as (`00000100`) is not defined and is
        // > reserved for future use. Vendors MUST set those to zero.
        let trace_flags = self.trace_flags() & TraceFlags::SAMPLED;
        Some(format!(
            "00-{}-{}-{:02x}",
            self.trace_id(),
            self.span_id(),
            trace_flags,
        ))
    }
}

/// The only invalid version.
///
/// See https://www.w3.org/TR/trace-context/#version.
const INVALID_VERSION: u8 = 0xFF;

/// A parsed `traceparent` header, version 0x00.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceParent {
    pub(crate) trace_id: TraceId,
    pub(crate) parent_id: SpanId,
    pub(crate) trace_flags: TraceFlags,
}

/// A `traceparent` header is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceParentError(pub(crate) TraceParentErrorInner);

impl fmt::Display for TraceParentError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for TraceParentError {}

impl TraceParentError {
    pub(crate) fn too_short() -> Self {
        Self(TraceParentErrorInner::TooShort)
    }
    pub(crate) fn too_long() -> Self {
        Self(TraceParentErrorInner::TooLong)
    }
    pub(crate) fn not_hex(index: usize) -> Self {
        Self(TraceParentErrorInner::NotHex(index))
    }
    pub(crate) fn not_delim(index: usize) -> Self {
        Self(TraceParentErrorInner::NotDelim(index))
    }
    pub(crate) fn invalid_version() -> Self {
        Self(TraceParentErrorInner::InvalidVersion)
    }
    pub(crate) fn trace_id() -> Self {
        Self(TraceParentErrorInner::TraceId)
    }
    pub(crate) fn parent_id() -> Self {
        Self(TraceParentErrorInner::ParentId)
    }
}

/// A `traceparent` header is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TraceParentErrorInner {
    /// The header is too short. 55 or more ASCII characters are required for
    /// all versions.
    TooShort,
    /// If the version is 0x00, exactly 55 ASCII characters are required.
    TooLong,
    /// A character was expected to be a hexadecimal digit, but wasn't.
    NotHex(usize),
    /// A character was expected to be a delimiter/`-`, but wasn't.
    NotDelim(usize),
    /// The version is 0xFF, which is invalid.
    InvalidVersion,
    /// The trace ID is invalid (all zeros).
    TraceId,
    /// The parent ID is invalid (all zeros).
    ParentId,
}

impl fmt::Display for TraceParentErrorInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TooShort => f.write_str("value is too short"),
            Self::TooLong => f.write_str("value is too long"),
            Self::NotHex(index) => write!(f, "expected a hexadecimal value at byte {index}"),
            Self::NotDelim(index) => write!(f, "expected a delimiter at byte {index}"),
            Self::InvalidVersion => f.write_str("version is invalid"),
            Self::TraceId => f.write_str("trace ID is invalid"),
            Self::ParentId => f.write_str("parent ID is invalid"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastByte {
    None,
    Dash,
    Other,
}

macro_rules! lower_hex_byte {
    ($b:ident, $i:expr) => {{
        let i = $i + 0;
        let j = $i + 1;
        let hi = lower_hex_digit($b[i]).ok_or(TraceParentError::not_hex(i))?;
        let lo = lower_hex_digit($b[j]).ok_or(TraceParentError::not_hex(j))?;
        hi << 4 | lo
    }};
}
pub(super) use lower_hex_byte;

impl TraceParent {
    /// A trace parent with invalid trace and parent IDs.
    pub const NONE: Self = Self {
        trace_id: TraceId::INVALID,
        parent_id: SpanId::INVALID,
        trace_flags: TraceFlags::NOT_SAMPLED,
    };

    /// Convert this trace parent to a remote span context.
    #[inline]
    pub fn remote_span_context(self, trace_state: TraceState) -> SpanContext {
        let Self {
            trace_id,
            parent_id,
            trace_flags,
        } = self;
        SpanContext::new(trace_id, parent_id, trace_flags, true, trace_state)
    }

    /// Parse trace parent information from a `traceparent` header.
    pub fn from_header(header: &[u8]) -> Result<Self, TraceParentError> {
        // https://www.w3.org/TR/trace-context/#traceparent-header-field-values
        // https://www.w3.org/TR/trace-context/#versioning-of-traceparent
        //
        // > If the size of the header is shorter than 55 characters, the vendor
        // > should not parse the header and should restart the trace.
        let (b, last_b) = match header.len().cmp(&55) {
            Ordering::Equal => {
                let b = <&[u8; 55]>::try_from(header).expect("infallible");
                (b, LastByte::None)
            }
            Ordering::Greater => {
                // later versions may be longer
                let b = <&[u8; 55]>::try_from(&header[..55]).expect("infallible");
                // when validating the flags, it must be at the end of the header or
                // followed by a dash.
                let last_byte = if header[55] == b'-' {
                    LastByte::Dash
                } else {
                    LastByte::Other
                };
                (b, last_byte)
            }
            Ordering::Less => {
                return Err(TraceParentError::too_short());
            }
        };

        // https://www.w3.org/TR/trace-context/#version
        //
        // > When the version prefix cannot be parsed (it's not 2 hex characters
        // followed by a dash (-)), the implementation should restart the trace.
        let version = lower_hex_byte!(b, 0);
        if b[2] != b'-' {
            return Err(TraceParentError::not_delim(2));
        }
        if version == INVALID_VERSION {
            return Err(TraceParentError::invalid_version());
        }
        if version == 0x00 && last_b != LastByte::None {
            return Err(TraceParentError::too_long());
        }
        // note: if version is higher than the supported version, proceed with
        // parsing as if it were the supported version.
        //
        // see https://www.w3.org/TR/trace-context/#a-traceparent-is-received

        // https://www.w3.org/TR/trace-context/#versioning-of-traceparent
        //
        // > Parse `trace-id` (from the first dash through the next 32
        // > characters). Vendors MUST check that the 32 characters are hex, and
        // > that they are followed by a dash (-).
        let mut bytes = [0u8; 16];
        let mut j = 3;
        for t in bytes.iter_mut() {
            *t = lower_hex_byte!(b, j);
            j += 2;
        }
        if b[35] != b'-' {
            return Err(TraceParentError::not_delim(35));
        }
        // https://www.w3.org/TR/trace-context/#trace-id
        //
        // > If the `trace-id` value is invalid, vendors MUST ignore the
        // `traceparent`.
        let trace_id = TraceId::from_bytes(bytes);
        if trace_id == TraceId::INVALID {
            return Err(TraceParentError::trace_id());
        }

        // https://www.w3.org/TR/trace-context/#versioning-of-traceparent
        //
        // > Parse `parent-id` (from the second dash at the 35th position
        // > through the next 16 characters). Vendors MUST check that the 16
        // > characters are hex and followed by a dash.
        let mut bytes = [0u8; 8];
        let mut j = 36;
        for t in bytes.iter_mut() {
            *t = lower_hex_byte!(b, j);
            j += 2;
        }
        if b[52] != b'-' {
            return Err(TraceParentError::not_delim(52));
        }
        // https://www.w3.org/TR/trace-context/#parent-id
        //
        // > Vendors MUST ignore the `traceparent` when the `parent-id` is
        // invalid.
        let parent_id = SpanId::from_bytes(bytes);
        if parent_id == SpanId::INVALID {
            return Err(TraceParentError::parent_id());
        }

        // https://www.w3.org/TR/trace-context/#versioning-of-traceparent
        //
        // > Parse the `sampled` bit of `flags` (2 characters from the third
        // > dash). Vendors MUST check that the 2 characters are either the end
        // > of the string or a dash.
        let flags = lower_hex_byte!(b, 53);
        if last_b == LastByte::Other {
            return Err(TraceParentError::not_delim(55));
        }
        // https://www.w3.org/TR/trace-context/#other-flags
        //
        // > The behavior of other flags, such as (`00000100`) is not defined and is
        // > reserved for future use. Vendors MUST set those to zero.
        //
        // technically, if the version is 0x00, then only the `sampled` flag
        // may be set. however, the W3C TR does not mention if this is a hard
        // error.
        let trace_flags = TraceFlags::new(flags) & TraceFlags::SAMPLED;

        Ok(Self {
            trace_id,
            parent_id,
            trace_flags,
        })
    }
}

/// Converts a lowercase hexadecimal UTF-8 byte to a digit.
///
/// Basically a [`u8`] and hex-specific version of [`char::to_digit()`].
const fn lower_hex_digit(b: u8) -> Option<u8> {
    let mut digit = b.wrapping_sub(b'0');
    if digit < 10 {
        return Some(digit);
    }
    digit = b.wrapping_sub(b'a').saturating_add(10);
    if digit < 16 {
        return Some(digit);
    }
    None
}

#[cfg(test)]
mod tests;
