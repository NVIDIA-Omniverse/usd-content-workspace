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

use crate::state::{ServicesResponse, SharedState};
use crate::v1::{self, V1EndpointMapping};
use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, LOCATION};
use axum::http::{HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Json;
use axum::Router;
use std::sync::Arc;

const SERVICES_CACHE_CONTROL: HeaderValue = HeaderValue::from_static("max-age=30, must-revalidate");

#[derive(Clone)]
pub struct RestState {
    services: SharedState,
    auth_config_json: Option<String>,
    root_redirect: Option<RootRedirect>,
    v1_mappings: Arc<Vec<V1EndpointMapping>>,
}

#[derive(Clone)]
pub struct RootRedirect {
    url: String,
    status: StatusCode,
}

impl RootRedirect {
    pub fn new(url: impl Into<String>, status_code: u16) -> Option<Self> {
        let url = url.into().trim().to_string();
        if url.is_empty() {
            return None;
        }
        let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::FOUND);
        Some(Self { url, status })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }
}

pub fn router(
    services: SharedState,
    auth_config_json: Option<String>,
    root_redirect: Option<RootRedirect>,
    v1_mappings: Arc<Vec<V1EndpointMapping>>,
) -> Router {
    Router::new()
        .route("/", get(handle_root_redirect))
        .route("/api/v2beta/services", get(list_services))
        // Compatibility alias for clients written before the v2beta catalog path.
        .route("/api/v2/services", get(list_services))
        .route("/api/v1/services", get(list_services_v1))
        .route("/api/v1alpha/services", get(list_services_v1))
        .route("/api/v1/auth-config", get(auth_config))
        .route("/api/v1alpha/auth-config", get(auth_config))
        .route("/api/v2/auth-config", get(auth_config))
        .route("/api/v2beta/auth-config", get(auth_config))
        .with_state(RestState {
            services,
            auth_config_json,
            root_redirect,
            v1_mappings,
        })
}

pub async fn handle_root_redirect(State(state): State<RestState>) -> impl IntoResponse {
    let Some(redirect) = state.root_redirect.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };

    (redirect.status, [(LOCATION, redirect.url.clone())]).into_response()
}

pub async fn list_services(State(state): State<RestState>) -> impl IntoResponse {
    let services = state.services.read().await;
    let mut services: Vec<_> = services.values().cloned().collect();
    services.sort_by(|a, b| a.name.cmp(&b.name));
    let response = ServicesResponse {
        schema_version: 2,
        services,
    };
    (
        StatusCode::OK,
        [(CACHE_CONTROL, SERVICES_CACHE_CONTROL)],
        Json(response),
    )
}

pub async fn list_services_v1(State(state): State<RestState>) -> impl IntoResponse {
    if state.v1_mappings.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }

    let services = state.services.read().await;
    let response = v1::build_v1_response(&state.v1_mappings, &services);
    (
        StatusCode::OK,
        [(CACHE_CONTROL, SERVICES_CACHE_CONTROL)],
        Json(response),
    )
        .into_response()
}

pub async fn auth_config(State(state): State<RestState>) -> impl IntoResponse {
    let Some(auth_config_json) = state
        .auth_config_json
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return StatusCode::NOT_FOUND.into_response();
    };

    match serde_json::from_str::<serde_json::Value>(auth_config_json) {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => {
            tracing::error!(%error, "invalid AUTH_CONFIG_JSON");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
