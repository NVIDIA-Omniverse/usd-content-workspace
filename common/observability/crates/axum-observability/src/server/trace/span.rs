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

//! Functions to create and annotate a server span.
use crate::common::error_chain::error_chain;
use opentelemetry_semantic_conventions::attribute as sem;
use std::error::Error;
use std::fmt;
use tracing::Span;

pub mod __macro_support {
    //! Not a public API, macro only. No stability guarantees.
    #![doc(hidden)]
    pub use super::MakeSpan;
    pub use crate::common::extract::{
        method, req_http_host, req_user_agent_from_headers, route, span_name, version,
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __server_trace_otel_make_span {
    () => {
        $crate::__server_trace_otel_make_span!($crate::__macro_support::tracing::Level::INFO, ::core::module_path!())
    };
    ($target:expr) => {
        $crate::__server_trace_otel_make_span!($crate::__macro_support::tracing::Level::INFO, $target)
    };
    ($level:expr, $target:expr) => {{
        fn make_span(parts: &$crate::__macro_support::http::request::Parts) -> $crate::__macro_support::tracing::Span {
            use $crate::__macro_support::{server_trace, tracing, otel, sem};
            let method = server_trace::method(&parts.method);

            // https://opentelemetry.io/docs/specs/semconv/http/http-spans/#name
            let span_method = method.unwrap_or("HTTP");
            let route = server_trace::route(&parts.extensions);
            let otel_name = server_trace::span_name(span_method, route);

            let attr_method = method.unwrap_or("_OTHER");

            let version = server_trace::version(parts.version);
            let user_agent = server_trace::req_user_agent_from_headers(&parts.headers);
            let http_host = server_trace::req_http_host(parts);

            let span = tracing::span!(
                target: $target,
                $level,
                "HTTP request",
                // --- `tracing_opentelemetry` special fields
                // https://docs.rs/tracing-opentelemetry/latest/tracing_opentelemetry/#special-fields
                // otel.name: Override the span name sent to OpenTelemetry exporters.
                otel.name = %otel_name,
                otel.kind = ?otel::SpanKind::Server,
                { sem::OTEL_STATUS_CODE } = tracing::field::Empty,
                { sem::OTEL_STATUS_DESCRIPTION } = tracing::field::Empty,
                // --- semconv fields ---
                // https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-server-span
                { sem::ERROR_TYPE } = tracing::field::Empty,
                { sem::HTTP_REQUEST_METHOD } = %attr_method,
                { sem::HTTP_REQUEST_METHOD_ORIGINAL } = tracing::field::Empty,
                { sem::HTTP_RESPONSE_STATUS_CODE } = tracing::field::Empty,
                { sem::HTTP_ROUTE } = tracing::field::Empty,
                { sem::NETWORK_PROTOCOL_VERSION } = %version,
                { sem::SERVER_ADDRESS } = http_host,
                { sem::URL_PATH } = %parts.uri.path(),
                { sem::USER_AGENT_ORIGINAL } = user_agent,
            );

            if method.is_none() {
                span.record(sem::HTTP_REQUEST_METHOD_ORIGINAL, parts.method.as_str());
            }

            if let Some(path) = route {
                span.record(sem::HTTP_ROUTE, path);
            }

            span
        }
        unsafe { $crate::__macro_support::server_trace::MakeSpan::__macro_only_no_really_do_not_call_this(make_span) }
    }};
}

#[doc(hidden)]
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct MakeSpan(pub(super) fn(parts: &http::request::Parts) -> Span);

impl fmt::Debug for MakeSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(stringify!(MakeSpan)).finish_non_exhaustive()
    }
}

impl MakeSpan {
    /// Internal function for [`make_span!`](crate::server::trace::make_span).
    ///
    /// # Safety
    ///
    /// There is no safe way to call this.
    ///
    /// The span must be constructed with the correct fields for the trace
    /// layer. The only way to do this reliably is via the macro.
    #[doc(hidden)]
    pub const unsafe fn __macro_only_no_really_do_not_call_this(
        f: fn(parts: &http::request::Parts) -> Span,
    ) -> Self {
        Self(f)
    }
}

/// Record the HTTP response status code on the span.
///
/// This is always a single write (from response headers), so there is no
/// risk of duplicate attributes.
pub(super) fn record_http_status(span: &Span, code: http::StatusCode) {
    // per semconv, this is a numeric code (int)
    // https://opentelemetry.io/docs/specs/semconv/registry/attributes/http/
    span.record(sem::HTTP_RESPONSE_STATUS_CODE, code.as_u16());
}

/// The span status.
///
/// We must defer recording the span status. As of writing, `tracing`/the
/// integration with `opentelemetry` records fields as a `Vec`. So fields
/// can potentially be emitted multiple times.
#[derive(Debug)]
pub(super) enum SpanStatus {
    StatusCode(http::StatusCode),
    Error {
        error_type: &'static str,
        message: String,
    },
}

impl SpanStatus {
    pub(super) fn new(status: http::StatusCode) -> Self {
        Self::StatusCode(status)
    }

    /// Record a response body streaming error, and emit an exception event.
    pub(super) fn record_body_error<E>(&mut self, e: &E)
    where
        E: Error + 'static,
    {
        let error_type = std::any::type_name::<E>();
        let message = e.to_string();

        // body streaming errors take precedence over status codes
        *self = Self::Error {
            error_type,
            message,
        };

        // emit an exception event per the HTTP exception semconv (severity
        // should be ERROR)
        tracing::error!(
            { sem::EXCEPTION_TYPE } = error_type,
            { sem::EXCEPTION_MESSAGE } = %e,
            { sem::EXCEPTION_STACKTRACE } = error_chain(e),
            "http.server.request.exception",
        );
    }

    pub(super) fn finalize_span(&self, span: &Span) {
        match self {
            Self::StatusCode(status) if status.is_server_error() => {
                span.record(sem::OTEL_STATUS_CODE, "ERROR");
                // use the numeric value as per the example:
                // https://opentelemetry.io/docs/specs/semconv/registry/attributes/error/#error-type
                span.record(sem::ERROR_TYPE, status.as_str());
                // don't emit a message/description, it contains no further
                // information
                // https://opentelemetry.io/docs/specs/semconv/general/recording-errors/#recording-errors-on-spans
            }
            Self::StatusCode(_status) => {}
            Self::Error {
                error_type,
                message,
            } => {
                span.record(sem::OTEL_STATUS_CODE, "ERROR");
                span.record(sem::ERROR_TYPE, error_type);
                span.record(sem::OTEL_STATUS_DESCRIPTION, message);
            }
        }
    }
}
