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

use super::TokenRefresher;
use crate::service_identity;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::Instant;
use tonic::service::Interceptor;
use tonic::{Request, Status};

pub use service_identity::ClientCredentialsArgs;

#[derive(Clone)]
pub struct ClientCredentialsInterceptor {
    token: Arc<Mutex<Option<Token>>>,
}

impl ClientCredentialsInterceptor {
    pub fn new(token: Arc<Mutex<Option<Token>>>) -> Self {
        Self { token }
    }

    fn get_current_token(&self) -> Option<String> {
        if let Ok(token_guard) = self.token.lock() {
            if let Some(token) = token_guard.as_ref() {
                if !token.is_expired() {
                    return Some(token.access_token.clone());
                }
            }
        }
        None
    }
}

impl Interceptor for ClientCredentialsInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        match self.get_current_token() {
            Some(token) => {
                let auth_value = format!("Bearer {}", token);
                let header_value = auth_value.parse().map_err(|e| {
                    tracing::error!(
                        "Client credentials token contains invalid header characters: {}",
                        e
                    );
                    Status::internal("Client credentials token is not a valid HTTP header value")
                })?;
                request.metadata_mut().insert("authorization", header_value);
                Ok(request)
            }
            None => {
                tracing::error!("No valid token available for client credentials");
                Err(Status::internal("No valid authentication token available"))
            }
        }
    }
}

#[derive(Clone)]
pub struct Token {
    pub access_token: String,
    expires_at: Instant,
}

impl Token {
    pub fn new(access_token: String, expires_in: u64) -> Self {
        Self {
            access_token,
            expires_at: Instant::now() + Duration::from_secs(expires_in),
        }
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    pub fn is_expiring_soon(&self) -> bool {
        Instant::now() >= self.expires_at - Duration::from_secs(120)
    }
}

pub struct TokenManager {
    client: Arc<reqwest::Client>,
    pub client_id: String,
    pub client_secret: String,
    pub client_scope: String,
    pub extra_headers: HashMap<String, String>,
    pub extra_body_params: HashMap<String, String>,
    pub refresh_interval: Duration,
    pub token_received: Arc<Notify>,
    pub token_failure: Arc<Notify>,
    pub token_endpoint: String,
    token: Arc<Mutex<Option<Token>>>,
}

impl TokenManager {
    pub fn new(
        client: Arc<reqwest::Client>,
        client_id: String,
        client_secret: String,
        client_scope: String,
        token_endpoint: String,
        extra_headers: HashMap<String, String>,
        extra_body_params: HashMap<String, String>,
    ) -> Self {
        Self {
            client,
            client_id,
            client_secret,
            client_scope,
            extra_headers,
            extra_body_params,
            token_received: Arc::new(Notify::new()),
            token_failure: Arc::new(Notify::new()),
            token_endpoint,
            refresh_interval: Duration::from_secs(60),
            token: Arc::new(Mutex::new(None)),
        }
    }

    pub fn get_token_handle(&self) -> Arc<Mutex<Option<Token>>> {
        self.token.clone()
    }

    pub async fn exchange_token(&self) -> Result<Token, Box<dyn std::error::Error + Send + Sync>> {
        let response = service_identity::exchange_client_credentials_token(
            &self.client,
            &self.token_endpoint,
            &self.client_id,
            &self.client_secret,
            &self.client_scope,
            &self.extra_headers,
            &self.extra_body_params,
        )
        .await?;

        let expires_in = response.expires_in.unwrap_or(3600);
        Ok(Token::new(response.access_token, expires_in))
    }

    pub async fn start_refresh_loop(&self) {
        let mut interval = tokio::time::interval(self.refresh_interval);
        loop {
            interval.tick().await;

            let needs_refresh = {
                if let Ok(token_guard) = self.token.lock() {
                    match token_guard.as_ref() {
                        None => true,
                        Some(token) => token.is_expired() || token.is_expiring_soon(),
                    }
                } else {
                    true
                }
            };

            if needs_refresh {
                self.refresh_token().await;
            }
        }
    }

    async fn refresh_token(&self) {
        match self.exchange_token().await {
            Ok(new_token) => {
                if let Ok(mut token_guard) = self.token.lock() {
                    *token_guard = Some(new_token);
                    self.token_received.notify_waiters();
                    tracing::debug!("Successfully refreshed client credentials token.");
                }
            }
            Err(err) => {
                tracing::error!("Failed to refresh client credentials token: {}", err);
                self.token_failure.notify_waiters();
            }
        }
    }
}

#[tonic::async_trait]
impl TokenRefresher for TokenManager {
    async fn refresh(&self) {
        tracing::info!("Refreshing client credentials token after authentication failure");
        self.refresh_token().await;
    }
}
