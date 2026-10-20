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

use super::TokenRefresher;
use crate::service_identity::{self, AccessTokenFileToken};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::SystemTime;
use tokio::time::Duration;
use tonic::service::Interceptor;
use tonic::{Request, Status};

pub use service_identity::AccessTokenFileArgs;

struct CachedToken {
    token: Option<AccessTokenFileToken>,
    last_modified: Option<SystemTime>,
}

#[derive(Clone)]
pub struct AccessTokenFileProvider {
    file_path: PathBuf,
    cache: Arc<Mutex<CachedToken>>,
    poll_interval: Duration,
}

impl AccessTokenFileProvider {
    pub fn new(file_path: PathBuf, poll_interval: Duration) -> Self {
        Self {
            file_path,
            cache: Arc::new(Mutex::new(CachedToken {
                token: None,
                last_modified: None,
            })),
            poll_interval,
        }
    }

    pub async fn load(&self) {
        if self.read_and_cache_token().await.is_some() {
            tracing::info!(
                "Access token file loaded successfully from {:?}",
                self.file_path
            );
        }
    }

    pub fn get_token(&self) -> Option<String> {
        if let Ok(guard) = self.cache.lock() {
            if let Some(token) = &guard.token {
                if !token.is_expiring_soon() {
                    return Some(token.access_token.clone());
                }
            }
        }
        None
    }

    pub fn invalidate(&self) {
        if let Ok(mut guard) = self.cache.lock() {
            guard.token = None;
            guard.last_modified = None;
        }
    }

    async fn file_modified_time(&self) -> Option<SystemTime> {
        tokio::fs::metadata(&self.file_path)
            .await
            .and_then(|m| m.modified())
            .ok()
    }

    async fn read_and_cache_token(&self) -> Option<String> {
        let mtime = self.file_modified_time().await;
        match tokio::fs::read_to_string(&self.file_path).await {
            Ok(contents) => self.store_token(contents, mtime),
            Err(e) => {
                tracing::error!(
                    "Failed to read access token file {:?}: {}",
                    self.file_path,
                    e
                );
                None
            }
        }
    }

    fn store_token(&self, contents: String, mtime: Option<SystemTime>) -> Option<String> {
        let token = match service_identity::parse_access_token_file(&contents) {
            Ok(token) => token,
            Err(e) => {
                tracing::error!("Invalid access token file {:?}: {}", self.file_path, e);
                return None;
            }
        };
        let access_token = token.access_token.clone();
        if let Ok(mut guard) = self.cache.lock() {
            guard.token = Some(token);
            guard.last_modified = mtime;
        }
        Some(access_token)
    }

    pub async fn start_watch_loop(&self) {
        let mut interval = tokio::time::interval(self.poll_interval);
        loop {
            interval.tick().await;

            let current_mtime = self.file_modified_time().await;
            let stored_mtime = self.cache.lock().ok().and_then(|g| g.last_modified);

            let changed = match (current_mtime, stored_mtime) {
                (Some(current), Some(stored)) => current != stored,
                (Some(_), None) => true,
                _ => false,
            };

            if changed {
                tracing::info!(
                    "Access token file {:?} changed on disk, updating cached token",
                    self.file_path
                );
                self.read_and_cache_token().await;
            }
        }
    }
}

#[derive(Clone)]
pub struct AccessTokenFileInterceptor {
    provider: Arc<AccessTokenFileProvider>,
}

impl AccessTokenFileInterceptor {
    pub fn new(provider: Arc<AccessTokenFileProvider>) -> Self {
        Self { provider }
    }
}

impl Interceptor for AccessTokenFileInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        match self.provider.get_token() {
            Some(token) => {
                let auth_value = format!("Bearer {}", token);
                let header_value = auth_value.parse().map_err(|e| {
                    tracing::error!("Access token contains invalid header characters: {}", e);
                    Status::internal("Access token is not a valid HTTP header value")
                })?;
                request.metadata_mut().insert("authorization", header_value);
                Ok(request)
            }
            None => {
                tracing::error!("No valid token available from access token file");
                Err(Status::internal("No valid authentication token available"))
            }
        }
    }
}

#[tonic::async_trait]
impl TokenRefresher for AccessTokenFileProvider {
    async fn refresh(&self) {
        tracing::info!("Re-reading access token from file after authentication failure");
        self.invalidate();
        self.read_and_cache_token().await;
    }
}
