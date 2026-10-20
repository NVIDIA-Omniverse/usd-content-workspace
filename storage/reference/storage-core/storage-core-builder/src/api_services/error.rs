// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::LazyLock;

use axum::body::Body;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use storage_core_util::observability::error_severity::error_severity;
use storage_core_util::observability::trace_limited::TraceLimiter;
use storage_core_util::trace_at_level_limited;
use tonic::Status;
use tonic_types::StatusExt;

use crate::traits::error::{status_with_default_error_info, SourceLocation, StorageErrorExt};
use snafu::prelude::*;

/// Number of [`tonic::Code`] variants (Ok through Unauthenticated = 0..=16).
const TONIC_CODE_COUNT: usize = tonic::Code::Unauthenticated as usize + 1;

// ---------------------------------------------------------------------------
// Per-error-code rate limiters for API boundary logging
// ---------------------------------------------------------------------------

static GRPC_LIMITERS: LazyLock<Vec<TraceLimiter>> =
    LazyLock::new(|| (0..TONIC_CODE_COUNT).map(|_| TraceLimiter::default()).collect());

static HTTP_LIMITERS: LazyLock<Vec<TraceLimiter>> =
    LazyLock::new(|| (0..TONIC_CODE_COUNT).map(|_| TraceLimiter::default()).collect());

/// Fallback limiter for any tonic::Code value outside the known 0..=16 range.
/// Shared across all unknown codes — safe because TraceLimiter is thread-safe.
static FALLBACK_LIMITER: LazyLock<TraceLimiter> = LazyLock::new(TraceLimiter::default);

fn grpc_limiter(code: tonic::Code) -> &'static TraceLimiter {
    GRPC_LIMITERS.get(code as usize).unwrap_or(&FALLBACK_LIMITER)
}

fn http_limiter(code: tonic::Code) -> &'static TraceLimiter {
    HTTP_LIMITERS.get(code as usize).unwrap_or(&FALLBACK_LIMITER)
}

// ---------------------------------------------------------------------------
// StorageApiServiceError
// ---------------------------------------------------------------------------

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), context(suffix(false)))]
pub enum StorageApiServiceError {
    #[snafu(display("{message}"))]
    InvalidArgument {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
}

impl StorageApiServiceError {
    #[track_caller]
    pub fn invalid_argument(message: String) -> Self {
        InvalidArgument { message }.build()
    }
}

impl StorageErrorExt for StorageApiServiceError {
    fn tonic_status(&self) -> tonic::Status {
        match self {
            StorageApiServiceError::InvalidArgument { .. } => {
                status_with_default_error_info(tonic::Code::InvalidArgument, self.to_string(), "InvalidArgument")
            },
        }
    }

    fn http_status(&self) -> http::StatusCode {
        match self {
            StorageApiServiceError::InvalidArgument { .. } => http::StatusCode::BAD_REQUEST,
        }
    }

    fn get_size(&self) -> usize {
        match self {
            StorageApiServiceError::InvalidArgument { message, .. } => {
                std::mem::size_of::<String>() + message.capacity()
            },
        }
    }

    fn source_location(&self) -> Option<SourceLocation> {
        match self {
            StorageApiServiceError::InvalidArgument { loc, .. } => Some(loc.into()),
        }
    }
}

// ---------------------------------------------------------------------------
// API boundary error logging
// ---------------------------------------------------------------------------

/// Logs an error at the severity configured for its gRPC status code (with
/// per-error-code rate limiting), then converts it to a [`tonic::Status`].
pub fn log_grpc_error(e: &dyn StorageErrorExt, context: &str) -> Status {
    let s = e.tonic_status();
    let level = error_severity().level(s.code());
    let location = e.source_location();
    trace_at_level_limited!(
        limit: grpc_limiter(s.code()),
        level,
        code = ?s.code(), context,
        source_location = %LocationDisplay(&location),
        error = %e
    );
    s
}

/// Logs an error at the severity configured for its gRPC status code (with
/// per-error-code rate limiting), then returns an RFC 9457 [`ProblemDetails`]
/// response populated from the error's gRPC `ErrorInfo` details.
pub fn log_http_error(e: &dyn StorageErrorExt, context: &str) -> ProblemDetails {
    let status = e.http_status();
    let tonic_status = e.tonic_status();
    let code = tonic_status.code();
    let level = error_severity().level(code);
    let location = e.source_location();
    trace_at_level_limited!(
        limit: http_limiter(code),
        level,
        code = %status, context,
        source_location = %LocationDisplay(&location),
        error = %e
    );
    ProblemDetails::from_status(status, &tonic_status)
}

/// Display wrapper that formats `Option<SourceLocation>` without allocating.
struct LocationDisplay<'a>(&'a Option<SourceLocation>);

