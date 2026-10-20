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

use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use opentelemetry_proto::tonic::trace::v1::SpanFlags;

pub(crate) fn try_make_trace_id(trace_id: &[u8]) -> Result<TraceId, String> {
    if trace_id.is_empty() {
        Ok(TraceId::INVALID)
    } else {
        let bytes: [u8; 16] = trace_id
            .try_into()
            .map_err(|_e| format!("invalid trace ID `{}`", hex::encode(trace_id)))?;
        Ok(TraceId::from_bytes(bytes))
    }
}

pub(crate) fn make_trace_id(trace_id: &[u8]) -> TraceId {
    try_make_trace_id(trace_id).unwrap_or_else(|msg| panic!("{msg}"))
}

pub(crate) fn try_make_span_id(span_id: &[u8]) -> Result<SpanId, String> {
    if span_id.is_empty() {
        Ok(SpanId::INVALID)
    } else {
        let bytes: [u8; 8] = span_id
            .try_into()
            .map_err(|_e| format!("invalid span ID `{}`", hex::encode(span_id)))?;
        Ok(SpanId::from_bytes(bytes))
    }
}

pub(crate) fn make_span_id(span_id: &[u8]) -> SpanId {
    try_make_span_id(span_id).unwrap_or_else(|msg| panic!("{msg}"))
}

fn make_flags(flags: u32) -> (TraceFlags, bool) {
    const HAS_IS_REMOTE: u32 = SpanFlags::ContextHasIsRemoteMask as _;
    const IS_REMOTE: u32 = SpanFlags::ContextIsRemoteMask as _;
    const MASK_REMOTE: u32 = !(HAS_IS_REMOTE | IS_REMOTE);

    let is_remote = if (flags & HAS_IS_REMOTE) != HAS_IS_REMOTE {
        false
    } else {
        (flags & IS_REMOTE) == IS_REMOTE
    };

    let trace_flags = match flags & MASK_REMOTE {
        0 => TraceFlags::NOT_SAMPLED,
        1 => TraceFlags::SAMPLED,
        _ => panic!("invalid flags {flags:#010x}"),
    };

    (trace_flags, is_remote)
}

fn make_state(state: &str) -> TraceState {
    if state.is_empty() {
        TraceState::NONE
    } else {
        use std::str::FromStr as _;
        TraceState::from_str(state).unwrap_or_else(|e| panic!("invalid trace state `{state}`: {e}"))
    }
}

pub(crate) fn make(trace_id: &[u8], span_id: &[u8], flags: u32, state: &str) -> SpanContext {
    let trace_id = make_trace_id(trace_id);
    let span_id = make_span_id(span_id);
    let (trace_flags, is_remote) = make_flags(flags);
    let trace_state = make_state(state);

    SpanContext::new(trace_id, span_id, trace_flags, is_remote, trace_state)
}
