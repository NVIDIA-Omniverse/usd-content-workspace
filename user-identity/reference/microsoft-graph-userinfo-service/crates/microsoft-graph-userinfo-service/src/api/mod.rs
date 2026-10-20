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

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::extract::{MatchedPath, State};
use axum::http::StatusCode;
use axum::routing::get;
use tracing::{Instrument, warn};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::access::AccessLog;
use crate::config::ForwardHeaders;
use crate::models::Page;
use crate::permission::{
    Action, AuthorizationDecision, PermissionChecker, PermissionError, Resource, action_for_route,
};
use crate::provider::IdentityProvider;

pub mod errors;
pub mod groups;
pub mod users;

#[cfg(test)]
mod tests;

/// Single source of truth for the public REST API version, used by the axum
/// route declarations and advertised by the capabilities API.
macro_rules! rest_api_version {
    () => {
        "v1alpha"
    };
}
pub(crate) use rest_api_version;

/// Single source of truth for the public REST API URL prefix (including the
/// `/api/` mount), used by the axum route declarations and advertised by the
/// capabilities API.
macro_rules! rest_api_prefix {
    () => {
        concat!("/api/", rest_api_version!())
    };
}
pub(crate) use rest_api_prefix;

#[derive(Clone)]
pub struct AppState {
    pub provider: Arc<dyn IdentityProvider>,
    pub ready: Arc<AtomicBool>,
    pub permission_checker: Option<Arc<PermissionChecker>>,
    pub access: Arc<AccessLog>,
    pub forward_headers: Arc<ForwardHeaders>,
}

/// Access-record `response.detail` fields a handler or error response attaches
/// to its HTTP response for the access-log middleware to pick up (e.g.
/// `item_count` for list endpoints, `message` for errors).
#[derive(Clone)]
pub(crate) struct AccessResponseDetail(pub(crate) serde_json::Map<String, serde_json::Value>);

pub(crate) fn item_count_detail(count: usize) -> AccessResponseDetail {
    let mut detail = serde_json::Map::new();
    detail.insert("item_count".to_owned(), count.into());
    AccessResponseDetail(detail)
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn readyz(State(state): State<AppState>) -> StatusCode {
    if state.ready.load(Ordering::Acquire) {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn trace_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let parent_cx = crate::telemetry::extract_context(request.headers());
    let method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| request.uri().path().to_owned());

    let span = tracing::info_span!(
        "http.request",
        http.method = %method,
        http.route = %route,
        http.status_code = tracing::field::Empty,
        otel.kind = "server",
        otel.status_code = tracing::field::Empty,
    );
    let _ = span.set_parent(parent_cx);

    let start = std::time::Instant::now();
    let response = next.run(request).instrument(span.clone()).await;
    let status = response.status().as_u16();
    span.record("http.status_code", status);
    if response.status().is_server_error() {
        span.record("otel.status_code", "ERROR");
    }
    crate::metrics::record_http_request(
        method.as_str(),
        &route,
        status,
        start.elapsed().as_secs_f64(),
    );
    response
}

/// Emits an access-log record for every REST call except infrastructure
/// probes. Runs as an outer layer so it observes the final status and the
/// `response.detail` attached by handlers (`item_count`) and error responses
/// (`message`). A matched business route yields a combined record with
/// `request.params`; a request rejected before a handler — an unknown route,
/// an unauthenticated call, or a malformed request — yields a call-level record
/// with no `request`. Health and readiness probes are never logged.
async fn access_log_middleware(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned());

    if !state.access.is_enabled()
        || route
            .as_deref()
            .map(crate::access::is_infra_route)
            .unwrap_or(false)
    {
        return next.run(request).await;
    }

    let is_business = route
        .as_deref()
        .map(crate::access::is_business_route)
        .unwrap_or(false);
    let operation = match &route {
        Some(template) if is_business => format!("{} {template}", request.method()),
        _ => format!("{} {}", request.method(), request.uri().path()),
    };
    let header_fields = state.access.header_fields_http(request.headers());
    let combined_params = route
        .as_deref()
        .filter(|_| is_business)
        .map(|template| crate::access::rest_params(template, request.uri()));

    let response = next.run(request).await;

    let status = response.status().as_u16();
    let detail = response
        .extensions()
        .get::<AccessResponseDetail>()
        .map(|d| d.0.clone())
        .unwrap_or_default();

    // A business route that resolved to a handler carries params (combined
    // record). A pre-handler rejection — unknown route, unauthenticated, or
    // malformed request — is call-level with no `request`.
    let params = if is_business && !matches!(status, 400 | 401 | 405) {
        combined_params
    } else {
        None
    };

    state
        .access
        .emit_rest(header_fields, operation, params, status, detail);
    response
}

