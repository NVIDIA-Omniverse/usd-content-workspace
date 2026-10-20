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

use clap::{ArgAction, Args, ValueEnum};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::Duration;

/// Upper bound (inclusive, in seconds) accepted for `JWT_LEEWAY`.
///
/// JWT leeway widens both `exp` and `nbf` checks symmetrically, so any
/// value materially larger than realistic clock skew between the IdP and
/// this service erodes the time-based validity guarantees of the token.
/// Five minutes covers credible NTP drift while keeping the window of
/// acceptance tight enough that revocation/expiry continues to mean
/// something.
pub const MAX_JWT_LEEWAY: u64 = 300;

#[derive(Args, Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Toggles service authentication.
    /// Enabled by default.
    /// Disable only for testing purposes.
    #[arg(
        id = "enable-auth",
        long = "enable-auth",
        env = "ENABLE_AUTH",
        action = ArgAction::Set,
        default_value = "true",
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = false
    )]
    pub enabled: bool,

    /// Defines how authentication tokens must be validated:
    ///
    /// - "jwt": Validates user tokens by pulling JSON Web Keys from the configured URL and verifying JWT signature.
    ///
    /// - "jwtuserinfo": Same as "jwt" but also includes calling the configured userinfo URL to get detailed user information.
    ///
    /// - "opaque": Calls the configured userinfo URL with the passed token to verify its validity and get detailed user information.
    #[arg(long, default_value = "jwt", env = "AUTH_TYPE")]
    pub authentication_type: AuthType,

    /// The expected value of `aud` claim in JWTs passed in the Authorization header.
    #[arg(
        long,
        env = "JWT_AUDIENCE",
        required_if_eq("enable-auth", "true"),
        required_unless_present("enable-auth")
    )]
    pub jwt_audience: Option<Vec<String>>,

    /// The full set of accepted values for the `iss` claim in JWTs passed in the
    /// Authorization header. When unset or empty, the issuer claim is not
    /// validated. The discovery-document issuer is not added implicitly: operators
    /// must list every issuer they trust (e.g. the IdP's canonical issuer URL,
    /// per-tenant issuers for Microsoft Entra ID multi-tenant deployments, or
    /// both v1 and v2 issuers during a staged migration).
    #[arg(long, env = "JWT_ISSUERS")]
    pub jwt_issuers: Option<Vec<String>>,

    /// The endpoint from the identity provider for discovering metadata such as endpoints and keys
    #[arg(
        long,
        env = "OPENID_CONFIGURATION",
        required_if_eq("enable-auth", "true"),
        required_unless_present("enable-auth")
    )]
    pub openid_configuration: Option<String>,

    /// Time-to-live for public keys retrieved from the identity provider.
    #[arg(long, env = "JWKS_TTL", default_value = "3h", value_parser = humantime::parse_duration)]
    pub jwks_ttl: Duration,

    /// Negative-cache window applied after a failed JWKS fetch.
    ///
    /// While this window is active, every request that would otherwise try
    /// to refresh the JWKS short-circuits with an unauthenticated error
    /// without contacting the identity provider. The goal is to absorb
    /// transient IdP outages and prevent a thundering-herd retry storm
    /// after `jwks_ttl` expires (each in-flight auth request would otherwise
    /// queue on the JWKS write lock and re-issue the same failing call).
    /// A small value is intentional: the service must recover quickly once
    /// the IdP is reachable again.
    #[arg(long, env = "JWKS_FAILURE_BACKOFF", default_value = "10s", value_parser = humantime::parse_duration)]
    pub jwks_failure_backoff: Duration,

    /// Adds leeway for JWT expiration in seconds. Capped at
    /// `MAX_JWT_LEEWAY` (300 s / 5 min) so that operator misconfiguration
    /// cannot effectively disable the `exp` / `nbf` checks by setting an
    /// unreasonably large skew (e.g. a year). The cap is enforced by clap
    /// at argument-parsing time.
    #[arg(
        long,
        env = "JWT_LEEWAY",
        default_value_t = 60,
        value_parser = clap::value_parser!(u64).range(0..=MAX_JWT_LEEWAY),
    )]
    pub jwt_leeway: u64,

    /// The LRU cache size for validated tokens and their principals.
    #[arg(long, env = "TOKEN_CACHE_SIZE", default_value_t = 2048)]
    pub token_cache_size: u64,

    /// Defines how long access tokens must be cached.
    /// Must not be greater than the actual token duration.
    /// Only used for opaque access tokens or if the 'exp' claim is not specified in JWTs.
    #[arg(long, env = "TOKEN_TTL", default_value = "30min", value_parser = humantime::parse_duration
    )]
    pub token_ttl: Duration,

    /// Capacity of the negative cache used to short-circuit recently rejected
    /// bearer tokens. Kept much smaller than `token_cache_size` so that a
    /// flood of bogus bearers cannot evict verified principals from the
    /// positive cache (the eviction policy is per moka instance). The
    /// negative cache's only purpose is to deflect tight retry loops on the
    /// same bad token over a short window.
    #[arg(long, env = "INVALID_TOKEN_CACHE_SIZE", default_value_t = 256)]
    pub invalid_token_cache_size: u64,

    /// How long an entry remains in the negative cache. Kept much shorter
    /// than `token_ttl` so that an opaque token that is rejected by the IdP
    /// once and then re-issued / rotated within minutes is not falsely
    /// considered invalid for the rest of `token_ttl`. Clamped to
    /// `min(invalid_token_cache_ttl, token_ttl)` at startup.
    #[arg(long, env = "INVALID_TOKEN_CACHE_TTL", default_value = "60s", value_parser = humantime::parse_duration)]
    pub invalid_token_cache_ttl: Duration,

    /// The JWT claim used as the principal identifier.
    /// By default, the standard "sub" claim is used.
    #[arg(long, env = "PRINCIPAL_ID_CLAIM", default_value = "sub")]
    pub principal_id_claim: String,

    /// Maximum accepted size, in bytes, of a bearer token presented in the
    /// `Authorization` header. Tokens whose byte length exceeds this cap are
    /// rejected before any caching, header parsing, or outbound forwarding to
    /// the identity provider. The default of 8 KiB comfortably accommodates
    /// realistic JWTs (including those with rich claim sets) while preventing
    /// attackers from poisoning the token cache, amplifying traffic to the
    /// IdP `userinfo` endpoint, or forcing expensive base64 decoding on
    /// arbitrarily large inputs.
    #[arg(long, env = "MAX_TOKEN_LENGTH", default_value_t = 8192)]
    pub max_token_length: usize,

    #[arg(skip)]
    #[serde(skip)]
    pub openid_response: Option<OpenIdConfigResponse>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        AuthConfig {
            enabled: true,
            openid_configuration: None,
            openid_response: None,
            authentication_type: AuthType::JwtWithUserInfo,
            jwt_audience: None,
            jwt_issuers: None,
            jwks_ttl: Duration::new(60 * 180, 0),
            jwks_failure_backoff: Duration::from_secs(10),
            jwt_leeway: 60,
            token_cache_size: 2048,
            token_ttl: Duration::new(60 * 30, 0),
            invalid_token_cache_size: 256,
            invalid_token_cache_ttl: Duration::from_secs(60),
            principal_id_claim: "sub".to_string(),
            max_token_length: 8192,
        }
    }
}

