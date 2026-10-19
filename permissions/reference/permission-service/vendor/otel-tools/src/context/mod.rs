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

//! W3C trace context utilities.
//!
//! The trace layers handle W3C trace context propagation, so usually it is
//! not necessary to use these utilities. However, they are exposed for unusual
//! propagation scenarios.
pub mod propagate;
pub(crate) mod trace_parent;

/// Re-exported for macro.
#[doc(hidden)]
pub use ::tracing_opentelemetry::OpenTelemetrySpanExt;
use opentelemetry::trace::SpanContext;
pub use trace_parent::{SpanContextToHeader, TraceParent, TraceParentError};
use tracing::Span;

#[doc(hidden)]
#[macro_export]
macro_rules! __get_span_context {
    ($span_context:ident = $span:ident) => {
        let span: &::tracing::Span = &$span;
        let context = {
            use $crate::context::OpenTelemetrySpanExt as _;
            span.context()
        };
        let span_ref = {
            use ::opentelemetry::trace::TraceContextExt as _;
            context.span()
        };
        let $span_context = span_ref.span_context();
    };
}

/// Get the span's context. See also [`set_span_context`].
///
/// This is harder than it seems: `context()` creates an owned value, as does
/// `span()`. So this is a macro instead of a function.
///
/// ```rust
/// use otel_tools::context::get_span_context;
/// let current_span = tracing::Span::current();
/// get_span_context!(span_context = current_span);
/// ```
#[doc(inline)]
pub use crate::__get_span_context as get_span_context;

/// Set the span's context to a remote context. See also [`get_span_context`].
///
/// WARNING: Due to a change in `tracing-opentelemetry`, it isn't possible to
/// set a span's parent after it has been built.
///
/// Callers must ensure the [`SpanContext`] is valid before calling this
/// function. The context must be remote.
#[must_use]
pub fn set_span_context(span: &Span, span_context: SpanContext) -> bool {
    // create context? this weird dance is necessary... `span.add_link()` does
    // not work
    let cx = {
        use opentelemetry::trace::TraceContextExt as _;
        opentelemetry::Context::map_current(|cx| cx.with_remote_span_context(span_context))
    };
    {
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;
        // `SetParentError` is currently not exported:
        // https://github.com/tokio-rs/tracing-opentelemetry/issues/236
        span.set_parent(cx).is_ok()
    }
}
