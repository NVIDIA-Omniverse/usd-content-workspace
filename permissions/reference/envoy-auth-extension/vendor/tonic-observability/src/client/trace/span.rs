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

//! Functions to create and annotate a client span.
use crate::common::classify::client;
use crate::common::error_chain::error_chain;
use crate::common::extract;
use opentelemetry_semantic_conventions::attribute as sem;
use std::error::Error;
use std::fmt;
use tonic::codegen::http;
use tracing::Span;

pub mod __macro_support {
    //! Not a public API, macro only. No stability guarantees.
    #![doc(hidden)]
    pub use super::{MakeSpan, RpcInfo, rpc_info};
}

/// Not a public API, macro only. No stability guarantees.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub enum RpcInfo<'a> {
    Known(String),
    Unknown(&'a str),
}

impl RpcInfo<'_> {
    pub fn span_name(&self) -> &str {
        match self {
            Self::Known(s) => s,
            Self::Unknown(_) => {
                // when `rpc.method` is `_OTHER`, the span name should be
                // `{rpc.system.name}` per the RPC span name spec.
                "grpc"
            }
        }
    }

    pub fn rpc_method(&self) -> &str {
        match self {
            Self::Known(s) => s,
            Self::Unknown(_) => "_OTHER",
        }
    }

    pub fn rpc_method_original(&self) -> Option<&str> {
        match self {
            Self::Known(_) => None,
            Self::Unknown(s) => Some(s),
        }
    }
}

/// Not a public API, macro only. No stability guarantees.
#[doc(hidden)]
pub fn rpc_info<'a>(parts: &'a http::request::Parts) -> RpcInfo<'a> {
    // the middleware sits after tonic, so `GrpcMethod` should always exist.
    match parts.extensions.get::<tonic::codegen::GrpcMethod>() {
        Some(grpc_method) => {
            // the extension returns the service and method separately, and we
            // need the value for both the span name and attribute, so
            // allocating a string here probably makes sense
            RpcInfo::Known(format!(
                "{}/{}",
                grpc_method.service(),
                grpc_method.method(),
            ))
        }
        None => {
            let path = parts.uri.path();
            let path = path.strip_prefix('/').unwrap_or(path);
            RpcInfo::Unknown(path)
        }
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __client_trace_otel_make_span {
    () => {
        $crate::__client_trace_otel_make_span!($crate::__macro_support::tracing::Level::INFO, ::core::module_path!())
    };
    ($target:expr) => {
        $crate::__client_trace_otel_make_span!($crate::__macro_support::tracing::Level::INFO, $target)
    };
    ($level:expr, $target:expr) => {{
        fn make_span(parts: &$crate::__macro_support::http::request::Parts, server_address: Option<&str>) -> $crate::__macro_support::tracing::Span {
            use $crate::__macro_support::{tracing, otel, sem, client_trace};
            let rpc_info = client_trace::rpc_info(parts);
            tracing::span!(
                target: $target,
                $level,
                "gRPC request",
                // --- `tracing_opentelemetry` special fields
                // https://docs.rs/tracing-opentelemetry/latest/tracing_opentelemetry/#special-fields
                // otel.name: Override the span name sent to OpenTelemetry exporters.
                otel.name = rpc_info.span_name(),
                otel.kind = ?otel::SpanKind::Client,
                { sem::OTEL_STATUS_CODE } = tracing::field::Empty,
                { sem::OTEL_STATUS_DESCRIPTION } = tracing::field::Empty,
                // --- semconv fields ---
                // https://opentelemetry.io/docs/specs/semconv/rpc/rpc-spans/
                // https://opentelemetry.io/docs/specs/semconv/rpc/grpc/
                //
                // note: the gRPC semconv provides specific guidance on
                // `server.address`; it's similar to `grpc.target`. we
                // specifically opt to not emit `server.port` separately, to
                // match the gRPC A66 metric spec.
                { sem::SERVER_ADDRESS } = server_address,
                // TODO: use upstream constants when
                // opentelemetry-semantic-conventions is updated.
                "rpc.system.name" = "grpc",
                { sem::RPC_METHOD } = rpc_info.rpc_method(),
                "rpc.method_original" = rpc_info.rpc_method_original(),
                "rpc.response.status_code" = tracing::field::Empty,
                { sem::ERROR_TYPE } = tracing::field::Empty,
            )
        }
        unsafe { $crate::__macro_support::client_trace::MakeSpan::__macro_only_no_really_do_not_call_this(make_span) }
    }};
}

#[doc(hidden)]
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct MakeSpan(
    pub(super) fn(parts: &http::request::Parts, server_address: Option<&str>) -> Span,
);

impl fmt::Debug for MakeSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(stringify!(MakeSpan)).finish_non_exhaustive()
    }
}

impl MakeSpan {
    /// Internal function for [`make_span!`](crate::client::trace::make_span).
    ///
    /// # Safety
    ///
    /// There is no safe way to call this.
    ///
    /// The span must be constructed with the correct fields for the trace
    /// layer. The only way to do this reliably is via the macro.
    #[doc(hidden)]
    pub const unsafe fn __macro_only_no_really_do_not_call_this(
        f: fn(parts: &http::request::Parts, server_address: Option<&str>) -> Span,
    ) -> Self {
        Self(f)
    }
}

