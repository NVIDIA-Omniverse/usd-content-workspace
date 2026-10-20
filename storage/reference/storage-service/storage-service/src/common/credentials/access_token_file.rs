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

use std::{fmt::Debug, path::PathBuf, sync::Arc, time::Duration};

use serde::{de::DeserializeOwned, Deserialize};
use snafu::{IntoError, Location, Snafu};
use time::OffsetDateTime;
use tokio::{io::AsyncReadExt, sync::RwLock};
use tracing::debug;

use crate::common::{
    credentials::{self, CredentialsProvider},
    secret::Secret,
};

const ACCESS_TOKEN_CACHE_TOKEN_EXPIRY_BUFFER: Duration = Duration::from_secs(60);

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), context(suffix(false)), module)]
pub enum Error {
    #[snafu(display("failed to open the token file"))]
    FileOpen {
        source: std::io::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to read the token file"))]
    FileRead {
        source: std::io::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to parse the token file"))]
    Parse {
        source: serde_json::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

pub trait Expirable {
    fn expires_on(&self) -> OffsetDateTime;
}

#[derive(Debug, Deserialize, Clone)]
pub struct AccessToken {
    #[serde(with = "time::serde::rfc3339")]
    pub expires_on: OffsetDateTime,
    pub access_token: Secret<String>,
}

impl Expirable for AccessToken {
    fn expires_on(&self) -> OffsetDateTime {
        self.expires_on
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct AccessTokenFileCredentialsProvider<T = AccessToken> {
    file_path: PathBuf,
    cached_token: RwLock<Option<Arc<T>>>,
}

impl<T: Debug + DeserializeOwned + Expirable> AccessTokenFileCredentialsProvider<T> {
    pub fn new(file_path: impl Into<PathBuf>) -> Self {
        Self {
            file_path: file_path.into(),
            cached_token: Default::default(),
        }
    }

    pub fn from_config(config: Config) -> Self {
        Self::new(config.path)
    }

    pub async fn try_get_or_parse_token(&self) -> Result<Arc<T>, Error> {
        let read = self.cached_token.read().await;
        if let Some(token) = read.as_ref() {
            if !is_token_expired(token.as_ref()) {
                debug!(?token, "using a cached token");

                return Ok(token.clone());
            }

            debug!(?token, "found an expired cached token");
        }

        drop(read);

        debug!("cached token not found, fetching");

        let mut write = self.cached_token.write().await;
        if let Some(token) = write.as_ref() {
            if !is_token_expired(token.as_ref()) {
                debug!(?token, "using a cached token");

                return Ok(token.clone());
            }
        }

        let token = Arc::new(self.try_parse_token_file().await?);

        *write = Some(token.clone());

        Ok(token)
    }

    pub async fn invalidate_cache(&self) {
        *self.cached_token.write().await = None;
    }

    async fn try_parse_token_file(&self) -> Result<T, Error> {
        use tokio::fs::File;

        debug!(path = ?self.file_path, "parsing access token file");

        let mut file = File::open(&self.file_path)
            .await
            .map_err(|source| error::FileOpen.into_error(source))?;

        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)
            .await
            .map_err(|source| error::FileRead.into_error(source))?;

        let token: T = serde_json::from_slice(&buffer).map_err(|source| error::Parse.into_error(source))?;

        debug!(path = ?self.file_path, ?token, "access token file parsed");

        Ok(token)
    }
}

#[async_trait::async_trait]
impl CredentialsProvider for AccessTokenFileCredentialsProvider<AccessToken> {
    async fn get_credentials(&self) -> credentials::Result<credentials::Credentials> {
        let access_token = self.try_get_or_parse_token().await?;
        Ok(credentials::Credentials::Bearer(
            access_token.access_token.inner().clone(),
        ))
    }
}

fn is_token_expired(token: &impl Expirable) -> bool {
    token.expires_on() < OffsetDateTime::now_utc() + ACCESS_TOKEN_CACHE_TOKEN_EXPIRY_BUFFER
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use assert_matches::assert_matches;
    use tempfile::tempdir;
    use time::{format_description::well_known::Rfc3339, OffsetDateTime};
    use tokio::io::AsyncWriteExt;

    use crate::common::credentials::{Credentials, CredentialsProvider};

    use super::{AccessToken, AccessTokenFileCredentialsProvider};

    type Provider = AccessTokenFileCredentialsProvider<AccessToken>;

    #[test_log::test(tokio::test)]
    async fn test_provider_parses_and_caches_token() {
        let temp_dir = tempdir().unwrap();
        let token_file_path = temp_dir.path().join("token");
        let provider = Provider::new(&token_file_path);
        write_access_token_file(
            &token_file_path,
            "a",
            OffsetDateTime::now_utc() + Duration::from_hours(1),
        )
        .await;

        let credentials_1 = provider.get_credentials().await.unwrap();

        assert_matches!(credentials_1, Credentials::Bearer(token) => token == "a");

        temp_dir.close().unwrap();

        let credentials_2 = provider.get_credentials().await.unwrap();

        assert_matches!(credentials_2, Credentials::Bearer(token) => token == "a");
    }

    #[test_log::test(tokio::test)]
    async fn test_provider_renews_expired_token() {
        let temp_dir = tempdir().unwrap();
        let token_file_path = temp_dir.path().join("token");
        let provider = Provider::new(&token_file_path);
        write_access_token_file(
            &token_file_path,
            "a",
            OffsetDateTime::now_utc() + Duration::from_secs(10),
        )
        .await;

        let credentials_1 = provider.get_credentials().await.unwrap();

        assert_matches!(credentials_1, Credentials::Bearer(token) => token == "a");

        write_access_token_file(
            &token_file_path,
            "b",
            OffsetDateTime::now_utc() + Duration::from_hours(1),
        )
        .await;

        let credentials_2 = provider.get_credentials().await.unwrap();

        assert_matches!(credentials_2, Credentials::Bearer(token) => token == "b");
    }

    async fn write_access_token_file(
        path: impl AsRef<Path>,
        access_token: impl Into<String>,
        expires_on: OffsetDateTime,
    ) {
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .await
            .unwrap();
        let encoded_token = serde_json::to_vec(&serde_json::json!({
            "access_token": access_token.into(),
            "expires_on": expires_on.format(&Rfc3339).unwrap(),
        }))
        .unwrap();

        file.write_all(&encoded_token).await.unwrap();
        file.flush().await.unwrap();
    }
}
