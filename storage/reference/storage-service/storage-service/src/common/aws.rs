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

use std::path::PathBuf;

use aws_config::{ConfigLoader, Region};
use aws_credential_types::{
    provider::{error::CredentialsError, future::ProvideCredentials as ProvideCredentialsFuture, ProvideCredentials},
    Credentials,
};
use futures::TryFutureExt;
use serde::Deserialize;
use snafu::{IntoError, Location, Snafu};
use time::OffsetDateTime;
use tokio::{fs::File, io::AsyncReadExt};
use tracing::debug;

use crate::common::secret::Secret;

pub trait ConfigLoaderExt {
    /// Optionally, configure the service endpoint url.
    fn maybe_endpoint_url(self, endpoint_url: Option<impl Into<String>>) -> Self;

    /// Optionally, configure the region name.
    fn maybe_region(self, region: Option<impl Into<String>>) -> Self;
}

impl ConfigLoaderExt for ConfigLoader {
    fn maybe_endpoint_url(self, endpoint_url: Option<impl Into<String>>) -> Self {
        if let Some(endpoint_url) = endpoint_url {
            self.endpoint_url(endpoint_url)
        } else {
            self
        }
    }

    fn maybe_region(self, region: Option<impl Into<String>>) -> Self {
        if let Some(region) = region {
            self.region(Region::new(region.into()))
        } else {
            self
        }
    }
}

#[derive(Debug)]
pub struct FileCredentials {
    file_path: PathBuf,
}

impl FileCredentials {
    pub fn new(file_path: impl Into<PathBuf>) -> Self {
        Self {
            file_path: file_path.into(),
        }
    }
}

impl FileCredentials {
    async fn read_credentials(&self) -> Result<Credentials, FileCredentialsError> {
        debug!(path = ?self.file_path, "parsing credentials file");

        let mut file = File::open(&self.file_path)
            .await
            .map_err(|source| file_credentials_error::FileOpen.into_error(source))?;

        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)
            .await
            .map_err(|source| file_credentials_error::FileRead.into_error(source))?;

        let parsed: EncodedCredentials =
            serde_json::from_slice(&buffer).map_err(|source| file_credentials_error::Parse.into_error(source))?;

        debug!(path = ?self.file_path, ?parsed, "credentials file parsed");

        Ok(Credentials::new(
            parsed.access_key_id,
            parsed.secret_access_key.into_inner(),
            parsed.session_token.map(Secret::into_inner),
            parsed.expires_on.map(Into::into),
            "file",
        ))
    }
}

impl ProvideCredentials for FileCredentials {
    fn provide_credentials<'a>(&'a self) -> ProvideCredentialsFuture<'a>
    where
        Self: 'a,
    {
        ProvideCredentialsFuture::new(self.read_credentials().map_err(CredentialsError::unhandled))
    }
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), module, context(suffix(false)))]
enum FileCredentialsError {
    #[snafu(display("failed to read open the credentials file"))]
    FileOpen {
        source: std::io::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to read client the credentials file"))]
    FileRead {
        source: std::io::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to parse client credentials"))]
    Parse {
        source: serde_json::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

#[derive(Deserialize, Debug)]
struct EncodedCredentials {
    access_key_id: String,
    secret_access_key: Secret<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    expires_on: Option<OffsetDateTime>,
    #[serde(default)]
    session_token: Option<Secret<String>>,
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use aws_sdk_s3::config::ProvideCredentials;
    use rstest::rstest;
    use tempfile::tempdir;
    use time::{format_description::well_known::Rfc3339, OffsetDateTime};
    use tokio::io::AsyncWriteExt;

    use super::FileCredentials;

    #[rstest]
    #[case(None, None)]
    #[case(Some("st"), Some(OffsetDateTime::now_utc().truncate_to_second() + Duration::from_secs(10)))]
    #[test_log::test(tokio::test)]
    async fn test_file_credentials_loads_access_key_creds_from_file(
        #[case] session_token: Option<&str>,
        #[case] expires_on: Option<OffsetDateTime>,
    ) {
        let temp_dir = tempdir().unwrap();
        let token_file_path = temp_dir.path().join("token");
        let provider = FileCredentials::new(&token_file_path);
        write_access_key_creds_file(&token_file_path, "ak", "sk", session_token, expires_on).await;

        let credentials = provider.provide_credentials().await.unwrap();
        assert_eq!(credentials.access_key_id(), "ak");
        assert_eq!(credentials.secret_access_key(), "sk");
        assert_eq!(credentials.session_token(), session_token);
        assert_eq!(credentials.expiry(), expires_on.map(Into::into));
    }

    async fn write_access_key_creds_file(
        path: &Path,
        access_key_id: &str,
        secret_access_key: &str,
        session_token: Option<&str>,
        expires_on: Option<OffsetDateTime>,
    ) {
        let mut token_file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .await
            .unwrap();

        let mut token = serde_json::json!({
            "access_key_id": access_key_id,
            "secret_access_key": secret_access_key,
        });

        if let Some(session_token) = session_token {
            token
                .as_object_mut()
                .unwrap()
                .insert("session_token".into(), session_token.into());
        }

        if let Some(expires_on) = expires_on {
            token
                .as_object_mut()
                .unwrap()
                .insert("expires_on".into(), expires_on.format(&Rfc3339).unwrap().into());
        }

        let encoded_token = serde_json::to_vec(&token).unwrap();

        token_file.write_all(&encoded_token).await.unwrap();
        // tokio's File buffers internally and does not flush on drop, so without
        // this the reader can observe an empty file.
        token_file.flush().await.unwrap();
    }
}
