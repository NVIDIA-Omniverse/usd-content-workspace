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

//! Shared CLI args and token acquisition for outbound service identity
//! (client credentials and access-token-file flows).

use clap::Args;
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tokio::time::Duration;

// ---------------------------------------------------------------------------
// Client credentials CLI args
// ---------------------------------------------------------------------------

#[derive(Args, Clone, Debug)]
pub struct ClientCredentialsArgs {
    #[arg(
        id = "enable-client-credentials",
        long = "enable-client-credentials",
        env = "ENABLE_CLIENT_CREDENTIALS",
        conflicts_with = "enable-access-token-file",
        requires_ifs = [
            ("true", "client_credentials_openid_configuration"),
            ("true", "client_id"),
            ("true", "client_secret_group"),
            ("true", "client_scope"),
        ]
    )]
    pub enabled: bool,

    /// The OpenID configuration URI used for the client credentials token exchange.
    #[arg(
        id = "client_credentials_openid_configuration",
        long = "client-credentials-openid-configuration",
        env = "CLIENT_CREDENTIALS_OPENID_CONFIGURATION",
        default_value = ""
    )]
    pub openid_configuration: String,

    /// Defines client ID used for client credentials flow to obtain a service token.
    #[arg(long, env = "CLIENT_ID", default_value = "")]
    pub client_id: String,

    /// Defines client secret used for client credentials flow to obtain a service token.
    #[arg(
        id = "client-secret",
        long,
        env = "CLIENT_SECRET",
        group = "client_secret_group"
    )]
    pub client_secret: Option<String>,

    /// Defines the path to a file with a client secret used for client credentials flow to obtain a service token.
    #[arg(
        id = "client-secret-file",
        long,
        env = "CLIENT_SECRET_FILE",
        group = "client_secret_group"
    )]
    pub client_secret_file: Option<PathBuf>,

    /// Specifies the value for `scope` field passed to the identity provider with the client credentials flow.
    #[arg(long, env = "CLIENT_SCOPE", default_value = "")]
    pub client_scope: String,

    /// JSON object of additional HTTP headers to include in the token exchange request.
    #[arg(long, env = "CLIENT_CREDENTIALS_EXTRA_HEADERS", default_value = "{}")]
    pub extra_headers: String,

    /// JSON object of additional body parameters to include in the token exchange request.
    #[arg(
        long,
        env = "CLIENT_CREDENTIALS_EXTRA_BODY_PARAMS",
        default_value = "{}"
    )]
    pub extra_body_params: String,
}

impl ClientCredentialsArgs {
    pub fn get_client_secret(&self) -> Result<String, Box<dyn std::error::Error>> {
        if let Some(client_secret) = &self.client_secret {
            Ok(client_secret.clone())
        } else if let Some(path) = &self.client_secret_file {
            let client_secret = fs::read_to_string(path)?.trim().to_string();
            Ok(client_secret)
        } else {
            Ok(String::new())
        }
    }

    pub fn get_extra_headers(&self) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
        serde_json::from_str(&self.extra_headers).map_err(|e| {
            format!(
                "Invalid JSON in CLIENT_CREDENTIALS_EXTRA_HEADERS environment variable: {e}. \
                 Expected a JSON object with string keys and values, \
                 e.g. '{{\"X-Custom-Header\": \"value\"}}'. Received: '{}'",
                self.extra_headers
            )
            .into()
        })
    }

    pub fn get_extra_body_params(
        &self,
    ) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
        serde_json::from_str(&self.extra_body_params).map_err(|e| {
            format!(
                "Invalid JSON in CLIENT_CREDENTIALS_EXTRA_BODY_PARAMS environment variable: {e}. \
                 Expected a JSON object with string keys and values, \
                 e.g. '{{\"audience\": \"https://api.example.com\"}}'. Received: '{}'",
                self.extra_body_params
            )
            .into()
        })
    }
}

// ---------------------------------------------------------------------------
// Access token file CLI args
// ---------------------------------------------------------------------------