impl AuthConfig {
    /// Loads OpenID configuration from the identity provider with the configured metadata URL.
    /// Intended to be called once before the configuration is used in the application.
    pub async fn load(&mut self, client: &Client) -> Result<(), String> {
        // Cap the negative cache TTL to `token_ttl` regardless of where the
        // service runs (auth-enabled or not). A misconfigured very-long
        // negative TTL would re-introduce the long-window staleness the
        // split cache is designed to eliminate.
        if self.invalid_token_cache_ttl > self.token_ttl {
            tracing::warn!(
                invalid_token_cache_ttl = ?self.invalid_token_cache_ttl,
                token_ttl = ?self.token_ttl,
                "INVALID_TOKEN_CACHE_TTL exceeds TOKEN_TTL; clamping to TOKEN_TTL",
            );
            self.invalid_token_cache_ttl = self.token_ttl;
        }

        if !self.enabled {
            return Ok(());
        }

        let openid_configuration = self
            .openid_configuration
            .clone()
            .ok_or("OpenID configuration is not specified.")?;

        let discovery = load_openid_configuration(&openid_configuration, client)
            .await
            .map_err(|err| format!("Failed to load OpenID configuration: {}", err))?;
        if matches!(
            self.authentication_type,
            AuthType::JwtWithUserInfo | AuthType::Opaque
        ) && discovery.userinfo_endpoint.trim().is_empty()
        {
            return Err(
                "userinfo_endpoint is required for the configured authentication type.".into(),
            );
        }
        self.openid_response = Some(discovery);

        Ok(())
    }
}