async fn permission_middleware(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, errors::ApiError> {
    let checker = match &state.permission_checker {
        Some(c) => c,
        None => return Ok(next.run(request).await),
    };

    let descriptor = request
        .extensions()
        .get::<MatchedPath>()
        .and_then(|p| action_for_route(p.as_str()));

    let descriptor = match descriptor {
        Some(d) => d,
        None => return Ok(next.run(request).await),
    };

    let token = crate::http::extract_bearer(request.headers()).ok_or_else(|| {
        errors::ApiError::Unauthorized("missing or invalid Authorization header".into())
    })?;

    let resource = descriptor.extract_resource(request.uri().path());
    let extra_headers = state.forward_headers.select_http(request.headers());

    match checker
        .check(token, descriptor.action, resource, extra_headers)
        .await
    {
        Ok(AuthorizationDecision::Allow | AuthorizationDecision::Skip) => {
            Ok(next.run(request).await)
        }
        Ok(AuthorizationDecision::Deny) => {
            Err(errors::ApiError::Forbidden("permission denied".into()))
        }
        Err(PermissionError::Unauthorized(msg)) => Err(errors::ApiError::Unauthorized(msg)),
        Err(PermissionError::RateLimited { retry_after }) => {
            Err(errors::ApiError::RateLimited { retry_after })
        }
        Err(e) => {
            warn!(%descriptor.action, error = %e, "permission check failed");
            Err(errors::ApiError::Internal(
                "authorization check failed".into(),
            ))
        }
    }
}

/// Post-filters a page of items by batch-checking per-item read access.
pub(crate) async fn filter_page<T>(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    action: Action,
    mut page: Page<T>,
) -> Result<Page<T>, errors::ApiError>
where
    for<'a> Resource: From<&'a T>,
{
    if let Some(checker) = &state.permission_checker
        && let Some(token) = crate::http::extract_bearer(headers)
    {
        let resources = page.items.iter().map(Resource::from).collect();
        let extra_headers = state.forward_headers.select_http(headers);
        let allowed = checker
            .batch_check(token, action, resources, extra_headers)
            .await
            .map_err(|e| {
                warn!(%action, error = %e, "batch permission check failed");
                errors::ApiError::Internal("authorization check failed".into())
            })?;
        page.items = allowed
            .into_iter()
            .zip(page.items)
            .filter_map(|(ok, item)| ok.then_some(item))
            .collect();
    }
    Ok(page)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(readyz))
        .route(
            concat!(rest_api_prefix!(), "/users/me"),
            get(users::get_current_user),
        )
        .route(
            concat!(rest_api_prefix!(), "/users"),
            get(users::list_users),
        )
        .route(
            concat!(rest_api_prefix!(), "/users/{userId}"),
            get(users::get_user),
        )
        .route(
            concat!(rest_api_prefix!(), "/users/{userId}/groups"),
            get(users::list_user_groups),
        )
        .route(
            concat!(rest_api_prefix!(), "/users/{userId}/groups/{groupId}"),
            get(users::get_user_group),
        )
        .route(
            concat!(rest_api_prefix!(), "/groups"),
            get(groups::list_groups),
        )
        .route(
            concat!(rest_api_prefix!(), "/groups/{groupId}"),
            get(groups::get_group),
        )
        .route(
            concat!(rest_api_prefix!(), "/groups/{groupId}/users"),
            get(groups::list_group_members),
        )
        .route(
            concat!(rest_api_prefix!(), "/groups/{groupId}/users/{userId}"),
            get(groups::get_group_member),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            permission_middleware,
        ))
        .layer(axum::middleware::from_fn(trace_middleware))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            access_log_middleware,
        ))
        .with_state(state)
}

/// Minimal router exposing only the health and readiness probes, used by the
/// writer which performs background sync but does not serve the read APIs.
pub fn health_router(ready: Arc<AtomicBool>) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(readyz_flag))
        .layer(axum::middleware::from_fn(trace_middleware))
        .with_state(ready)
}

async fn readyz_flag(State(ready): State<Arc<AtomicBool>>) -> StatusCode {
    if ready.load(Ordering::Acquire) {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
