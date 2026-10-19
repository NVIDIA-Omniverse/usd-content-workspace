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

//! REST access-log middleware. Installed as the outermost global layer (after
//! routing, around the auth layer, the fallback, and the Swagger UI catch-all) so
//! it records every business access, including requests rejected before the
//! handler (`401`/`403`), unmatched routes (`404`), and method-not-allowed
//! (`405`). It installs the request-scoped collector (see [`crate::access_log`])
//! and emits exactly one set of access lines after the response is available,
//! using the real HTTP status. Handlers record `params` (and an optional success
//! `detail`) through the [`AccessLog`] handle; for failures that never reach a
//! handler the middleware captures the error body as the failure `detail`.
//! Infrastructure and discovery endpoints (`GET /health`, `GET /openapi.json`, and
//! the Swagger UI) are never logged.

pub use crate::access_log::AccessLog;
use crate::access_log::{emit_rest, resolve_header_extras, scoped, AccessLogState};
use crate::state::AppState;
use axum::body::{to_bytes, Body};
use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;

/// Upper bound on the error body buffered to populate the failure `detail`. Error
/// bodies are the small `ErrorString` payload (max 2048 chars).
const MAX_ERROR_BODY: usize = 8 * 1024;

/// Matched route templates that are never logged, keyed by the `MatchedPath`:
/// the health probe, the OpenAPI document, and the Swagger UI index served at the
/// root. Swagger static assets are served by the root catch-all (see
/// [`is_catch_all`]) and are skipped separately.
const SKIP_OPERATION_PATHS: [&str; 3] = ["/health", "/openapi.json", "/"];

/// The Swagger UI is mounted at the root and installs a wildcard catch-all
/// (`/{*rest}`) that matches every otherwise-unmatched path. A successful response
/// under it is a served UI asset (infra, not logged); an error response is an
/// unknown-route access attempt worth auditing.
fn is_catch_all(path: &str) -> bool {
    path.contains("{*")
}

/// Installs the per-request collector, runs the inner stack, then emits one set of
/// access lines with the real HTTP status. Matched routes log the templated
/// operation (`METHOD /template`); unmatched requests (and the Swagger catch-all)
/// log the raw `METHOD /path`.
pub async fn access_log_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let config = state
        .config
        .access_log
        .config(state.config.tracing.structured);
    request.extensions_mut().insert(AccessLog);

    let matched = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_string());

    let skipped = matched
        .as_deref()
        .is_some_and(|path| SKIP_OPERATION_PATHS.contains(&path));
    if !config.enabled || skipped {
        return next.run(request).await;
    }

    let catch_all = matched.as_deref().is_some_and(is_catch_all);
    let operation = match &matched {
        Some(path) if !catch_all => format!("{} {}", request.method(), path),
        _ => format!("{} {}", request.method(), request.uri().path()),
    };
    let header_extras = resolve_header_extras(&config, |header| {
        request
            .headers()
            .get(header)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    });

    let state = AccessLogState::new(operation, header_extras, config);
    scoped(state, async move {
        let response = next.run(request).await;
        let status = response.status().as_u16();
        if catch_all && status < 400 {
            return response;
        }
        let response = capture_error_detail(response).await;
        emit_rest(status);
        response
    })
    .await
}

/// For error responses that carry no handler-recorded failure, buffers the
/// (bounded) response body and records it as the whole-call failure `detail` so
/// pre-handler rejections still carry a reason. The body is rebuilt unchanged.
async fn capture_error_detail(response: Response) -> Response {
    if !(response.status().is_client_error() || response.status().is_server_error()) {
        return response;
    }

    let (parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_ERROR_BODY).await else {
        return Response::from_parts(parts, Body::empty());
    };

    let message = std::str::from_utf8(&bytes)
        .unwrap_or_default()
        .trim()
        .trim_matches('"');
    if !message.is_empty() {
        AccessLog.record_failure(message);
    }

    Response::from_parts(parts, Body::from(bytes))
}
