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

use std::{
    future::Future,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};

use azure_core::{
    auth::{AccessToken, TokenCredential},
    HttpClient,
};
use azure_identity::federated_credentials_flow;
use futures::future::BoxFuture;
use snafu::{IntoError, Location, Snafu};
use time::OffsetDateTime;
use tokio::{io::AsyncReadExt, sync::RwLock};
use tonic::async_trait;
use tracing::debug;
use url::Url;

use crate::common::{credentials::access_token_file::AccessTokenFileCredentialsProvider, secret::Secret};

const ACCESS_TOKEN_CACHE_TOKEN_EXPIRY_BUFFER: Duration = Duration::from_secs(60);

const DEFAULT_AZURE_AUTHORITY_HOST: &str = "https://login.microsoftonline.com";

/// Token fetch errors.
#[derive(Debug, Snafu)]
#[snafu(module, context(suffix(false)), visibility(pub(crate)))]
pub enum GetTokenError {
    #[snafu(display("failed to open the token file"))]
    TokenFileOpen {
        source: std::io::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to read the token file"))]
    TokenFileRead {
        source: std::io::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to authenticate with federated credentials"))]
    FederatedCredentialsAuth {
        source: azure_core::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

#[derive(Debug, Clone)]
pub enum StorageCredentials {
    StorageKey(Secret<String>),
    TokenCredential(Arc<dyn TokenCredential>),
}

#[derive(Debug, Snafu)]
pub enum IntoCredentialsError {
    #[snafu(display("invalid authority host"))]
    InvalidAuthorityHost {
        #[snafu(source)]
        source: url::ParseError,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

#[derive(Debug, Clone)]
pub struct SharedTokenCredential(pub Arc<dyn TokenCredential>);

impl TokenCredential for SharedTokenCredential {
    fn get_token<'a, 'b, 'c, 'r>(&'a self, scopes: &'b [&'c str]) -> BoxFuture<'r, azure_core::Result<AccessToken>>
    where
        'a: 'r,
        'b: 'r,
        'c: 'r,
        Self: 'r,
    {
        self.0.get_token(scopes)
    }

    fn clear_cache<'a, 'r>(&'a self) -> BoxFuture<'r, azure_core::Result<()>>
    where
        'a: 'r,
        Self: 'r,
    {
        self.0.clear_cache()
    }
}

/// A thin wrapper over the azure_identity::ClientSecretCredential type.
#[derive(Debug)]
pub struct ClientSecretCredential {
    inner: azure_identity::ClientSecretCredential,
}

impl ClientSecretCredential {
    /// Create a ClientSecretCredential with the specified parameters.
    pub fn new(
        http_client: Arc<dyn HttpClient>,
        authority_host: Option<Url>,
        tenant_id: String,
        client_id: String,
        client_secret: String,
    ) -> Self {
        Self {
            inner: azure_identity::ClientSecretCredential::new(
                http_client,
                authority_host.unwrap_or_else(|| get_default_authority_host().clone()),
                tenant_id,
                client_id,
                client_secret,
            ),
        }
    }
}

impl TokenCredential for ClientSecretCredential {
    fn get_token<'a, 'b, 'c, 'r>(&'a self, scopes: &'b [&'c str]) -> BoxFuture<'r, azure_core::Result<AccessToken>>
    where
        'a: 'r,
        'b: 'r,
        'c: 'r,
        Self: 'r,
    {
        self.inner.get_token(scopes)
    }

    fn clear_cache<'a, 'r>(&'a self) -> BoxFuture<'r, azure_core::Result<()>>
    where
        'a: 'r,
        Self: 'r,
    {
        self.inner.clear_cache()
    }
}

/// Allows authenticating with a token issued by an external identity provider.
pub struct FederatedCredentialsFlowCredential {
    cache: AccessTokenCache,
    http_client: Arc<dyn HttpClient>,
    file_path: PathBuf,
    client_id: String,
    tenant_id: String,
    authority_host: Url,
}

impl FederatedCredentialsFlowCredential {
    pub fn new(
        http_client: Arc<dyn HttpClient>,
        file_path: PathBuf,
        client_id: String,
        tenant_id: String,
        authority_host: Option<Url>,
    ) -> Self {
        Self {
            cache: Default::default(),
            authority_host: authority_host.unwrap_or_else(|| get_default_authority_host().clone()),
            file_path,
            http_client,
            client_id,
            tenant_id,
        }
    }

    async fn try_fetch_token(&self, scopes: &[&str]) -> Result<AccessToken, GetTokenError> {
        let token = self.try_load_token_file().await?;

        debug!(?scopes, "obtaining service credentials from federated credentials");

        let response = federated_credentials_flow::perform(
            self.http_client.clone(),
            &self.client_id,
            &token,
            scopes,
            &self.tenant_id,
            &self.authority_host,
        )
        .await
        .map_err(|source| get_token_error::FederatedCredentialsAuth.into_error(source))?;

        debug!(?scopes, expires_in = ?response.expires_in, expires_on = ?response.expires_on, "credentials received");

        let access_token = AccessToken::new(
            response.access_token,
            response
                .expires_on
                .unwrap_or_else(|| OffsetDateTime::now_utc() + Duration::from_secs(response.expires_in)),
        );

        Ok(access_token)
    }

    async fn try_load_token_file(&self) -> Result<String, GetTokenError> {
        use tokio::fs::File;

        // todo: maybe we could cache the token instead of reading it every time?

        debug!(path = ?self.file_path, "reading the projected token");

        let mut file = File::open(&self.file_path)
            .await
            .map_err(|source| get_token_error::TokenFileOpen.into_error(source))?;

        let mut buffer = String::new();
        file.read_to_string(&mut buffer)
            .await
            .map_err(|source| get_token_error::TokenFileRead.into_error(source))?;

        Ok(buffer)
    }
}

impl std::fmt::Debug for FederatedCredentialsFlowCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkloadIdentityCredential")
            .field("file_path", &self.file_path)
            .field("client_id", &self.client_id)
            .field("tenant_id", &self.tenant_id)
            .field("authority_host", &self.authority_host)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl TokenCredential for FederatedCredentialsFlowCredential {
    async fn get_token(&self, scopes: &[&str]) -> azure_core::Result<AccessToken> {
        use azure_core::{error::ErrorKind, Error};

        self.cache
            .get_or_try_fetch(scopes, async { self.try_fetch_token(scopes).await })
            .await
            .map_err(|source| Error::new(ErrorKind::Other, source))
    }

    async fn clear_cache(&self) -> azure_core::Result<()> {
        self.cache.clear().await;
        Ok(())
    }
}

#[derive(Debug)]
pub struct AccessTokenFileCredential {
    inner: AccessTokenFileCredentialsProvider,
}

impl AccessTokenFileCredential {
    pub fn new(file_path: impl Into<PathBuf>) -> Self {
        Self {
            inner: AccessTokenFileCredentialsProvider::new(file_path),
        }
    }
}

#[async_trait]
impl TokenCredential for AccessTokenFileCredential {
    async fn get_token(&self, _scopes: &[&str]) -> azure_core::Result<AccessToken> {
        use azure_core::{error::ErrorKind, Error};

        self.inner
            .try_get_or_parse_token()
            .await
            .map_err(|source| Error::new(ErrorKind::Other, source))
            .map(|token| AccessToken::new(token.access_token.inner().clone(), token.expires_on))
    }

    async fn clear_cache(&self) -> azure_core::Result<()> {
        self.inner.invalidate_cache().await;
        Ok(())
    }
}

/// Azure access token cache.
#[derive(Default)]
struct AccessTokenCache {
    inner: RwLock<hashbrown::HashMap<TokenCacheKey, AccessToken>>,
}

impl AccessTokenCache {
    async fn get_or_try_fetch<F, E>(&self, scopes: &[&str], try_fetch: F) -> Result<AccessToken, E>
    where
        F: Future<Output = Result<AccessToken, E>>,
    {
        debug!(?scopes, "fetching a token from cache");

        let key = TokenCacheKeyRef(scopes);

        let read = self.inner.read().await;
        if let Some(token) = read.get(&key) {
            if !is_token_expired(token) {
                debug!(expires_on = ?token.expires_on, "using a cached token");
                return Ok(token.clone());
            }

            debug!(expires_on = ?token.expires_on, "found an expired cached token");
        }

        drop(read);

        debug!("token not found in the cache, fetching");

        let mut write = self.inner.write().await;
        if let Some(token) = write.get(&key) {
            if !is_token_expired(token) {
                debug!(expires_on = %token.expires_on, "using a cached token");
                return Ok(token.clone());
            }
        }

        let token = try_fetch.await?;

        debug!(expires_on = ?token.expires_on, "token obtained");

        write.insert(
            TokenCacheKey(scopes.iter().map(|scope| (*scope).into()).collect()),
            token.clone(),
        );

        Ok(token)
    }

    async fn clear(&self) {
        self.inner.write().await.clear();
    }
}

impl std::fmt::Debug for AccessTokenCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessTokenCache").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct TokenCacheKey(Vec<String>);

impl std::hash::Hash for TokenCacheKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for key in self.0.iter() {
            key.as_str().hash(state);
        }
    }
}

struct TokenCacheKeyRef<'a>(&'a [&'a str]);

impl std::hash::Hash for TokenCacheKeyRef<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for key in self.0 {
            key.hash(state);
        }
    }
}

impl hashbrown::Equivalent<TokenCacheKey> for TokenCacheKeyRef<'_> {
    fn equivalent(&self, key: &TokenCacheKey) -> bool {
        self.0.len() == key.0.len() && self.0.iter().zip(key.0.iter()).all(|(a, b)| a == b)
    }
}

fn get_default_authority_host() -> &'static Url {
    static DEFAULT_AUTHORITY_HOST: OnceLock<Url> = OnceLock::new();

    DEFAULT_AUTHORITY_HOST.get_or_init(|| Url::parse(DEFAULT_AZURE_AUTHORITY_HOST).unwrap())
}

fn is_token_expired(token: &AccessToken) -> bool {
    token.expires_on < OffsetDateTime::now_utc() + ACCESS_TOKEN_CACHE_TOKEN_EXPIRY_BUFFER
}
