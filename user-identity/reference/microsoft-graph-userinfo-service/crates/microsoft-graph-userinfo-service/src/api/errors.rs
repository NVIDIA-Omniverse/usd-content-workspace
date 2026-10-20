// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::provider::ProviderError;

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
    message: String,
}

#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    Unauthorized(String),
    Forbidden(String),
    BadRequest(String),
    BadGateway(String),
    Internal(String),
    RateLimited { retry_after: u64 },
}

impl From<ProviderError> for ApiError {
    fn from(e: ProviderError) -> Self {
        match e {
            ProviderError::NotFound(msg) => Self::NotFound(msg),
            ProviderError::Unauthorized(msg) => Self::Unauthorized(msg),
            ProviderError::Forbidden(msg) => Self::Forbidden(msg),
            ProviderError::InvalidCursor(msg) => Self::BadRequest(msg),
            ProviderError::Upstream(msg) => Self::BadGateway(msg),
            ProviderError::Internal(msg) => Self::Internal(msg),
            ProviderError::RateLimited { retry_after } => Self::RateLimited { retry_after },
            ProviderError::DeltaExpired => Self::Internal("delta token expired".into()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // Client-visible messages are stable, generic strings that never
        // embed upstream identity-provider diagnostics, internal error
        // chains, or anything else that could echo attacker-controlled
        // input back to the caller. The verbose `detail` is kept only
        // for server-side logs.
        let (status, error_tag, client_message, detail, retry_after) = match self {
            Self::NotFound(m) => (
                StatusCode::NOT_FOUND,
                "not_found",
                "not found",
                Some(m),
                None,
            ),
            Self::Unauthorized(m) => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "not authorized by identity provider",
                Some(m),
                None,
            ),
            Self::Forbidden(m) => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "forbidden by identity provider",
                Some(m),
                None,
            ),
            Self::BadRequest(m) => (
                StatusCode::BAD_REQUEST,
                "bad_request",
                "bad request",
                Some(m),
                None,
            ),
            Self::BadGateway(m) => (
                StatusCode::BAD_GATEWAY,
                "bad_gateway",
                "upstream identity provider error",
                Some(m),
                None,
            ),
            Self::Internal(m) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal error",
                Some(m),
                None,
            ),
            Self::RateLimited { retry_after } => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "rate limited",
                None,
                Some(retry_after),
            ),
        };

        // Log level is chosen by response status class:
        //
        // - 5xx responses indicate a server-side problem (upstream IdP
        //   failure, internal bug) and are logged at `error!` so they
        //   surface on operator dashboards.
        // - `429 Too Many Requests` is a client-visible status but is
        //   operator-actionable (may signal upstream throttling or a
        //   runaway caller), so it is logged at `warn!`.
        // - All other 4xx responses (`401`, `403`, `404`) are expected
        //   client-side mistakes — an unknown user id, a missing bearer
        //   token, a caller without permission. They are normal traffic
        //   and MUST NOT pollute the error log; they are logged at
        //   `debug!` and only surface when explicitly enabled.
        let detail_for_log = detail.as_deref().unwrap_or("");
        if status.is_server_error() {
            tracing::error!(
                status = status.as_u16(),
                error = error_tag,
                detail = %detail_for_log,
            );
        } else if status == StatusCode::TOO_MANY_REQUESTS {
            tracing::warn!(
                status = status.as_u16(),
                error = error_tag,
                retry_after = retry_after.unwrap_or(0),
            );
        } else {
            tracing::debug!(
                status = status.as_u16(),
                error = error_tag,
                detail = %detail_for_log,
            );
        }

        let mut resp = (
            status,
            axum::Json(ErrorBody {
                error: error_tag.to_owned(),
                message: client_message.to_owned(),
            }),
        )
            .into_response();

        if let Some(secs) = retry_after {
            resp.headers_mut()
                .insert("retry-after", axum::http::HeaderValue::from(secs));
        }

        // Surface the internal reason to the access-log middleware as
        // `response.detail.message`; it may be more specific than the generic
        // client-visible message above.
        if let Some(message) = detail {
            let mut fields = serde_json::Map::new();
            fields.insert("message".to_owned(), message.into());
            resp.extensions_mut()
                .insert(super::AccessResponseDetail(fields));
        }

        resp
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(m) => write!(f, "not found: {m}"),
            Self::Unauthorized(m) => write!(f, "unauthorized: {m}"),
            Self::Forbidden(m) => write!(f, "forbidden: {m}"),
            Self::BadRequest(m) => write!(f, "bad request: {m}"),
            Self::BadGateway(m) => write!(f, "bad gateway: {m}"),
            Self::Internal(m) => write!(f, "internal error: {m}"),
            Self::RateLimited { retry_after } => {
                write!(f, "rate limited (retry after {retry_after}s)")
            }
        }
    }
}