/// Annotate a span based on a transport error.
pub(super) fn record_transport_err(span: &Span, e: &tonic::transport::Error) {
    let code = client::transport_err_to_code(e);
    // code won't be Ok, so we always record an error. also, the code is always
    // known; we cannot produce codes incompatible with `tonic::Code` here.
    let name = extract::resp_known_status_code_to_name(code);

    // record `rpc.response.status_code` as a canonical name
    // TODO: use upstream constant when available
    const RPC_RESPONSE_STATUS_CODE: &str = "rpc.response.status_code";
    span.record(RPC_RESPONSE_STATUS_CODE, name);

    // record `otel.status_code`/the span status as an error
    span.record(sem::OTEL_STATUS_CODE, "ERROR");
    // record `otel.status_description`/the span message
    span.record(sem::OTEL_STATUS_DESCRIPTION, tracing::field::display(e));

    // record `error.type` as a canonical name
    span.record(sem::ERROR_TYPE, name);

    // always emit an exception event (severity should be WARN)
    tracing::warn!(
        { sem::EXCEPTION_TYPE } = stringify!(tonic::transport::Error),
        { sem::EXCEPTION_MESSAGE } = %e,
        { sem::EXCEPTION_STACKTRACE } = %error_chain(e),
        "rpc.client.call.exception",
    );
}

/// The span status.
///
/// We must defer recording the span status. As of writing, `tracing`/the
/// integration with `opentelemetry` records fields as a `Vec`. So field can
/// potentially be emitted multiple times.
#[derive(Debug, PartialEq)]
pub(super) struct SpanStatus {
    pub(super) code: Option<u16>,
    pub(super) message: Option<String>,
}

impl SpanStatus {
    pub(super) fn new() -> Self {
        Self {
            code: None,
            message: None,
        }
    }

    /// Record the span status based on the response's gRPC status header.
    pub(super) fn record_grpc_status(
        &mut self,
        status: &http::HeaderValue,
        message: Option<&http::HeaderValue>,
    ) {
        let Some(code) = extract::resp_grpc_status_from_header_value(status) else {
            return;
        };

        self.code = Some(code);

        // for a client, everything other than Ok is considered an error
        if client::code_is_error(code) {
            self.message = message.and_then(|v| v.to_str().map(|s| s.to_string()).ok());
        } else {
            self.message = None;
        }
    }

    /// Record the span status based on the response's HTTP status if not
    /// OK/200.
    ///
    /// It is invalid to call this function if the status code is OK/200. This
    /// function is to handle errors from e.g. broken proxies. In a gRPC stream,
    /// a status code of OK/200 is meaningless as the true error is carried via
    /// other mechanisms.
    pub(super) fn record_http_status(&mut self, status: http::StatusCode) {
        let code = client::http_to_tonic_status(status) as u16;

        self.code = Some(code);
        // code won't be Ok, so we always record an error
        // message is e.g. "502 Bad Gateway"
        self.message = Some(status.to_string());
    }

    /// Record the span status based on the response's [`tonic::Status`].
    pub(super) fn record_tonic_status(&mut self, status: &tonic::Status) {
        let code = status.code() as u16;

        self.code = Some(code);

        // for a client, everything other than Ok is considered an error
        if client::code_is_error(code) {
            self.message = Some(status.message().to_string());

            // only emit an exception event if the status has a source (severity
            // should be WARN)
            if status.source().is_some() {
                tracing::warn!(
                    { sem::EXCEPTION_TYPE } = stringify!(tonic::Status),
                    { sem::EXCEPTION_MESSAGE } = status.message(),
                    { sem::EXCEPTION_STACKTRACE } = %error_chain(status),
                    "rpc.client.call.exception",
                );
            }
        } else {
            self.message = None;
        }
    }

    pub(super) fn finalize_span(&self, span: &Span) {
        let Some(code) = self.code else { return };
        let name = extract::resp_grpc_status_code_to_name(code);

        // record `rpc.response.status_code` as a canonical name, or the raw
        // code to be future compatible
        // TODO: use upstream constant when available
        const RPC_RESPONSE_STATUS_CODE: &str = "rpc.response.status_code";
        match name {
            Some(name) => span.record(RPC_RESPONSE_STATUS_CODE, name),
            None => span.record(RPC_RESPONSE_STATUS_CODE, tracing::field::display(code)),
        };

        if client::code_is_error(code) {
            // record `otel.status_code`/the span status as an error
            span.record(sem::OTEL_STATUS_CODE, "ERROR");
            // record `otel.status_description`/the span message if applicable
            if let Some(value) = self.message.as_deref() {
                span.record(sem::OTEL_STATUS_DESCRIPTION, value);
            }

            // record `error.type` as a canonical name, or the raw code to be
            // future compatible
            match name {
                Some(name) => span.record(sem::ERROR_TYPE, name),
                None => span.record(sem::ERROR_TYPE, tracing::field::display(code)),
            };
        }
    }
}