#[derive(Args, Clone, Debug)]
pub struct AccessTokenFileArgs {
    #[arg(
        id = "enable-access-token-file",
        long = "enable-access-token-file",
        env = "ENABLE_ACCESS_TOKEN_FILE",
        conflicts_with = "enable-client-credentials",
        requires_ifs = [
            ("true", "access_token_file_path"),
        ]
    )]
    pub enabled: bool,

    #[arg(
        id = "access_token_file_path",
        long = "access-token-file-path",
        env = "ACCESS_TOKEN_FILE_PATH",
        default_value = ""
    )]
    pub file_path: String,

    #[arg(
        id = "access-token-file-poll-interval",
        long = "access-token-file-poll-interval",
        env = "ACCESS_TOKEN_FILE_POLL_INTERVAL",
        default_value = "15s",
        value_parser = humantime::parse_duration,
    )]
    pub poll_interval: Duration,
}

// ---------------------------------------------------------------------------
// Token response and error types
// ---------------------------------------------------------------------------

pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: Option<u64>,
}

const EXPIRY_SKEW: time::Duration = time::Duration::seconds(120);

#[derive(Clone, Debug)]
pub struct AccessTokenFileToken {
    pub access_token: String,
    pub expires_on: OffsetDateTime,
}

impl AccessTokenFileToken {
    pub fn is_expired(&self) -> bool {
        OffsetDateTime::now_utc() >= self.expires_on
    }

    pub fn is_expiring_soon(&self) -> bool {
        let ttl = self.expires_on - OffsetDateTime::now_utc();
        ttl <= EXPIRY_SKEW
    }
}

#[derive(Deserialize)]
struct AccessTokenFileTokenJson {
    access_token: String,
    expires_on: String,
}

#[derive(Debug)]
pub enum ServiceIdentityError {
    SecretResolution(String),
    TokenExchange(String),
    FileRead(String),
    InvalidConfig(String),
}

impl std::error::Error for ServiceIdentityError {}

impl fmt::Display for ServiceIdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SecretResolution(msg) => write!(f, "secret resolution error: {}", msg),
            Self::TokenExchange(msg) => write!(f, "token exchange error: {}", msg),
            Self::FileRead(msg) => write!(f, "file read error: {}", msg),
            Self::InvalidConfig(msg) => write!(f, "invalid config: {}", msg),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared functions
// ---------------------------------------------------------------------------

/// Performs an OAuth2 client_credentials token exchange against the given endpoint.
///
/// `token_endpoint` is passed separately because discovery via
/// `load_openid_configuration` is a distinct step.
pub async fn exchange_client_credentials_token(
    client: &reqwest::Client,
    token_endpoint: &str,
    client_id: &str,
    client_secret: &str,
    client_scope: &str,
    extra_headers: &HashMap<String, String>,
    extra_body_params: &HashMap<String, String>,
) -> Result<TokenResponse, ServiceIdentityError> {
    let mut params: Vec<(&str, &str)> = vec![
        ("grant_type", "client_credentials"),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("scope", client_scope),
    ];

    for (key, value) in extra_body_params {
        params.push((key.as_str(), value.as_str()));
    }

    let mut request = client.post(token_endpoint);
    for (key, value) in extra_headers {
        request = request.header(key, value);
    }

    let response = request
        .form(&params)
        .send()
        .await
        .map_err(|e| ServiceIdentityError::TokenExchange(e.to_string()))?;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(ServiceIdentityError::TokenExchange(format!(
            "Token exchange failed: {}",
            error_text
        )));
    }

    let token_response: serde_json::Value = response
        .json()
        .await
        .map_err(|e| ServiceIdentityError::TokenExchange(e.to_string()))?;

    let access_token = token_response["access_token"]
        .as_str()
        .ok_or_else(|| {
            ServiceIdentityError::TokenExchange("No access_token in response".to_string())
        })?
        .to_string();

    let expires_in = token_response["expires_in"].as_u64();

    Ok(TokenResponse {
        access_token,
        expires_in,
    })
}

