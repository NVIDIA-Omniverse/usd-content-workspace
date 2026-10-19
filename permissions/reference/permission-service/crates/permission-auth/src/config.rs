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

use crate::claim_rules::{parse_claim_rules, ClaimRules};
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
    /// Required when authentication is enabled with the "jwt" or "jwtuserinfo" authentication type.
    #[arg(
        long,
        env = "JWT_AUDIENCE",
        value_delimiter = ',',
        value_parser = parse_trimmed_non_empty
    )]
    pub jwt_audience: Option<Vec<String>>,

    /// The full set of accepted values for the `iss` claim in JWTs passed in the
    /// Authorization header. When unset or empty, the issuer claim is not
    /// validated. The discovery-document issuer is not added implicitly: operators
    /// must list every issuer they trust (e.g. the IdP's canonical issuer URL,
    /// per-tenant issuers for Microsoft Entra ID multi-tenant deployments, or
    /// both v1 and v2 issuers during a staged migration).
    #[arg(long, env = "JWT_ISSUERS", value_delimiter = ',', value_parser = parse_trimmed_non_empty)]
    pub jwt_issuers: Option<Vec<String>>,

    /// JSON array of rules that claims of every verified token must satisfy, in addition to the
    /// `aud` and `iss` checks. Each rule names a top-level `claim`, an `operator` (`equals`,
    /// `notEquals`, `in`, `notIn`, `contains`, `containsAny`, `containsAll`, or `exists`), and a
    /// string, integer, or boolean `value` or `values` list depending on the operator, e.g.
    /// `[{"claim":"appid","operator":"in","values":["web","cli"]}]`. Tokens that fail any rule
    /// are rejected as unauthorized. Invalid rules fail startup.
    #[arg(long, env = "JWT_REQUIRED_CLAIMS", default_value = "[]", value_parser = parse_claim_rules)]
    pub jwt_required_claims: ClaimRules,

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

    /// The JWT claim holding the principal's group memberships, used to
    /// evaluate `principal in Group::"<id>"` policy scopes. The claim must be a
    /// JSON array of group id strings. By default, the "groups" claim is used.
    #[arg(long, env = "PRINCIPAL_GROUPS_CLAIM", default_value = "groups", value_parser = parse_trimmed_non_empty)]
    pub principal_groups_claim: String,

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

/// Trims surrounding whitespace from a comma-delimited list entry and rejects
/// entries that are empty after trimming, so a stray separator or padding in
/// `JWT_AUDIENCE` / `JWT_ISSUERS` (e.g. `"aud1, aud2"`) does not yield blank or
/// space-prefixed values.
fn parse_trimmed_non_empty(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("value must not be empty".to_string());
    }
    Ok(trimmed.to_string())
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
            jwt_required_claims: ClaimRules::default(),
            jwks_ttl: Duration::new(60 * 180, 0),
            jwks_failure_backoff: Duration::from_secs(10),
            jwt_leeway: 60,
            token_cache_size: 2048,
            token_ttl: Duration::new(60 * 30, 0),
            invalid_token_cache_size: 256,
            invalid_token_cache_ttl: Duration::from_secs(60),
            principal_id_claim: "sub".to_string(),
            principal_groups_claim: "groups".to_string(),
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

        if self.authentication_type != AuthType::Opaque && self.jwt_audience.is_none() {
            return Err(
                "JWT audience is not specified. Set JWT_AUDIENCE for the 'jwt' and 'jwtuserinfo' \
                authentication types."
                    .to_string(),
            );
        }

        let openid_configuration = self
            .openid_configuration
            .clone()
            .ok_or("OpenID configuration is not specified.")?;

        self.openid_response = Some(
            load_openid_configuration(&openid_configuration, client)
                .await
                .map_err(|err| format!("Failed to load OpenID configuration: {}", err))?,
        );

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
                    Expected JSON with 'userinfo_endpoint' and 'jwks_uri' fields. \
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
    #[value(name = "jwtuserinfo", alias = "jwt-with-user-info")]
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
    pub token_endpoint: String,

    /// The endpoint from the identity provider for getting detailed user information.
    pub userinfo_endpoint: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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

    #[tokio::test]
    async fn load_requires_jwt_audience_for_jwt_authentication_types() {
        let server = start_openid_server().await;

        for authentication_type in [AuthType::Jwt, AuthType::JwtWithUserInfo] {
            let mut config = AuthConfig {
                authentication_type: authentication_type.clone(),
                openid_configuration: Some(openid_configuration_uri(&server)),
                ..AuthConfig::default()
            };

            let err = config.load(&Client::new()).await.unwrap_err();

            assert!(
                err.contains("JWT_AUDIENCE"),
                "{authentication_type:?}: {err}"
            );
            assert!(config.openid_response.is_none());
        }
    }

    #[tokio::test]
    async fn load_does_not_require_jwt_audience_for_opaque_authentication_type() {
        let server = start_openid_server().await;
        let mut config = AuthConfig {
            authentication_type: AuthType::Opaque,
            openid_configuration: Some(openid_configuration_uri(&server)),
            ..AuthConfig::default()
        };

        config.load(&Client::new()).await.unwrap();

        assert!(config.openid_response.is_some());
    }

    #[tokio::test]
    async fn load_accepts_jwt_audience_for_jwt_authentication_type() {
        let server = start_openid_server().await;
        let mut config = AuthConfig {
            authentication_type: AuthType::Jwt,
            openid_configuration: Some(openid_configuration_uri(&server)),
            jwt_audience: Some(vec!["permission-service".to_string()]),
            ..AuthConfig::default()
        };

        config.load(&Client::new()).await.unwrap();

        assert!(config.openid_response.is_some());
    }

    async fn start_openid_server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(OPENID_CONFIGURATION_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": server.uri(),
                "jwks_uri": format!("{}/jwks", server.uri()),
                "token_endpoint": format!("{}/token", server.uri()),
                "userinfo_endpoint": format!("{}/userinfo", server.uri()),
            })))
            .mount(&server)
            .await;
        server
    }

    const OPENID_CONFIGURATION_PATH: &str = "/.well-known/openid-configuration";

    fn openid_configuration_uri(server: &MockServer) -> String {
        format!("{}{}", server.uri(), OPENID_CONFIGURATION_PATH)
    }

    #[test]
    fn auth_type_argument_values_match_serialized_names() {
        for authentication_type in AuthType::value_variants() {
            let possible_value = authentication_type.to_possible_value().unwrap();
            let serialized = serde_json::to_value(authentication_type).unwrap();

            assert_eq!(Some(possible_value.get_name()), serialized.as_str());
            assert_eq!(
                <AuthType as ValueEnum>::from_str(possible_value.get_name(), false).as_ref(),
                Ok(authentication_type)
            );
        }
    }

    #[test]
    fn auth_type_accepts_legacy_jwt_with_user_info_value() {
        assert_eq!(
            <AuthType as ValueEnum>::from_str("jwt-with-user-info", false),
            Ok(AuthType::JwtWithUserInfo)
        );
    }
}
