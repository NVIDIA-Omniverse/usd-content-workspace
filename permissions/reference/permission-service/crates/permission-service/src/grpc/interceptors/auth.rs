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
use permission_auth::AuthError;
use permission_core::Principal;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tonic::{Request, Status};
use tonic_async_interceptor::AsyncInterceptor;

#[derive(Clone)]
pub struct AuthInterceptor {
    state: AppState,
}

impl AuthInterceptor {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    pub fn authenticate(
        &self,
        mut request: Request<()>,
    ) -> impl Future<Output = Result<Request<()>, Status>> + Send + 'static {
        let state = self.state.clone();

        async move {
            if !state.config.auth.enabled {
                request.extensions_mut().insert(Arc::new(Principal {
                    id: String::new(),
                    info: None,
                }));
                return Ok(request);
            }

            let metadata = request.metadata();
            let auth_header = match metadata.get("authorization") {
                Some(token) => match token.to_str() {
                    Ok(token_str) => token_str,
                    Err(_) => {
                        tracing::debug!("Invalid authorization header format");
                        return Err(Status::unauthenticated(
                            "Invalid authorization header format",
                        ));
                    }
                },
                None => {
                    tracing::debug!("No authorization header");
                    return Err(Status::unauthenticated("No authorization header"));
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
                            Ok(request)
                        }
                        Err(err) => match err {
                            AuthError::Forbidden() => {
                                tracing::debug!("Failed to verify user token: {}", err);
                                Err(Status::permission_denied("Forbidden"))
                            }
                            AuthError::Unauthorized() => {
                                tracing::debug!("Failed to verify user token: {}", err);
                                Err(Status::unauthenticated("Unauthorized"))
                            }
                            _ => {
                                tracing::error!("Failed to verify user token: {}", err);
                                Err(Status::unauthenticated("Unauthorized"))
                            }
                        },
                    }
                }
                _ => {
                    tracing::warn!("Authorization header is not in Bearer format");
                    Err(Status::unauthenticated(
                        "Authorization header is not in Bearer format",
                    ))
                }
            }
        }
    }
}

impl AsyncInterceptor for AuthInterceptor {
    type Future = Pin<Box<dyn Future<Output = Result<Request<()>, Status>> + Send + 'static>>;

    fn call(&mut self, request: Request<()>) -> Self::Future {
        let fut = self.authenticate(request);
        Box::pin(fut)
    }
}