impl std::fmt::Display for LocationDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(loc) => write!(f, "{loc}"),
            None => write!(f, "unknown"),
        }
    }
}

// ---------------------------------------------------------------------------
// RFC 9457 Problem Details
// ---------------------------------------------------------------------------

/// An [RFC 9457] Problem Details response for HTTP APIs.
///
/// When a REST handler returns an error, this struct is serialized as
/// `application/problem+json`. The `reason`, `domain`, and `metadata`
/// extension members are populated from the [`ErrorInfo`] embedded in
/// the gRPC status (via [`tonic_types::StatusExt`]), giving REST clients
/// the same structured error information that gRPC clients receive
/// through the `grpc-status-details-bin` trailer.
///
/// [RFC 9457]: https://www.rfc-editor.org/rfc/rfc9457
/// [`ErrorInfo`]: tonic_types::ErrorInfo
#[derive(Debug, serde::Serialize)]
pub struct ProblemDetails {
    /// A URI reference identifying the problem type (default: `"about:blank"`).
    #[serde(rename = "type")]
    pub problem_type: &'static str,

    /// The HTTP status code.
    pub status: u16,

    /// A short, human-readable summary of the problem type.
    pub title: &'static str,

    /// A human-readable explanation specific to this occurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,

    /// Machine-readable error reason from gRPC `ErrorInfo`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Error domain from gRPC `ErrorInfo` (e.g. `"storage-core"`, `"aws.s3"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,

    /// Additional metadata from gRPC `ErrorInfo` (e.g. `{"backend": "s3"}`).
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl ProblemDetails {
    /// Build a `ProblemDetails` from an HTTP status code and a [`tonic::Status`]
    /// whose `grpc-status-details-bin` trailer carries `ErrorInfo`.
    pub fn from_status(http_status: StatusCode, tonic_status: &tonic::Status) -> Self {
        let error_info = tonic_status.get_details_error_info();
        let detail = {
            let msg = tonic_status.message();
            if msg.is_empty() {
                None
            } else {
                Some(msg.to_string())
            }
        };
        Self {
            problem_type: "about:blank",
            status: http_status.as_u16(),
            title: http_status.canonical_reason().unwrap_or("Unknown Error"),
            detail,
            reason: error_info
                .as_ref()
                .map(|ei| &ei.reason)
                .filter(|s| !s.is_empty())
                .cloned(),
            domain: error_info
                .as_ref()
                .map(|ei| &ei.domain)
                .filter(|s| !s.is_empty())
                .cloned(),
            metadata: error_info.map(|ei| ei.metadata).unwrap_or_default(),
        }
    }

    /// Build a `ProblemDetails` from any [`StorageErrorExt`] implementor.
    pub fn from_error(e: &dyn StorageErrorExt) -> Self {
        Self::from_status(e.http_status(), &e.tonic_status())
    }
}

/// Converts a bare [`StatusCode`] into a minimal `ProblemDetails` with no
/// extended error information. This keeps existing `Err(StatusCode::*)` sites
/// working after the handler return type changes.
impl From<StatusCode> for ProblemDetails {
    fn from(status: StatusCode) -> Self {
        Self {
            problem_type: "about:blank",
            status: status.as_u16(),
            title: status.canonical_reason().unwrap_or("Unknown Error"),
            detail: None,
            reason: None,
            domain: None,
            metadata: HashMap::new(),
        }
    }
}

impl IntoResponse for ProblemDetails {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        match serde_json::to_string(&self) {
            Ok(body) => Response::builder()
                .status(status)
                .header("Content-Type", "application/problem+json")
                .body(Body::from(body))
                .unwrap_or_else(|_| {
                    let mut resp = Response::new(Body::from("Internal Server Error"));
                    *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                    resp
                }),
            Err(_) => {
                let mut resp = Response::new(Body::from("Internal Server Error"));
                *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                resp
            },
        }
    }
}