/// Parses access-token-file JSON containing `access_token` and RFC3339 `expires_on`.
pub fn parse_access_token_file(
    contents: &str,
) -> Result<AccessTokenFileToken, ServiceIdentityError> {
    let parsed: AccessTokenFileTokenJson = serde_json::from_str(contents).map_err(|e| {
        ServiceIdentityError::FileRead(format!(
            "Invalid access token file JSON: {e}. Expected a JSON object with access_token and expires_on fields"
        ))
    })?;

    let access_token = parsed.access_token.trim().to_string();
    if access_token.is_empty() {
        return Err(ServiceIdentityError::FileRead(
            "Access token file access_token is empty".to_string(),
        ));
    }

    let expires_on = OffsetDateTime::parse(parsed.expires_on.trim(), &Rfc3339).map_err(|e| {
        ServiceIdentityError::FileRead(format!(
            "Invalid access token file expires_on value {:?}: {e}. Expected RFC3339 format",
            parsed.expires_on
        ))
    })?;

    let token = AccessTokenFileToken {
        access_token,
        expires_on,
    };
    if token.is_expired() {
        return Err(ServiceIdentityError::FileRead(format!(
            "Access token file token expired at {:?}",
            parsed.expires_on
        )));
    }

    Ok(token)
}

/// Reads bearer token JSON from a file.
pub fn read_token_file(path: &Path) -> Result<String, ServiceIdentityError> {
    let contents = fs::read_to_string(path).map_err(|e| {
        ServiceIdentityError::FileRead(format!("Failed to read token file {:?}: {}", path, e))
    })?;
    parse_access_token_file(&contents).map(|token| token.access_token)
}

#[cfg(test)]
mod tests {
    use super::{parse_access_token_file, AccessTokenFileToken};
    use time::OffsetDateTime;

    #[test]
    fn parses_access_token_file_json() {
        let token = parse_access_token_file(
            r#"{"access_token":"my-test-token","expires_on":"2999-01-01T00:00:00Z"}"#,
        )
        .unwrap();

        assert_eq!(token.access_token, "my-test-token");
        assert!(!token.is_expired());
    }

    #[test]
    fn ignores_unknown_access_token_file_json_fields() {
        let token = parse_access_token_file(
            r#"{"access_token":"my-test-token","expires_on":"2999-01-01T00:00:00Z","token_type":"Bearer"}"#,
        )
        .unwrap();

        assert_eq!(token.access_token, "my-test-token");
    }

    #[test]
    fn rejects_access_token_file_json_without_rfc3339_expiry() {
        let err = parse_access_token_file(
            r#"{"access_token":"my-test-token","expires_on":"2999-01-01 00:00:00"}"#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("Expected RFC3339 format"));
    }

    #[test]
    fn rejects_expired_access_token_file_json() {
        let err = parse_access_token_file(
            r#"{"access_token":"my-test-token","expires_on":"2000-01-01T00:00:00Z"}"#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("expired"));
    }

    #[test]
    fn is_expiring_soon_true_for_already_expired_token() {
        let token = AccessTokenFileToken {
            access_token: "expired".to_string(),
            expires_on: OffsetDateTime::now_utc() - time::Duration::seconds(3600),
        };
        assert!(token.is_expired());
        assert!(token.is_expiring_soon());
    }

    #[test]
    fn is_expiring_soon_true_within_skew_window() {
        let token = AccessTokenFileToken {
            access_token: "soon".to_string(),
            expires_on: OffsetDateTime::now_utc() + time::Duration::seconds(60),
        };
        assert!(!token.is_expired());
        assert!(token.is_expiring_soon());
    }

    #[test]
    fn is_expiring_soon_false_outside_skew_window() {
        let token = AccessTokenFileToken {
            access_token: "fresh".to_string(),
            expires_on: OffsetDateTime::now_utc() + time::Duration::seconds(600),
        };
        assert!(!token.is_expired());
        assert!(!token.is_expiring_soon());
    }
}
