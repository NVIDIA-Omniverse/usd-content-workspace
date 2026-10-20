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

use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use permission_auth::AuthError;
use permission_core::Principal;
use std::sync::Arc;

const PUBLIC_ENDPOINTS: [(&str, &str); 2] = [("GET", "/v1beta/idp/keys/"), ("GET", "/health")];

#[tracing::instrument(level = "debug", skip(state, request, next))]
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, (StatusCode, String)> {
    let method = request.method().as_str();
    let path = request.uri().path();

    if !state.config.auth.enabled || PUBLIC_ENDPOINTS.contains(&(method, path)) {
        request.extensions_mut().insert(Arc::new(Principal {
            id: String::new(),
            info: None,
        }));
        return Ok(next.run(request).await);
    }

    let auth_header = match request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
    {
        Some(header) => header,
        None => {
            tracing::debug!("No Authorization header");
            return Err((
                StatusCode::UNAUTHORIZED,
                "No Authorization header".to_string(),
            ));
        }
    };

    match auth_header.split_once(' ') {
        Some(("Bearer", token)) => {
            match permission_auth::verify_user_token(
                token,
                &state.config.auth,
                &state.client,
                &state.auth_cache,
            )
            .await
            {
                Ok(principal) => {
                    request.extensions_mut().insert(principal);
                    Ok(next.run(request).await)
                }
                Err(err) => match err {
                    AuthError::Forbidden() => {
                        tracing::debug!("Failed to verify user token: {}", err);
                        Err((StatusCode::FORBIDDEN, "Forbidden".to_string()))
                    }
                    AuthError::Unauthorized() => {
                        tracing::debug!("Failed to verify user token: {}", err);
                        Err((StatusCode::UNAUTHORIZED, "Unauthorized".to_string()))
                    }
                    _ => {
                        tracing::error!("Failed to verify user token: {}", err);
                        Err((StatusCode::UNAUTHORIZED, "Unauthorized".to_string()))
                    }
                },
            }
        }
        _ => {
            tracing::warn!("Authorization header is not in Bearer format");
            Err((
                StatusCode::UNAUTHORIZED,
                "Authorization header is not in Bearer format".to_string(),
            ))
        }
    }
}