/// Loads OpenID configuration from the identity provider with the configured metadata URL.
pub async fn load_openid_configuration(
    openid_configuration: &str,
    client: &Client,
) -> Result<OpenIdConfigResponse, String> {
    let response = client
        .get(openid_configuration)
        .send()
        .await
        .map_err(|err| format!("Failed to get OpenID configuration: {:?}", err))?;

    if response.status().is_success() {
        let json = response
            .json::<OpenIdConfigResponse>()
            .await
            .map_err(|err| {
                format!(
                    "Failed to parse OpenID configuration response from {}. \
                    Expected JSON with 'issuer' and 'jwks_uri' fields. \
                    Error: {}",
                    openid_configuration, err
                )
            })?;
        Ok(json)
    } else {
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| format!("Failed to parse OpenID configuration error: {:?}", err))?;
        Err(format!(
            "OpenID configuration failed with HTTP{}: {}",
            status.as_u16(),
            text
        ))
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize, ValueEnum)]
pub enum AuthType {
    #[serde(rename = "jwt")]
    Jwt,
    #[serde(rename = "jwtuserinfo")]
    JwtWithUserInfo,
    #[serde(rename = "opaque")]
    Opaque,
}

impl FromStr for AuthType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "jwt" => Ok(AuthType::Jwt),
            "jwtuserinfo" => Ok(AuthType::JwtWithUserInfo),
            "opaque" => Ok(AuthType::Opaque),
            _ => Err(format!("Unrecognized authentication type: {}", s)),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize)]
pub struct OpenIdConfigResponse {
    pub issuer: String,

    /// The endpoint from the identity provider for downloading public keys (JSON Web Key Sets).
    pub jwks_uri: String,

    /// The identity provider endpoint used to obtain and verify access tokens.
    /// Optional when the provider only supports validating existing JWTs.
    #[serde(default)]
    pub token_endpoint: String,

    /// The endpoint from the identity provider for getting detailed user information.
    /// Required only for authentication modes that request user information.
    #[serde(default)]
    pub userinfo_endpoint: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn regression_jwt_discovery_accepts_omitted_optional_endpoints() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/discovery"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": server.uri(),
                "jwks_uri": format!("{}/jwks", server.uri())
            })))
            .mount(&server)
            .await;

        let mut config = AuthConfig {
            authentication_type: AuthType::Jwt,
            openid_configuration: Some(format!("{}/discovery", server.uri())),
            ..AuthConfig::default()
        };
        config.load(&Client::new()).await.unwrap();
        let discovery = config.openid_response.unwrap();
        assert_eq!(discovery.jwks_uri, format!("{}/jwks", server.uri()));
        assert!(discovery.token_endpoint.is_empty());
        assert!(discovery.userinfo_endpoint.is_empty());
    }

    #[tokio::test]
    async fn regression_userinfo_modes_require_their_endpoint_at_startup() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": server.uri(),
                "jwks_uri": format!("{}/jwks", server.uri()),
                "token_endpoint": "",
                "userinfo_endpoint": ""
            })))
            .mount(&server)
            .await;
        for authentication_type in [AuthType::JwtWithUserInfo, AuthType::Opaque] {
            let mut config = AuthConfig {
                authentication_type,
                openid_configuration: Some(server.uri()),
                ..AuthConfig::default()
            };
            let error = config.load(&Client::new()).await.unwrap_err();
            assert!(error.contains("userinfo_endpoint"), "{error}");
            assert!(config.openid_response.is_none());
        }
    }

    #[tokio::test]
    async fn invalid_token_cache_ttl_is_clamped_to_token_ttl() {
        let mut config = AuthConfig {
            enabled: false,
            token_ttl: Duration::from_secs(60),
            invalid_token_cache_ttl: Duration::from_secs(3600),
            ..AuthConfig::default()
        };

        config.load(&Client::new()).await.unwrap();

        assert_eq!(config.invalid_token_cache_ttl, Duration::from_secs(60));
    }

    #[tokio::test]
    async fn invalid_token_cache_ttl_is_unchanged_when_within_bound() {
        let mut config = AuthConfig {
            enabled: false,
            token_ttl: Duration::from_secs(1800),
            invalid_token_cache_ttl: Duration::from_secs(60),
            ..AuthConfig::default()
        };

        config.load(&Client::new()).await.unwrap();

        assert_eq!(config.invalid_token_cache_ttl, Duration::from_secs(60));
    }
}
